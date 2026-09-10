import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Матрица трассируемости — единственный документ, который делает проверяемые
 * утверждения обо всём остальном: сколько в подсистеме требований и сколько из них
 * описано проверками. Числа в ней написаны руками, а значит устаревают молча.
 *
 * Часть колонок сверить нечем: «названо телом контракта» и «в коде» говорят про
 * контракт и репозиторий, которых в корпусе нет. Они сохраняются как заявленное,
 * но проверяются только те две, под которыми есть данные.
 */
export interface TraceabilityClaim {
  area: string;
  title: string;
  requirements: number;
  described: number;
  /** «названо телом контракта» — сверить нечем, храним как заявленное. */
  inContract: number | null;
  /** «в коде (домен)» — часто прочерк или пометка вроде «13 красных». */
  inCode: string;
  path: string;
}

const HEADER = ["Подсистема", "FR", "TC описано"];
const TOTAL_ROW = /^\s*(всего|итого)\s*$/i;
// «SIT ситуации» → код подсистемы SIT, дальше человеческое название.
const AREA = /^\s*([A-Z]{2,4})\b\s*(.*)$/;
const COUNT = /^\s*(\d+)/;

export interface TraceabilityRow {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

/** Находит нужную таблицу по её заголовку, а не по порядковому номеру. */
export function projectTraceability(path: string, cells: readonly TraceabilityRow[]): TraceabilityClaim[] {
  const byBlock = new Map<number, Map<number, string[]>>();
  for (const cell of cells) {
    const rows = byBlock.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    byBlock.set(cell.blockOrd, rows);
  }

  const claims: TraceabilityClaim[] = [];
  for (const rows of byBlock.values()) {
    const head = rows.get(0);
    if (!head || !HEADER.every((name, i) => (head[i] ?? "").trim() === name)) continue;
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const first = row[0] ?? "";
      if (TOTAL_ROW.test(first)) continue;
      const area = AREA.exec(first);
      if (!area) continue;
      const num = (value: string | undefined): number | null => {
        const m = COUNT.exec(value ?? "");
        return m ? Number(m[1]) : null;
      };
      claims.push({
        area: area[1]!,
        title: (area[2] ?? "").trim(),
        requirements: num(row[1]) ?? 0,
        described: num(row[2]) ?? 0,
        inContract: num(row[3]),
        inCode: (row[4] ?? "").trim(),
        path,
      });
    }
  }
  return claims.sort((a, b) => a.area.localeCompare(b.area));
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_traceability_claims(
    project_id TEXT NOT NULL, area TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
    requirements INTEGER NOT NULL DEFAULT 0, described INTEGER NOT NULL DEFAULT 0,
    in_contract INTEGER, in_code TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, area)
  )`,
];

type Row = Record<string, unknown>;

/** Что заявлено против того, что посчитано. */
export interface TraceabilityDrift {
  area: string;
  claimedRequirements: number;
  actualRequirements: number;
  claimedDescribed: number;
  actualDescribed: number;
}

export interface TraceabilityStore {
  replace(projectId: string, claims: readonly TraceabilityClaim[]): Promise<void>;
  list(projectId: string): Promise<TraceabilityClaim[]>;
  /** Подсистемы, где заявленное расходится с посчитанным. */
  drift(projectId: string): Promise<TraceabilityDrift[]>;
  /** Подсистема есть в требованиях, но матрица о ней молчит. */
  missingAreas(projectId: string): Promise<string[]>;
  close(): Promise<void>;
}

export function createPostgresTraceabilityStore(connectionString: string): TraceabilityStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, claims) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_traceability_claims WHERE project_id=$1`, [projectId]);
        for (const c of claims) {
          await client.query(
            `INSERT INTO project_traceability_claims(project_id, area, title, requirements, described, in_contract, in_code, path)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)`,
            [projectId, c.area, c.title, c.requirements, c.described, c.inContract, c.inCode, c.path],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_traceability_claims WHERE project_id=$1 ORDER BY area`, [
        projectId,
      ]);
      return rows.map((row) => {
        const r = row as Row;
        return {
          area: String(r["area"]),
          title: String(r["title"] ?? ""),
          requirements: Number(r["requirements"] ?? 0),
          described: Number(r["described"] ?? 0),
          inContract: r["in_contract"] === null ? null : Number(r["in_contract"]),
          inCode: String(r["in_code"] ?? ""),
          path: String(r["path"] ?? ""),
        };
      });
    },

    async drift(projectId) {
      const { rows } = await pg.query(
        `WITH факт AS (
           SELECT r.area,
                  count(*) AS требований,
                  count(*) FILTER (WHERE EXISTS (
                    SELECT 1 FROM project_checks c
                     WHERE c.project_id=r.project_id AND c.requirement_id=r.id)) AS описано
             FROM project_requirements r
            WHERE r.project_id=$1 AND r.kind='FR'
            GROUP BY r.area)
         SELECT t.area, t.requirements AS claimed_requirements, coalesce(f.требований, 0) AS actual_requirements,
                t.described AS claimed_described, coalesce(f.описано, 0) AS actual_described
           FROM project_traceability_claims t
           LEFT JOIN факт f ON f.area = t.area
          WHERE t.project_id=$1
            AND (t.requirements <> coalesce(f.требований, 0) OR t.described <> coalesce(f.описано, 0))
          ORDER BY t.area`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          area: String(r["area"]),
          claimedRequirements: Number(r["claimed_requirements"]),
          actualRequirements: Number(r["actual_requirements"]),
          claimedDescribed: Number(r["claimed_described"]),
          actualDescribed: Number(r["actual_described"]),
        };
      });
    },

    async missingAreas(projectId) {
      const { rows } = await pg.query(
        `SELECT DISTINCT r.area FROM project_requirements r
          WHERE r.project_id=$1 AND r.kind='FR' AND r.area <> '' AND NOT EXISTS (
            SELECT 1 FROM project_traceability_claims t
             WHERE t.project_id=r.project_id AND t.area=r.area)
          ORDER BY r.area`,
        [projectId],
      );
      return rows.map((row) => String((row as Row)["area"]));
    },

    close: () => pg.close(),
  };
}
