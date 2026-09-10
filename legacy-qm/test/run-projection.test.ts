import test from "node:test";
import assert from "node:assert/strict";
import { projectRuns } from "../src/projects/run-projection.ts";

const SOURCE = {
  paths: [
    "60-runs/v1/M0.md",
    "60-runs/v1/M0/M0-T1.md",
    "60-runs/v1/M0/M0-T11a.md",
    "60-runs/v1/README.md",
    "50-plan/v1/M0/M0-T1.md",
  ],
  titleOf: (p: string) => `Заголовок ${p}`,
  sectionTitlesOf: (p: string) =>
    p.endsWith("M0-T1.md")
      ? ["M0-T1 · Окружение базы", "Что появилось", "Оставлено открытым"]
      : ["Заголовок", "Что появилось"],
};

test("прогон опознаётся по пути, план прогоном не становится", () => {
  const runs = projectRuns(SOURCE);
  assert.deepEqual(
    runs.map((r) => r.path),
    ["60-runs/v1/M0.md", "60-runs/v1/M0/M0-T1.md", "60-runs/v1/M0/M0-T11a.md"],
    "документ плана и README прогонами не считаются",
  );
});

test("прогон этапа отличается от прогона задачи", () => {
  const runs = projectRuns(SOURCE);
  const milestone = runs.find((r) => r.path === "60-runs/v1/M0.md")!;
  assert.equal(milestone.isMilestone, true);
  assert.equal(milestone.id, "M0");
  assert.equal(runs.find((r) => r.id === "M0-T1")!.isMilestone, false);
});

test("буквенный хвост задачи сохраняется: M0-T11a — не M0-T11", () => {
  assert.ok(
    projectRuns(SOURCE).some((r) => r.id === "M0-T11a"),
    "иначе три подзадачи склеятся в одну и прогон потеряется",
  );
});

test("«Оставлено открытым» — признак долга, а не просто раздел", () => {
  const runs = projectRuns(SOURCE);
  assert.equal(runs.find((r) => r.id === "M0-T1")!.leftOpen, true);
  assert.equal(runs.find((r) => r.id === "M0-T11a")!.leftOpen, false);
});
