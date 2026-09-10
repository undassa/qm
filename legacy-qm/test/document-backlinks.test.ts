import test from "node:test";
import assert from "node:assert/strict";
import { resolveLinkTarget } from "../src/projects/document-structure.ts";
import { createMemoryProjectDocumentStore } from "../src/projects/project-document-store.ts";

const PROJECT = "backlinks";

test("ссылка приводится к пути набора, а чужая — отвергается", () => {
  assert.equal(resolveLinkTarget("50-plan/v1/M3/M3-T12.md", "../../../10-intent/ui-spec.md"), "10-intent/ui-spec.md");
  assert.equal(resolveLinkTarget("00-frame/glossary.md", "questions/Q-235.md"), "00-frame/questions/Q-235.md");
  assert.equal(resolveLinkTarget("10-intent/brs.md", "./srs.md"), "10-intent/srs.md");
  assert.equal(resolveLinkTarget("a/b.md", "c.md#якорь"), "a/c.md", "якорь в пути не участвует");
  for (const foreign of ["https://example.com/x.md", "mailto:a@b.c", "#раздел", "/files/x.md", ""]) {
    assert.equal(resolveLinkTarget("a/b.md", foreign), "", foreign);
  }
});

async function seed() {
  const store = createMemoryProjectDocumentStore(() => 1);
  const put = (path: string, content: string) => store.put({ projectId: PROJECT, path, content, author: "тест" });
  await put("10-intent/ui-spec.md", "# Экран\n\nтекст\n");
  await put("50-plan/v1/M3/M3-T12.md", "# Задача\n\nсмотри [`ui-spec.md`](../../../10-intent/ui-spec.md)\n");
  await put("00-frame/notes.md", "# Заметки\n\nи ещё [спека](../10-intent/ui-spec.md)\n");
  return store;
}

test("обратные ссылки находятся, хотя в документе путь написан относительным", async () => {
  const store = await seed();
  const incoming = await store.backlinks(PROJECT, "10-intent/ui-spec.md");
  assert.deepEqual(
    incoming.map((link) => link.path),
    ["00-frame/notes.md", "50-plan/v1/M3/M3-T12.md"],
    "сравнение сырой цели с абсолютным путём находило бы ноль — цель обязана разрешаться",
  );
  assert.equal(incoming[1]!.text, "`ui-spec.md`");
});

test("совпадение одного лишь имени файла обратной ссылкой не считается", async () => {
  const store = await seed();
  await store.put({
    projectId: PROJECT,
    path: "20-surface/other.md",
    content: "# Другое\n\n[та же спека, но иная](./ui-spec.md)\n",
    author: "тест",
  });
  const incoming = await store.backlinks(PROJECT, "10-intent/ui-spec.md");
  assert.deepEqual(
    incoming.map((link) => link.path),
    ["00-frame/notes.md", "50-plan/v1/M3/M3-T12.md"],
    "20-surface/ui-spec.md — другой документ, пусть имя и совпало",
  );
});

test("документ не ссылается сам на себя", async () => {
  const store = await seed();
  await store.put({
    projectId: PROJECT,
    path: "10-intent/self.md",
    content: "# Сам\n\n[я](./self.md)\n",
    author: "тест",
  });
  assert.deepEqual(await store.backlinks(PROJECT, "10-intent/self.md"), []);
});
