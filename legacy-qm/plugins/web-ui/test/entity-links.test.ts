import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { findEntityIds, linkEntities, type EntityRef } from "../src/entity-links.ts";

const KNOWN = new Map<string, EntityRef>([
  ["FR-CFG-01", { id: "FR-CFG-01", kind: "requirements", path: "10-intent/srs.md" }],
  ["NFR-06", { id: "NFR-06", kind: "requirements", path: "10-intent/srs.md" }],
  ["Q-10", { id: "Q-10", kind: "questions", path: "00-frame/questions/Q-10.md" }],
  ["ADR-0014", { id: "ADR-0014", kind: "decisions", path: "30-design/decisions/accepted/0014.md" }],
  ["Article 4", { id: "Article 4", kind: "articles", path: "00-frame/constitution.md", anchor: "article-4-ddd" }],
]);

function body(html: string): Element {
  const dom = new JSDOM(`<div id=root>${html}</div>`);
  return dom.window.document.getElementById("root")!;
}

test("идентификаторы разных наборов опознаются, длинный не съедается коротким", () => {
  assert.deepEqual(findEntityIds("см. NFR-06 и FR-CFG-01"), ["NFR-06", "FR-CFG-01"]);
  assert.deepEqual(findEntityIds("Article 4 правит M0-T11a"), ["Article 4", "M0-T11a"]);
});

test("известный идентификатор становится ссылкой, неизвестный остаётся текстом", () => {
  const root = body("<p>Смотри FR-CFG-01 и FR-XXX-99.</p>");
  assert.equal(linkEntities(root, KNOWN), 1);
  assert.equal(root.querySelectorAll(".entity-ref").length, 1);
  assert.equal(root.querySelector(".entity-ref")!.textContent, "FR-CFG-01");
  assert.match(root.textContent!, /FR-XXX-99/, "неизвестный идентификатор не потерян");
});

test("внутри кода и уже существующей ссылки ничего не переписывается", () => {
  const root = body('<p><code>FR-CFG-01</code> и <a href="x">Q-10</a> и <pre>ADR-0014</pre></p>');
  assert.equal(linkEntities(root, KNOWN), 0, "код и ссылка — не место для подстановки");
  assert.equal(root.querySelectorAll(".entity-ref").length, 0);
});

test("текст вокруг идентификатора сохраняется целиком", () => {
  const root = body("<p>до Q-10 после</p>");
  linkEntities(root, KNOWN);
  assert.equal(root.textContent, "до Q-10 после", "ни один символ не пропал");
});

test("ссылка несёт путь и якорь, чтобы открыть нужное место", () => {
  const root = body("<p>Article 4 требует</p>");
  linkEntities(root, KNOWN);
  const ref = root.querySelector(".entity-ref") as HTMLElement;
  assert.equal(ref.dataset["path"], "00-frame/constitution.md");
  assert.equal(ref.dataset["anchor"], "article-4-ddd");
});

test("ссылка на сам открытый документ не ставится: некуда вести", () => {
  const root = body("<p>Этот вопрос Q-10 про себя</p>");
  assert.equal(linkEntities(root, KNOWN, "00-frame/questions/Q-10.md"), 0);
});

test("повторный проход не вкладывает ссылку в ссылку", () => {
  const root = body("<p>Q-10 и ещё раз Q-10</p>");
  assert.equal(linkEntities(root, KNOWN), 2);
  assert.equal(linkEntities(root, KNOWN), 0, "второй проход ничего не добавляет");
  assert.equal(root.querySelectorAll(".entity-ref .entity-ref").length, 0);
});
