import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Риск описан разделом `R-nn · Заголовок`, а под ним таблицей необычной формы:
 * значения влияния и вероятности стоят в самой шапке («В / Вер | высокое /
 * среднее»), а строки ниже — владелец, признак, источник. Принятые как цена
 * решения и закрытые перечислены отдельными таблицами, и там у риска другое
 * состояние — открытым его считать нельзя.
 */
export type RiskState = "open" | "accepted" | "closed";

export interface ProjectRisk {
  id: string;
  number: number;
  title: string;
  state: RiskState;
  impact: string;
  probability: string;
  owner: string;
  /** По чему станет видно, что риск случился. */
  trigger: string;
  source: string;
  /** Чем принят или закрыт: решение, документ. Для открытых пусто. */
  settledBy: string;
  path: string;
}

const RISK_SECTION = /^\s*(R-(\d+))\s*[·—–-]\s*(.+?)\s*$/;
const HEAD = "В / Вер";
const SETTLED_HEADERS = ["Где записан", "Чем закрыт"];
const RISK_ID = /^\s*`?(R-(\d+))`?\s*$/;

export interface RiskCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface RiskSource {
  path: string;
  cells: readonly RiskCell[];
  /** Заголовок раздела над блоком: по нему опознаётся сам риск. */
  sectionOfBlock: (blockOrd: number) => string;
}

function rowsOf(cells: readonly RiskCell[]): Map<number, Map<number, string[]>> {
  const blocks = new Map<number, Map<number, string[]>>();
  for (const cell of cells) {
    const rows = blocks.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    blocks.set(cell.blockOrd, rows);
  }
  return blocks;
}

/** «высокое / среднее» — влияние и вероятность одной ячейкой. */
export function splitImpact(value: string): { impact: string; probability: string } {
  const parts = value.split("/").map((part) => part.trim());
  return { impact: parts[0] ?? "", probability: parts[1] ?? "" };
}

export function projectRisks(source: RiskSource): ProjectRisk[] {
  const blocks = rowsOf(source.cells);
  const risks = new Map<string, ProjectRisk>();

  for (const [blockOrd, rows] of [...blocks.entries()].sort((a, b) => a[0] - b[0])) {
    const head = rows.get(0);
    if (!head) continue;

    // Открытый риск: собственный раздел плюс таблица со шапкой «В / Вер».
    if ((head[0] ?? "").trim() === HEAD) {
      const section = RISK_SECTION.exec(source.sectionOfBlock(blockOrd));
      if (!section) continue;
      const field = (name: string): string => {
        for (const row of rows.values()) {
          if ((row[0] ?? "").trim() === name) return (row[1] ?? "").trim();
        }
        return "";
      };
      const { impact, probability } = splitImpact(head[1] ?? "");
      risks.set(section[1]!, {
        id: section[1]!,
        number: Number(section[2]),
        title: section[3]!,
        state: "open",
        impact,
        probability,
        owner: field("Владелец"),
        trigger: field("Признак"),
        source: field("Источник"),
        settledBy: "",
        path: source.path,
      });
      continue;
    }

    // Улаженные: перечень «Риск | Где записан» или «Риск | Чем закрыт».
    const settledColumn = SETTLED_HEADERS.indexOf((head[2] ?? "").trim());
    if (settledColumn < 0) continue;
    const state: RiskState = (head[2] ?? "").trim() === "Чем закрыт" ? "closed" : "accepted";
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const match = RISK_ID.exec(row[0] ?? "");
      if (!match || risks.has(match[1]!)) continue;
      risks.set(match[1]!, {
        id: match[1]!,
        number: Number(match[2]),
        title: (row[1] ?? "").trim(),
        state,
        impact: "",
        probability: "",
        owner: "",
        trigger: "",
        source: "",
        settledBy: (row[2] ?? "").trim(),
        path: source.path,
      });
    }
  }

  return [...risks.values()].sort((a, b) => a.number - b.number);
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_risks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('open','accepted','closed')),
    impact TEXT NOT NULL DEFAULT '', probability TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL DEFAULT '', trigger_sign TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '', settled_by TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
];

type Row = Record<string, unknown>;

function rowToRisk(row: Row): ProjectRisk {
  return {
    id: String(row["id"]),
    number: Number(row["number"]),
    title: String(row["title"] ?? ""),
    state: String(row["state"]) as RiskState,
    impact: String(row["impact"] ?? ""),
    probability: String(row["probability"] ?? ""),
    owner: String(row["owner"] ?? ""),
    trigger: String(row["trigger_sign"] ?? ""),
    source: String(row["source"] ?? ""),
    settledBy: String(row["settled_by"] ?? ""),
    path: String(row["path"] ?? ""),
  };
}

export interface RiskStore {
  replace(projectId: string, risks: readonly ProjectRisk[]): Promise<void>;
  list(projectId: string): Promise<ProjectRisk[]>;
  counts(projectId: string): Promise<{ total: number; open: number; accepted: number; closed: number }>;
  /** Открытый риск без владельца или без признака: некому смотреть и не по чему. */
  openWithoutOwner(projectId: string): Promise<ProjectRisk[]>;
  /** Улажен решением, которого нет среди решений проекта. */
  settledByMissingDecision(projectId: string): Promise<ProjectRisk[]>;
  close(): Promise<void>;
}

export function createPostgresRiskStore(connectionString: string): RiskStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, risks) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_risks WHERE project_id=$1`, [projectId]);
        for (const r of risks) {
          await client.query(
            `INSERT INTO project_risks(project_id, id, number, title, state, impact, probability, owner, trigger_sign, source, settled_by, path)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)`,
            [
              projectId,
              r.id,
              r.number,
              r.title,
              r.state,
              r.impact,
              r.probability,
              r.owner,
              r.trigger,
              r.source,
              r.settledBy,
              r.path,
            ],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_risks WHERE project_id=$1 ORDER BY number`, [projectId]);
      return rows.map((row) => rowToRisk(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS total,
                count(*) FILTER (WHERE state='open')     AS open,
                count(*) FILTER (WHERE state='accepted') AS accepted,
                count(*) FILTER (WHERE state='closed')   AS closed
           FROM project_risks WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return {
        total: Number(r["total"]),
        open: Number(r["open"]),
        accepted: Number(r["accepted"]),
        closed: Number(r["closed"]),
      };
    },

    async openWithoutOwner(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_risks
          WHERE project_id=$1 AND state='open' AND (owner='' OR trigger_sign='') ORDER BY number`,
        [projectId],
      );
      return rows.map((row) => rowToRisk(row as Row));
    },

    async settledByMissingDecision(projectId) {
      const { rows } = await pg.query(
        `SELECT r.* FROM project_risks r
          WHERE r.project_id=$1 AND r.settled_by ~ 'ADR-[0-9]{3,4}' AND NOT EXISTS (
            SELECT 1 FROM project_decisions d
             WHERE d.project_id=r.project_id
               AND d.id = 'ADR-' || (regexp_match(r.settled_by, 'ADR-([0-9]{3,4})'))[1])
          ORDER BY r.number`,
        [projectId],
      );
      return rows.map((row) => rowToRisk(row as Row));
    },

    close: () => pg.close(),
  };
}
