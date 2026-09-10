import test from "node:test";
import assert from "node:assert/strict";
import { projectSurface } from "../src/projects/surface-projection.ts";

function surfaceFields(path: string): [string, string][] {
  if (path.includes("US-ESC-11")) {
    return [
      ["Персона", "дежурный"],
      ["Фаза пути", "разбор"],
      ["Фича", "цепочки эскалации"],
      ["Экраны", "`SCR-CFG-19` · `SCR-OPS-11`"],
    ];
  }
  if (path.includes("M1-T1")) return [["Экраны", "`SCR-OPS-11` · `SCR-MOB-99`"]];
  return [];
}

const SOURCE = {
  paths: [
    "10-intent/use-cases/escalation-chains/US-ESC-11.md",
    "20-surface/configure/agent.md",
    "20-surface/configure/README.md",
    "20-surface/ops/board.md",
    "50-plan/v1/M1/M1-T1.md",
    "30-design/sdd.md",
    "20-surface/shell.md",
  ],
  titleOf: (p: string) => {
    if (p.endsWith("agent.md")) return "`SCR-CFG-19` · Агент в кластере";
    if (p.endsWith("board.md")) return "`SCR-OPS-11` · Доска";
    if (p.endsWith("README.md")) return "Экраны области";
    return `Заголовок ${p.split("/").at(-1)}`;
  },
  sectionTitlesOf: (p: string) =>
    p.endsWith("shell.md") ? ["Оболочка", "SCR-SHELL-01 · Шапка", "SCR-SHELL-02 · Рельс"] : [],
  fieldsOf: (p: string) => new Map(surfaceFields(p)),
  sectionBodyOf: (p: string, title: string) =>
    p.includes("US-ESC-11") && title === "Требования, на которые опирается"
      ? "Опирается на `FR-ESC-05` и `NFR-15`, а проверка `TC-ESC-01` названа ниже."
      : "",
};

test("история разбирается с её полями пути", () => {
  const story = projectSurface(SOURCE).stories[0]!;
  assert.equal(story.id, "US-ESC-11");
  assert.equal(story.area, "escalation-chains");
  assert.equal(story.persona, "дежурный");
  assert.equal(story.phase, "разбор");
  assert.equal(story.feature, "цепочки эскалации");
});

test("идентификатор опознаётся и без обратных кавычек — секции хранятся без разметки", () => {
  const stripped = projectSurface({
    ...SOURCE,
    paths: ["20-surface/configure/agent.md"],
    titleOf: () => "SCR-CFG-19 · Агент в кластере",
  });
  assert.deepEqual(
    stripped.screens.map((s) => s.id),
    ["SCR-CFG-19"],
  );
});

test("экран опознаётся по идентификатору в заголовке, README экраном не считается", () => {
  const screens = projectSurface(SOURCE).screens;
  assert.deepEqual(
    screens.filter((s) => s.area !== "shell").map((s) => s.id),
    ["SCR-CFG-19", "SCR-OPS-11"],
    "README без идентификатора отброшен",
  );
  assert.equal(screens[0]!.area, "configure");
});

test("ссылки на экраны собираются и от историй, и от задач", () => {
  const refs = projectSurface(SOURCE).references;
  assert.deepEqual(
    refs.map((r) => [r.source, r.sourceKind, r.screenId]),
    [
      ["US-ESC-11", "story", "SCR-CFG-19"],
      ["US-ESC-11", "story", "SCR-OPS-11"],
      ["M1-T1", "task", "SCR-OPS-11"],
      ["M1-T1", "task", "SCR-MOB-99"],
    ],
  );
});

test("один источник не цитирует один экран дважды", () => {
  const refs = projectSurface({
    ...SOURCE,
    paths: ["50-plan/v1/M1/M1-T1.md"],
    fieldsOf: () => new Map([["Экраны", "`SCR-OPS-11` и снова `SCR-OPS-11`"]]),
  }).references;
  assert.equal(refs.length, 1);
});

test("экраны, описанные разделами одного документа, тоже становятся экранами", () => {
  const screens = projectSurface(SOURCE).screens;
  assert.ok(
    screens.some((s) => s.id === "SCR-SHELL-01") && screens.some((s) => s.id === "SCR-SHELL-02"),
    "восемь элементов оболочки живут в одном документе — по разделу на элемент",
  );
  assert.equal(screens.find((s) => s.id === "SCR-SHELL-01")!.area, "shell");
});

test("требования истории берутся из своего раздела, а не из всего документа", () => {
  const { storyRequirements } = projectSurface(SOURCE);
  assert.deepEqual(
    storyRequirements.map((r) => r.requirementId),
    ["FR-ESC-05", "NFR-15"],
    "проверка TC-ESC-01 из соседнего раздела сюда не попадает",
  );
  assert.equal(storyRequirements[0]!.storyId, "US-ESC-11");
});

test("история без раздела требований остаётся без связей, а не выдумывает их", () => {
  const quiet = { ...SOURCE, sectionBodyOf: () => "" };
  assert.deepEqual(projectSurface(quiet).storyRequirements, []);
});
