import test from "node:test";
import assert from "node:assert/strict";
import { duplicateIdentifiers, projectGlossary, type GlossaryCell } from "../src/projects/glossary-projection.ts";

function table(blockOrd: number, rows: string[][]): GlossaryCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: GlossaryCell[] = [
  ...table(92, [
    ["Термин", "Идентификатор", "Что значит"],
    ["Взять на себя", "`ack`", "заявить, что ситуацией занимаешься"],
    ["Передать", "`handover`", "передать ситуацию другому"],
  ]),
  ...table(96, [
    ["Термин", "Идентификатор", "Что значит"],
    ["Принять", "`ack`", "то же самое другими словами"],
    ["Откатить", "rollback", "вернуть предыдущий релиз"],
  ]),
  // Соседняя таблица состояний — не словарь.
  ...table(44, [
    ["state", "Смысл", "Цвет"],
    ["open", "открыта", "красный"],
  ]),
];

const SOURCE = {
  path: "00-frame/glossary.md",
  cells: CELLS,
  areaOfBlock: (ord: number) => (ord === 92 ? "Работа с ситуацией" : "Выпуск"),
};

test("словарь опознаётся по заголовку, таблица состояний не примешивается", () => {
  assert.deepEqual(
    projectGlossary(SOURCE).map((t) => t.id),
    ["ack", "handover", "rollback"],
  );
});

test("обратные кавычки вокруг машинного имени снимаются", () => {
  const ack = projectGlossary(SOURCE).find((t) => t.id === "ack")!;
  assert.equal(ack.term, "Взять на себя");
  assert.equal(ack.area, "Работа с ситуацией");
});

test("повтор машинного имени не создаёт вторую запись, но остаётся видимым", () => {
  assert.equal(projectGlossary(SOURCE).filter((t) => t.id === "ack").length, 1);
  assert.deepEqual(duplicateIdentifiers(SOURCE), [{ id: "ack", terms: ["Взять на себя", "Принять"] }]);
});

test("строка без машинного имени термином не становится", () => {
  const cells = table(92, [
    ["Термин", "Идентификатор", "Что значит"],
    ["Что-то", "—", "без имени"],
  ]);
  assert.deepEqual(projectGlossary({ ...SOURCE, cells }), []);
});

test("таблица переименований дублем не считается: там то же имя во второй колонке", () => {
  const cells: GlossaryCell[] = [
    ...table(92, [
      ["Термин", "Идентификатор", "Что значит"],
      ["Проверка живости", "Heartbeat", "монитор молчания"],
    ]),
    ...table(154, [
      ["Сегодня", "Становится", "Цена"],
      ["watchdog", "Heartbeat", "переименование"],
    ]),
  ];
  const source = { ...SOURCE, cells };
  assert.deepEqual(duplicateIdentifiers(source), [], "переименование — не второе объявление термина");
  assert.deepEqual(
    projectGlossary(source).map((t) => t.id),
    ["Heartbeat"],
  );
});
