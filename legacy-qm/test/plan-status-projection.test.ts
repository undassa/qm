import test from "node:test";
import assert from "node:assert/strict";
import { projectPlanStatus, type StatusCell } from "../src/projects/plan-status-projection.ts";

function table(blockOrd: number, rows: string[][]): StatusCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: StatusCell[] = [
  ...table(10, [
    ["Задача", "Состояние", "Коммиты"],
    ["M0-T1", "закрыта", "99e187a"],
    ["M0-T11a", "закрыта", "34a1c1c, 77aabcb"],
    ["M0-T12", "закрыта", "—"],
  ]),
  ...table(14, [
    ["Задача", "Состояние", "Коммиты"],
    ["M1-T7", "не начата", ""],
    ["M1-T8", "в работе", ""],
  ]),
  // Соседняя таблица про другое — перечнем задач не является.
  ...table(60, [
    ["Этап", "Готовность"],
    ["M0", "100%"],
  ]),
];

const SOURCE = {
  path: "50-plan/v1/status.md",
  cells: CELLS,
  milestoneOfBlock: (ord: number) => (ord === 10 ? "M0" : "M1"),
};

test("доска опознаётся по заголовку, соседняя таблица не примешивается", () => {
  assert.deepEqual(
    projectPlanStatus(SOURCE).map((r) => r.taskId),
    ["M0-T1", "M0-T11a", "M0-T12", "M1-T7", "M1-T8"],
  );
});

test("буквенный хвост задачи сохраняется, этап берётся из раздела", () => {
  const rows = projectPlanStatus(SOURCE);
  const t11a = rows.find((r) => r.taskId === "M0-T11a")!;
  assert.equal(t11a.milestone, "M0");
  assert.equal(rows.find((r) => r.taskId === "M1-T7")!.milestone, "M1");
});

test("состояние читается тем же словарём, что и поле задачи", () => {
  const rows = projectPlanStatus(SOURCE);
  assert.equal(rows.find((r) => r.taskId === "M0-T1")!.state, "closed");
  assert.equal(rows.find((r) => r.taskId === "M1-T7")!.state, "not_started");
  assert.equal(rows.find((r) => r.taskId === "M1-T8")!.state, "claimed");
});

test("коммит берётся первый из перечисленных, прочерк коммитом не считается", () => {
  const rows = projectPlanStatus(SOURCE);
  assert.equal(rows.find((r) => r.taskId === "M0-T11a")!.commit, "34a1c1c");
  assert.equal(rows.find((r) => r.taskId === "M0-T12")!.commit, "", "«—» — это отсутствие, а не хеш");
});
