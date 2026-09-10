import assert from "node:assert/strict";
import test from "node:test";
import { classifyAnswer, isIdentifier, resolvePath, sectionOf } from "../src/projects/question-answer.ts";

const docs = new Set(["30-design/decisions/accepted/0027-fingerprint.md", "10-intent/srs.md"]);
const names = new Set(["FR-RT-01", "TC-ORG-28", "ADR-0087"]);
const classify = (body: string | null, dir = "00-frame/questions") =>
  classifyAnswer({ body, dir, hasDocument: (p) => docs.has(p), hasName: (n) => names.has(n) });

test("ответ ссылкой на живой документ — то, чего требует реестр", () => {
  const v = classify("[ADR-0027](../../30-design/decisions/accepted/0027-fingerprint.md)");
  assert.equal(v.kind, "link");
  assert.deepEqual(v.broken, []);
});

test("адрес, который не разрешается, хуже слова: он обещает проверяемость", () => {
  const v = classify("[auth.rs](../../40-proof/auth.rs.md)");
  assert.equal(v.kind, "dead-link");
  assert.deepEqual(v.broken, ["40-proof/auth.rs.md"]);
});

test("имя без ссылки — место найдётся, но искать придётся", () => {
  assert.equal(classify("несёт `FR-RT-01`, проверка `TC-ORG-28`").kind, "name");
});

test("имя, которого в наборе нет", () => {
  const v = classify("несёт `FR-RT-09`");
  assert.equal(v.kind, "unknown-name");
  assert.deepEqual(v.broken, ["FR-RT-09"]);
});

test("слово состояния ответом не считается", () => {
  assert.equal(classify("исполнено (проход комментариев)").kind, "words");
  assert.equal(classify("решено").kind, "words");
});

test("раздела «Ответ» нет — это отдельный случай, а не пустой ответ", () => {
  assert.equal(classify(null).kind, "missing");
  assert.equal(classify("   ").kind, "missing");
});

test("внешний адрес местом в корпусе не является", () => {
  // Он никем не ведётся и корпусом не проверяется — по мерке реестра это не ответ.
  assert.equal(classify("см. [RFC 9110](https://example.com/rfc9110)").kind, "words");
});

test("идентификатор набора кончается числом", () => {
  assert.equal(isIdentifier("FR-RT-09"), true);
  assert.equal(isIdentifier("ADR-0087"), true);
  assert.equal(isIdentifier("TC-ORG-28"), true);
  assert.equal(isIdentifier("Idempotency-Key"), false, "заголовок HTTP именем набора не является");
  assert.equal(isIdentifier("X-Yabeda-User-Id"), false);
  assert.equal(isIdentifier("TC-nn"), false, "образец в прозе — не ссылка на проверку");
});

test("заголовок HTTP в ответе не делает вопрос сломанным", () => {
  // Q-306 отвечает через `Idempotency-Key`: имени набора там нет вовсе.
  assert.equal(classify("заголовок `Idempotency-Key` и `X-Yabeda-User-Id`").kind, "words");
});

test("относительный адрес считается от каталога вопроса", () => {
  assert.equal(resolvePath("00-frame/questions", "../../10-intent/srs.md"), "10-intent/srs.md");
  assert.equal(resolvePath("00-frame/questions", "./Q-12.md"), "00-frame/questions/Q-12.md");
});

test("раздел вынимается по заголовку любого уровня и кончается следующим", () => {
  const doc = "# Q-1 — что-то\n\n## Вопрос\n\nтекст\n\n## Ответ\n\nвот он\n\n## История\n\nдальше";
  assert.equal(sectionOf(doc, "Ответ"), "вот он");
  assert.equal(sectionOf(doc, "Ничего"), null);
});
