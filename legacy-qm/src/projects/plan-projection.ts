import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

export type TaskState = "not_started" | "claimed" | "closed";

/**
 * Вид задачи: разработка или написание проверок. В документе это поле «Вид»;
 * пока его нет — задача считается разработческой, потому что так было до разделения.
 */
export type TaskKind = "dev" | "test";

const TEST_WORDS = /^\s*(тест|тесты|проверк|test)/i;
/** Трек проверок живёт в своём пространстве идентификаторов: `V3-T17` — тесты к `M3-T17`. */
const TEST_ID = /^V\d/;

/** Поле «Вид» сильнее: им можно переопределить то, что говорит идентификатор. */
export function kindOf(field: string | undefined, taskId = ""): TaskKind {
  if (field && field.trim()) return TEST_WORDS.test(field) ? "test" : "dev";
  return TEST_ID.test(taskId) ? "test" : "dev";
}

export interface PlanVersion {
  id: string;
  path: string;
}

export interface PlanMilestone {
  id: string;
  versionId: string;
  ord: number;
  title: string;
  path: string;
}

export interface PlanTask {
  id: string;
  milestoneId: string;
  ord: number;
  title: string;
  path: string;
  size: string;
  kind: TaskKind;
  state: TaskState;
  closingCommit: string | null;
}

export interface PlanProjection {
  versions: PlanVersion[];
  milestones: PlanMilestone[];
  tasks: PlanTask[];
  dependencies: { taskId: string; dependsOn: string }[];
  requirements: { taskId: string; requirementId: string }[];
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_plan_versions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE TABLE IF NOT EXISTS project_plan_milestones(
    project_id TEXT NOT NULL, id TEXT NOT NULL, version_id TEXT NOT NULL,
    ord INTEGER NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, version_id) REFERENCES project_plan_versions(project_id, id) ON DELETE CASCADE
  )`,
  `CREATE TABLE IF NOT EXISTS project_plan_tasks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, milestone_id TEXT NOT NULL,
    ord INTEGER NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL, size TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('not_started','claimed','closed')),
    kind TEXT NOT NULL DEFAULT 'dev' CHECK (kind IN ('dev','test')),
    closing_commit TEXT,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, milestone_id) REFERENCES project_plan_milestones(project_id, id) ON DELETE CASCADE
  )`,
  `ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'dev'`,
  `CREATE TABLE IF NOT EXISTS project_plan_task_deps(
    project_id TEXT NOT NULL, task_id TEXT NOT NULL, depends_on TEXT NOT NULL,
    PRIMARY KEY (project_id, task_id, depends_on),
    CHECK (task_id <> depends_on),
    FOREIGN KEY (project_id, task_id) REFERENCES project_plan_tasks(project_id, id) ON DELETE CASCADE
  )`,
];

const VERSION_PATH = /^50-plan\/(v\d+)\.md$/;
const MILESTONE_PATH = /^50-plan\/(v\d+)\/([MV]\d+)\.md$/;
const TASK_PATH = /^50-plan\/(v\d+)\/([MV]\d+)\/([MV]\d+-T[0-9a-z]+)\.md$/;
const IDENTIFIER = /`([A-Z]+\d*-[A-Za-z0-9-]+)`/g;
const COMMIT = /\b([0-9a-f]{7,40})\b/;

export function stateOf(field: string | undefined): { state: TaskState; commit: string | null } {
  const head = (field ?? "").split("·")[0]!.trim().toLowerCase();
  const commit = COMMIT.exec(field ?? "")?.[1] ?? null;
  if (head.startsWith("закрыт") || head.startsWith("closed")) return { state: "closed", commit };
  if (head.startsWith("в работе") || head.startsWith("claimed")) return { state: "claimed", commit: null };
  return { state: "not_started", commit: null };
}

export function identifiersIn(value: string): string[] {
  return [...value.matchAll(IDENTIFIER)].map((m) => m[1]!);
}

export interface ProjectionSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  fieldsOf: (path: string) => ReadonlyMap<string, string>;
}

export function projectPlan(source: ProjectionSource): PlanProjection {
  const versions: PlanVersion[] = [];
  const milestones: PlanMilestone[] = [];
  const tasks: PlanTask[] = [];
  const dependencies: { taskId: string; dependsOn: string }[] = [];
  const requirements: { taskId: string; requirementId: string }[] = [];

  for (const path of [...source.paths].sort()) {
    const version = VERSION_PATH.exec(path);
    if (version) {
      versions.push({ id: version[1]!, path });
      continue;
    }
    const milestone = MILESTONE_PATH.exec(path);
    if (milestone) {
      milestones.push({
        id: milestone[2]!,
        versionId: milestone[1]!,
        ord: milestones.length,
        title: source.titleOf(path),
        path,
      });
      continue;
    }
    const task = TASK_PATH.exec(path);
    if (!task) continue;
    const fields = source.fieldsOf(path);
    const { state, commit } = stateOf(fields.get("Состояние"));
    tasks.push({
      id: task[3]!,
      milestoneId: task[2]!,
      ord: tasks.length,
      title: source.titleOf(path),
      path,
      size: fields.get("Размер") ?? "",
      kind: kindOf(fields.get("Вид"), task[3]!),
      state,
      closingCommit: commit,
    });
    for (const dependency of identifiersIn(fields.get("Зависит от") ?? "")) {
      dependencies.push({ taskId: task[3]!, dependsOn: dependency });
    }
    for (const requirement of identifiersIn(fields.get("Требования") ?? "")) {
      requirements.push({ taskId: task[3]!, requirementId: requirement });
    }
  }
  return { versions, milestones, tasks, dependencies, requirements };
}

export interface PlanStore {
  replace(projectId: string, projection: PlanProjection): Promise<void>;
  readyTasks(projectId: string): Promise<string[]>;
  counts(
    projectId: string,
  ): Promise<{ versions: number; milestones: number; tasks: number; closed: number; tests: number }>;
  close(): Promise<void>;
}

export function createPostgresPlanStore(connectionString: string): PlanStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_plan_versions WHERE project_id=$1`, [projectId]);
        for (const version of projection.versions) {
          await client.query(`INSERT INTO project_plan_versions(project_id, id, path) VALUES ($1,$2,$3)`, [
            projectId,
            version.id,
            version.path,
          ]);
        }
        for (const milestone of projection.milestones) {
          await client.query(
            `INSERT INTO project_plan_milestones(project_id, id, version_id, ord, title, path) VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, milestone.id, milestone.versionId, milestone.ord, milestone.title, milestone.path],
          );
        }
        for (const task of projection.tasks) {
          await client.query(
            `INSERT INTO project_plan_tasks(project_id, id, milestone_id, ord, title, path, size, kind, state, closing_commit)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)`,
            [
              projectId,
              task.id,
              task.milestoneId,
              task.ord,
              task.title,
              task.path,
              task.size,
              task.kind,
              task.state,
              task.closingCommit,
            ],
          );
        }
        const known = new Set(projection.tasks.map((task) => task.id));
        for (const dependency of projection.dependencies) {
          if (!known.has(dependency.dependsOn) || dependency.taskId === dependency.dependsOn) continue;
          await client.query(
            `INSERT INTO project_plan_task_deps(project_id, task_id, depends_on) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING`,
            [projectId, dependency.taskId, dependency.dependsOn],
          );
        }
        // ФОРК. Связь задача→требование здесь БОЛЬШЕ НЕ ПИШЕТСЯ. Её считает
        // сервер (`task_requirement`), и по правилу «из объяснения отсутствия
        // связи не добываются»: разбор поля вытаскивал имена из фразы «нет
        // собственных; исполняет решения Q-277 и Q-282» и записывал их
        // требованиями задачи — 86 связей из 52 таких фраз, 23 из них
        // неотличимые от настоящих. Два писателя в один факт расходятся молча.
      });
    },

    async readyTasks(projectId) {
      // ADR-0082: весь трек проверок предшествует всему коду. Пока открыта хоть одна
      // проверка, ни одна задача кода не готова — сколько бы её собственные зависимости
      // ни были закрыты. Правило сильнее пары «проверка за своей задачей».
      const { rows } = await pg.query(
        `SELECT t.id FROM project_plan_tasks t
          WHERE t.project_id=$1 AND t.state='not_started'
            AND NOT EXISTS (
              SELECT 1 FROM project_plan_task_deps d
                JOIN project_plan_tasks p ON p.project_id=d.project_id AND p.id=d.depends_on
               WHERE d.project_id=t.project_id AND d.task_id=t.id AND p.state <> 'closed')
            AND (t.kind = 'test' OR NOT EXISTS (
              SELECT 1 FROM project_plan_tasks v
               WHERE v.project_id=t.project_id AND v.kind='red' AND v.state <> 'closed'))
          ORDER BY t.ord`,
        [projectId],
      );
      return rows.map((row) => String((row as Record<string, unknown>)["id"]));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT
           (SELECT count(*) FROM project_plan_versions WHERE project_id=$1)  AS versions,
           (SELECT count(*) FROM project_plan_milestones WHERE project_id=$1) AS milestones,
           (SELECT count(*) FROM project_plan_tasks WHERE project_id=$1)      AS tasks,
           (SELECT count(*) FROM project_plan_tasks WHERE project_id=$1 AND state='closed') AS closed,
           (SELECT count(*) FROM project_plan_tasks WHERE project_id=$1 AND kind='red') AS tests`,
        [projectId],
      );
      const r = rows[0] as Record<string, unknown>;
      return {
        versions: Number(r["versions"]),
        milestones: Number(r["milestones"]),
        tasks: Number(r["tasks"]),
        closed: Number(r["closed"]),
        tests: Number(r["tests"]),
      };
    },

    close: () => pg.close(),
  };
}
