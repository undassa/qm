import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { anchorOf, parseDocument, renderDocument, stripMarkup } from "../src/projects/document-structure.ts";

const SAMPLES: Record<string, string> = {
  "заголовки, проза и пустые строки": "# Заголовок\n\nПроза первая.\n\n## Раздел\n\nПроза вторая.\n",
  "таблица с полями":
    "# M0-T1 · Задача\n\n| | |\n|---|---|\n| **Этап** | [`M0`](../M0.md) |\n| **Размер** | 1–2 часа |\n",
  "список с полями": "# Q-1 — Вопрос\n\n- **Состояние:** решено 2026-08-26\n- **Заведён:** 2026-08-25\n",
  "код с решёткой внутри": "# Пример\n\n```bash\n# это не заголовок\necho ok\n```\n\nПосле кода.\n",
  "вложенные заголовки": "# A\n## B\n### C\ntext\n## D\ntext\n",
  "без завершающего перевода строки": "# Хвост\n\nбез перевода в конце",
  "только пустые строки": "\n\n\n",
  "таблица без обрамляющих труб": "# T\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
  "html-блок": "# H\n\n<details>\n<summary>тык</summary>\n</details>\n",
  "подряд две таблицы": "# T\n\n| a |\n|---|\n| 1 |\n\n| b |\n|---|\n| 2 |\n",
};

test("сборка из блоков совпадает с исходником побайтово", () => {
  for (const [name, content] of Object.entries(SAMPLES)) {
    const { blocks } = parseDocument(content);
    assert.equal(renderDocument(blocks), content, name);
  }
});

test("блоки покрывают файл без зазоров и нахлёстов", () => {
  for (const [name, content] of Object.entries(SAMPLES)) {
    const { blocks } = parseDocument(content);
    assert.deepEqual(
      blocks.map((b) => b.ord),
      blocks.map((_, i) => i),
      `${name}: порядковые номера идут подряд`,
    );
    assert.equal(
      blocks.reduce((n, b) => n + b.raw.length, 0),
      content.length,
      `${name}: длина сходится`,
    );
  }
});

test("решётка внутри кода не становится заголовком", () => {
  const { blocks, sections } = parseDocument(SAMPLES["код с решёткой внутри"]!);
  assert.equal(sections.length, 1);
  assert.equal(sections[0]!.title, "Пример");
  assert.equal(blocks.filter((b) => b.kind === "code").length, 1);
});

test("секции складываются в дерево и знают свой диапазон блоков", () => {
  const { sections } = parseDocument(SAMPLES["вложенные заголовки"]!);
  assert.deepEqual(
    sections.map((s) => [s.title, s.level, s.parentOrd]),
    [
      ["A", 1, null],
      ["B", 2, 0],
      ["C", 3, 1],
      ["D", 2, 0],
    ],
  );
  const b = sections.find((s) => s.title === "B")!;
  const d = sections.find((s) => s.title === "D")!;
  assert.equal(b.lastBlock, d.ord - 1, "раздел кончается там, где начинается следующий того же уровня");
});

test("ячейка хранится дважды: как написана и голым значением", () => {
  const { cells } = parseDocument(SAMPLES["таблица с полями"]!);
  const written = cells.find((c) => c.raw.includes("M0"))!;
  assert.equal(written.raw, " [`M0`](../M0.md) ");
  assert.equal(written.value, "M0");
});

test("поля берутся из обеих форм — строки таблицы и пункта списка", () => {
  const row = parseDocument(SAMPLES["таблица с полями"]!).fields;
  assert.deepEqual(
    row.map((f) => [f.name, f.shape, f.value]),
    [
      ["Этап", "row", "M0"],
      ["Размер", "row", "1–2 часа"],
    ],
  );
  const bullet = parseDocument(SAMPLES["список с полями"]!).fields;
  assert.deepEqual(
    bullet.map((f) => [f.name, f.shape, f.value]),
    [
      ["Состояние", "bullet", "решено 2026-08-26"],
      ["Заведён", "bullet", "2026-08-25"],
    ],
  );
});

test("ссылка разбирается на путь и якорь", () => {
  const { links } = parseDocument("# T\n\nсм. [требование](../10-intent/srs.md#3-требования) и [голую](a.md)\n");
  assert.deepEqual(
    links.map((l) => [l.text, l.targetPath, l.targetAnchor]),
    [
      ["требование", "../10-intent/srs.md", "3-требования"],
      ["голую", "a.md", ""],
    ],
  );
});

test("якорь и голый текст снимают разметку", () => {
  assert.equal(stripMarkup("**жирный** и `код` и [ссылка](x.md)"), "жирный и код и ссылка");
  assert.equal(anchorOf("## 3. Требования — и `код`"), "3-требования-и-код");
});

const VAULT = process.env.DOCUMENT_FIXTURE_ROOT;
const vaultSkip = VAULT && existsSync(VAULT) ? false : "set DOCUMENT_FIXTURE_ROOT to a document set to round-trip it";

function walk(root: string, base = root, out: string[] = []): string[] {
  for (const entry of readdirSync(base, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name.startsWith(".")) continue;
    const full = join(base, entry.name);
    if (entry.isDirectory()) walk(root, full, out);
    else if (entry.name.endsWith(".md")) out.push(full);
  }
  return out;
}

test("настоящий набор собирается обратно побайтово, весь", { skip: vaultSkip }, () => {
  const files = walk(VAULT!);
  assert.ok(files.length > 0, "в наборе есть документы");
  const broken: string[] = [];
  let bytes = 0;
  for (const file of files) {
    if (statSync(file).size > 4 * 1024 * 1024) continue;
    const content = readFileSync(file, "utf8");
    bytes += Buffer.byteLength(content, "utf8");
    if (renderDocument(parseDocument(content).blocks) !== content) broken.push(file.slice(VAULT!.length + 1));
  }
  assert.deepEqual(broken, [], `${broken.length} из ${files.length} не собрались обратно`);
  assert.ok(bytes > 0);
});

test("перенесённый пункт читается целиком, а не обрывается на конце строки", () => {
  const { fields } = parseDocument(
    [
      "# Заголовок",
      "",
      "- **Status**: Accepted — but its **package LAYOUT (package-per-bounded-context) is",
      "  superseded by [ADR-0016](x.md)** (global layers).",
      "- **Date**: 2026-07-27",
      "",
    ].join("\n"),
  );
  const status = fields.find((f) => f.name === "Status")!;
  assert.match(status.value, /global layers/, "продолжение строки — часть того же значения");
  assert.doesNotMatch(status.value, /\*\*/, "незакрытой разметки в значении не остаётся");
  assert.equal(fields.find((f) => f.name === "Date")?.value, "2026-07-27", "следующий пункт не съеден");
});

test("вложенный подпункт продолжением не считается", () => {
  const { fields } = parseDocument(
    ["# З", "", "- **Решают**: владелец", "  - подпункт остаётся своим пунктом", ""].join("\n"),
  );
  assert.equal(fields.find((f) => f.name === "Решают")?.value, "владелец");
});

test("экранированная черта — часть значения, а не граница ячейки", () => {
  const { cells } = parseDocument(
    ["# З", "", "| Имя | Замер |", "|---|---|", "| `adr/` | считаем `ls a/*.md \\| wc -l` и всё |", ""].join("\n"),
  );
  // Строка 0 — шапка, строка 1 — данные: разделитель `|---|` в ячейки не идёт.
  const row = cells.filter((c) => c.row === 1);
  assert.equal(row.length, 2, "строка на две колонки лежит двумя ячейками, а не тремя");
  assert.match(row[1]!.raw, /ls a\/\*\.md \| wc -l/, "экранирование снято, черта осталась в значении");
});

test("черта внутри кодовой вставки границей ячейки не является", () => {
  const { cells } = parseDocument(["# З", "", "| A | `x | y` |", "|---|---|", "| 1 | 2 |", ""].join("\n"));
  assert.equal(cells.filter((c) => c.row === 0).length, 2);
});

test("метка ссылки не переходит на другую строку", () => {
  // Открывающая скобка без закрывающей — запись полуоткрытого интервала — не должна
  // дотягиваться до `]` настоящей ссылки строкой ниже и съедать её вместе с прозой.
  const { links } = parseDocument(
    ["# З", "", "Период `[since, until)` — начало входит.", "Записано", "[ADR-0162](d/0162-x.md) и всё.", ""].join("\n"),
  );
  assert.equal(links.length, 1, "ссылка ровно одна");
  assert.equal(links[0]!.text, "ADR-0162", "метка своя, а не три строки прозы");
  assert.equal(links[0]!.targetPath, "d/0162-x.md");
});
