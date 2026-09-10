import test, { before } from "node:test";
import assert from "node:assert/strict";
import { createPostgresQuestionStore, projectQuestions, stateOfQuestion } from "../src/projects/question-projection.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres question tests";
const PROJECT = "question-projection-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  await p.query("DELETE FROM project_questions WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("состояние берётся из объявленного поля, а «решено» не то же, что «закрыт»", () => {
  assert.equal(stateOfQuestion("закрыт 2026-08-26").state, "closed");
  assert.equal(stateOfQuestion("решено 2026-08-26 · исполняет проход гейтов").state, "decided");
  assert.equal(stateOfQuestion("открыт").state, "open");
});

test("неизвестное состояние считается открытым, а не закрытым", () => {
  assert.equal(stateOfQuestion(undefined).state, "open", "нет поля — вопрос не закрыт молча");
  assert.equal(stateOfQuestion("").state, "open");
  assert.equal(stateOfQuestion("непонятно что").state, "open");
});

function fieldsFor(path: string): [string, string][] {
  if (path.endsWith("Q-1.md")) {
    return [
      ["Состояние", "закрыт 2026-08-26"],
      ["Заведён", "2026-08-23"],
      ["Закрыт", "2026-08-26 · исполнением"],
    ];
  }
  if (path.endsWith("Q-2.md")) return [["Состояние", "решено 2026-09-01 · исполняет проход"]];
  return [["Состояние", "открыт"]];
}

const SOURCE = {
  paths: [
    "00-frame/questions/Q-1.md",
    "00-frame/questions/Q-10.md",
    "00-frame/questions/Q-2.md",
    "00-frame/questions/README.md",
    "00-frame/questions/.archive/log.md",
  ],
  titleOf: (p: string) => `Заголовок ${p}`,
  fieldsOf: (p: string) => new Map(fieldsFor(p)),
  sectionTitlesOf: (p: string) => (p.endsWith("Q-1.md") ? ["Вопрос", "Ответ"] : ["Вопрос"]),
};

test("проекция берёт только пронумерованные вопросы и сортирует по номеру", () => {
  const questions = projectQuestions(SOURCE);
  assert.deepEqual(
    questions.map((q) => q.id),
    ["Q-1", "Q-2", "Q-10"],
    "README и архив вопросами не считаются, а Q-10 идёт после Q-2",
  );
});

test("даты вынимаются из поля, даже когда рядом стоит пояснение", () => {
  const [first] = projectQuestions(SOURCE);
  assert.equal(first!.openedAt, "2026-08-23");
  assert.equal(first!.closedAt, "2026-08-26", "«2026-08-26 · исполнением» даёт дату, а не всю строку");
  assert.equal(first!.hasAnswer, true);
});

test("решённый без раздела «Ответ» находится: решение есть, записи нет", { skip: pgSkip }, async (t) => {
  const store = createPostgresQuestionStore(URL!);
  t.after(() => store.close());
  await store.replace(PROJECT, projectQuestions(SOURCE));

  assert.deepEqual(await store.counts(PROJECT), { total: 3, open: 1, decided: 1, closed: 1 });
  assert.deepEqual(
    (await store.list(PROJECT, "open")).map((q) => q.id),
    ["Q-10"],
  );
  assert.deepEqual(
    (await store.decidedWithoutAnswer(PROJECT)).map((q) => q.id),
    ["Q-2"],
    "Q-1 закрыт и ответ записан, Q-2 решён и ответа в документе нет",
  );
});
