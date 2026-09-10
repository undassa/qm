import test, { before } from "node:test";
import assert from "node:assert/strict";
import {
  createMemoryAgentStore,
  createPostgresAgentStore,
  validateAgent,
  type AgentStore,
} from "../src/projects/agent-store.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres agent tests";
const PROJECT = "agent-store-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  await p.query("DELETE FROM project_agents WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("отказ называет поле, иначе в форме нечего исправлять", () => {
  assert.match(validateAgent({ name: "  ", harness: "claude" })!, /имя/);
  assert.match(validateAgent({ name: "a", harness: "нет-такого" as never })!, /харнес/);
  assert.match(validateAgent({ name: "a", harness: "claude", sandbox: "луна" as never })!, /песочница/);
  assert.match(validateAgent({ name: "a", harness: "claude", concurrency: 0 })!, /параллелизм/);
  assert.match(validateAgent({ name: "a", harness: "claude", concurrency: 99 })!, /параллелизм/);
  assert.equal(validateAgent({ name: "Сборщик", harness: "claude", concurrency: 3 }), null);
});

async function suite(name: string, make: () => AgentStore, skip: string | false) {
  test(`${name}: агент заводится с наследованием модели и песочницы`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const created = await store.create(PROJECT, { name: "Сборщик", harness: "claude", role: "пишет код" });
    assert.equal(created.status, "ok");
    if (created.status !== "ok") return;
    assert.equal(created.agent.model, null, "модель не задана — значит наследуется от проекта");
    assert.equal(created.agent.sandbox, null);
    assert.equal(created.agent.concurrency, 1, "по умолчанию агент ведёт одну задачу");
    assert.equal(created.agent.enabled, true);
  });

  test(`${name}: имя внутри проекта однозначно`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.create(PROJECT, { name: "Тестировщик", harness: "codex" });
    const again = await store.create(PROJECT, { name: "тестировщик", harness: "codex" });
    assert.equal(again.status, "duplicate", "регистр не делает имя другим — назначение по имени станет неоднозначным");
  });

  test(`${name}: правка не даёт занять чужое имя, но своё оставить можно`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const a = await store.create(PROJECT, { name: "Ревьюер", harness: "claude" });
    const b = await store.create(PROJECT, { name: "Мерджер", harness: "claude" });
    assert.equal(a.status, "ok");
    assert.equal(b.status, "ok");
    if (a.status !== "ok" || b.status !== "ok") return;

    assert.equal((await store.update(PROJECT, b.agent.id, { name: "Ревьюер" })).status, "duplicate");
    const same = await store.update(PROJECT, b.agent.id, { name: "Мерджер", concurrency: 4 });
    assert.equal(same.status, "ok", "своё имя занятым не считается");
    if (same.status === "ok") assert.equal(same.agent.concurrency, 4);
  });

  test(`${name}: чужой проект не видит и не правит агента`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const mine = await store.create(PROJECT, { name: "Свой", harness: "pi" });
    assert.equal(mine.status, "ok");
    if (mine.status !== "ok") return;
    assert.equal(await store.get("другой-проект", mine.agent.id), null);
    assert.equal((await store.update("другой-проект", mine.agent.id, { name: "Взлом" })).status, "not_found");
    assert.equal(await store.remove("другой-проект", mine.agent.id), false);
    assert.notEqual(await store.get(PROJECT, mine.agent.id), null, "агент на месте");
  });

  test(`${name}: удаление возвращает, было ли что удалять`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const created = await store.create(PROJECT, { name: "Временный", harness: "mock" });
    assert.equal(created.status, "ok");
    if (created.status !== "ok") return;
    assert.equal(await store.remove(PROJECT, created.agent.id), true);
    assert.equal(await store.remove(PROJECT, created.agent.id), false);
  });
}

await suite("память", () => createMemoryAgentStore(() => 1), false);
await suite("postgres", () => createPostgresAgentStore(URL!), pgSkip);
