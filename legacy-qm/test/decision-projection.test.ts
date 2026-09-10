import test from "node:test";
import assert from "node:assert/strict";
import { projectDecisions, statusOfFolder } from "../src/projects/decision-projection.ts";

const SOURCE = {
  paths: [
    "30-design/decisions/accepted/0082-no-generators.md",
    "30-design/decisions/accepted/0080-contract-first.md",
    "30-design/decisions/superseded/0012-old-way.md",
    "30-design/decisions/0000-template.md",
    "30-design/sdd.md",
  ],
  titleOf: (p: string) => `Заголовок ${p.split("/").at(-1)}`,
  fieldsOf: (p: string) =>
    new Map(
      p.includes("0082")
        ? [
            ["Статус", "Принято"],
            ["Дата", "2026-08-16"],
            ["Решают", "владелец продукта"],
            ["Уточняет", "[ADR-0080](0080-contract-first.md) — генерация отменяется"],
            ["Связано", "FR-EXT-01, FR-EXT-02 и TC-EXT-05"],
          ]
        : [],
    ),
};

test("состояние решения берётся из каталога", () => {
  assert.equal(statusOfFolder("accepted"), "accepted");
  assert.equal(statusOfFolder("superseded"), "superseded");
  assert.equal(statusOfFolder("нечто"), "template");
});

test("шаблон и обычные документы решениями не считаются", () => {
  const p = projectDecisions(SOURCE);
  assert.deepEqual(
    p.decisions.map((d) => d.id),
    ["ADR-0012", "ADR-0080", "ADR-0082"],
    "0000-template и sdd.md отброшены, порядок по номеру",
  );
});

test("поля решения разбираются, дата отделяется от пояснения", () => {
  const adr = projectDecisions(SOURCE).decisions.find((d) => d.id === "ADR-0082")!;
  assert.equal(adr.status, "accepted");
  assert.equal(adr.statusText, "Принято");
  assert.equal(adr.date, "2026-08-16");
  assert.equal(adr.deciders, "владелец продукта");
});

test("связи разбираются по видам: уточнение — на решение, связанное — на требования", () => {
  const links = projectDecisions(SOURCE).links;
  assert.deepEqual(
    links.filter((l) => l.kind === "refines"),
    [{ decisionId: "ADR-0082", kind: "refines", target: "ADR-0080" }],
  );
  assert.deepEqual(
    links.filter((l) => l.kind === "related").map((l) => l.target),
    ["FR-EXT-01", "FR-EXT-02", "TC-EXT-05"],
    "перечисление через запятую и союз разбирается целиком",
  );
});

test("решение не ссылается само на себя", () => {
  const self = projectDecisions({
    ...SOURCE,
    paths: ["30-design/decisions/accepted/0082-x.md"],
    fieldsOf: () => new Map([["Уточняет", "ADR-0082 сам себя"]]),
  });
  assert.deepEqual(self.links, [], "самоссылка отброшена");
});

/** Два словаря корпуса: ранние решения описаны по-английски, поздние по-русски. */
const BILINGUAL = {
  paths: ["30-design/decisions/accepted/0014-ddd.md", "30-design/decisions/accepted/0076-retention.md"],
  titleOf: (p: string) => `Заголовок ${p}`,
  fieldsOf: (p: string) =>
    new Map(
      p.includes("0014")
        ? [
            ["Status", "Accepted"],
            ["Date", "2026-07-27"],
            ["Deciders", "core"],
            ["Amends", "Constitution Article 4 (see below) and ADR-0007"],
            ["Related", "ADR-0001 (we record decisions), ADR-0012"],
            ["Supersedes", "the layout of ADR-0007 (project layout)"],
          ]
        : [
            ["Статус", "принято"],
            ["Дата", "2026-08-30"],
            ["Решают", "владелец"],
            ["Связано", "FR-RT-06 и NFR-SEC-01"],
            ["Закрывает", "Q-10 — что видит арендатор сам"],
          ],
    ),
};

test("английские поля решения читаются наравне с русскими", () => {
  const { decisions } = projectDecisions(BILINGUAL);
  const early = decisions.find((d) => d.id === "ADR-0014")!;
  assert.equal(early.date, "2026-07-27", "поле Date такое же поле, как Дата");
  assert.equal(early.deciders, "core");
  assert.equal(early.statusText, "Accepted");
});

test("«Related» указывает на решения, а «Связано» — на требования: это разные связи", () => {
  const { links } = projectDecisions(BILINGUAL);
  const kinds = (kind: string) => links.filter((l) => l.kind === kind).map((l) => l.target);
  assert.deepEqual(kinds("relates-to-decision"), ["ADR-0001", "ADR-0012"]);
  assert.deepEqual(kinds("related"), ["FR-RT-06", "NFR-SEC-01"], "требования не смешаны с решениями");
  assert.ok(
    !kinds("related").includes("ADR-0001"),
    "иначе ADR-0001 объявили бы несуществующим требованием и выдумали нарушение гейта",
  );
});

test("правка статьи конституции — отдельная связь, а не ссылка на решение", () => {
  const { links } = projectDecisions(BILINGUAL);
  assert.deepEqual(
    links.filter((l) => l.kind === "amends-article").map((l) => l.target),
    ["4"],
  );
  assert.ok(
    links.some((l) => l.kind === "refines" && l.target === "ADR-0007"),
    "в том же поле названо и решение — оно остаётся уточнением",
  );
});

test("«Закрывает» связывает решение с вопросом", () => {
  const { links } = projectDecisions(BILINGUAL);
  assert.deepEqual(
    links.filter((l) => l.kind === "closes").map((l) => l.target),
    ["Q-10"],
  );
});
