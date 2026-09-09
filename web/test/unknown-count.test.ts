import { test } from "node:test";
import assert from "node:assert/strict";
import { headline, titleOf, unknownTotal } from "../src/unknown-count.ts";

const tile = (done: number, open: number, unknown: number, percent: number | null) =>
  ({ tile: "т", done, open, unknown, percent, says: "" });

test("незнание считается одним счётом на весь интерфейс", () => {
  // Два экрана, считавшие его по-своему, показывали 1757 и 1754 — оба «правду».
  assert.equal(unknownTotal([tile(1, 0, 1737, 100), tile(10, 2, 6, 83)], 3), 1746);
});

test("плитка, где неизвестного больше отвечаемого, не кричит процентом", () => {
  assert.deepEqual(headline(tile(1, 0, 1737, 100)), { text: "1737 неизвестно", dim: true });
});

test("обычная плитка показывает процент", () => {
  assert.deepEqual(headline(tile(99, 128, 0, 44)), { text: "44 %", dim: false });
});

test("нечего мерить — так и сказано", () => {
  assert.deepEqual(headline(tile(0, 0, 11, null)), { text: "не измеряется", dim: true });
});

test("заголовок задачи не повторяет её имя", () => {
  assert.equal(titleOf("M0-T12", "M0-T12 · Подсказка пробуждения"), "Подсказка пробуждения");
  assert.equal(titleOf("M0-T12", "Подсказка пробуждения"), "Подсказка пробуждения");
});
