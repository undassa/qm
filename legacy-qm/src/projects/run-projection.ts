import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Прогон — запись о том, как выполнялась задача: `60-runs/v1/M0/M0-T1.md`.
 * Полей у прогона нет, всё сказано разделами, и самый ценный из них —
 * «Оставлено открытым»: это долг, который прогон осознанно оставил после себя.
 */
export interface ProjectRun {
  /** Совпадает с идентификатором задачи: у задачи не больше одного прогона. */
  id: string;
  title: string;
  path: string;
  version: string;
  milestone: string;
  /** Прогон этапа целиком, а не отдельной задачи. */
  isMilestone: boolean;
  leftOpen: boolean;
  sections: number;
}

const TASK_RUN = /^60-runs\/(v\d+)\/([MV]\d+)\/([MV]\d+-T[0-9a-z]+)\.md$/;
const MILESTONE_RUN = /^60-runs\/(v\d+)\/([MV]\d+)\.md$/;
const LEFT_OPEN = "Оставлено открытым";

export interface RunSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  sectionTitlesOf: (path: string) => readonly string[];
}

export function projectRuns(source: RunSource): ProjectRun[] {
  const runs: ProjectRun[] = [];
  for (const path of [...source.paths].sort()) {
    const task = TASK_RUN.exec(path);
    const milestone = task ? null : MILESTONE_RUN.exec(path);
    if (!task && !milestone) continue;
    const titles = source.sectionTitlesOf(path);
    runs.push({
      id: task ? task[3]! : milestone![2]!,
      title: source.titleOf(path),
      path,
      version: (task ?? milestone!)[1]!,
      milestone: (task ?? milestone!)[2]!,
      isMilestone: !task,
      leftOpen: titles.some((t) => t.trim() === LEFT_OPEN),
      sections: titles.length,
    });
  }
  return runs;
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_runs_log(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '', milestone TEXT NOT NULL DEFAULT '',
    is_milestone BOOLEAN NOT NULL DEFAULT FALSE,
    left_open BOOLEAN NOT NULL DEFAULT FALSE, sections INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, path)
  )`,
  `CREATE INDEX IF NOT EXISTS project_runs_log_by_id ON project_runs_log(project_id, id)`,
];

type Row = Record<string, unknown>;

function rowToRun(row: Row): ProjectRun {
  return {
    id: String(row["id"]),
    title: String(row["title"]),
    path: String(row["path"]),
    version: String(row["version"] ?? ""),
    milestone: String(row["milestone"] ?? ""),
    isMilestone: row["is_milestone"] === true,
    leftOpen: row["left_open"] === true,
    sections: Number(row["sections"] ?? 0),
  };
}

export interface RunLogStore {
  replace(projectId: string, runs: readonly ProjectRun[]): Promise<void>;
  list(projectId: string): Promise<ProjectRun[]>;
  counts(projectId: string): Promise<{ total: number; tasks: number; leftOpen: number }>;
  /** Прогон задачи, которой нет в плане. */
  danglingTasks(projectId: string): Promise<ProjectRun[]>;
  /** Закрытая задача без записи о прогоне: сделано, но не рассказано. */
  closedWithoutRun(projectId: string): Promise<{ id: string; title: string }[]>;
  close(): Promise<void>;
}

export function createPostgresRunLogStore(connectionString: string): RunLogStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, runs) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_runs_log WHERE project_id=$1`, [projectId]);
        for (const r of runs) {
          await client.query(
            `INSERT INTO project_runs_log(project_id, id, title, path, version, milestone, is_milestone, left_open, sections)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)`,
            [projectId, r.id, r.title, r.path, r.version, r.milestone, r.isMilestone, r.leftOpen, r.sections],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_runs_log WHERE project_id=$1 ORDER BY path`, [projectId]);
      return rows.map((row) => rowToRun(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS total,
                count(*) FILTER (WHERE NOT is_milestone) AS tasks,
                count(*) FILTER (WHERE left_open)        AS left_open
           FROM project_runs_log WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { total: Number(r["total"]), tasks: Number(r["tasks"]), leftOpen: Number(r["left_open"]) };
    },

    async danglingTasks(projectId) {
      const { rows } = await pg.query(
        `SELECT r.* FROM project_runs_log r
          LEFT JOIN project_plan_tasks t ON t.project_id=r.project_id AND t.id=r.id
         WHERE r.project_id=$1 AND NOT r.is_milestone AND t.id IS NULL ORDER BY r.path`,
        [projectId],
      );
      return rows.map((row) => rowToRun(row as Row));
    },

    async closedWithoutRun(projectId) {
      const { rows } = await pg.query(
        `SELECT t.id, t.title FROM project_plan_tasks t
          WHERE t.project_id=$1 AND t.state='closed' AND NOT EXISTS (
            SELECT 1 FROM project_runs_log r
             WHERE r.project_id=t.project_id AND r.id=t.id AND NOT r.is_milestone)
          ORDER BY t.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return { id: String(r["id"]), title: String(r["title"]) };
      });
    },

    close: () => pg.close(),
  };
}
