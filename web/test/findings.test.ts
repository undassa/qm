import assert from "node:assert/strict";
import test from "node:test";
import { keysOf, plural, rank, rowKey, tone } from "../src/findings.ts";

test("противоречие разбирают раньше, чем заявленное и забытое", () => {
  const ranked = rank([
    { kind: "terms", item: "термин не употреблён", count: 300 },
    { kind: "decisions", item: "ссылка на удалённое требование", count: 3 },
  ]);
  assert.deepEqual(ranked.map((f) => f.kind), ["decisions", "terms"]);
});

test("сверка с кодом отмечена своим цветом", () => {
  assert.equal(tone("dbTables"), "code");
  assert.equal(tone("decisions"), "hard");
  assert.equal(tone("plan"), "soft");
});

test("строка опознаётся по первому подходящему полю", () => {
  assert.equal(rowKey({ decisionId: "ADR-0053", target: "FR-RB-01" }), "ADR-0053");
  assert.equal(rowKey({ id: "Q-217", title: "…" }), "Q-217");
  assert.equal(rowKey({ name: "heartbeats", migration: "0009" }), "heartbeats");
  assert.equal(rowKey({ text: "без опознания" }), "", "выдумывать ключ нельзя");
});

test("ключи находки собираются без пустых", () => {
  const keys = keysOf([{ decisionId: "ADR-1" }, { decisionId: "ADR-1" }, { text: "нет ключа" }]);
  assert.deepEqual([...keys], ["ADR-1"], "повтор не удваивает, безымянное не попадает");
});

test("числительное согласуется", () => {
  assert.equal(plural(1, "запись", "записи", "записей"), "запись");
  assert.equal(plural(3, "запись", "записи", "записей"), "записи");
  assert.equal(plural(11, "запись", "записи", "записей"), "записей");
  assert.equal(plural(31, "запись", "записи", "записей"), "запись");
});
