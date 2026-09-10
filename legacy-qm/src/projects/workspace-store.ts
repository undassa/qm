import { randomUUID } from "node:crypto";
import { createPgPool } from "../persistence/pg-pool.ts";

export type WorkspaceState = "preparing" | "ready" | "failed";

/** Рабочая копия задачи: где именно агент будет править код. */
export interface TaskWorkspace {
  id: string;
  projectId: string;
  taskRunId: string;
  repositoryId: string;
  state: WorkspaceState;
  /** Каталог внутри песочницы, относительно её корня. */
  path: string;
  baseBranch: string;
  /** Коммит, от которого отведена ветка задачи — по нему потом считается диф. */
  baseCommit: string | null;
  branch: string;
  detail: string;
  createdAt: number;
  updatedAt: number;
}

/**
 * Имя ветки задачи. Идентификаторы плана вида `M1-T10` в имени ветки безопасны,
 * но приводим к нижнему регистру и чистим всё, что git в ссылке не любит.
 */
export function branchNameFor(taskId: string, attempt: number): string {
  const slug = taskId
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "")
    .slice(0, 60);
  const base = slug || "task";
  return attempt > 1 ? `task/${base}-${attempt}` : `task/${base}`;
}

export interface WorkspaceStore {
  begin(input: {
    projectId: string;
    taskRunId: string;
    repositoryId: string;
    path: string;
    baseBranch: string;
    branch: string;
  }): Promise<TaskWorkspace>;
  ready(id: string, baseCommit: string): Promise<TaskWorkspace | null>;
  fail(id: string, detail: string): Promise<TaskWorkspace | null>;
  forRun(projectId: string, taskRunId: string): Promise<TaskWorkspace | null>;
  close?(): Promise<void>;
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_task_workspaces(
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    state TEXT NOT NULL,
    path TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    base_commit TEXT,
    branch TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
  )`,
  // У прогона одна рабочая копия: вторая означала бы два места, где живёт его код.
  `CREATE UNIQUE INDEX IF NOT EXISTS project_task_workspaces_one_per_run
    ON project_task_workspaces(task_run_id)`,
];

type Row = Record<string, unknown>;

function rowToWorkspace(row: Row): TaskWorkspace {
  return {
    id: String(row["id"]),
    projectId: String(row["project_id"]),
    taskRunId: String(row["task_run_id"]),
    repositoryId: String(row["repository_id"]),
    state: String(row["state"]) as WorkspaceState,
    path: String(row["path"]),
    baseBranch: String(row["base_branch"]),
    baseCommit: row["base_commit"] === null ? null : String(row["base_commit"]),
    branch: String(row["branch"]),
    detail: String(row["detail"] ?? ""),
    createdAt: Number(row["created_at"]),
    updatedAt: Number(row["updated_at"]),
  };
}

export function createPostgresWorkspaceStore(
  connectionString: string,
  opts: { now?: () => number } = {},
): WorkspaceStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, SCHEMA);

  return {
    async begin(input) {
      const at = now();
      // Повторная подготовка того же прогона переписывает запись: копия готовится заново.
      const { rows } = await pg.query(
        `INSERT INTO project_task_workspaces(id, project_id, task_run_id, repository_id, state, path, base_branch, branch, created_at, updated_at)
         VALUES ($1,$2,$3,$4,'preparing',$5,$6,$7,$8,$8)
         ON CONFLICT (task_run_id) DO UPDATE SET
           repository_id = EXCLUDED.repository_id, state = 'preparing', path = EXCLUDED.path,
           base_branch = EXCLUDED.base_branch, base_commit = NULL, branch = EXCLUDED.branch,
           detail = '', updated_at = EXCLUDED.updated_at
         RETURNING *`,
        [
          randomUUID(),
          input.projectId,
          input.taskRunId,
          input.repositoryId,
          input.path,
          input.baseBranch,
          input.branch,
          at,
        ],
      );
      return rowToWorkspace(rows[0] as Row);
    },

    async ready(id, baseCommit) {
      const { rows } = await pg.query(
        `UPDATE project_task_workspaces SET state='ready', base_commit=$2, detail='', updated_at=$3
          WHERE id=$1 RETURNING *`,
        [id, baseCommit, now()],
      );
      return rows[0] ? rowToWorkspace(rows[0] as Row) : null;
    },

    async fail(id, detail) {
      const { rows } = await pg.query(
        `UPDATE project_task_workspaces SET state='failed', detail=$2, updated_at=$3 WHERE id=$1 RETURNING *`,
        [id, detail.slice(0, 2000), now()],
      );
      return rows[0] ? rowToWorkspace(rows[0] as Row) : null;
    },

    async forRun(projectId, taskRunId) {
      const { rows } = await pg.query(`SELECT * FROM project_task_workspaces WHERE project_id=$1 AND task_run_id=$2`, [
        projectId,
        taskRunId,
      ]);
      return rows[0] ? rowToWorkspace(rows[0] as Row) : null;
    },

    close: () => pg.close(),
  };
}
