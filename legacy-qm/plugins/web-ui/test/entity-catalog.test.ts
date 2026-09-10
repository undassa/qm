import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { ENTITY_KINDS, ENTITY_META, ENTITY_REPLACES_KIND } from "../../../src/api/routes/project-entities.ts";
import { KIND_ORDER } from "../../../src/projects/document-kind.ts";

const dom = new JSDOM("<div id=host></div>");
// lit берёт document один раз при загрузке модуля, поэтому глобаль ставится до импорта.
(globalThis as { document?: Document }).document = dom.window.document as unknown as Document;
const { render } = await import("lit");
const { chipLabel, columnsFor, entityTableTpl, findingsTpl, plural, titleWithoutId } =
  await import("../src/project-entity-catalog.ts");
const host = dom.window.document.getElementById("host")!;
const draw = (tpl: unknown) => {
  render(tpl as Parameters<typeof render>[0], host as unknown as HTMLElement);
  return host.textContent ?? "";
};

test("у каждой сущности каталога своя раскладка колонок", () => {
  for (const kind of ENTITY_KINDS) {
    const columns = columnsFor(kind);
    assert.ok(columns.length >= 3, `${kind}: колонок мало — сущность смотрят по нескольким признакам`);
  }
  assert.notDeepEqual(
    columnsFor("questions").map((c) => c.key),
    columnsFor("screens").map((c) => c.key),
    "вопрос и экран читаются по-разному: общая таблица скрыла бы состояние",
  );
});

test("каждый вид сущности описан для сайдбара", () => {
  for (const kind of ENTITY_KINDS) {
    assert.ok(ENTITY_META[kind].title, `${kind}: нет заголовка`);
    assert.ok(ENTITY_META[kind].source, `${kind}: не сказано, откуда берётся`);
  }
});

test("состояние выводится словом, а не кодом", () => {
  const text = draw(entityTableTpl("questions", [{ id: "Q-1", state: "open", title: "Заголовок" }], () => {}));
  assert.match(text, /открыт/, "состояние open показано по-русски");
  assert.doesNotMatch(text, /\bopen\b/, "внутреннего кода в таблице быть не должно");
});

test("пустая ячейка не исчезает, а помечается", () => {
  const text = draw(entityTableTpl("decisions", [{ id: "ADR-1", status: "accepted", title: "Т", date: "" }], () => {}));
  assert.match(text, /—/, "пустая дата видна прочерком, а не пустотой");
});

test("находки показываются только своей сущности", () => {
  const findings = [
    { kind: "questions", item: "решено, но ответ не записан", count: 45 },
    { kind: "screens", item: "ссылка на несуществующий экран", count: 3 },
  ];
  assert.match(draw(findingsTpl("questions", findings)), /45/);
  assert.doesNotMatch(draw(findingsTpl("questions", findings)), /несуществующий экран/);
});

test("строка ведёт к своему документу", () => {
  const opened: string[] = [];
  render(
    entityTableTpl("screens", [{ id: "SCR-1", path: "20-surface/a/b.md", title: "Т" }], (row) =>
      opened.push(String(row["path"])),
    ),
    host as unknown as HTMLElement,
  );
  host.querySelector("tbody tr")!.dispatchEvent(new dom.window.Event("click"));
  assert.deepEqual(opened, ["20-surface/a/b.md"], "щелчок по строке открывает документ-источник");
});

test("заголовок не повторяет идентификатор из соседней колонки", () => {
  assert.equal(titleWithoutId("Q-01", "Q-01 — Владелец отпечатка"), "Владелец отпечатка");
  assert.equal(titleWithoutId("SCR-CFG-01", "SCR-CFG-01 · Мониторы"), "Мониторы");
  assert.equal(titleWithoutId("ADR-0001", "ADR-0001: Record architecture decisions"), "Record architecture decisions");
  assert.equal(
    titleWithoutId("Q-02", "Совсем другой заголовок"),
    "Совсем другой заголовок",
    "чужой префикс не трогаем",
  );
  assert.equal(titleWithoutId("Q-03", "Q-03"), "Q-03", "если кроме идентификатора ничего нет — оставляем его");
});

test("каждая сущность объявляет, какой вид документов она вытесняет из сайдбара", () => {
  const doubled = ENTITY_KINDS.filter((kind) => ENTITY_META[kind].title === "Вопросы" && !ENTITY_REPLACES_KIND[kind]);
  assert.deepEqual(doubled, [], "иначе в сайдбаре два счётчика об одном и том же");
  for (const [entity, kind] of Object.entries(ENTITY_REPLACES_KIND)) {
    assert.ok(KIND_ORDER.includes(kind as never), `${entity}: вытесняет несуществующий вид «${kind}»`);
  }
});

test("одинаковое состояние у разных сущностей подписано по-своему", () => {
  assert.equal(chipLabel("questions", "closed"), "закрыт");
  assert.equal(chipLabel("board", "closed"), "закрыта", "у задачи род другой, и общий словарь это терял");
  assert.equal(chipLabel("plan", "unknown"), "не сказано");
  assert.equal(chipLabel("needs", "unknown"), "не указан");
});

test("незнакомое состояние показывается как есть, а не пропадает", () => {
  assert.equal(chipLabel("board", "непонятно"), "непонятно");
  assert.equal(chipLabel("нет-такой-сущности", "closed"), "closed");
});

test("числительное согласуется с существительным", () => {
  const записи = (n: number) => `${n} ${plural(n, "запись", "записи", "записей")}`;
  assert.equal(записи(1), "1 запись");
  assert.equal(записи(2), "2 записи");
  assert.equal(записи(5), "5 записей");
  assert.equal(записи(11), "11 записей", "одиннадцать — исключение, не «одна»");
  assert.equal(записи(21), "21 запись");
  assert.equal(записи(31), "31 запись", "именно это и читалось как небрежность");
  assert.equal(записи(114), "114 записей");
});
