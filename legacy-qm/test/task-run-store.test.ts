import test, { before } from "node:test";
import assert from "node:assert/strict";
import {
  canMove,
  createPostgresTaskRunStore,
  isTerminal,
  TASK_RUN_STATES,
  type TaskRunState,
} from "../src/projects/task-run-store.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres task-run tests";
const PROJECT = "task-run-test";

const LIMITS: Record<string, number> = { one: 1, wide: 3, finisher: 1 };
const deps = {
  async agentConcurrency(_projectId: string, agentId: string) {
    return LIMITS[agentId] ?? null;
  },
};

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  await p
    .query(
      `DELETE FROM project_task_run_events WHERE task_run_id IN
         (SELECT id FROM project_task_runs WHERE project_id=$1)`,
      [PROJECT],
    )
    .catch(() => undefined);
  await p.query("DELETE FROM project_task_runs WHERE project_id=$1", [PROJECT]).catch(() => undefined);
  await p.end();
});

test("работа идёт вперёд, но из проверок можно вернуться к правкам", () => {
  assert.equal(canMove("queued", "provisioning"), true);
  assert.equal(canMove("testing", "running"), true, "упавший тест — это правки, а не провал прогона");
  assert.equal(canMove("review", "running"), true, "ревью попросило изменений");
  assert.equal(canMove("queued", "done"), false, "нельзя закрыть, не начав");
  assert.equal(canMove("running", "merging"), false, "слияние — только после ревью");
});

test("из конечного состояния выхода нет", () => {
  for (const state of ["done", "failed", "cancelled"] as TaskRunState[]) {
    assert.equal(isTerminal(state), true, state);
    for (const to of TASK_RUN_STATES) assert.equal(canMove(state, to), false, `${state} → ${to}`);
  }
});

test("у задачи один живой прогон — иначе два агента возьмут её разом", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps);
  t.after(() => store.close?.());
  const first = await store.assign(PROJECT, "T-одна", "wide", "тест");
  assert.equal(first.status, "ok");

  const second = await store.assign(PROJECT, "T-одна", "wide", "тест");
  assert.equal(second.status, "busy_task", "вторая попытка отклоняется, пока первая жива");

  if (first.status !== "ok") return;
  await store.move(PROJECT, first.run.id, "cancelled", "тест", "освобождаю");
  const third = await store.assign(PROJECT, "T-одна", "wide", "тест");
  assert.equal(third.status, "ok", "после закрытия прогона задачу можно назначить снова");
  if (third.status === "ok") assert.equal(third.run.attempt, 2, "попытка считается по всем прогонам задачи");
});

test("агент не берёт больше своего предела", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps);
  t.after(() => store.close?.());
  assert.equal((await store.assign(PROJECT, "T-предел-1", "one", "тест")).status, "ok");
  const over = await store.assign(PROJECT, "T-предел-2", "one", "тест");
  assert.equal(over.status, "busy_agent");
  if (over.status === "busy_agent") assert.equal(over.limit, 1);
});

test("незнакомый агент задачу не получает", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps);
  t.after(() => store.close?.());
  assert.equal((await store.assign(PROJECT, "T-ничей", "нет-такого", "тест")).status, "not_found");
});

test("недопустимый переход отвергается и не пишет событие", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps);
  t.after(() => store.close?.());
  const assigned = await store.assign(PROJECT, "T-переход", "wide", "тест");
  assert.equal(assigned.status, "ok");
  if (assigned.status !== "ok") return;

  const jump = await store.move(PROJECT, assigned.run.id, "done", "тест");
  assert.equal(jump.status, "illegal");
  if (jump.status === "illegal") assert.equal(jump.from, "queued");

  assert.deepEqual(
    (await store.events(PROJECT, assigned.run.id)).map((e) => e.toState),
    ["queued"],
    "отвергнутый переход не оставляет следа в ленте",
  );
});

test("лента хранит переходы с причиной и автором", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps);
  t.after(() => store.close?.());
  const assigned = await store.assign(PROJECT, "T-лента", "wide", "аня");
  assert.equal(assigned.status, "ok");
  if (assigned.status !== "ok") return;
  const id = assigned.run.id;

  await store.move(PROJECT, id, "provisioning", "система");
  await store.move(PROJECT, id, "running", "система");
  await store.move(PROJECT, id, "awaiting_human", "агент", "не понял границу");

  const events = await store.events(PROJECT, id);
  assert.deepEqual(
    events.map((e) => e.toState),
    ["queued", "provisioning", "running", "awaiting_human"],
  );
  assert.equal(events[0]!.actor, "аня");
  assert.equal(events.at(-1)!.reason, "не понял границу");

  const [current] = await store.forTask(PROJECT, "T-лента");
  assert.equal(current!.state, "awaiting_human");
  assert.equal(current!.finishedAt, null, "прогон не завершён — времени окончания нет");
});

test("завершение проставляет время окончания и убирает прогон из живых", { skip: pgSkip }, async (t) => {
  const store = createPostgresTaskRunStore(URL!, deps, { now: () => 777 });
  t.after(() => store.close?.());
  const assigned = await store.assign(PROJECT, "T-конец", "finisher", "тест");
  assert.equal(assigned.status, "ok");
  if (assigned.status !== "ok") return;
  await store.move(PROJECT, assigned.run.id, "failed", "тест", "сломалось");

  const [run] = await store.forTask(PROJECT, "T-конец");
  assert.equal(run!.state, "failed");
  assert.equal(run!.finishedAt, 777);
  assert.equal(
    (await store.active(PROJECT)).some((r) => r.taskId === "T-конец"),
    false,
  );
});
