import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * План документов — самоописание корпуса: перечень «Документ | Что содержит |
 * Состояние». Он утверждает две проверяемые вещи: что документ есть (или что его
 * намеренно нет) и сколько в нём чего — «15 статей», «357 TC-nn». Написано это
 * руками, а значит устаревает молча.
 */
export type PlanClaim = "present" | "absent" | "self" | "unknown";

export interface DocumentPlanEntry {
  /** Имя как названо в плане: может быть каталогом и может быть устаревшим. */
  name: string;
  level: string;
  contains: string;
  stateText: string;
  claim: PlanClaim;
  path: string;
}

/** Утверждение о количестве: «есть: 271 требований в 18 подсистемах». */
export interface PlanCount {
  name: string;
  subject: string;
  claimed: number;
}

export interface DocumentPlanProjection {
  entries: DocumentPlanEntry[];
  counts: PlanCount[];
}

const HEADER = ["Документ", "Что содержит", "Состояние"];
// Скобочное уточнение — часть подписи, а не имени: «sdd.md (IEEE 1016)».
const PARENTHETICAL = /\s*\([^)]*\)\s*$/;

/**
 * Предметы, о которых план говорит числом. Список закрытый нарочно: вытаскивать
 * любое число рядом с любым словом значит выдумывать утверждения за документ.
 *
 * Число бывает не переписью, а покрытием: «телом названо 227 требований из 271».
 * Такое считать инвентарём нельзя — это утверждение о контракте, а не о наборе.
 */
export const COUNT_SUBJECTS: { subject: string; pattern: RegExp }[] = [
  { subject: "articles", pattern: /(\d+)\s+стат(?:ья|ьи|ей)/ },
  { subject: "needs", pattern: /(\d+)\s+ST-nn/ },
  { subject: "requirements", pattern: /(\d+)\s+требован/ },
  { subject: "areas", pattern: /(\d+)\s+подсистем/ },
  { subject: "nfr", pattern: /\+\s*(\d+)\s+NFR/ },
  { subject: "screens", pattern: /(\d+)\s+экран/ },
  { subject: "checks", pattern: /(\d+)\s+TC-nn/ },
  { subject: "decisionFiles", pattern: /(\d+)\s+файл/ },
];

const COVERAGE_BEFORE = /(названо|назван|покрыт|описано|из)\s*$/i;
const COVERAGE_AFTER = /^\s*\S*\s*из\s+\d/i;

/** Число внутри оборота о покрытии переписью не считается. */
export function isCoverage(text: string, at: number, length: number): boolean {
  return COVERAGE_BEFORE.test(text.slice(Math.max(0, at - 24), at)) || COVERAGE_AFTER.test(text.slice(at + length));
}

export function claimOf(stateText: string): PlanClaim {
  const head = stateText.trim().toLowerCase();
  if (head.startsWith("этот файл")) return "self";
  if (head.startsWith("есть")) return "present";
  return head.startsWith("нет") ? "absent" : "unknown";
}

/** Имя очищается от скобочного уточнения; несколько имён в ячейке разделены запятой. */
export function namesOf(cell: string): string[] {
  return cell
    .replace(PARENTHETICAL, "")
    .split(",")
    .map((part) => part.trim())
    .filter((part) => /[./]/.test(part));
}

export interface PlanCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface DocumentPlanSource {
  path: string;
  cells: readonly PlanCell[];
  /** Заголовок раздела над таблицей: «Уровень 0 — рамка» и далее. */
  levelOfBlock: (blockOrd: number) => string;
}

export function projectDocumentPlan(source: DocumentPlanSource): DocumentPlanProjection {
  const blocks = new Map<number, Map<number, string[]>>();
  for (const cell of source.cells) {
    const rows = blocks.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    blocks.set(cell.blockOrd, rows);
  }

  const entries: DocumentPlanEntry[] = [];
  const counts: PlanCount[] = [];
  const seen = new Set<string>();

  for (const [blockOrd, rows] of [...blocks.entries()].sort((a, b) => a[0] - b[0])) {
    const head = rows.get(0);
    if (!head || !HEADER.every((name, i) => (head[i] ?? "").trim() === name)) continue;
    const level = source.levelOfBlock(blockOrd);
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const stateText = (row[2] ?? "").trim();
      const claim = claimOf(stateText);
      for (const name of namesOf(row[0] ?? "")) {
        if (seen.has(name)) continue;
        seen.add(name);
        entries.push({
          name,
          level,
          contains: (row[1] ?? "").trim(),
          stateText,
          claim,
          path: source.path,
        });
        // Число живёт и в «Что содержит» («79 ST-nn»), и в «Состояние» («357 TC-nn»).
        for (const { subject, pattern } of COUNT_SUBJECTS) {
          const scan = new RegExp(pattern.source, "g");
          let taken = false;
          for (const text of [row[1] ?? "", stateText]) {
            if (taken) break;
            for (const match of text.matchAll(scan)) {
              if (isCoverage(text, match.index!, match[0]!.length)) continue;
              counts.push({ name, subject, claimed: Number(match[1]) });
              taken = true;
              break;
            }
          }
        }
      }
    }
  }

  return { entries, counts };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_document_plan(
    project_id TEXT NOT NULL, name TEXT NOT NULL, level TEXT NOT NULL DEFAULT '',
    contains TEXT NOT NULL DEFAULT '', state_text TEXT NOT NULL DEFAULT '',
    claim TEXT NOT NULL CHECK (claim IN ('present','absent','self','unknown')),
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, name)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_plan_counts(
    project_id TEXT NOT NULL, name TEXT NOT NULL, subject TEXT NOT NULL, claimed INTEGER NOT NULL,
    PRIMARY KEY (project_id, name, subject)
  )`,
];

type Row = Record<string, unknown>;

export interface PlanResolution extends DocumentPlanEntry {
  /** Сколько документов корпуса откликнулось на это имя. */
  matches: number;
}

export interface DocumentPlanStore {
  replace(projectId: string, projection: DocumentPlanProjection): Promise<void>;
  list(projectId: string): Promise<PlanResolution[]>;
  /** Имя из плана не отзывается ни одним документом: план отстал от дерева. */
  unresolved(projectId: string): Promise<PlanResolution[]>;
  /** Объявлено «нет», а документ есть — план противоречит корпусу. */
  presentButDeclaredAbsent(projectId: string): Promise<PlanResolution[]>;
  counts(projectId: string): Promise<PlanCount[]>;
  /** Заявленное число против посчитанного проекциями. */
  countDrift(projectId: string): Promise<(PlanCount & { actual: number })[]>;
  close(): Promise<void>;
}

/** Имя из плана отзывается документом с тем же хвостом пути; каталог — по префиксу. */
const MATCHES = `(
  SELECT count(*) FROM project_documents d
   WHERE d.project_id = p.project_id
     AND CASE WHEN right(p.name, 1) = '/'
              THEN position('/' || p.name in '/' || d.path) > 0
              ELSE d.path = p.name OR d.path LIKE '%/' || p.name
         END)`;

/**
 * Чем сверяется каждое утверждение о числе. Предмет без записи здесь не
 * проверяется молча — он и не попадёт в расхождения, а останется просто заявленным.
 */
const COUNT_SOURCE: Record<string, string> = {
  articles: `SELECT count(*) FROM project_articles WHERE project_id=$1`,
  needs: `SELECT count(*) FROM project_needs WHERE project_id=$1`,
  requirements: `SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='FR'`,
  areas: `SELECT count(DISTINCT area) FROM project_requirements WHERE project_id=$1 AND kind='FR'`,
  nfr: `SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='NFR'`,
  screens: `SELECT count(*) FROM project_screens WHERE project_id=$1`,
  checks: `SELECT count(*) FROM project_checks WHERE project_id=$1`,
  decisionFiles: `SELECT count(*) FROM project_documents WHERE project_id=$1 AND path LIKE '30-design/decisions/%'`,
};

function rowToResolution(row: Row): PlanResolution {
  return {
    name: String(row["name"]),
    level: String(row["level"] ?? ""),
    contains: String(row["contains"] ?? ""),
    stateText: String(row["state_text"] ?? ""),
    claim: String(row["claim"]) as PlanClaim,
    path: String(row["path"] ?? ""),
    matches: Number(row["matches"] ?? 0),
  };
}

export function createPostgresDocumentPlanStore(connectionString: string): DocumentPlanStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_document_plan_counts WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_document_plan WHERE project_id=$1`, [projectId]);
        for (const e of projection.entries) {
          await client.query(
            `INSERT INTO project_document_plan(project_id, name, level, contains, state_text, claim, path)
             VALUES ($1,$2,$3,$4,$5,$6,$7)`,
            [projectId, e.name, e.level, e.contains, e.stateText, e.claim, e.path],
          );
        }
        for (const c of projection.counts) {
          await client.query(
            `INSERT INTO project_document_plan_counts(project_id, name, subject, claimed) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING`,
            [projectId, c.name, c.subject, c.claimed],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(
        `SELECT p.*, ${MATCHES} AS matches FROM project_document_plan p WHERE p.project_id=$1 ORDER BY p.name`,
        [projectId],
      );
      return rows.map((row) => rowToResolution(row as Row));
    },

    async unresolved(projectId) {
      const { rows } = await pg.query(
        `SELECT p.*, ${MATCHES} AS matches FROM project_document_plan p
          WHERE p.project_id=$1 AND p.claim IN ('present','self') AND ${MATCHES} = 0
          ORDER BY p.name`,
        [projectId],
      );
      return rows.map((row) => rowToResolution(row as Row));
    },

    async presentButDeclaredAbsent(projectId) {
      const { rows } = await pg.query(
        `SELECT p.*, ${MATCHES} AS matches FROM project_document_plan p
          WHERE p.project_id=$1 AND p.claim='absent' AND ${MATCHES} > 0
          ORDER BY p.name`,
        [projectId],
      );
      return rows.map((row) => rowToResolution(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT name, subject, claimed FROM project_document_plan_counts WHERE project_id=$1 ORDER BY name, subject`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return { name: String(r["name"]), subject: String(r["subject"]), claimed: Number(r["claimed"]) };
      });
    },

    async countDrift(projectId) {
      const drift: (PlanCount & { actual: number })[] = [];
      const { rows } = await pg.query(
        `SELECT name, subject, claimed FROM project_document_plan_counts WHERE project_id=$1 ORDER BY name, subject`,
        [projectId],
      );
      for (const row of rows) {
        const r = row as Row;
        const subject = String(r["subject"]);
        const sql = COUNT_SOURCE[subject];
        if (!sql) continue;
        const got = await pg.query(sql, [projectId]);
        const actual = Number((got.rows[0] as Row)["count"]);
        const claimed = Number(r["claimed"]);
        if (actual !== claimed) drift.push({ name: String(r["name"]), subject, claimed, actual });
      }
      return drift;
    },

    close: () => pg.close(),
  };
}
