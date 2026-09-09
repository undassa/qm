import assert from "node:assert/strict";
import test from "node:test";
import { escapeHtml, inline, safeHref, toBlocks } from "../src/markdown.ts";

test("разметка документа не может стать разметкой страницы", () => {
  assert.equal(inline("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");
  assert.equal(escapeHtml('он сказал "да" & ушёл'), "он сказал &quot;да&quot; &amp; ушёл");
});

test("жирный, курсив и код размечаются", () => {
  assert.equal(inline("**строго** и *мягко*"), "<b>строго</b> и <i>мягко</i>");
  assert.equal(inline("колонка `workspace_id` обязательна"), "колонка <code>workspace_id</code> обязательна");
});

test("внутри кода разметки нет", () => {
  assert.equal(inline("`**не жирный**`"), "<code>**не жирный**</code>", "звёздочки внутри кода — часть кода");
});

test("число из прозы остаётся числом рядом с кодом", () => {
  // Метка подстановки когда-то стояла на пробелах вокруг числа, и «статья 1»
  // подменялась вынутым куском кода. Числа в статьях рядовые: статьи, версии.
  assert.equal(
    inline("`a` и `b`, см. статью 1 и пункт 0 в главе 12"),
    "<code>a</code> и <code>b</code>, см. статью 1 и пункт 0 в главе 12",
  );
});

test("метку подстановки нельзя принести из документа", () => {
  const nul = String.fromCharCode(0);
  assert.equal(inline(`${nul}0${nul} и \`код\``), "0 и <code>код</code>", "NUL из текста выброшен, а не принят за метку");
});

test("ссылка со схемой javascript ссылкой не становится", () => {
  assert.equal(safeHref("https://example.com/x"), "https://example.com/x");
  assert.equal(safeHref("javascript:alert(1)"), null);
  assert.equal(safeHref("../../etc/passwd"), null, "выход вверх по дереву ссылкой не считается");
  assert.equal(safeHref("30-design/sdd.md"), null, "путь корпуса приложением не отдаётся — это не ссылка");
  assert.equal(inline("[тык](javascript:alert)"), "тык", "неразрешённый адрес рисуется именем, без сырой разметки");
});

test("путь корпуса рисуется именем, а не сырым markdown", () => {
  // Иначе в размеченном тексте остаётся исходник — ровно то, от чего уходили.
  assert.equal(inline("правит ([ADR-0138](../30-design/decisions/accepted/0138-the-role.md))"), "правит (ADR-0138)");
  assert.equal(inline("см. [`acceptance.md`](../40-proof/acceptance.md)"), "см. <code>acceptance.md</code>");
});

test("обычная ссылка размечается и открывается безопасно", () => {
  const html = inline("см. [sdd](https://example.com/sdd)");
  assert.match(html, /<a href="https:\/\/example\.com\/sdd" target="_blank" rel="noreferrer noopener">sdd<\/a>/);
});

test("блоки делятся пустой строкой, цитата опознаётся", () => {
  const blocks = toBlocks("Первый абзац\nс переносом.\n\n> цитата\n> в две строки\n\nВторой.");
  assert.deepEqual(
    blocks.map((b) => b.kind),
    ["para", "quote", "para"],
  );
  assert.equal(blocks[0]!.kind === "para" ? blocks[0]!.html : "", "Первый абзац с переносом.");
  assert.equal(blocks[1]!.kind === "quote" ? blocks[1]!.html : "", "цитата в две строки");
});

test("отступ в четыре пробела — блок кода, а не абзац", () => {
  const blocks = toBlocks("текст\n\n    id text PK\n    account_id");
  assert.equal(blocks[1]!.kind, "code");
  assert.equal(blocks[1]!.kind === "code" ? blocks[1]!.text : "", "id text PK\naccount_id");
});

test("заголовок становится заголовком, а не решёткой в тексте", () => {
  const blocks = toBlocks("## Вопрос и контекст\n\nтекст\n\n### Решение\n\nещё");
  assert.deepEqual(
    blocks.map((b) => b.kind),
    ["heading", "para", "heading", "para"],
  );
  const first = blocks[0]!;
  assert.equal(first.kind === "heading" ? first.level : 0, 2);
  assert.equal(first.kind === "heading" ? first.html : "", "Вопрос и контекст");
});

test("заголовок отделяется от абзаца и без пустой строки", () => {
  // Иначе он прилипает к соседу и остаётся видимой решёткой.
  const blocks = toBlocks("## Решение\n—");
  assert.deepEqual(
    blocks.map((b) => b.kind),
    ["heading", "para"],
  );
});

test("таблица опознаётся разделителем, а не палками", () => {
  const blocks = toBlocks("| Дата | Событие |\n|---|---|\n| 2026-09-02 | заведён |");
  assert.equal(blocks[0]!.kind, "table");
  const t = blocks[0]!;
  if (t.kind !== "table") throw new Error("не таблица");
  assert.deepEqual(t.head, ["Дата", "Событие"]);
  assert.deepEqual(t.rows, [["2026-09-02", "заведён"]]);
});

test("палки в прозе таблицей не делают", () => {
  const blocks = toBlocks("выбор такой: | либо одно, | либо другое\nи это не таблица");
  assert.equal(blocks[0]!.kind, "para");
});

test("в ячейках таблицы разметка работает", () => {
  const blocks = toBlocks("| Имя | Что |\n|---|---|\n| `FR-01` | **держит** |");
  const t = blocks[0]!;
  if (t.kind !== "table") throw new Error("не таблица");
  assert.deepEqual(t.rows, [["<code>FR-01</code>", "<b>держит</b>"]]);
});

test("черта отбивает шапку, а не читается абзацем", () => {
  const blocks = toBlocks("шапка\n\n---\n\n1. Введение");
  assert.deepEqual(blocks.map((b) => b.kind), ["para", "rule", "para"]);
});

test("дефисы внутри строки чертой не становятся", () => {
  // «-- » в прозе и разделитель таблицы приходят теми же знаками; черта — это
  // строка целиком, и только она.
  const blocks = toBlocks("что-то --- ещё");
  assert.deepEqual(blocks.map((b) => b.kind), ["para"]);
});
