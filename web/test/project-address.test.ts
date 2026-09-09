import { test } from "node:test";
import assert from "node:assert/strict";
import { chooseProject } from "../src/project-address.ts";

const list = [{ projectId: "a" }, { projectId: "b" }];

test("адрес без параметра означает первый проект", () => {
  // Это же значение получает «назад», вернувшийся на страницу без параметра;
  // иначе он не возвращает никуда.
  assert.deepEqual(chooseProject(list, null), { project: { projectId: "a" }, unknown: "" });
});

test("названный проект открывается, а не первый по списку", () => {
  assert.deepEqual(chooseProject(list, "b"), { project: { projectId: "b" }, unknown: "" });
});

test("чужой номер не проглатывается молча", () => {
  const choice = chooseProject(list, "нет-такого");
  assert.deepEqual(choice.project, { projectId: "a" });
  assert.equal(choice.unknown, "нет-такого");
});

test("пустой список не выдаёт себя за проект", () => {
  assert.deepEqual(chooseProject([], "b"), { project: null, unknown: "b" });
});
