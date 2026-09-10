import assert from "node:assert/strict";
import test from "node:test";
import { discussionPrompt, findingTone, jointState, rankFindings } from "../src/project-chain.ts";

test("зелёным помечается только проверенное", () => {
  assert.equal(jointState({ from: "a", to: "b", checked: true, broken: 0, what: "" }), "clean");
  assert.equal(jointState({ from: "a", to: "b", checked: true, broken: 3, what: "" }), "broken");
  assert.equal(
    jointState({ from: "a", to: "b", checked: false, broken: 0, what: "" }),
    "unchecked",
    "стык без данных о связи зелёным быть не может: иначе зелёный перестанет что-то значить",
  );
});

test("непроверяемый стык не выдаёт себя за целый даже при нуле разрывов", () => {
  const noData = { from: "checks", to: "tasks", checked: false, broken: 0, what: "" };
  assert.notEqual(jointState(noData), "clean");
});

test("противоречие весит больше, чем заявленное и забытое", () => {
  const ranked = rankFindings([
    { kind: "terms", item: "термин не употреблён", count: 300 },
    { kind: "decisions", item: "ссылка на удалённое требование", count: 3 },
  ]);
  assert.deepEqual(
    ranked.map((f) => f.kind),
    ["decisions", "terms"],
    "число не перебивает род находки: спор документов разбирают раньше",
  );
});

test("внутри одного веса больше число идёт первым", () => {
  const ranked = rankFindings([
    { kind: "questions", item: "a", count: 14 },
    { kind: "questions", item: "b", count: 45 },
  ]);
  assert.deepEqual(
    ranked.map((f) => f.count),
    [45, 14],
  );
});

test("сверка с кодом отмечена своим цветом, а не общим", () => {
  assert.equal(findingTone("dbTables"), "code");
  assert.equal(findingTone("decisions"), "hard");
  assert.equal(findingTone("terms"), "soft");
});

test("просьба к разговору называет находку точно, а не «разберись тут»", () => {
  const prompt = discussionPrompt({ kind: "decisions", item: "ссылается на удалённое требование", count: 31 });
  assert.match(prompt, /ссылается на удалённое требование/, "название находки названо дословно");
  assert.match(prompt, /31/, "величина названа: от неё зависит, как разбирать");
  assert.match(prompt, /decisions/, "набор назван, иначе в чате придётся объяснять заново");
});
