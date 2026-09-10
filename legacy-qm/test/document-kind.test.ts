import test from "node:test";
import assert from "node:assert/strict";
import { documentId, KIND_ORDER, kindOfPath, DOCUMENT_KINDS } from "../src/projects/document-kind.ts";

test("вид выводится из пути, а не из содержимого", () => {
  assert.equal(kindOfPath("00-frame/questions/Q-328.md"), "question");
  assert.equal(kindOfPath("30-design/decisions/accepted/0082-tests-first.md"), "decision");
  assert.equal(kindOfPath("10-intent/use-cases/escalation-chains/US-ESC-11.md"), "story");
  assert.equal(kindOfPath("10-intent/functional/escalation-chains.md"), "feature");
  assert.equal(kindOfPath("20-surface/configure/escalation.md"), "screen");
  assert.equal(kindOfPath("50-plan/v1/M1/M1-T10.md"), "task");
  assert.equal(kindOfPath("50-plan/v1/V3/V3-T17.md"), "task", "трек проверок — тоже задачи");
  assert.equal(kindOfPath("60-runs/v1/M0/M0-T5.md"), "run");
});

test("частное правило сильнее общего: вопрос не становится рамкой", () => {
  assert.equal(kindOfPath("00-frame/questions/Q-1.md"), "question");
  assert.equal(kindOfPath("00-frame/constitution.md"), "frame");
  assert.equal(kindOfPath("30-design/decisions/x.md"), "decision");
  assert.equal(kindOfPath("30-design/sdd.md"), "design");
  assert.equal(kindOfPath("10-intent/use-cases/x/US-1.md"), "story");
  assert.equal(kindOfPath("10-intent/srs.md"), "intent");
});

test("этап и задача различаются уровнем пути, а не именем", () => {
  assert.equal(kindOfPath("50-plan/v1/M1.md"), "milestone");
  assert.equal(kindOfPath("50-plan/v1/M1/M1-T1.md"), "task");
});

test("незнакомый путь получает «прочее», а не угаданный вид", () => {
  assert.equal(kindOfPath("README.md"), "other");
  assert.equal(kindOfPath("50-plan/v1/order.md"), "other");
});

test("идентификатор берётся из имени файла, у решений — из номера", () => {
  assert.equal(documentId("00-frame/questions/Q-328.md"), "Q-328");
  assert.equal(documentId("50-plan/v1/M1/M1-T10.md"), "M1-T10");
  assert.equal(documentId("10-intent/use-cases/x/US-ESC-11.md"), "US-ESC-11");
  assert.equal(documentId("30-design/decisions/accepted/0082-tests-first.md"), "ADR-0082");
  assert.equal(documentId("00-frame/constitution.md"), null, "у документа без идентификатора его нет");
});

test("порядок разделов перечисляет каждый вид ровно один раз", () => {
  assert.equal(KIND_ORDER.length, DOCUMENT_KINDS.length);
  assert.equal(new Set(KIND_ORDER).size, KIND_ORDER.length, "повторов нет");
  for (const kind of DOCUMENT_KINDS) assert.ok(KIND_ORDER.includes(kind), kind);
});
