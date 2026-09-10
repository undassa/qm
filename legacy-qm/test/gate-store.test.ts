import test, { before } from "node:test";
import assert from "node:assert/strict";
import { createPostgresGateStore, refuseQuery } from "../src/projects/gate-store.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres gate tests";
const PROJECT = "gate-store-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  await p.query("DELETE FROM project_gates WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.query("DELETE FROM project_requirements WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("запрос, способный что-то изменить, отвергается до выполнения", () => {
  for (const query of [
    "delete from project_requirements",
    "select 1; drop table project_gates",
    "update project_gates set state='passed'",
    "insert into project_gates values (1)",
    "with x as (delete from project_checks returning 1) select * from x",
    "truncate project_documents",
  ]) {
    assert.notEqual(refuseQuery(query), null, query);
  }
  assert.equal(refuseQuery("select id from project_requirements where project_id=$1"), null);
});

test("гейт объявляется полным: у запросного есть запрос, у подписного — владелец", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  await store.declare(PROJECT, {
    phase: "G1",
    item: "требование объявлено",
    kind: "query",
    query: "select id from project_requirements where project_id=$1",
    owner: null,
  });
  await assert.rejects(
    () =>
      store.declare(PROJECT, {
        phase: "G1",
        item: "без запроса",
        kind: "query",
        query: null,
        owner: null,
      }),
    /violates check constraint/,
    "запросный гейт без запроса не заводится — это ограничение схемы, а не проверка в коде",
  );
});

test("гейт есть запрос: пусто — пройден, строки — нарушения", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  const pg = (await import("pg")).default;
  const pool = new pg.Pool({ connectionString: URL! });
  t.after(() => pool.end());

  await store.declare(PROJECT, {
    phase: "G1",
    item: "нет требования без текста",
    kind: "query",
    query: "select id from project_requirements where project_id=$1 and text = ''",
    owner: null,
  });

  const passed = await store.run(PROJECT, "G1");
  assert.equal(passed.find((o) => o.item === "нет требования без текста")?.state, "passed");

  await pool.query(
    `INSERT INTO project_requirements(project_id, id, kind, area, text, path, satisfied)
     VALUES ($1,'FR-X-01','FR','X','','srs.md',false) ON CONFLICT DO NOTHING`,
    [PROJECT],
  );
  const failed = await store.run(PROJECT, "G1");
  const outcome = failed.find((o) => o.item === "нет требования без текста")!;
  assert.equal(outcome.state, "failed");
  assert.equal(outcome.violations, 1);
  assert.match(outcome.detail, /FR-X-01/, "исход называет нарушителя, а не только число");
});

test("сломанный запрос отвергается, а не выглядит пройденным", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  await store.declare(PROJECT, {
    phase: "G9",
    item: "битый",
    kind: "query",
    query: "select nosuchcolumn from project_requirements where project_id=$1",
    owner: null,
  });
  const [outcome] = await store.run(PROJECT, "G9");
  assert.equal(outcome!.state, "refused", "ошибка запроса не должна читаться как отсутствие нарушений");
  assert.match(outcome!.detail, /nosuchcolumn/);
});

test("подписной гейт машина не закрывает", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  await store.declare(PROJECT, {
    phase: "G5",
    item: "приёмку подписал владелец",
    kind: "signed",
    query: null,
    owner: "ann",
  });
  const [outcome] = await store.run(PROJECT, "G5");
  assert.equal(outcome!.state, "unknown", "подпись человека прогоном не заменяется");
});

test("подписной гейт закрывается человеком, и запись помнит кто", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!, { now: () => 555 });
  t.after(() => store.close());
  await store.declare(PROJECT, { phase: "GX", item: "рамка принята", kind: "signed", query: null, owner: "владелец" });

  const before = (await store.list(PROJECT, "GX"))[0]!;
  assert.equal(before.state, "unknown", "без подписи гейт не пройден");

  const signed = await store.sign(PROJECT, "GX", "рамка принята", "mail@undassa.com");
  assert.equal(signed?.state, "passed");
  assert.equal(signed?.signedBy, "mail@undassa.com");
  assert.equal(signed?.signedAt, 555);

  // прогон не трогает подписное: машина за человека не подписывает
  await store.run(PROJECT, "GX");
  assert.equal((await store.list(PROJECT, "GX"))[0]!.state, "passed");

  const off = await store.unsign(PROJECT, "GX", "рамка принята");
  assert.equal(off?.state, "unknown");
  assert.equal(off?.signedBy, null);
});

test("подпись переживает переобъявление гейта", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  const gate = { phase: "GY", item: "проект полон", kind: "signed" as const, query: null, owner: "владелец" };
  await store.declare(PROJECT, gate);
  await store.sign(PROJECT, "GY", "проект полон", "аня");

  await store.declare(PROJECT, gate);
  const after = (await store.list(PROJECT, "GY"))[0]!;
  assert.equal(after.state, "passed", "переобъявление не стирает подпись человека");
  assert.equal(after.signedBy, "аня");
});

test("запросный гейт подписью не закрывается", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  await store.declare(PROJECT, {
    phase: "GZ",
    item: "машинный",
    kind: "query",
    query: "select id from project_requirements where project_id=$1 and text=''",
    owner: null,
  });
  assert.equal(await store.sign(PROJECT, "GZ", "машинный", "аня"), null, "подписать можно только подписное");
});

test("подпись держится, пока прочитанное не изменилось", { skip: pgSkip }, async (t) => {
  const store = createPostgresGateStore(URL!);
  t.after(() => store.close());
  const pg = (await import("pg")).default;
  const pool = new pg.Pool({ connectionString: URL! });
  t.after(() => pool.end());

  // фаза G3 читает 40-proof/ — кладём туда документ этого проекта
  await pool.query(
    `INSERT INTO project_documents(project_id, path, content, content_hash, bytes, revision, updated_at, updated_by)
     VALUES ($1,'40-proof/tdd.md','порядок','hash-1',7,1,1,'тест')
     ON CONFLICT (project_id, path) DO UPDATE SET content_hash='hash-1'`,
    [PROJECT],
  );
  await store.declare(PROJECT, {
    phase: "G3",
    item: "тесты спроектированы",
    kind: "signed",
    query: null,
    owner: "владелец",
  });
  await store.sign(PROJECT, "G3", "тесты спроектированы", "аня");

  const fresh = (await store.list(PROJECT, "G3")).find((g) => g.item === "тесты спроектированы")!;
  assert.equal(fresh.state, "passed");
  assert.deepEqual(fresh.staleDocuments, [], "ничего не менялось — подпись держит");

  await pool.query(
    `UPDATE project_documents SET content_hash='hash-2' WHERE project_id=$1 AND path='40-proof/tdd.md'`,
    [PROJECT],
  );
  const after = (await store.list(PROJECT, "G3")).find((g) => g.item === "тесты спроектированы")!;
  assert.deepEqual(
    after.staleDocuments,
    ["40-proof/tdd.md"],
    "правка прочитанного переоткрывает гейт и называет документ",
  );

  await pool.query(`DELETE FROM project_documents WHERE project_id=$1 AND path='40-proof/tdd.md'`, [PROJECT]);
  const gone = (await store.list(PROJECT, "G3")).find((g) => g.item === "тесты спроектированы")!;
  assert.deepEqual(gone.staleDocuments, ["40-proof/tdd.md"], "исчезнувший документ — тоже расхождение");
});
