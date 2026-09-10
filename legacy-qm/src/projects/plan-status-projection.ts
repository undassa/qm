import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";
import { stateOf, type TaskState } from "./plan-projection.ts";

/**
 * Доска состояний `50-plan/v1/status.md` — второе объявление того же факта:
 * состояние задачи записано ещё и полем в самой задаче. Два места расходятся
 * молча, поэтому доска разбирается отдельно и сверяется с планом.
 */
export interface PlanStatusRow {
  taskId: string;
  milestone: string;
  state: TaskState;
  /** Коммит, которым задача закрыта; у незакрытых пусто. */
  commit: string;
  path: string;
}

const HEADER = ["Задача", "Состояние", "Коммиты"];
const TASK_ID = /^\s*`?([MV]\d+-T[0-9a-z]+)`?\s*$/;
// В ячейке коммита бывает несколько хешей и прочерк.
const COMMIT = /\b([0-9a-f]{7,40})\b/;

export interface StatusCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface PlanStatusSource {
  path: string;
  cells: readonly StatusCell[];
  /** Заголовок раздела над таблицей — это и есть этап: «M0», «M1»… */
  milestoneOfBlock: (blockOrd: number) => string;
}

export function projectPlanStatus(source: PlanStatusSource): PlanStatusRow[] {
  const blocks = new Map<number, Map<number, string[]>>();
  for (const cell of source.cells) {
    const rows = blocks.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    blocks.set(cell.blockOrd, rows);
  }

  const out: PlanStatusRow[] = [];
  const seen = new Set<string>();
  for (const [blockOrd, rows] of [...blocks.entries()].sort((a, b) => a[0] - b[0])) {
    const head = rows.get(0);
    if (!head || !HEADER.every((name, i) => (head[i] ?? "").trim() === name)) continue;
    const milestone = source.milestoneOfBlock(blockOrd);
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const id = TASK_ID.exec(row[0] ?? "")?.[1];
      if (!id || seen.has(id)) continue;
      seen.add(id);
      // Состояние читается тем же разбором, что и поле задачи: словарь один.
      const { state } = stateOf(row[1] ?? "");
      out.push({
        taskId: id,
        milestone,
        state,
        commit: COMMIT.exec(row[2] ?? "")?.[1] ?? "",
        path: source.path,
      });
    }
  }
  return out.sort((a, b) => a.taskId.localeCompare(b.taskId));
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_plan_status(
    project_id TEXT NOT NULL, task_id TEXT NOT NULL, milestone TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('not_started','claimed','closed')),
    commit_hash TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, task_id)
  )`,
];

type Row = Record<string, unknown>;

/** Расхождение доски и самой задачи. */
export interface StatusDisagreement {
  taskId: string;
  boardState: TaskState;
  taskState: TaskState;
}

export interface PlanStatusStore {
  replace(projectId: string, rows: readonly PlanStatusRow[]): Promise<void>;
  list(projectId: string): Promise<PlanStatusRow[]>;
  counts(projectId: string): Promise<{ total: number; closed: number; claimed: number; withCommit: number }>;
  /** Доска и задача говорят разное об одном и том же. */
  disagreements(projectId: string): Promise<StatusDisagreement[]>;
  /** Доска называет задачу, которой нет в плане. */
  unknownTasks(projectId: string): Promise<PlanStatusRow[]>;
  /** Задача есть в плане, но на доске её нет. */
  missingFromBoard(projectId: string): Promise<{ id: string; title: string }[]>;
  /** Закрыта, а коммита не названо: чем закрыта — неизвестно. */
  closedWithoutCommit(projectId: string): Promise<PlanStatusRow[]>;
  close(): Promise<void>;
}

function rowToStatus(row: Row): PlanStatusRow {
  return {
    taskId: String(row["task_id"]),
    milestone: String(row["milestone"] ?? ""),
    state: String(row["state"]) as TaskState,
    commit: String(row["commit_hash"] ?? ""),
    path: String(row["path"] ?? ""),
  };
}

export function createPostgresPlanStatusStore(connectionString: string): PlanStatusStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, rows) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_plan_status WHERE project_id=$1`, [projectId]);
        for (const r of rows) {
          await client.query(
            `INSERT INTO project_plan_status(project_id, task_id, milestone, state, commit_hash, path)
             VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, r.taskId, r.milestone, r.state, r.commit, r.path],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_plan_status WHERE project_id=$1 ORDER BY task_id`, [
        projectId,
      ]);
      return rows.map((row) => rowToStatus(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS total,
                count(*) FILTER (WHERE state='closed')   AS closed,
                count(*) FILTER (WHERE state='claimed')  AS claimed,
                count(*) FILTER (WHERE commit_hash <> '') AS with_commit
           FROM project_plan_status WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return {
        total: Number(r["total"]),
        closed: Number(r["closed"]),
        claimed: Number(r["claimed"]),
        withCommit: Number(r["with_commit"]),
      };
    },

    async disagreements(projectId) {
      const { rows } = await pg.query(
        `SELECT s.task_id, s.state AS board_state, t.state AS task_state
           FROM project_plan_status s
           JOIN project_plan_tasks t ON t.project_id=s.project_id AND t.id=s.task_id
          WHERE s.project_id=$1 AND s.state <> t.state
          ORDER BY s.task_id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          taskId: String(r["task_id"]),
          boardState: String(r["board_state"]) as TaskState,
          taskState: String(r["task_state"]) as TaskState,
        };
      });
    },

    async unknownTasks(projectId) {
      const { rows } = await pg.query(
        `SELECT s.* FROM project_plan_status s
          LEFT JOIN project_plan_tasks t ON t.project_id=s.project_id AND t.id=s.task_id
         WHERE s.project_id=$1 AND t.id IS NULL ORDER BY s.task_id`,
        [projectId],
      );
      return rows.map((row) => rowToStatus(row as Row));
    },

    async missingFromBoard(projectId) {
      const { rows } = await pg.query(
        `SELECT t.id, t.title FROM project_plan_tasks t
          WHERE t.project_id=$1 AND NOT EXISTS (
            SELECT 1 FROM project_plan_status s WHERE s.project_id=t.project_id AND s.task_id=t.id)
          ORDER BY t.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return { id: String(r["id"]), title: String(r["title"]) };
      });
    },

    async closedWithoutCommit(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_plan_status
          WHERE project_id=$1 AND state='closed' AND commit_hash='' ORDER BY task_id`,
        [projectId],
      );
      return rows.map((row) => rowToStatus(row as Row));
    },

    close: () => pg.close(),
  };
}
