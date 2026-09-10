import assert from "node:assert/strict";
import test from "node:test";
import { headerFields } from "../src/document-blocks.ts";

/** Как это выглядит в корпусе: сверху свойства решения, ниже — пункты раздела «Решение». */
const FIELDS = [
  { sectionOrd: 0, name: "Status", value: "Accepted" },
  { sectionOrd: 0, name: "Date", value: "2026-07-27" },
  { sectionOrd: 10, name: "The consumer owns the interface", value: "«accept interfaces, return structs»" },
  { sectionOrd: 36, name: "Value objects", value: "validate at construction" },
];

test("в свойства попадает только шапка документа", () => {
  assert.deepEqual(
    headerFields(FIELDS).map((f) => f.name),
    ["Status", "Date"],
  );
});

test("документ без шапки не показывает выдержки из прозы вместо свойств", () => {
  assert.deepEqual(headerFields(FIELDS.filter((f) => f.sectionOrd > 0)), []);
});
