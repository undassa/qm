import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ENTITY_KINDS, ENTITY_META, ENTITY_REPLACES_KIND } from "../src/api/routes/project-entities.ts";
import { KIND_ORDER } from "../src/projects/document-kind.ts";

/**
 * Вид находки — это вид сущности: по нему сайдбар решает, у какой строки зажечь
 * счётчик. Опечатка здесь не падает, а тихо гасит находку, поэтому проверяем текстом.
 */
test("каждая находка помечена настоящим видом сущности", () => {
  const source = readFileSync(new URL("../src/api/routes/project-entities.ts", import.meta.url), "utf8");
  const findings = source.slice(source.indexOf("findings: ["), source.indexOf("].filter((f) => f.count > 0)"));
  const kinds = [...findings.matchAll(/kind:\s*"([^"]+)"/g)].map((m) => m[1]!);
  assert.ok(kinds.length >= 10, "находки перечислены прямо в маршруте");
  const unknown = kinds.filter((kind) => !(ENTITY_KINDS as readonly string[]).includes(kind));
  assert.deepEqual(unknown, [], "такой сущности нет — находка не найдёт свою строку в сайдбаре");
});

test("вид сущности описан и не вытесняет несуществующий вид документов", () => {
  for (const kind of ENTITY_KINDS) {
    assert.ok(ENTITY_META[kind]?.title, `${kind}: нет заголовка`);
    assert.ok(ENTITY_META[kind]?.source, `${kind}: не сказано, откуда берётся`);
  }
  for (const [entity, kind] of Object.entries(ENTITY_REPLACES_KIND)) {
    assert.ok(KIND_ORDER.includes(kind as never), `${entity}: вытесняет несуществующий вид «${kind}»`);
  }
});
