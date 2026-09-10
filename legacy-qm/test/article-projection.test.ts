import test, { before } from "node:test";
import assert from "node:assert/strict";
import { articleAnchor, createPostgresArticleStore, projectArticles } from "../src/projects/article-projection.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres article tests";
const PROJECT = "article-projection-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  for (const table of ["project_article_references", "project_articles"]) {
    await p.query(`DELETE FROM ${table} WHERE project_id=$1`, [PROJECT]).catch(() => undefined);
  }
  await p.end();
});

const CONSTITUTION = {
  path: "00-frame/constitution.md",
  content: [
    "# Myack Constitution",
    "",
    "вступление, не статья",
    "",
    "## Article 1 — Tenant isolation is mandatory",
    "",
    "тело первой статьи",
    "",
    "## Article 2 — Optimistic concurrency, always",
    "",
    "тело второй",
    "",
    "## Приложение",
    "",
    "это не статья и в тело второй попасть не должно",
    "",
  ].join("\n"),
};

test("статьи выделяются из заголовков, вступление статьёй не становится", () => {
  const p = projectArticles({ constitution: CONSTITUTION, documents: [] });
  assert.deepEqual(
    p.articles.map((a) => [a.number, a.title]),
    [
      [1, "Tenant isolation is mandatory"],
      [2, "Optimistic concurrency, always"],
    ],
  );
  assert.equal(p.articles[0]!.body, "тело первой статьи");
});

test("раздел не про статью закрывает предыдущую, а не дописывается в её тело", () => {
  const p = projectArticles({ constitution: CONSTITUTION, documents: [] });
  assert.equal(p.articles[1]!.body, "тело второй", "«Приложение» в тело второй статьи не попало");
});

test("ссылка на статью считается один раз на документ", () => {
  const p = projectArticles({
    constitution: CONSTITUTION,
    documents: [
      { path: "40-proof/tdd.md", content: "Article 1 и снова Article 1 и ещё Article 2" },
      { path: "10-intent/srs.md", content: "ничего не цитирует" },
    ],
  });
  assert.deepEqual(p.references, [
    { path: "40-proof/tdd.md", number: 1 },
    { path: "40-proof/tdd.md", number: 2 },
  ]);
});

test("якорь статьи пригоден для ссылки", () => {
  assert.equal(
    articleAnchor(7, "Postgres is the single source of truth"),
    "article-7-postgres-is-the-single-source-of-truth",
  );
  assert.equal(articleAnchor(1, "Тенант: изоляция!"), "article-1-тенант-изоляция");
});

test("ссылка на несуществующую статью находится — ради этого статьи и стали таблицей", { skip: pgSkip }, async (t) => {
  const store = createPostgresArticleStore(URL!);
  t.after(() => store.close());
  await store.replace(PROJECT, {
    articles: projectArticles({ constitution: CONSTITUTION, documents: [] }).articles,
    references: [
      { path: "40-proof/tdd.md", number: 1 },
      { path: "40-proof/tdd.md", number: 99 },
    ],
  });

  const articles = await store.list(PROJECT);
  assert.equal(articles.length, 2);
  assert.equal(articles[0]!.citedBy, 1, "первая статья процитирована один раз");
  assert.equal(articles[1]!.citedBy, 0);

  assert.deepEqual(await store.danglingReferences(PROJECT), [{ path: "40-proof/tdd.md", number: 99 }]);
});
