import test from "node:test";
import assert from "node:assert/strict";
import { projectDataModel, type DataModelCell } from "../src/projects/data-model-projection.ts";

function table(blockOrd: number, rows: string[][]): DataModelCell[] {
  return rows.flatMap((row, r) => row.map((value, col) => ({ blockOrd, row: r, col, value })));
}

const CELLS: DataModelCell[] = [
  ...table(68, [
    ["№", "Файл", "Таблицы"],
    ["0001", "accounts", "accounts · users · teams"],
    ["0002", "inventory", "services · environments"],
    ["0003", "signals", "signals · signal_sources"],
    ["итого", "—", "не строка миграции"],
  ]),
  ...table(118, [
    ["Таблица", "Ключевые колонки"],
    ["accounts", "id, kind CHECK (kind IN ('personal','organization'))"],
    ["users", "id, email UNIQUE, password_hash"],
  ]),
  ...table(482, [
    ["Таблица", "Колонки"],
    ["exporters", "id, account_id, kind, name, config jsonb"],
  ]),
  // Соседняя таблица про запросы — не описание схемы.
  ...table(440, [
    ["Запрос", "Кто его задаёт", "Индекс"],
    ["лента ситуаций", "дежурный", "by_account_time"],
  ]),
];

const SOURCE = { path: "30-design/data-model.md", cells: CELLS };

test("таблицы собираются из обеих шапок описания, чужая таблица не примешивается", () => {
  assert.deepEqual(
    projectDataModel(SOURCE).tables.map((t) => t.name),
    ["accounts", "environments", "exporters", "services", "signal_sources", "signals", "teams", "users"],
  );
});

test("миграция связывается с таблицей, а строка «итого» миграцией не считается", () => {
  const { migrations, tables } = projectDataModel(SOURCE);
  assert.deepEqual(
    migrations.map((m) => m.number),
    ["0001", "0002", "0003"],
  );
  assert.equal(tables.find((t) => t.name === "accounts")!.migration, "0001");
  assert.equal(tables.find((t) => t.name === "services")!.migrationFile, "inventory");
});

test("таблица из миграции без описания колонок не теряется, а остаётся видимой", () => {
  const teams = projectDataModel(SOURCE).tables.find((t) => t.name === "teams")!;
  assert.equal(teams.columns, "", "колонок нигде не описано — это находка, а не повод её выбросить");
  assert.equal(teams.migration, "0001");
});

test("описанная таблица без миграции видна отдельно", () => {
  const exporters = projectDataModel(SOURCE).tables.find((t) => t.name === "exporters")!;
  assert.equal(exporters.migration, "", "какая миграция её заводит — не сказано");
  assert.ok(exporters.columns.length > 0);
});

test("колонки из раздела с блоком кода считаются описанием", () => {
  const source = {
    ...SOURCE,
    sections: [
      { ord: 284, title: "`signals` — факт, а не процесс" },
      { ord: 292, title: "`signal_sources`" },
      { ord: 300, title: "Сроки хранения" },
    ],
    blocks: [
      { ord: 286, kind: "code", raw: "```\nid text PK · account_id · source_id → signal_sources\n```" },
      { ord: 288, kind: "prose", raw: "Ни state, ни rev" },
      { ord: 294, kind: "code", raw: "```\nid text PK · account_id · kind\n```" },
      { ord: 302, kind: "prose", raw: "события живут 30 суток" },
    ],
  };
  const tables = projectDataModel(source).tables;
  const signals = tables.find((t) => t.name === "signals")!;
  assert.match(signals.columns, /id text PK/, "описание живёт в блоке кода под заголовком");
  assert.ok(!tables.some((t) => t.name === "Сроки"), "раздел без имени таблицы таблицей не становится");
});

test("строка описания сильнее блока кода: она ближе к перечню", () => {
  const source = {
    ...SOURCE,
    sections: [{ ord: 600, title: "`accounts`" }],
    blocks: [{ ord: 602, kind: "code", raw: "```\nсовсем другие колонки\n```" }],
  };
  const accounts = projectDataModel(source).tables.find((t) => t.name === "accounts")!;
  assert.match(accounts.columns, /kind CHECK/, "то, что объявлено таблицей, не подменяется кодом");
});

test("заголовок про несколько таблиц: каждая строка кода относится к своей", () => {
  const source = {
    path: "x.md",
    cells: table(68, [
      ["№", "Файл", "Таблицы"],
      ["0004", "policy", "policies · policy_versions"],
    ]),
    sections: [{ ord: 338, title: "policies и policy_versions" }],
    blocks: [
      {
        ord: 340,
        kind: "code",
        raw: "```\npolicies         id · account_id · name · rev\npolicy_versions  policy_id · body jsonb\n```",
      },
    ],
  };
  const tables = projectDataModel(source).tables;
  assert.equal(tables.find((t) => t.name === "policies")!.columns, "id · account_id · name · rev");
  assert.equal(tables.find((t) => t.name === "policy_versions")!.columns, "policy_id · body jsonb");
});

test("блок кода описывает только известную таблицу, но не заводит новую", () => {
  const source = {
    ...SOURCE,
    sections: [{ ord: 900, title: "5. Корреляция" }],
    blocks: [
      {
        ord: 902,
        kind: "code",
        raw: "```\ncorrelation_rules  id · account_id · position int\nвыдуманное_имя     ни на что не ссылается\n```",
      },
    ],
  };
  const names = projectDataModel(source).tables.map((t) => t.name);
  assert.ok(!names.includes("correlation_rules"), "миграция её не заводит — и код не должен");
  assert.ok(!names.includes("выдуманное_имя"), "случайное слово таблицей не становится");
});
