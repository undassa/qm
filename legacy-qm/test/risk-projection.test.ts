import test from "node:test";
import assert from "node:assert/strict";
import { projectRisks, splitImpact, type RiskCell } from "../src/projects/risk-projection.ts";

function table(blockOrd: number, rows: string[][]): RiskCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: RiskCell[] = [
  ...table(16, [
    ["В / Вер", "высокое / среднее"],
    ["Владелец", "владелец продукта"],
    ["Признак", "заявка отклонена, либо ответа нет к подаче"],
    ["Источник", "concept.md §8"],
  ]),
  ...table(26, [
    ["В / Вер", "среднее / низкая"],
    ["Признак", "реплика отстаёт"],
  ]),
  ...table(82, [
    ["", "Риск", "Где записан"],
    ["R-08", "Асинхронная оболочка дороже", "ADR-0024"],
    ["R-09", "Храповик мутантов дорог", "ADR-9999"],
  ]),
  ...table(90, [
    ["", "Риск", "Чем закрыт"],
    ["R-14", "Старый риск", "снят решением владельца"],
  ]),
];

const SECTIONS: Record<number, string> = {
  16: "R-01 · Apple не выдаёт разрешение",
  26: "R-07 · Реплика теряет транзакции",
};

const SOURCE = {
  path: "10-intent/risks.md",
  cells: CELLS,
  sectionOfBlock: (ord: number) => SECTIONS[ord] ?? "Принятые",
};

test("влияние и вероятность стоят в шапке одной ячейкой и разделяются", () => {
  assert.deepEqual(splitImpact("высокое / среднее"), { impact: "высокое", probability: "среднее" });
  assert.deepEqual(splitImpact("низкая"), { impact: "низкая", probability: "" }, "половина — не повод выдумать вторую");
});

test("риск собирается из раздела и своей таблицы", () => {
  const r01 = projectRisks(SOURCE).find((r) => r.id === "R-01")!;
  assert.equal(r01.title, "Apple не выдаёт разрешение");
  assert.equal(r01.state, "open");
  assert.equal(r01.impact, "высокое");
  assert.equal(r01.probability, "среднее");
  assert.equal(r01.owner, "владелец продукта");
  assert.equal(r01.source, "concept.md §8");
});

test("принятые и закрытые открытыми не считаются", () => {
  const risks = projectRisks(SOURCE);
  assert.equal(risks.find((r) => r.id === "R-08")!.state, "accepted");
  assert.equal(risks.find((r) => r.id === "R-14")!.state, "closed");
  assert.deepEqual(
    risks.filter((r) => r.state === "open").map((r) => r.id),
    ["R-01", "R-07"],
  );
});

test("риски идут по номеру, а не по порядку встречи в документе", () => {
  assert.deepEqual(
    projectRisks(SOURCE).map((r) => r.id),
    ["R-01", "R-07", "R-08", "R-09", "R-14"],
  );
});

test("открытый риск без владельца виден: смотреть некому", () => {
  const r07 = projectRisks(SOURCE).find((r) => r.id === "R-07")!;
  assert.equal(r07.owner, "", "владельца в таблице нет, и выдумывать его нельзя");
  assert.equal(r07.trigger, "реплика отстаёт");
});
