import assert from "node:assert/strict";
import test from "node:test";
import { linkTo, relativeTo } from "../src/relative.ts";

test("вверх из каталога вопросов к решению", () => {
  assert.equal(
    relativeTo("00-frame/questions/Q-331.md", "30-design/decisions/accepted/0138-the-role.md"),
    "../../30-design/decisions/accepted/0138-the-role.md",
  );
});

test("вверх на один уровень", () => {
  assert.equal(relativeTo("00-frame/questions/Q-01.md", "00-frame/constitution.md"), "../constitution.md");
});

test("сосед пишется так же, как его пишет корпус — без «./»", () => {
  // README реестра ссылается на соседа как `[Q-272](Q-272.md)`; своё соглашение
  // завело бы второй способ писать одно и то же.
  assert.equal(relativeTo("00-frame/questions/Q-01.md", "00-frame/questions/Q-02.md"), "Q-02.md");
});

test("из корня вниз", () => {
  assert.equal(relativeTo("README.md", "10-intent/srs.md"), "10-intent/srs.md");
});

test("общий путь не режется по половине имени каталога", () => {
  // «10-intent» и «10-intentions» — разные каталоги, и общего у них ничего нет.
  assert.equal(relativeTo("10-intent/a.md", "10-intentions/b.md"), "../10-intentions/b.md");
});

test("ссылка собирается целиком", () => {
  assert.equal(
    linkTo("00-frame/questions/Q-163.md", "10-intent/srs.md", "srs.md"),
    "[srs.md](../../10-intent/srs.md)",
  );
});
