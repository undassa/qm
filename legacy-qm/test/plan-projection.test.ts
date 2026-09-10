import test, { before } from "node:test";
import assert from "node:assert/strict";
import {
  createPostgresPlanStore,
  identifiersIn,
  projectPlan,
  stateOf,
  type ProjectionSource,
  kindOf,
} from "../src/projects/plan-projection.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres plan-projection tests";
const PROJECT = "plan-projection-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  await p.query("DELETE FROM project_plan_versions WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("состояние читается до разделителя, коммит — из хвоста", () => {
  assert.deepEqual(stateOf("закрыта · 99e187a"), { state: "closed", commit: "99e187a" });
  assert.deepEqual(
    stateOf("в работе · коммитов 1, последний 757cf2e"),
    { state: "claimed", commit: null },
    "коммит взятой задачи не закрывающий — колонка называет именно закрытие",
  );
  assert.deepEqual(stateOf("не начата"), { state: "not_started", commit: null });
  assert.deepEqual(stateOf(undefined), { state: "not_started", commit: null });
  assert.equal(stateOf("не начата · 99e187a").commit, null, "у незакрытой задачи коммита нет по определению");
});

test("идентификатор берётся из разметки, а не из очищенного текста", () => {
  assert.deepEqual(identifiersIn("[`M0-T2c`](M0-T2c.md)"), ["M0-T2c"]);
  assert.deepEqual(identifiersIn("обслуживает `NFR-31` и `TC-ORG-02`"), ["NFR-31", "TC-ORG-02"]);
  assert.deepEqual(identifiersIn("ничего"), [], "без обратных кавычек идентификатор не опознаётся");
});

const SOURCE: ProjectionSource = {
  paths: [
    "50-plan/v1.md",
    "50-plan/v1/M0.md",
    "50-plan/v1/M0/M0-T1.md",
    "50-plan/v1/M0/M0-T2.md",
    "50-plan/v1/M1.md",
    "50-plan/v1/M1/M1-T1.md",
    "10-intent/srs.md",
  ],
  titleOf: (path) => `заголовок ${path}`,
  fieldsOf: (path) =>
    new Map<string, string>(
      (
        {
          "50-plan/v1/M0/M0-T1.md": [
            ["Состояние", "закрыта · 99e187a"],
            ["Размер", "1–2 часа"],
            ["Зависит от", "ничего"],
            ["Требования", "обслуживает `NFR-31`"],
          ],
          "50-plan/v1/M0/M0-T2.md": [
            ["Состояние", "не начата"],
            ["Зависит от", "[`M0-T1`](M0-T1.md)"],
          ],
          "50-plan/v1/M1/M1-T1.md": [
            ["Состояние", "не начата"],
            ["Зависит от", "[`M0-T2`](../M0/M0-T2.md)"],
          ],
        } as Record<string, [string, string][]>
      )[path] ?? [],
    ),
};

test("проекция берёт уровни из пути, а прочие документы не трогает", () => {
  const p = projectPlan(SOURCE);
  assert.deepEqual(
    p.versions.map((v) => v.id),
    ["v1"],
  );
  assert.deepEqual(
    p.milestones.map((m) => [m.id, m.versionId]),
    [
      ["M0", "v1"],
      ["M1", "v1"],
    ],
  );
  assert.deepEqual(
    p.tasks.map((t) => [t.id, t.milestoneId, t.state]),
    [
      ["M0-T1", "M0", "closed"],
      ["M0-T2", "M0", "not_started"],
      ["M1-T1", "M1", "not_started"],
    ],
  );
  assert.equal(p.tasks[0]!.closingCommit, "99e187a");
  assert.deepEqual(p.dependencies, [
    { taskId: "M0-T2", dependsOn: "M0-T1" },
    { taskId: "M1-T1", dependsOn: "M0-T2" },
  ]);
  assert.deepEqual(p.requirements, [{ taskId: "M0-T1", requirementId: "NFR-31" }]);
});

test("готовность к взятию считается запросом: цепочка, а не один шаг", { skip: pgSkip }, async (t) => {
  const store = createPostgresPlanStore(URL!);
  t.after(() => store.close());
  await store.replace(PROJECT, projectPlan(SOURCE));

  assert.deepEqual(await store.counts(PROJECT), { versions: 1, milestones: 2, tasks: 3, closed: 1 });
  assert.deepEqual(
    await store.readyTasks(PROJECT),
    ["M0-T2"],
    "M1-T1 ждёт M0-T2, которая сама ещё не закрыта — по цепочке, а не по первому шагу",
  );
});

test("зависимость на несуществующую задачу не заводит призрака", { skip: pgSkip }, async (t) => {
  const store = createPostgresPlanStore(URL!);
  t.after(() => store.close());
  const projection = projectPlan(SOURCE);
  projection.dependencies.push({ taskId: "M0-T2", dependsOn: "M9-T9" });
  await store.replace(PROJECT, projection);
  assert.deepEqual(await store.readyTasks(PROJECT), ["M0-T2"], "неизвестная зависимость отброшена, а не заблокировала");
});

test("вид задачи читается из поля, а без поля задача считается разработческой", () => {
  assert.equal(kindOf("тесты"), "test");
  assert.equal(kindOf("Тесты · пишем проверки"), "test");
  assert.equal(kindOf("проверки"), "test");
  assert.equal(kindOf("test"), "test");
  assert.equal(kindOf("разработка"), "dev");
  assert.equal(kindOf(undefined), "dev", "до разделения все задачи были разработческими");
  assert.equal(kindOf(""), "dev");
  assert.equal(kindOf("тестирование стенда — не про вид"), "test", "образец нарочно широкий: слово в начале решает");
});

test("вид берётся из идентификатора, когда поля нет: V3-T17 — проверки к M3-T17", () => {
  assert.equal(kindOf(undefined, "V3-T17"), "test");
  assert.equal(kindOf(undefined, "V5-T1"), "test");
  assert.equal(kindOf(undefined, "M3-T17"), "dev");
  assert.equal(kindOf("разработка", "V3-T17"), "dev", "поле сильнее идентификатора");
  assert.equal(kindOf("тесты", "M1-T1"), "test");
});
