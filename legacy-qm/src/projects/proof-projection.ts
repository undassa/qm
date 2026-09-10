import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

export type RequirementKind = "FR" | "NFR";

export interface ProjectRequirement {
  id: string;
  kind: RequirementKind;
  area: string;
  text: string;
  path: string;
  satisfied: boolean;
  /**
   * Приоритет — только у функционального: `О` обязательное, `Ж` желательное, `П` позже.
   * Замер на наборе: О 222 · Ж 42 · П 8. Пусто у нефункциональных — у них колонки нет.
   */
  priority: string;
  /**
   * Способ измерения — только у НЕфункционального: у его таблицы третья колонка «Как
   * измеряется», а не «← ST». Замер: заполнен у всех 32. Формы таблиц разные, и это
   * единственное место, где две ветви расходятся.
   */
  measuredBy: string;
}

/**
 * Родитель-потребность — СВЯЗЬ, а не поле: у 17 требований в ячейке «← ST» стоит больше
 * одного имени (`FR-SIT-09` — `ST-13, ST-43`). Колонка вместила бы первое и молча потеряла
 * остальные.
 */
export interface RequirementNeed {
  requirementId: string;
  needId: string;
}

export interface ProjectCheck {
  id: string;
  area: string;
  requirementId: string | null;
  spec: string;
  path: string;
}

export interface ProofProjection {
  requirements: ProjectRequirement[];
  checks: ProjectCheck[];
  requirementNeeds: RequirementNeed[];
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_requirements(
    project_id TEXT NOT NULL, id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('FR','NFR')),
    area TEXT NOT NULL, text TEXT NOT NULL, path TEXT NOT NULL,
    satisfied BOOLEAN NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
  // Три поля, которые документ несёт, а база теряла: приоритет и способ измерения —
  // колонками, родитель-потребность — таблицей связи (у 17 требований родителей больше
  // одного).
  `ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS priority TEXT NOT NULL DEFAULT ''`,
  `ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS measured_by TEXT NOT NULL DEFAULT ''`,
  `CREATE TABLE IF NOT EXISTS project_requirement_needs(
    project_id TEXT NOT NULL, requirement_id TEXT NOT NULL, need_id TEXT NOT NULL,
    PRIMARY KEY (project_id, requirement_id, need_id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_requirement_needs_by_need
    ON project_requirement_needs(project_id, need_id)`,
  `CREATE TABLE IF NOT EXISTS project_checks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, area TEXT NOT NULL,
    requirement_id TEXT, spec TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_checks_by_requirement
    ON project_checks(project_id, requirement_id)`,
];

const REQUIREMENT_CELL = /^\s*`?((FR|NFR)-([A-Z0-9]+)(?:-\d+[a-z]?)?)`?\s*(.*)$/;
const CHECK_CELL = /^\s*`(TC-([A-Z0-9]+)-\d+[a-z]?)`\s*$/;
const REFERENCED = /`((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)`/;
const SATISFIED = /✓/;
const NEED_REFERENCE = /\bST-(\d+)\b/g;
/** Приоритет записан одной буквой: обязательное · желательное · позже. */
const PRIORITY = /^\s*`?([ОЖП])`?\s*$/;
/**
 * Кто объявляет, а кто цитирует. Тот же идентификатор стоит первой ячейкой и в
 * задачах плана, и в историях — но там это ссылка. Объявление живёт в наборе
 * доказательства, и оно всегда сильнее цитаты, в каком бы порядке ни пришли ячейки.
 */
const DECLARES_CHECK = /^40-proof\//;
const DECLARES_REQUIREMENT = /^10-intent\//;

function declaresOver(path: string, existing: string | undefined, declaring: RegExp): boolean {
  if (existing === undefined) return true;
  return declaring.test(path) && !declaring.test(existing);
}

export interface Cell {
  path: string;
  blockOrd: number;
  row: number;
  col: number;
  raw: string;
  value: string;
}

export function projectProof(cells: readonly Cell[]): ProofProjection {
  const rows = new Map<string, Cell[]>();
  for (const cell of cells) {
    const key = `${cell.path}\0${cell.blockOrd}\0${cell.row}`;
    const bucket = rows.get(key) ?? [];
    bucket.push(cell);
    rows.set(key, bucket);
  }

  const requirements = new Map<string, ProjectRequirement>();
  const checks = new Map<string, ProjectCheck>();
  const requirementNeeds: RequirementNeed[] = [];
  const seenNeed = new Set<string>();

  for (const bucket of rows.values()) {
    const ordered = [...bucket].sort((a, b) => a.col - b.col);
    const first = ordered[0];
    if (!first) continue;

    const check = CHECK_CELL.exec(first.raw);
    if (check && ordered.length >= 2) {
      if (!declaresOver(first.path, checks.get(check[1]!)?.path, DECLARES_CHECK)) continue;
      const requirement = REFERENCED.exec(ordered[1]!.raw)?.[1] ?? null;
      checks.set(check[1]!, {
        id: check[1]!,
        area: check[2]!,
        requirementId: requirement,
        spec: ordered[2]?.value ?? "",
        path: first.path,
      });
      continue;
    }

    const requirement = REQUIREMENT_CELL.exec(first.raw);
    if (!requirement || ordered.length < 2) continue;
    const id = requirement[1]!;
    if (checks.has(id)) continue;
    if (!declaresOver(first.path, requirements.get(id)?.path, DECLARES_REQUIREMENT)) continue;
    const kind = requirement[2] as RequirementKind;
    // Третья колонка значит РАЗНОЕ у двух форм таблицы: у функционального это родитель
    // («← ST»), у нефункционального — способ измерения («Как измеряется»).
    const third = ordered[2]?.value ?? "";
    requirements.set(id, {
      id,
      kind,
      // У нефункционального требования области нет: `NFR-15` — это номер, а не «область 15».
      area: kind === "NFR" ? "" : requirement[3]!,
      text: ordered[1]!.value,
      path: first.path,
      satisfied: SATISFIED.test(requirement[4] ?? ""),
      priority: kind === "FR" ? (PRIORITY.exec(ordered[3]?.value ?? "")?.[1] ?? "") : "",
      measuredBy: kind === "NFR" ? third : "",
    });
    if (kind === "FR") {
      for (const m of third.matchAll(NEED_REFERENCE)) {
        const needId = `ST-${m[1]}`;
        const key = `${id} ${needId}`;
        if (seenNeed.has(key)) continue;
        seenNeed.add(key);
        requirementNeeds.push({ requirementId: id, needId });
      }
    }
  }

  return {
    requirements: [...requirements.values()].sort((a, b) => a.id.localeCompare(b.id)),
    checks: [...checks.values()].sort((a, b) => a.id.localeCompare(b.id)),
    requirementNeeds: requirementNeeds.sort((a, b) => a.requirementId.localeCompare(b.requirementId)),
  };
}

export interface ProofStore {
  replace(projectId: string, projection: ProofProjection): Promise<void>;
  uncoveredRequirements(projectId: string): Promise<string[]>;
  /** Нефункциональные без единой проверки: заявлено, но ничем не доказывается. */
  uncoveredNonFunctional(projectId: string): Promise<{ id: string; text: string }[]>;
  undeclaredReferences(projectId: string): Promise<{ checkId: string; requirementId: string }[]>;
  counts(projectId: string): Promise<{ requirements: number; nfr: number; checks: number; covered: number }>;
  /** Требования с числом покрывающих проверок — по нему видно, что доказано, а что заявлено. */
  listRequirements(projectId: string): Promise<(ProjectRequirement & { checks: number })[]>;
  listChecks(projectId: string): Promise<ProjectCheck[]>;
  /**
   * Имена требований, на которые корпус ссылается, а в наборе их нет. Ссылка на
   * снятое имя гниёт молча: разрешение адреса её не ловит, потому что адреса
   * здесь и нет — есть имя, которому больше ничто не отвечает.
   */
  citedButAbsent(
    projectId: string,
  ): Promise<{ id: string; documents: number; where: string[]; inSrs: boolean }[]>;
  close(): Promise<void>;
}

export function createPostgresProofStore(connectionString: string): ProofStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_checks WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_requirement_needs WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_requirements WHERE project_id=$1`, [projectId]);
        for (const r of projection.requirements) {
          await client.query(
            `INSERT INTO project_requirements(project_id, id, kind, area, text, path, satisfied,
                                              priority, measured_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)`,
            [projectId, r.id, r.kind, r.area, r.text, r.path, r.satisfied, r.priority, r.measuredBy],
          );
        }
        for (const n of projection.requirementNeeds) {
          await client.query(
            `INSERT INTO project_requirement_needs(project_id, requirement_id, need_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING`,
            [projectId, n.requirementId, n.needId],
          );
        }
        for (const c of projection.checks) {
          await client.query(
            `INSERT INTO project_checks(project_id, id, area, requirement_id, spec, path)
             VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, c.id, c.area, c.requirementId, c.spec, c.path],
          );
        }
      });
    },

    async uncoveredRequirements(projectId) {
      const { rows } = await pg.query(
        `SELECT r.id FROM project_requirements r
          LEFT JOIN project_checks c ON c.project_id=r.project_id AND c.requirement_id=r.id
         WHERE r.project_id=$1 AND r.kind='FR' AND c.id IS NULL
         ORDER BY r.id`,
        [projectId],
      );
      return rows.map((row) => String((row as Record<string, unknown>)["id"]));
    },

    async uncoveredNonFunctional(projectId) {
      const { rows } = await pg.query(
        `SELECT r.id, r.text FROM project_requirements r
          WHERE r.project_id=$1 AND r.kind='NFR' AND NOT EXISTS (
            SELECT 1 FROM project_checks c WHERE c.project_id=r.project_id AND c.requirement_id=r.id)
          ORDER BY r.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return { id: String(r["id"]), text: String(r["text"] ?? "") };
      });
    },

    async undeclaredReferences(projectId) {
      const { rows } = await pg.query(
        `SELECT c.id AS check_id, c.requirement_id FROM project_checks c
          LEFT JOIN project_requirements r ON r.project_id=c.project_id AND r.id=c.requirement_id
         WHERE c.project_id=$1 AND c.requirement_id IS NOT NULL AND r.id IS NULL
         ORDER BY c.requirement_id, c.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return { checkId: String(r["check_id"]), requirementId: String(r["requirement_id"]) };
      });
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT
           (SELECT count(*) FROM project_requirements WHERE project_id=$1) AS requirements,
           (SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='NFR') AS nfr,
           (SELECT count(*) FROM project_checks WHERE project_id=$1) AS checks,
           (SELECT count(DISTINCT requirement_id) FROM project_checks
             WHERE project_id=$1 AND requirement_id IS NOT NULL) AS covered`,
        [projectId],
      );
      const r = rows[0] as Record<string, unknown>;
      return {
        requirements: Number(r["requirements"]),
        nfr: Number(r["nfr"]),
        checks: Number(r["checks"]),
        covered: Number(r["covered"]),
      };
    },

    async listRequirements(projectId) {
      const { rows } = await pg.query(
        `SELECT r.*, (SELECT count(*) FROM project_checks c
                       WHERE c.project_id=r.project_id AND c.requirement_id=r.id) AS checks
           FROM project_requirements r WHERE r.project_id=$1 ORDER BY r.kind, r.area, r.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return {
          id: String(r["id"]),
          kind: String(r["kind"]) as RequirementKind,
          area: String(r["area"] ?? ""),
          text: String(r["text"] ?? ""),
          path: String(r["path"] ?? ""),
          satisfied: r["satisfied"] === true,
          checks: Number(r["checks"] ?? 0),
        };
      });
    },

    async listChecks(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_checks WHERE project_id=$1 ORDER BY area, id`, [
        projectId,
      ]);
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return {
          id: String(r["id"]),
          area: String(r["area"] ?? ""),
          requirementId: r["requirement_id"] === null ? null : String(r["requirement_id"]),
          spec: String(r["spec"] ?? ""),
          path: String(r["path"] ?? ""),
        };
      });
    },

    async citedButAbsent(projectId) {
      // Имя требования кончается числом и стоит в обратных кавычках: так его
      // пишет корпус, и так его отличают от прозы.
      const { rows } = await pg.query(
        `WITH cited AS (
           SELECT DISTINCT (regexp_matches(
             d.content, chr(96) || '((?:N?FR)-[A-Z]*-?\\d+)' || chr(96), 'g'))[1] AS id, d.path
             FROM project_documents d WHERE d.project_id=$1)
         SELECT c.id, count(*) AS documents,
                EXISTS (SELECT 1 FROM project_documents s
                         WHERE s.project_id=$1 AND s.path='10-intent/srs.md'
                           AND position(c.id in s.content) > 0) AS in_srs,
                (array_agg(c.path ORDER BY (c.path LIKE '%/.archive/%'), (c.path LIKE '00-frame/decisions-log%'), c.path))[1:3] AS where_
           FROM cited c
          WHERE NOT EXISTS (
            SELECT 1 FROM project_requirements r WHERE r.project_id=$1 AND r.id=c.id)
          GROUP BY c.id ORDER BY count(*) DESC, c.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return {
          id: String(r["id"]),
          documents: Number(r["documents"]),
          where: (r["where_"] as string[]) ?? [],
          inSrs: Boolean(r["in_srs"]),
        };
      });
    },

    close: () => pg.close(),
  };
}
