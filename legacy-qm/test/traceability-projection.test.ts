import test from "node:test";
import assert from "node:assert/strict";
import { projectTraceability, type TraceabilityRow } from "../src/projects/traceability-projection.ts";

function table(blockOrd: number, rows: string[][]): TraceabilityRow[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

/** В документе несколько таблиц: нужную находим по заголовку, а не по номеру. */
const CELLS: TraceabilityRow[] = [
  ...table(20, [
    ["", "Всего", "Покрыто", "Дыра"],
    ["ST — потребности", "79", "78", "1 — ST-63"],
  ]),
  ...table(43, [
    ["Подсистема", "FR", "TC описано", "названо телом контракта", "в коде (домен)"],
    ["SIT ситуации", "24", "24", "21", "15 (13 красных)"],
    ["PAG мобильный пейдж", "22", "22", "10", "—"],
    ["всего", "271", "271", "227", "120"],
  ]),
];

test("нужная таблица опознаётся по заголовку, соседние не примешиваются", () => {
  const claims = projectTraceability("10-intent/traceability.md", CELLS);
  assert.deepEqual(
    claims.map((c) => c.area),
    ["PAG", "SIT"],
    "таблица сводки с колонкой «Всего» сюда не попадает",
  );
});

test("строка «всего» — итог, а не подсистема", () => {
  assert.ok(
    !projectTraceability("x.md", CELLS).some((c) => c.requirements === 271),
    "иначе итог считался бы девятнадцатой подсистемой",
  );
});

test("код подсистемы отделяется от её названия", () => {
  const sit = projectTraceability("x.md", CELLS).find((c) => c.area === "SIT")!;
  assert.equal(sit.title, "ситуации");
  assert.equal(sit.requirements, 24);
  assert.equal(sit.described, 24);
  assert.equal(sit.inContract, 21);
});

test("непроверяемые колонки сохраняются как есть, а не подменяются нулём", () => {
  const pag = projectTraceability("x.md", CELLS).find((c) => c.area === "PAG")!;
  assert.equal(pag.inContract, 10);
  assert.equal(pag.inCode, "—", "прочерк — это «неизвестно», а не «ноль»");
  const sit = projectTraceability("x.md", CELLS).find((c) => c.area === "SIT")!;
  assert.equal(sit.inCode, "15 (13 красных)", "пометка рядом с числом не теряется");
});
