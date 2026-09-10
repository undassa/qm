import test from "node:test";
import assert from "node:assert/strict";
import { priorityOf, projectNeeds, type NeedCell } from "../src/projects/need-projection.ts";

function table(blockOrd: number, rows: string[][]): NeedCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: NeedCell[] = [
  ...table(36, [
    ["ID", "Потребность", "Сторона", "Источник", "Пр."],
    ["ST-01", "Читать одну карточку вместо череды", "ДЕЖ", "М1, И-01", "О"],
    ["ST-02", "Не будить на всплеск", "ДЕЖ", "И-03", "Ж"],
  ]),
  ...table(84, [
    ["ID", "Потребность", "Сторона", "Источник", "Пр."],
    ["ST-63", "Видеть нагрузку на команду", "РУК", "И-05, И-06", "П"],
  ]),
  // Та же метка встречается в прозе раздела «Открытые вопросы» — это не объявление.
  ...table(132, [["ST-01 упомянута повторно", "закрыта без нового элемента"]]),
];

const SOURCE = {
  registerPath: "10-intent/strs.md",
  cells: CELLS,
  themeOfBlock: (ord: number) => (ord === 36 ? "A. Шум и сон" : "J. Руководитель"),
  storyPaths: [
    "10-intent/use-cases/monitors/US-MON-01.md",
    "10-intent/use-cases/monitors/README.md",
    "10-intent/functional/monitors.md",
  ],
  needsSectionOf: (path: string) =>
    path.endsWith("US-MON-01.md") ? "Выросло из `ST-01` и `ST-02`, а также из `ST-99`." : "",
  requirementsSectionOf: (path: string) =>
    path.endsWith("US-MON-01.md") ? "| `FR-MON-03` | текст | ST-02 |\n| `FR-MON-04` | текст | ST-65 |" : "",
};

test("потребности берутся из своих таблиц, заголовок таблицы потребностью не считается", () => {
  const { needs } = projectNeeds(SOURCE);
  assert.deepEqual(
    needs.map((n) => n.id),
    ["ST-01", "ST-02", "ST-63"],
  );
});

test("приоритет читается по объявленной в документе легенде", () => {
  assert.equal(priorityOf("О"), "must");
  assert.equal(priorityOf("Ж"), "should");
  assert.equal(priorityOf("П"), "later");
  assert.equal(priorityOf("—"), "unknown", "прочерк — это «неизвестно», а не «можно после»");
});

test("тема берётся из раздела, под которым стоит таблица", () => {
  const { needs } = projectNeeds(SOURCE);
  assert.equal(needs.find((n) => n.id === "ST-01")!.theme, "A. Шум и сон");
  assert.equal(needs.find((n) => n.id === "ST-63")!.theme, "J. Руководитель");
});

test("повторное упоминание в прозе не перебивает объявление", () => {
  const need = projectNeeds(SOURCE).needs.find((n) => n.id === "ST-01")!;
  assert.equal(need.text, "Читать одну карточку вместо череды", "объявление — первое, а не последнее");
  assert.equal(need.sides, "ДЕЖ");
});

test("связь с историей берётся из её раздела, README историей не считается", () => {
  const declared = projectNeeds(SOURCE).links.filter((l) => l.kind === "declared");
  assert.deepEqual(declared, [
    { needId: "ST-01", storyId: "US-MON-01", kind: "declared" },
    { needId: "ST-02", storyId: "US-MON-01", kind: "declared" },
    { needId: "ST-99", storyId: "US-MON-01", kind: "declared" },
  ]);
});

test("потребность, названная в таблице требований, — цитата, а не объявление", () => {
  const cited = projectNeeds(SOURCE).links.filter((l) => l.kind === "cited");
  assert.deepEqual(
    cited.map((l) => l.needId),
    ["ST-02", "ST-65"],
  );
  assert.ok(
    !cited.some((l) => l.needId === "ST-01"),
    "ST-01 объявлена, но в таблице требований не названа — это разные множества",
  );
});

test("ссылка на несуществующую потребность сохраняется, а не отбрасывается", () => {
  const { needs, links } = projectNeeds(SOURCE);
  assert.ok(
    !needs.some((n) => n.id === "ST-99") && links.some((l) => l.needId === "ST-99"),
    "иначе висячая ссылка исчезла бы вместе с находкой",
  );
});
