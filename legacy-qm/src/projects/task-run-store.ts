import { randomUUID } from "node:crypto";
import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Прогон задачи — попытка одного агента закрыть одну задачу плана.
 * Состояния идут по порядку работы, а не по статусу задачи: статус в плане меняется
 * только когда прогон дошёл до конца.
 */
export const TASK_RUN_STATES = [
  "queued",
  "provisioning",
  "running",
  "awaiting_human",
  "testing",
  "review",
  "merging",
  "done",
  "failed",
  "cancelled",
] as const;
export type TaskRunState = (typeof TASK_RUN_STATES)[number];

export const TERMINAL_STATES: readonly TaskRunState[] = ["done", "failed", "cancelled"];

/**
 * Разрешённые переходы. Список закрытый: неизвестный переход отвергается,
 * иначе состояние прогона перестанет что-либо означать.
 */
const TRANSITIONS: Record<TaskRunState, readonly TaskRunState[]> = {
  queued: ["provisioning", "cancelled", "failed"],
  provisioning: ["running", "failed", "cancelled"],
  running: ["awaiting_human", "testing", "failed", "cancelled"],
  awaiting_human: ["running", "failed", "cancelled"],
  // из проверок можно вернуться в работу: упавший тест — это правки, а не провал прогона
  testing: ["running", "review", "failed", "cancelled"],
  review: ["merging", "running", "failed", "cancelled"],
  merging: ["done", "failed"],
  done: [],
  failed: [],
  cancelled: [],
};

export function isTerminal(state: TaskRunState): boolean {
  return TERMINAL_STATES.includes(state);
}

/** Куда можно уйти отсюда — интерфейс рисует кнопки по этому списку, а не по своей копии правил. */
export function nextStates(from: TaskRunState): readonly TaskRunState[] {
  return TRANSITIONS[from] ?? [];
}

export function canMove(from: TaskRunState, to: TaskRunState): boolean {
  return TRANSITIONS[from]?.includes(to) ?? false;
}

export interface TaskRun {
  id: string;
  projectId: string;
  taskId: string;
  agentId: string;
  state: TaskRunState;
  attempt: number;
  /** Прогон в очереди ядра, когда он туда попадёт; пока назначение ручное — null. */
  runId: string | null;
  sessionId: string | null;
  note: string;
  createdAt: number;
  updatedAt: number;
  finishedAt: number | null;
}

export interface TaskRunEvent {
  id: number;
  taskRunId: string;
  fromState: TaskRunState | null;
  toState: TaskRunState;
  reason: string;
  actor: string;
  at: number;
}

export type AssignResult =
  | { status: "ok"; run: TaskRun }
  | { status: "busy_task"; run: TaskRun }
  | { status: "busy_agent"; limit: number }
  | { status: "not_found" };

export type MoveResult =
  { status: "ok"; run: TaskRun } | { status: "illegal"; from: TaskRunState } | { status: "not_found" };

export interface TaskRunStore {
  assign(projectId: string, taskId: string, agentId: string, actor: string): Promise<AssignResult>;
  move(projectId: string, runId: string, to: TaskRunState, actor: string, reason?: string): Promise<MoveResult>;
  active(projectId: string): Promise<TaskRun[]>;
  forTask(projectId: string, taskId: string): Promise<TaskRun[]>;
  events(projectId: string, runId: string): Promise<TaskRunEvent[]>;
  close?(): Promise<void>;
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_task_runs(
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    state TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,
    run_id TEXT,
    session_id TEXT,
    note TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    finished_at BIGINT
  )`,
  // У задачи не может быть двух живых прогонов: иначе два агента возьмут её разом.
  `CREATE UNIQUE INDEX IF NOT EXISTS project_task_runs_one_active
    ON project_task_runs(project_id, task_id)
    WHERE state NOT IN ('done','failed','cancelled')`,
  `CREATE INDEX IF NOT EXISTS project_task_runs_by_agent
    ON project_task_runs(project_id, agent_id, state)`,
  `CREATE TABLE IF NOT EXISTS project_task_run_events(
    id BIGSERIAL PRIMARY KEY,
    task_run_id TEXT NOT NULL REFERENCES project_task_runs(id) ON DELETE CASCADE,
    from_state TEXT,
    to_state TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    actor TEXT NOT NULL,
    at BIGINT NOT NULL
  )`,
  `CREATE INDEX IF NOT EXISTS project_task_run_events_by_run
    ON project_task_run_events(task_run_id, id)`,
];

type Row = Record<string, unknown>;

function rowToRun(row: Row): TaskRun {
  return {
    id: String(row["id"]),
    projectId: String(row["project_id"]),
    taskId: String(row["task_id"]),
    agentId: String(row["agent_id"]),
    state: String(row["state"]) as TaskRunState,
    attempt: Number(row["attempt"]),
    runId: row["run_id"] === null ? null : String(row["run_id"]),
    sessionId: row["session_id"] === null ? null : String(row["session_id"]),
    note: String(row["note"] ?? ""),
    createdAt: Number(row["created_at"]),
    updatedAt: Number(row["updated_at"]),
    finishedAt: row["finished_at"] === null ? null : Number(row["finished_at"]),
  };
}

function rowToEvent(row: Row): TaskRunEvent {
  return {
    id: Number(row["id"]),
    taskRunId: String(row["task_run_id"]),
    fromState: row["from_state"] === null ? null : (String(row["from_state"]) as TaskRunState),
    toState: String(row["to_state"]) as TaskRunState,
    reason: String(row["reason"] ?? ""),
    actor: String(row["actor"]),
    at: Number(row["at"]),
  };
}

export interface TaskRunDeps {
  /** Предел параллелизма агента; null — агента нет. */
  agentConcurrency(projectId: string, agentId: string): Promise<number | null>;
}

export function createPostgresTaskRunStore(
  connectionString: string,
  deps: TaskRunDeps,
  opts: { now?: () => number } = {},
): TaskRunStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, SCHEMA);

  return {
    async assign(projectId, taskId, agentId, actor) {
      const limit = await deps.agentConcurrency(projectId, agentId);
      if (limit === null) return { status: "not_found" };

      return withPgTransaction(await pg.pool(), async (client) => {
        const live = `state NOT IN ('done','failed','cancelled')`;
        const busy = await client.query(
          `SELECT * FROM project_task_runs WHERE project_id=$1 AND task_id=$2 AND ${live}`,
          [projectId, taskId],
        );
        if (busy.rows[0]) return { status: "busy_task", run: rowToRun(busy.rows[0] as Row) };

        const load = await client.query(
          `SELECT count(*) AS n FROM project_task_runs WHERE project_id=$1 AND agent_id=$2 AND ${live}`,
          [projectId, agentId],
        );
        if (Number((load.rows[0] as Row)["n"]) >= limit) return { status: "busy_agent", limit };

        const past = await client.query(
          `SELECT count(*) AS n FROM project_task_runs WHERE project_id=$1 AND task_id=$2`,
          [projectId, taskId],
        );
        const attempt = Number((past.rows[0] as Row)["n"]) + 1;
        const at = now();
        const id = randomUUID();
        const { rows } = await client.query(
          `INSERT INTO project_task_runs(id, project_id, task_id, agent_id, state, attempt, created_at, updated_at)
           VALUES ($1,$2,$3,$4,'queued',$5,$6,$6) RETURNING *`,
          [id, projectId, taskId, agentId, attempt, at],
        );
        await client.query(
          `INSERT INTO project_task_run_events(task_run_id, from_state, to_state, reason, actor, at)
           VALUES ($1,NULL,'queued','назначено',$2,$3)`,
          [id, actor, at],
        );
        return { status: "ok", run: rowToRun(rows[0] as Row) };
      });
    },

    async move(projectId, runId, to, actor, reason = "") {
      return withPgTransaction(await pg.pool(), async (client) => {
        const found = await client.query(`SELECT * FROM project_task_runs WHERE project_id=$1 AND id=$2 FOR UPDATE`, [
          projectId,
          runId,
        ]);
        if (!found.rows[0]) return { status: "not_found" };
        const current = rowToRun(found.rows[0] as Row);
        if (!canMove(current.state, to)) return { status: "illegal", from: current.state };

        const at = now();
        const { rows } = await client.query(
          `UPDATE project_task_runs SET state=$3, updated_at=$4, finished_at=$5
            WHERE project_id=$1 AND id=$2 RETURNING *`,
          [projectId, runId, to, at, isTerminal(to) ? at : null],
        );
        await client.query(
          `INSERT INTO project_task_run_events(task_run_id, from_state, to_state, reason, actor, at)
           VALUES ($1,$2,$3,$4,$5,$6)`,
          [runId, current.state, to, reason, actor, at],
        );
        return { status: "ok", run: rowToRun(rows[0] as Row) };
      });
    },

    async active(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_task_runs
          WHERE project_id=$1 AND state NOT IN ('done','failed','cancelled')
          ORDER BY created_at`,
        [projectId],
      );
      return rows.map((row) => rowToRun(row as Row));
    },

    async forTask(projectId, taskId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_task_runs WHERE project_id=$1 AND task_id=$2 ORDER BY attempt DESC`,
        [projectId, taskId],
      );
      return rows.map((row) => rowToRun(row as Row));
    },

    async events(projectId, runId) {
      const { rows } = await pg.query(
        `SELECT e.* FROM project_task_run_events e
           JOIN project_task_runs r ON r.id = e.task_run_id
          WHERE r.project_id=$1 AND e.task_run_id=$2
          ORDER BY e.id`,
        [projectId, runId],
      );
      return rows.map((row) => rowToEvent(row as Row));
    },

    close: () => pg.close(),
  };
}
