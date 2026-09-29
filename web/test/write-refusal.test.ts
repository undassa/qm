import { test } from "node:test";
import assert from "node:assert/strict";
import { write } from "../src/api.ts";

// Отказ двери по существу доезжает до пульта конвертом 502, и в `message`
// лежит ответ двери целиком — JSON. Показывать надо причину, а не JSON.
test("отказ двери показывает причину, а не сырой JSON", async () => {
  const door = { status: "taken", id: "Q-198", why: "вопрос Q-198 уже объявлен" };
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ error: "upstream_error", message: JSON.stringify(door) }), { status: 502 });
  const r = await write("p", "question-add", { id: "Q-198" });
  assert.equal(r.ok, false);
  assert.equal(r.why, "вопрос Q-198 уже объявлен");
});

test("слова в message остаются словами", async () => {
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ error: "upstream_error", message: "база не ответила" }), { status: 502 });
  const r = await write("p", "question-add", { id: "Q-1" });
  assert.equal(r.why, "база не ответила");
});
