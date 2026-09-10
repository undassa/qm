import test from "node:test";
import assert from "node:assert/strict";
import { claimOf, namesOf, projectDocumentPlan, type PlanCell } from "../src/projects/document-plan-projection.ts";

function table(blockOrd: number, rows: string[][]): PlanCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: PlanCell[] = [
  ...table(26, [
    ["Документ", "Что содержит", "Состояние"],
    ["constitution.md", "инварианты", "есть, 15 статей, v2.2"],
    ["document-plan.md", "этот документ", "этот файл"],
  ]),
  ...table(38, [
    ["Документ", "Что содержит", "Состояние"],
    ["sdd.md (IEEE 1016)", "восемь видов", "есть; вопросов нет"],
    ["test-cases.md", "проверки", "есть: 357 TC-nn со ссылкой на FR-nn"],
    ["guides/user-guide.md, admin-guide.md", "руководства", "нет: нужны к первому выпуску"],
  ]),
  // Соседняя таблица про стандарт — не перечень документов.
  ...table(52, [
    ["Элемент 29148", "У нас", ""],
    ["9.3", "brs.md", ""],
  ]),
];

const SOURCE = {
  path: "00-frame/document-plan.md",
  cells: CELLS,
  levelOfBlock: (ord: number) => (ord === 26 ? "Уровень 0 — рамка" : "Уровень 3 — проверка"),
};

test("перечень опознаётся по заголовку, соседние таблицы не примешиваются", () => {
  const { entries } = projectDocumentPlan(SOURCE);
  assert.deepEqual(
    entries.map((e) => e.name),
    ["constitution.md", "document-plan.md", "sdd.md", "test-cases.md", "guides/user-guide.md", "admin-guide.md"],
  );
});

test("скобочное уточнение — часть подписи, а не имени файла", () => {
  assert.deepEqual(namesOf("sdd.md (IEEE 1016)"), ["sdd.md"]);
  assert.deepEqual(namesOf("operations/runbook.md, ADR-0013"), ["operations/runbook.md"], "ADR — не путь");
});

test("состояние читается по первому слову, «этот файл» — особый случай", () => {
  assert.equal(claimOf("есть, 15 статей"), "present");
  assert.equal(claimOf("нет: писать нечего"), "absent");
  assert.equal(claimOf("этот файл"), "self");
  assert.equal(claimOf("непонятно"), "unknown", "неизвестное не считается ни наличием, ни отсутствием");
});

test("уровень берётся из раздела над таблицей", () => {
  const { entries } = projectDocumentPlan(SOURCE);
  assert.equal(entries.find((e) => e.name === "constitution.md")!.level, "Уровень 0 — рамка");
  assert.equal(entries.find((e) => e.name === "test-cases.md")!.level, "Уровень 3 — проверка");
});

test("числа снимаются только с объявленных предметов, а не с любого числа рядом", () => {
  const { counts } = projectDocumentPlan(SOURCE);
  assert.deepEqual(counts, [
    { name: "constitution.md", subject: "articles", claimed: 15 },
    { name: "test-cases.md", subject: "checks", claimed: 357 },
  ]);
  assert.ok(
    !counts.some((c) => c.claimed === 2),
    "«v2.2» — не количество; закрытый список предметов и нужен, чтобы не выдумывать за документ",
  );
});

test("число о покрытии переписью не считается", () => {
  const cells: PlanCell[] = table(26, [
    ["Документ", "Что содержит", "Состояние"],
    ["strs.md", "требования сторон: 79 ST-nn, каждое с источником", "есть; вопросов нет"],
    [
      "api/openapi.yaml",
      "контракт HTTP",
      "есть: 112 путей, 146 операций; телом названо 227 требований из 271, границей — 44",
    ],
  ]);
  const { counts } = projectDocumentPlan({ path: "x.md", cells, levelOfBlock: () => "" });
  assert.deepEqual(counts, [{ name: "strs.md", subject: "needs", claimed: 79 }]);
});
