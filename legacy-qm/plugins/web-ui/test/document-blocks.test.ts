import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { roleOf, splitBlocks, withAwaiting, type DocumentBlock } from "../src/document-blocks.ts";

test("роль раздела зависит от вида документа, а не от одного лишь заголовка", () => {
  assert.equal(roleOf("question", "Ответ"), "answer");
  assert.equal(roleOf("decision", "Решение"), "decision");
  assert.equal(roleOf("question", "Решение"), "decision");
  assert.equal(roleOf("decision", "Отвергнутые варианты"), "rejected");
  assert.equal(roleOf("story", "Критерий приёмки"), "criteria");
  assert.equal(roleOf("screen", "Открытые вопросы"), "attention");
  assert.equal(roleOf("run", "Оставлено открытым"), "attention");
});

test("регистр и язык не мешают: у решений есть английские разделы", () => {
  assert.equal(roleOf("decision", "CONTEXT"), "lead");
  assert.equal(roleOf("decision", "Decision"), "decision");
  assert.equal(roleOf("decision", "  Последствия  "), "consequence");
});

test("незнакомый раздел остаётся обычным блоком, а не пропадает", () => {
  assert.equal(roleOf("question", "Всякое разное"), "plain");
  assert.equal(roleOf("нет-такого-вида", "Ответ"), "plain");
});

test("общее правило ловит «Историю» с продолжением в заголовке", () => {
  assert.equal(roleOf("story", "История изменений документа"), "history");
});

function fragment(html: string): ParentNode {
  return new JSDOM(`<body>${html}</body>`).window.document.body;
}

test("разметка режется по заголовкам, и текст до первого заголовка не теряется", () => {
  const blocks = splitBlocks(
    "question",
    fragment("<p>вступление</p><h2>Вопрос</h2><p>суть</p><h2>Ответ</h2><p>да</p>"),
  );
  assert.deepEqual(
    blocks.map((b) => [b.title, b.role]),
    [
      ["", "plain"],
      ["Вопрос", "lead"],
      ["Ответ", "answer"],
    ],
  );
  assert.match(blocks[0]!.html, /вступление/, "текст без заголовка остаётся отдельным блоком");
  assert.match(blocks[2]!.html, /да/);
});

test("решётка внутри кода заголовком не считается", () => {
  const blocks = splitBlocks("plain", fragment("<h2>Раздел</h2><pre><code># не заголовок</code></pre>"));
  assert.equal(blocks.length, 1, "делим отрисованный DOM, а не текст");
  assert.match(blocks[0]!.html, /не заголовок/);
});

test("вопросу без «Ответа» блок ответа добавляется — это и есть открытый вопрос", () => {
  const blocks: DocumentBlock[] = [
    { title: "Вопрос", level: 2, role: "lead", html: "<p>?</p>" },
    { title: "История изменений", level: 2, role: "history", html: "<p>…</p>" },
  ];
  const withStub = withAwaiting("question", blocks);
  assert.deepEqual(
    withStub.map((b) => b.role),
    ["lead", "awaiting", "history"],
    "заглушка встаёт перед историей, а не в конец",
  );

  const answered: DocumentBlock[] = [{ title: "Ответ", level: 2, role: "answer", html: "<p>да</p>" }];
  assert.equal(withAwaiting("question", answered).length, 1, "у отвеченного ничего не добавляется");
  assert.equal(withAwaiting("decision", blocks).length, 2, "к другим видам правило не применяется");
});
