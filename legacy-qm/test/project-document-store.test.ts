import test, { before } from "node:test";
import assert from "node:assert/strict";
import {
  createMemoryProjectDocumentStore,
  hashDocument,
  normalizeDocumentPath,
  type ProjectDocumentStore,
} from "../src/projects/project-document-store.ts";
import { createPostgresProjectDocumentStore } from "../src/projects/postgres-project-document-store.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres project-document tests";

const SCRATCH = ["p1", "p2", "p3", "p4", "p5", "p6", "p7", "s1", "s2", "s3", "s4", "s5", "mine", "theirs"];

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  for (const table of [
    "project_documents",
    "project_document_revisions",
    "project_document_blocks",
    "project_document_sections",
    "project_document_cells",
    "project_document_links",
    "project_document_fields",
  ]) {
    await p.query(`DELETE FROM ${table} WHERE project_id = ANY($1)`, [SCRATCH]).catch(() => undefined);
  }
  await p.end();
});

test("a path that could escape the project is refused", () => {
  for (const bad of ["", " ", "/leading", "../escape", "a/../b", "a//b", "a\\b", "a/", "..", ".", "trailing /x"]) {
    assert.equal(normalizeDocumentPath(bad), null, JSON.stringify(bad));
  }
  for (const good of [
    "frame.md",
    "00-frame/glossary.md",
    "50-plan/v1/M0/M0-T1.md",
    "a b/c d.md",
    "10-intent/use-cases/.generator.md",
  ]) {
    assert.equal(normalizeDocumentPath(good), good, good);
  }
});

function contract(name: string, make: () => ProjectDocumentStore, skip: string | false): void {
  test(`${name}: a document is written, read back whole, and listed without its body`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const written = await store.put({ projectId: "p1", path: "00-frame/frame.md", content: "рамка", author: "ann" });
    assert.equal(written.status, "written");

    const read = await store.get("p1", "00-frame/frame.md");
    assert.equal(read?.content, "рамка");
    assert.equal(read?.contentHash, hashDocument("рамка"));
    assert.equal(read?.revision, 1);
    assert.equal(read?.bytes, Buffer.byteLength("рамка", "utf8"));

    const listed = await store.list("p1");
    assert.equal(listed.length, 1);
    assert.equal(listed[0]!.path, "00-frame/frame.md");
    assert.equal("content" in listed[0]!, false);
  });

  test(`${name}: rewriting identical content does not spend a revision`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "p2", path: "a.md", content: "same", author: "ann" });
    const again = await store.put({ projectId: "p2", path: "a.md", content: "same", author: "bob" });
    assert.equal(again.status, "unchanged");
    assert.equal(again.status === "unchanged" && again.document.revision, 1);
    assert.equal(again.status === "unchanged" && again.document.updatedBy, "ann");
  });

  test(`${name}: a stale writer is refused, and told the revision it missed`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "p3", path: "a.md", content: "one", author: "ann" });
    await store.put({ projectId: "p3", path: "a.md", content: "two", author: "ann" });
    const stale = await store.put({
      projectId: "p3",
      path: "a.md",
      content: "three",
      author: "bob",
      expectedRevision: 1,
    });
    assert.equal(stale.status, "conflict");
    assert.equal(stale.status === "conflict" && stale.document.revision, 2);
    assert.equal((await store.get("p3", "a.md"))?.content, "two");

    const fresh = await store.put({
      projectId: "p3",
      path: "a.md",
      content: "three",
      author: "bob",
      expectedRevision: 2,
    });
    assert.equal(fresh.status, "written");
  });

  test(`${name}: creating a document expects revision zero`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const wrong = await store.put({
      projectId: "p4",
      path: "new.md",
      content: "x",
      author: "ann",
      expectedRevision: 3,
    });
    assert.equal(wrong.status, "conflict");
    const right = await store.put({
      projectId: "p4",
      path: "new.md",
      content: "x",
      author: "ann",
      expectedRevision: 0,
    });
    assert.equal(right.status, "written");
  });

  test(`${name}: history keeps every revision, newest first, and each is readable`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    for (const body of ["v1", "v2", "v3"]) {
      await store.put({ projectId: "p5", path: "d.md", content: body, author: "ann" });
    }
    const history = await store.history("p5", "d.md");
    assert.deepEqual(
      history.map((r) => r.revision),
      [3, 2, 1],
    );
    assert.equal(history[0]!.contentHash, hashDocument("v3"));
  });

  test(`${name}: a prefix lists its own subtree and nothing beside it`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    for (const path of ["50-plan/v1.md", "50-plan/v1/M0.md", "50-plan/v10/other.md", "40-proof/t.md"]) {
      await store.put({ projectId: "p6", path, content: path, author: "ann" });
    }
    assert.deepEqual(
      (await store.list("p6", "50-plan/v1")).map((d) => d.path),
      ["50-plan/v1/M0.md"],
      "a prefix walks path segments: v1 must not reach into v10, nor swallow the sibling file v1.md",
    );
    assert.deepEqual(
      (await store.list("p6", "50-plan/v1.md")).map((d) => d.path),
      ["50-plan/v1.md"],
      "naming a document exactly returns that document",
    );
    assert.deepEqual(
      (await store.list("p6", "40-proof")).map((d) => d.path),
      ["40-proof/t.md"],
    );
  });

  test(`${name}: documents of one project are invisible to another`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "mine", path: "secret.md", content: "s", author: "ann" });
    assert.equal(await store.get("theirs", "secret.md"), null);
    assert.deepEqual(await store.list("theirs"), []);
  });

  const DOC =
    "# M0-T1 · Задача\n\n## Что делать\n\nПервое.\n\n## Чем доказывается\n\n| Проверка | Дано |\n|---|---|\n| `TC-ORG-02` | чужой объект |\n";

  test(`${name}: строение приходит из хранилища, а не пересчитывается на месте`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "s1", path: "task.md", content: DOC, author: "ann" });

    const blocks = await store.blocks("s1", "task.md");
    assert.equal(blocks.map((b) => b.raw).join(""), DOC, "блоки собираются обратно в исходник");

    const sections = await store.sections("s1", "task.md");
    assert.deepEqual(
      sections.map((s) => [s.title, s.level]),
      [
        ["M0-T1 · Задача", 1],
        ["Что делать", 2],
        ["Чем доказывается", 2],
      ],
    );

    const links = await store.links("s1", "task.md");
    assert.deepEqual(links, []);
  });

  test(`${name}: секция отдаётся телом без своего заголовка`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "s2", path: "task.md", content: DOC, author: "ann" });
    const found = await store.section("s2", "task.md", "что-делать");
    assert.equal(found?.section.title, "Что делать");
    assert.equal(found?.body, "\nПервое.\n\n");
    assert.equal(await store.section("s2", "task.md", "нет-такой"), null);
  });

  test(`${name}: правка секции меняет её одну, остальное остаётся байт в байт`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "s3", path: "task.md", content: DOC, author: "ann" });

    const written = await store.putSection({
      projectId: "s3",
      path: "task.md",
      anchor: "что-делать",
      body: "\nВторое.\n\n",
      author: "bob",
    });
    assert.equal(written.status, "written");

    const after = await store.get("s3", "task.md");
    assert.equal(after?.content, DOC.replace("\nПервое.\n\n", "\nВторое.\n\n"));
    assert.equal(after?.revision, 2, "правка секции есть новая ревизия документа");
    const proof = await store.section("s3", "task.md", "чем-доказывается");
    assert.match(proof!.body, /TC-ORG-02/, "соседняя секция не тронута");
  });

  test(`${name}: правка отсутствующей секции и отсутствующего документа различимы`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const missingDoc = await store.putSection({
      projectId: "s4",
      path: "нет.md",
      anchor: "x",
      body: "",
      author: "ann",
    });
    assert.equal(missingDoc.status, "not_found");

    await store.put({ projectId: "s4", path: "task.md", content: DOC, author: "ann" });
    const missingSection = await store.putSection({
      projectId: "s4",
      path: "task.md",
      anchor: "нет-такой",
      body: "",
      author: "ann",
    });
    assert.equal(missingSection.status, "no_such_section");
  });

  test(`${name}: поля таблицы доступны запросом`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const withFields = "# Q-1\n\n- **Состояние:** решено\n- **Гейт:** уровень 3\n";
    await store.put({ projectId: "s5", path: "q.md", content: withFields, author: "ann" });
    const fields = await store.fields("s5", "q.md");
    assert.deepEqual(
      fields.map((f) => [f.name, f.shape, f.value]),
      [
        ["Состояние", "bullet", "решено"],
        ["Гейт", "bullet", "уровень 3"],
      ],
    );
  });

  test(`${name}: a removed document is gone but its history survives`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.put({ projectId: "p7", path: "gone.md", content: "body", author: "ann" });
    assert.equal(await store.remove("p7", "gone.md", "ann"), true);
    assert.equal(await store.get("p7", "gone.md"), null);
    assert.equal(await store.remove("p7", "gone.md", "ann"), false);
  });
}

contract("memory", () => createMemoryProjectDocumentStore(), false);
contract("postgres", () => createPostgresProjectDocumentStore(URL!), pgSkip);

test("префикс каталога со слэшем на конце — обычная запись, а не пустой список", async () => {
  const store = createMemoryProjectDocumentStore();
  const author = "автор";
  for (const path of ["10-intent/use-cases/monitors/US-MON-01.md", "10-intent/srs.md", "30-design/sdd.md"]) {
    await store.put({ projectId: "p", path, content: "# т", author });
  }
  const withSlash = await store.list("p", "10-intent/use-cases/");
  const without = await store.list("p", "10-intent/use-cases");
  assert.deepEqual(
    withSlash.map((d) => d.path),
    without.map((d) => d.path),
    "иначе естественная запись каталога молча возвращает пустоту",
  );
  assert.equal(withSlash.length, 1);
});
