import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Словарь предметной области: «Термин | Идентификатор | Что значит». Русское имя
 * и машинное имя — одно понятие, и расхождение между ними ловится только тем,
 * что они лежат рядом. Термины сгруппированы разделами, и раздел — это область.
 */
export interface ProjectTerm {
  /** Машинное имя: `ack`, `handover`. Оно и есть ключ понятия. */
  id: string;
  term: string;
  meaning: string;
  area: string;
  path: string;
}

const HEADER = ["Термин", "Идентификатор", "Что значит"];
// Идентификатор — одно машинное слово, иногда с точкой или дефисом.
const IDENTIFIER = /^\s*`?([A-Za-z][A-Za-z0-9_.-]*)`?\s*$/;

export interface GlossaryCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface GlossarySource {
  path: string;
  cells: readonly GlossaryCell[];
  /** Заголовок раздела над таблицей: «Работа с ситуацией», «Инвентарь»… */
  areaOfBlock: (blockOrd: number) => string;
}

function glossaryBlocks(cells: readonly GlossaryCell[]): Map<number, Map<number, string[]>> {
  const blocks = new Map<number, Map<number, string[]>>();
  for (const cell of cells) {
    const rows = blocks.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    blocks.set(cell.blockOrd, rows);
  }
  // Только таблицы словаря: в документе есть и таблица переименований
  // «Сегодня | Становится | Цена», где то же машинное имя стоит во второй колонке.
  for (const [ord, rows] of blocks) {
    const head = rows.get(0);
    if (!head || !HEADER.every((name, i) => (head[i] ?? "").trim() === name)) blocks.delete(ord);
  }
  return blocks;
}

export function projectGlossary(source: GlossarySource): ProjectTerm[] {
  const blocks = glossaryBlocks(source.cells);

  const terms: ProjectTerm[] = [];
  const seen = new Set<string>();
  for (const [blockOrd, rows] of [...blocks.entries()].sort((a, b) => a[0] - b[0])) {
    const area = source.areaOfBlock(blockOrd);
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const id = IDENTIFIER.exec(row[1] ?? "")?.[1];
      const term = (row[0] ?? "").trim();
      if (!id || !term) continue;
      // Одно и то же машинное имя объявляется один раз; повтор — находка, не строка.
      if (seen.has(id)) continue;
      seen.add(id);
      terms.push({ id, term, meaning: (row[2] ?? "").trim(), area, path: source.path });
    }
  }
  return terms.sort((a, b) => a.id.localeCompare(b.id));
}

/** Повторно объявленные машинные имена — их проекция отбрасывает, но знать о них надо. */
export function duplicateIdentifiers(source: GlossarySource): { id: string; terms: string[] }[] {
  const byId = new Map<string, string[]>();
  for (const rows of glossaryBlocks(source.cells).values()) {
    for (const [ord, row] of rows) {
      if (ord === 0) continue;
      const id = IDENTIFIER.exec(row[1] ?? "")?.[1];
      const term = (row[0] ?? "").trim();
      if (!id || !term) continue;
      byId.set(id, [...(byId.get(id) ?? []), term]);
    }
  }
  return [...byId.entries()]
    .filter(([, terms]) => terms.length > 1)
    .map(([id, terms]) => ({ id, terms }))
    .sort((a, b) => a.id.localeCompare(b.id));
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_terms(
    project_id TEXT NOT NULL, id TEXT NOT NULL, term TEXT NOT NULL,
    meaning TEXT NOT NULL DEFAULT '', area TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_terms_by_term ON project_terms(project_id, term)`,
];

type Row = Record<string, unknown>;

function rowToTerm(row: Row): ProjectTerm & { used: number } {
  return {
    id: String(row["id"]),
    term: String(row["term"]),
    meaning: String(row["meaning"] ?? ""),
    area: String(row["area"] ?? ""),
    path: String(row["path"] ?? ""),
    used: Number(row["used"] ?? 0),
  };
}

export interface GlossaryStore {
  replace(projectId: string, terms: readonly ProjectTerm[]): Promise<void>;
  /** С числом документов, где машинное имя встречается: словарь без употребления мёртв. */
  list(projectId: string): Promise<(ProjectTerm & { used: number })[]>;
  counts(projectId: string): Promise<{ terms: number; areas: number }>;
  /** Термин объявлен, а машинного имени нет нигде, кроме самого словаря. */
  unused(projectId: string): Promise<ProjectTerm[]>;
  /**
   * Докуда термин дотянулся: этапы корпуса, где встречается машинное имя.
   * Словарь ценен не длиной, а охватом — и особенно тем, доходит ли он до места,
   * где поведение закрепляется проверкой.
   */
  reach(projectId: string): Promise<{ id: string; stage: string; documents: number }[]>;
  close(): Promise<void>;
}

/** Употребление считаем по слову целиком: `ack` не должен ловиться внутри `packet`. */
const USED = `(
  SELECT count(*) FROM project_documents d
   WHERE d.project_id = t.project_id AND d.path <> t.path
     AND d.content ~ ('\\y' || t.id || '\\y'))`;

export function createPostgresGlossaryStore(connectionString: string): GlossaryStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, terms) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_terms WHERE project_id=$1`, [projectId]);
        for (const t of terms) {
          await client.query(
            `INSERT INTO project_terms(project_id, id, term, meaning, area, path) VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, t.id, t.term, t.meaning, t.area, t.path],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(
        `SELECT t.*, ${USED} AS used FROM project_terms t WHERE t.project_id=$1 ORDER BY t.id`,
        [projectId],
      );
      return rows.map((row) => rowToTerm(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS terms, count(DISTINCT area) AS areas FROM project_terms WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { terms: Number(r["terms"]), areas: Number(r["areas"]) };
    },

    async unused(projectId) {
      const { rows } = await pg.query(
        `SELECT t.*, 0 AS used FROM project_terms t WHERE t.project_id=$1 AND ${USED} = 0 ORDER BY t.id`,
        [projectId],
      );
      return rows.map((row) => rowToTerm(row as Row));
    },

    async reach(projectId) {
      const { rows } = await pg.query(
        `SELECT t.id,
                CASE
                  WHEN d.path LIKE '00-frame/%'  THEN 'рамка'
                  WHEN d.path LIKE '10-intent/%' THEN 'намерение'
                  WHEN d.path LIKE '20-surface/%' THEN 'поверхность'
                  WHEN d.path LIKE '30-design/%' THEN 'устройство'
                  WHEN d.path LIKE '40-proof/%'  THEN 'проверки'
                  WHEN d.path LIKE '50-plan/%'   THEN 'план'
                  WHEN d.path LIKE '60-runs/%'   THEN 'прогоны'
                  ELSE 'прочее'
                END AS stage,
                count(*) AS documents
           FROM project_terms t
           JOIN project_documents d
             ON d.project_id = t.project_id AND d.path <> t.path
            AND d.content ~ ('\\y' || t.id || '\\y')
          WHERE t.project_id = $1
          GROUP BY t.id, 2
          ORDER BY t.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Record<string, unknown>;
        return { id: String(r["id"]), stage: String(r["stage"]), documents: Number(r["documents"]) };
      });
    },

    close: () => pg.close(),
  };
}
