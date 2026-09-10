import { createPgPool } from "../persistence/pg-pool.ts";

export interface PlanProgress {
  versions: number;
  milestones: number;
  tasks: number;
  closed: number;
  claimed: number;
  ready: number;
  /** Сколько из задач — на написание проверок. */
  tests: number;
}

export interface ProofProgress {
  requirements: number;
  functional: number;
  checks: number;
  /** Покрытие считается по функциональным требованиям — нефункциональные проверками не закрываются. */
  covered: number;
  uncovered: number;
}

export interface MilestoneProgress {
  id: string;
  title: string;
  /** Документ вехи — из него берётся описание рядом с доской. */
  path: string;
  ord: number;
  tasks: number;
  closed: number;
  claimed: number;
}

export interface BoardTask {
  id: string;
  title: string;
  milestoneId: string;
  state: string;
  size: string;
  /** Сколько предшественников ещё не закрыто — задача столько же раз «не готова». */
  blockedBy: number;
  /** Все предшественники, а не только незакрытые: из них строится дерево зависимостей. */
  dependsOn: string[];
  /** Разработка или написание проверок. */
  kind: string;
  /** Что задача трогает: экраны и операции контракта. Общий артефакт — та же зависимость. */
  artifacts: string[];
  /**
   * Чем задача будет доказана: проверок объявлено на её требования. Мост идёт
   * через требование — проверка объявляется на него, а не на задачу. Ноль значит,
   * что задачу нечем закрыть, кроме решения, что она сделана.
   */
  checks: number;
  /** Требований на задаче и сколько из них несёт хоть одну проверку. */
  requirements: number;
  covered: number;
}

export interface TaskCheck {
  id: string;
  spec: string;
}

export interface TaskRequirement {
  id: string;
  kind: string;
  text: string;
  satisfied: boolean;
  checks: TaskCheck[];
}

export interface TaskNeighbour {
  id: string;
  title: string;
  state: string;
}

export interface TaskDetail {
  id: string;
  title: string;
  path: string;
  milestoneId: string;
  kind: string;
  state: string;
  size: string;
  closingCommit: string | null;
  /** Документ прогона — запись сессии разработки; появляется, когда работа началась. */
  runPath: string | null;
  dependsOn: TaskNeighbour[];
  blocks: TaskNeighbour[];
  requirements: TaskRequirement[];
}

export interface GateProgress {
  phase: string;
  item: string;
  /** query — считает машина, signed — закрывает человек подписью. */
  kind: string;
  state: string;
  violations: number;
  signedBy: string | null;
  signedAt: number | null;
  /** Документы, изменившиеся после подписи: пока список непуст — гейт переоткрыт. */
  staleDocuments: string[];
}

export interface ProjectProgress {
  documents: number;
  /** Версия, которую делают сейчас: последняя объявленная в плане. */
  version: string | null;
  plan: PlanProgress | null;
  milestones: MilestoneProgress[];
  proof: ProofProgress | null;
  gates: GateProgress[];
}

export interface ProjectProgressStore {
  read(projectId: string): Promise<ProjectProgress>;
  board(projectId: string): Promise<BoardTask[]>;
  task(projectId: string, taskId: string): Promise<TaskDetail | null>;
  close(): Promise<void>;
}

type Row = Record<string, unknown>;
const num = (value: unknown): number => Number(value ?? 0);

/**
 * Проекции строятся отдельно и могут ещё не существовать: у проекта без плана нет и таблиц.
 * Поэтому каждая часть сводки читается сама по себе, а её отсутствие — это null, а не ошибка.
 */
async function optional<T>(read: () => Promise<T>): Promise<T | null> {
  try {
    return await read();
  } catch {
    return null;
  }
}

export function createPostgresProjectProgressStore(connectionString: string): ProjectProgressStore {
  const pg = createPgPool(connectionString, []);
  return {
    async read(projectId) {
      const documents = await optional(async () => {
        const { rows } = await pg.query(`SELECT count(*) AS n FROM project_documents WHERE project_id=$1`, [projectId]);
        return num((rows[0] as Row)["n"]);
      });

      const plan = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT
             (SELECT count(*) FROM project_plan_versions   WHERE project_id=$1) AS versions,
             (SELECT count(*) FROM project_plan_milestones WHERE project_id=$1) AS milestones,
             (SELECT count(*) FROM project_plan_tasks      WHERE project_id=$1) AS tasks,
             (SELECT count(*) FROM project_plan_tasks      WHERE project_id=$1 AND state='closed')  AS closed,
             (SELECT count(*) FROM project_plan_tasks      WHERE project_id=$1 AND state='claimed') AS claimed,
             (SELECT count(*) FROM project_plan_tasks      WHERE project_id=$1 AND kind='red')      AS tests,
             (SELECT count(*) FROM project_plan_tasks t
               WHERE t.project_id=$1 AND t.state='not_started'
                 AND NOT EXISTS (
                   SELECT 1 FROM project_plan_task_deps d
                     JOIN project_plan_tasks p ON p.project_id=d.project_id AND p.id=d.depends_on
                    WHERE d.project_id=t.project_id AND d.task_id=t.id AND p.state <> 'closed')
                 -- ADR-0082: код не берётся, пока открыта хоть одна проверка
                 AND (t.kind = 'test' OR NOT EXISTS (
                   SELECT 1 FROM project_plan_tasks v
                    WHERE v.project_id=t.project_id AND v.kind='red' AND v.state <> 'closed'))) AS ready`,
          [projectId],
        );
        const r = rows[0] as Row;
        return {
          versions: num(r["versions"]),
          milestones: num(r["milestones"]),
          tasks: num(r["tasks"]),
          closed: num(r["closed"]),
          claimed: num(r["claimed"]),
          tests: num(r["tests"]),
          ready: num(r["ready"]),
        };
      });

      const proof = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT
             (SELECT count(*) FROM project_requirements WHERE project_id=$1) AS requirements,
             (SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='FR') AS functional,
             (SELECT count(*) FROM project_checks       WHERE project_id=$1) AS checks,
             (SELECT count(*) FROM project_requirements r
               WHERE r.project_id=$1 AND r.kind='FR'
                 AND EXISTS (
                   SELECT 1 FROM project_checks c
                    WHERE c.project_id=r.project_id AND c.requirement_id=r.id)) AS covered,
             (SELECT count(*) FROM project_requirements r
               WHERE r.project_id=$1 AND r.kind='FR'
                 AND NOT EXISTS (
                   SELECT 1 FROM project_checks c
                    WHERE c.project_id=r.project_id AND c.requirement_id=r.id)) AS uncovered`,
          [projectId],
        );
        const r = rows[0] as Row;
        return {
          requirements: num(r["requirements"]),
          functional: num(r["functional"]),
          checks: num(r["checks"]),
          covered: num(r["covered"]),
          uncovered: num(r["uncovered"]),
        };
      });

      const gates = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT g.phase, g.item, g.kind, g.state, g.violations, g.signed_by, g.signed_at,
                  COALESCE((SELECT json_agg(s.path ORDER BY s.path)
                     FROM project_gate_signatures s
                     LEFT JOIN project_documents d ON d.project_id=s.project_id AND d.path=s.path
                    WHERE s.project_id=g.project_id AND s.phase=g.phase AND s.item=g.item
                      AND (d.path IS NULL OR d.content_hash <> s.content_hash)), '[]') AS stale
             FROM project_gates g WHERE g.project_id=$1 ORDER BY g.phase, g.item`,
          [projectId],
        );
        return rows.map((row) => {
          const r = row as Row;
          return {
            phase: String(r["phase"]),
            item: String(r["item"]),
            kind: String(r["kind"]),
            state: String(r["state"]),
            violations: num(r["violations"]),
            signedBy: r["signed_by"] === null ? null : String(r["signed_by"]),
            signedAt: r["signed_at"] === null ? null : num(r["signed_at"]),
            staleDocuments: (r["stale"] as string[]) ?? [],
          };
        });
      });

      const version = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT id FROM project_plan_versions WHERE project_id=$1 ORDER BY id DESC LIMIT 1`,
          [projectId],
        );
        return rows[0] ? String((rows[0] as Row)["id"]) : null;
      });

      const milestones = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT m.id, m.title, m.path, m.ord,
                  count(t.id)                                        AS tasks,
                  count(*) FILTER (WHERE t.state = 'closed')         AS closed,
                  count(*) FILTER (WHERE t.state = 'claimed')        AS claimed
             FROM project_plan_milestones m
             LEFT JOIN project_plan_tasks t ON t.project_id = m.project_id AND t.milestone_id = m.id
            WHERE m.project_id = $1
            GROUP BY m.id, m.title, m.path, m.ord
            ORDER BY m.ord`,
          [projectId],
        );
        return rows.map((row) => {
          const r = row as Row;
          return {
            id: String(r["id"]),
            title: String(r["title"]),
            path: String(r["path"]),
            ord: num(r["ord"]),
            tasks: num(r["tasks"]),
            closed: num(r["closed"]),
            claimed: num(r["claimed"]),
          };
        });
      });

      return {
        documents: documents ?? 0,
        version: version ?? null,
        plan,
        milestones: milestones ?? [],
        proof,
        gates: gates ?? [],
      };
    },

    async board(projectId) {
      const rows = await optional(async () => {
        const result = await pg.query(
          `SELECT t.id, t.title, t.milestone_id, t.state, t.size, t.kind,
                  (SELECT count(*) FROM project_plan_task_deps d
                     JOIN project_plan_tasks p ON p.project_id = d.project_id AND p.id = d.depends_on
                    WHERE d.project_id = t.project_id AND d.task_id = t.id AND p.state <> 'closed') AS blocked_by,
                  COALESCE((SELECT json_agg(d.depends_on ORDER BY d.depends_on)
                     FROM project_plan_task_deps d
                    WHERE d.project_id = t.project_id AND d.task_id = t.id), '[]') AS depends_on,
                  COALESCE((SELECT json_agg(DISTINCT f.name || ':' || trim(piece))
                     FROM project_document_fields f,
                          LATERAL unnest(string_to_array(f.value, '·')) AS piece
                    WHERE f.project_id = t.project_id AND f.path = t.path
                      AND f.name IN ('Экраны', 'Операции контракта')
                      -- в поле бывает проза («нет: политики идут мимо HTTP»);
                      -- артефактом считается только настоящий идентификатор
                      AND (trim(piece) ~ '^SCR-[A-Z0-9]+-[0-9]+$'
                        OR trim(piece) ~ '^(GET|POST|PUT|PATCH|DELETE) /')), '[]') AS artifacts,
                  (SELECT count(DISTINCT c.id)
                     FROM task_requirement r
                     JOIN project_checks c
                       ON c.project_id = r.project_id AND c.requirement_id = r.requirement_id
                    WHERE r.project_id = t.project_id AND r.task_id = t.id) AS checks,
                  (SELECT count(*) FROM task_requirement r
                    WHERE r.project_id = t.project_id AND r.task_id = t.id) AS requirements,
                  (SELECT count(*) FROM task_requirement r
                    WHERE r.project_id = t.project_id AND r.task_id = t.id
                      AND EXISTS (SELECT 1 FROM project_checks c
                                   WHERE c.project_id = r.project_id
                                     AND c.requirement_id = r.requirement_id)) AS covered
             FROM project_plan_tasks t
            WHERE t.project_id = $1
            ORDER BY t.ord`,
          [projectId],
        );
        return result.rows.map((row) => {
          const r = row as Row;
          return {
            id: String(r["id"]),
            title: String(r["title"]),
            milestoneId: String(r["milestone_id"]),
            state: String(r["state"]),
            size: String(r["size"] ?? ""),
            blockedBy: num(r["blocked_by"]),
            dependsOn: (r["depends_on"] as string[]) ?? [],
            kind: String(r["kind"] ?? "dev"),
            artifacts: (r["artifacts"] as string[]) ?? [],
            checks: num(r["checks"]),
            requirements: num(r["requirements"]),
            covered: num(r["covered"]),
          };
        });
      });
      return rows ?? [];
    },

    async task(projectId, taskId) {
      const found = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT id, title, path, milestone_id, kind, state, size, closing_commit
             FROM project_plan_tasks WHERE project_id=$1 AND id=$2`,
          [projectId, taskId],
        );
        return (rows[0] as Row | undefined) ?? null;
      });
      if (!found) return null;

      const neighbours = async (sql: string) => {
        const result = await optional(async () => {
          const { rows } = await pg.query(sql, [projectId, taskId]);
          return rows.map((row) => {
            const r = row as Row;
            return { id: String(r["id"]), title: String(r["title"]), state: String(r["state"]) };
          });
        });
        return result ?? [];
      };

      const dependsOn = await neighbours(
        `SELECT t.id, t.title, t.state FROM project_plan_task_deps d
           JOIN project_plan_tasks t ON t.project_id=d.project_id AND t.id=d.depends_on
          WHERE d.project_id=$1 AND d.task_id=$2 ORDER BY t.ord`,
      );
      const blocks = await neighbours(
        `SELECT t.id, t.title, t.state FROM project_plan_task_deps d
           JOIN project_plan_tasks t ON t.project_id=d.project_id AND t.id=d.task_id
          WHERE d.project_id=$1 AND d.depends_on=$2 ORDER BY t.ord`,
      );

      const requirements = await optional(async () => {
        const { rows } = await pg.query(
          `SELECT r.id, r.kind, r.text, r.satisfied,
                  COALESCE(json_agg(json_build_object('id', c.id, 'spec', c.spec)
                    ORDER BY c.id) FILTER (WHERE c.id IS NOT NULL), '[]') AS checks
             FROM task_requirement tr
             JOIN project_requirements r ON r.project_id=tr.project_id AND r.id=tr.requirement_id
             LEFT JOIN project_checks c ON c.project_id=r.project_id AND c.requirement_id=r.id
            WHERE tr.project_id=$1 AND tr.task_id=$2
            GROUP BY r.id, r.kind, r.text, r.satisfied
            ORDER BY r.id`,
          [projectId, taskId],
        );
        return rows.map((row) => {
          const r = row as Row;
          return {
            id: String(r["id"]),
            kind: String(r["kind"]),
            text: String(r["text"]),
            satisfied: r["satisfied"] === true,
            checks: (r["checks"] as TaskCheck[]) ?? [],
          };
        });
      });

      const path = String(found["path"]);
      const candidate = path.replace(/^50-plan\//, "60-runs/");
      const runPath = await optional(async () => {
        const { rows } = await pg.query(`SELECT path FROM project_documents WHERE project_id=$1 AND path=$2`, [
          projectId,
          candidate,
        ]);
        return rows[0] ? candidate : null;
      });

      return {
        id: String(found["id"]),
        title: String(found["title"]),
        path,
        milestoneId: String(found["milestone_id"]),
        kind: String(found["kind"] ?? "dev"),
        state: String(found["state"]),
        size: String(found["size"] ?? ""),
        closingCommit: found["closing_commit"] === null ? null : String(found["closing_commit"]),
        runPath: runPath ?? null,
        dependsOn,
        blocks,
        requirements: requirements ?? [],
      };
    },

    close: () => pg.close(),
  };
}

export function createEmptyProjectProgressStore(): ProjectProgressStore {
  return {
    async read() {
      return { documents: 0, version: null, plan: null, milestones: [], proof: null, gates: [] };
    },
    async board() {
      return [];
    },
    async task() {
      return null;
    },
    async close() {
      // хранить нечего
    },
  };
}
