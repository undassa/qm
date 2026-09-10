import test, { before } from "node:test";
import assert from "node:assert/strict";
import {
  createMemoryRepositoryStore,
  createPostgresRepositoryStore,
  providerOf,
  repositoryHost,
  validateRepository,
  type RepositoryStore,
} from "../src/projects/repository-store.ts";

const URL_PG = process.env.DATABASE_URL;
const pgSkip = URL_PG ? false : "set DATABASE_URL (a Postgres) to run the Postgres repository tests";
const PROJECT = "repository-store-test";

before(async () => {
  if (!URL_PG) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL_PG });
  await p.query("DELETE FROM project_repositories WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("адресом считается только то, из чего виден хост", () => {
  assert.equal(repositoryHost("https://github.com/undassa/mh.git"), "github.com");
  assert.equal(repositoryHost("git@github.com:undassa/mh.git"), "github.com");
  assert.equal(repositoryHost("ssh://git@gitlab.example.com/team/app.git"), "gitlab.example.com");
  for (const bad of ["/opt/src/mh", "mh", "http://github.com/a/b", "javascript:alert(1)", ""]) {
    assert.equal(repositoryHost(bad), null, bad);
  }
});

test("провайдер выводится из хоста — от него зависит, как спрашивать состояние PR", () => {
  assert.equal(providerOf("https://github.com/undassa/mh.git"), "github");
  assert.equal(providerOf("git@github.com:undassa/mh.git"), "github");
  assert.equal(providerOf("https://gitlab.com/team/app.git"), "gitlab");
  assert.equal(providerOf("https://gitlab.corp.example/team/app.git"), "gitlab");
  assert.equal(providerOf("git@git.example.org:team/app.git"), "other", "самостоятельный git — не ошибка");
});

test("отказ называет поле", () => {
  assert.match(validateRepository({ name: " ", url: "https://github.com/a/b" })!, /имя/);
  assert.match(validateRepository({ name: "main", url: "не адрес" })!, /адрес/);
  assert.match(validateRepository({ name: "main", url: "https://github.com/a/b", baseBranch: "две ветки" })!, /пробел/);
  assert.equal(validateRepository({ name: "main", url: "https://github.com/a/b" }), null);
});

async function suite(label: string, make: () => RepositoryStore, skip: string | false) {
  test(`${label}: первый репозиторий становится основным сам`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const first = await store.create(PROJECT, { name: "код", url: "https://github.com/undassa/mh.git" });
    assert.equal(first.status, "ok");
    if (first.status !== "ok") return;
    assert.equal(first.repository.isDefault, true, "иначе «куда писать» осталось бы без ответа");
    assert.equal(first.repository.provider, "github", "провайдер выведен из адреса");
    assert.equal(first.repository.baseBranch, "main");
    assert.equal((await store.primary(PROJECT))?.id, first.repository.id);
  });

  test(`${label}: основной ровно один`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const a = await store.create(PROJECT, { name: "первый", url: "https://github.com/a/b.git" });
    const b = await store.create(PROJECT, {
      name: "второй",
      url: "https://github.com/c/d.git",
      isDefault: true,
    });
    assert.equal(a.status, "ok");
    assert.equal(b.status, "ok");
    const all = await store.list(PROJECT);
    assert.deepEqual(
      all.filter((r) => r.isDefault).map((r) => r.name),
      ["второй"],
      "назначение нового основного снимает прежний",
    );
  });

  test(`${label}: имя в проекте однозначно`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    await store.create(PROJECT, { name: "Ядро", url: "https://github.com/a/core.git" });
    const again = await store.create(PROJECT, { name: "ядро", url: "https://github.com/a/other.git" });
    assert.equal(again.status, "duplicate");
  });

  test(`${label}: удаление основного передаёт роль оставшемуся`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const main = await store.create(PROJECT, { name: "главный", url: "https://github.com/a/main.git" });
    await store.create(PROJECT, { name: "второй-1", url: "https://github.com/a/second.git" });
    assert.equal(main.status, "ok");
    if (main.status !== "ok") return;

    assert.equal(await store.remove(PROJECT, main.repository.id), true);
    const left = await store.primary(PROJECT);
    assert.equal(left?.isDefault, true, "проект не остаётся без основного, пока есть репозитории");
  });

  test(`${label}: чужой проект не видит и не правит`, { skip }, async (t) => {
    const store = make();
    t.after(() => store.close?.());
    const mine = await store.create(PROJECT, { name: "свой-репо", url: "https://github.com/a/mine.git" });
    assert.equal(mine.status, "ok");
    if (mine.status !== "ok") return;
    assert.equal(await store.get("другой", mine.repository.id), null);
    assert.equal((await store.update("другой", mine.repository.id, { name: "чужой" })).status, "not_found");
    assert.equal(await store.remove("другой", mine.repository.id), false);
  });
}

await suite("память", () => createMemoryRepositoryStore(() => 1), false);
await suite("postgres", () => createPostgresRepositoryStore(URL_PG!), pgSkip);
