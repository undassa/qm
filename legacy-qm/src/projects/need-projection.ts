import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Потребность заинтересованной стороны — начало цепочки: потребность → история →
 * требование → проверка → задача. Объявлена таблицами в `10-intent/strs.md`,
 * сгруппованными по темам, а истории называют её в своём разделе.
 */
export type NeedPriority = "must" | "should" | "later" | "unknown";

export interface ProjectNeed {
  id: string;
  number: number;
  text: string;
  /** Кому это нужно: ДЕЖ, КОНФ, РУК… — сторон может быть несколько. */
  sides: string;
  /** Откуда взялось: мокап, полевое исследование, конституция, сверка. */
  sources: string;
  priority: NeedPriority;
  /** Тема — заголовок раздела, под которым стоит таблица. */
  theme: string;
  path: string;
}

/**
 * Потребность названа историей в двух местах: разделом «Потребности» (объявление)
 * и колонкой в таблице требований (цитата). Они расходятся, и это находка,
 * а не повод слить их в одну связь.
 */
export type NeedLinkKind = "declared" | "cited";

export interface NeedStoryLink {
  needId: string;
  storyId: string;
  kind: NeedLinkKind;
}

export interface NeedProjection {
  needs: ProjectNeed[];
  links: NeedStoryLink[];
}

const NEED_ID = /^\s*(ST-(\d+))\s*$/;
const NEED_IN_TEXT = /\b(ST-\d+)\b/g;
export const STORY_NEEDS_SECTION = "Потребности, из которых это выросло";
export const STORY_REQUIREMENTS_SECTION = "Требования, на которые опирается";
const STORY_ID = /^10-intent\/use-cases\/[^/]+\/(US-[A-Z0-9]+-\d+)\.md$/;

/** «О» обязательно · «Ж» желательно · «П» можно после — так объявлено в самом документе. */
export function priorityOf(mark: string): NeedPriority {
  const value = mark.trim();
  if (value === "О") return "must";
  if (value === "Ж") return "should";
  return value === "П" ? "later" : "unknown";
}

export interface NeedCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface NeedSource {
  registerPath: string;
  cells: readonly NeedCell[];
  /** Заголовок раздела, под которым стоит блок: темы «A. Шум и сон» и далее. */
  themeOfBlock: (blockOrd: number) => string;
  storyPaths: readonly string[];
  /** Тело раздела истории с потребностями; из всего документа брать нельзя. */
  needsSectionOf: (path: string) => string;
  /** Тело раздела требований той же истории: там потребность стоит колонкой. */
  requirementsSectionOf?: (path: string) => string;
}

export function projectNeeds(source: NeedSource): NeedProjection {
  const rows = new Map<string, string[]>();
  for (const cell of source.cells) {
    const key = `${cell.blockOrd} ${cell.row}`;
    const row = rows.get(key) ?? [];
    row[cell.col] = cell.value;
    rows.set(key, row);
  }

  const needs = new Map<string, ProjectNeed>();
  for (const [key, row] of rows) {
    const match = NEED_ID.exec(row[0] ?? "");
    if (!match) continue;
    // Тот же ST встречается и в прозе раздела «Открытые вопросы»: объявление — первое.
    if (needs.has(match[1]!)) continue;
    const blockOrd = Number(key.split(" ")[0]);
    needs.set(match[1]!, {
      id: match[1]!,
      number: Number(match[2]),
      text: (row[1] ?? "").trim(),
      sides: (row[2] ?? "").trim(),
      sources: (row[3] ?? "").trim(),
      priority: priorityOf(row[4] ?? ""),
      theme: source.themeOfBlock(blockOrd),
      path: source.registerPath,
    });
  }

  const links: NeedStoryLink[] = [];
  const seen = new Set<string>();
  for (const path of [...source.storyPaths].sort()) {
    const story = STORY_ID.exec(path);
    if (!story) continue;
    const add = (text: string, kind: NeedLinkKind) => {
      for (const m of text.matchAll(NEED_IN_TEXT)) {
        const key = `${m[1]} ${story[1]} ${kind}`;
        if (seen.has(key)) continue;
        seen.add(key);
        links.push({ needId: m[1]!, storyId: story[1]!, kind });
      }
    };
    add(source.needsSectionOf(path), "declared");
    add(source.requirementsSectionOf?.(path) ?? "", "cited");
  }

  return { needs: [...needs.values()].sort((a, b) => a.number - b.number), links };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_needs(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    text TEXT NOT NULL DEFAULT '', sides TEXT NOT NULL DEFAULT '',
    sources TEXT NOT NULL DEFAULT '', theme TEXT NOT NULL DEFAULT '',
    priority TEXT NOT NULL CHECK (priority IN ('must','should','later','unknown')),
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE TABLE IF NOT EXISTS project_need_stories(
    project_id TEXT NOT NULL, need_id TEXT NOT NULL, story_id TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'declared' CHECK (kind IN ('declared','cited')),
    PRIMARY KEY (project_id, need_id, story_id, kind)
  )`,
  // Вид связи добавлен после первой выкладки: CREATE TABLE IF NOT EXISTS её не меняет.
  `ALTER TABLE project_need_stories ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'declared'`,
  `ALTER TABLE project_need_stories DROP CONSTRAINT IF EXISTS project_need_stories_kind_check`,
  `ALTER TABLE project_need_stories ADD CONSTRAINT project_need_stories_kind_check
     CHECK (kind IN ('declared','cited'))`,
  `ALTER TABLE project_need_stories DROP CONSTRAINT IF EXISTS project_need_stories_pkey`,
  `ALTER TABLE project_need_stories ADD PRIMARY KEY (project_id, need_id, story_id, kind)`,
  `CREATE INDEX IF NOT EXISTS project_need_stories_by_story ON project_need_stories(project_id, story_id)`,
];

type Row = Record<string, unknown>;

function linkOf(row: Row): NeedStoryLink {
  return {
    needId: String(row["need_id"]),
    storyId: String(row["story_id"]),
    kind: String(row["kind"]) as NeedLinkKind,
  };
}

function rowToNeed(row: Row): ProjectNeed & { stories: number } {
  return {
    id: String(row["id"]),
    number: Number(row["number"]),
    text: String(row["text"] ?? ""),
    sides: String(row["sides"] ?? ""),
    sources: String(row["sources"] ?? ""),
    priority: String(row["priority"]) as NeedPriority,
    theme: String(row["theme"] ?? ""),
    path: String(row["path"] ?? ""),
    stories: Number(row["stories"] ?? 0),
  };
}

export interface NeedStore {
  replace(projectId: string, projection: NeedProjection): Promise<void>;
  list(projectId: string): Promise<(ProjectNeed & { stories: number })[]>;
  counts(projectId: string): Promise<{ needs: number; links: number; must: number }>;
  /** Потребность, которую не подхватила ни одна история: заявлено и забыто. */
  withoutStory(projectId: string): Promise<ProjectNeed[]>;
  /** История ссылается на потребность, которой нет в реестре. */
  danglingNeeds(projectId: string): Promise<NeedStoryLink[]>;
  /**
   * История называет потребность в таблице требований, но её собственный раздел
   * «Потребности» о ней молчит — документ спорит сам с собой.
   */
  citedButNotDeclared(projectId: string): Promise<NeedStoryLink[]>;
  close(): Promise<void>;
}

export function createPostgresNeedStore(connectionString: string): NeedStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_need_stories WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_needs WHERE project_id=$1`, [projectId]);
        for (const n of projection.needs) {
          await client.query(
            `INSERT INTO project_needs(project_id, id, number, text, sides, sources, theme, priority, path)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)`,
            [projectId, n.id, n.number, n.text, n.sides, n.sources, n.theme, n.priority, n.path],
          );
        }
        for (const l of projection.links) {
          await client.query(
            `INSERT INTO project_need_stories(project_id, need_id, story_id, kind) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING`,
            [projectId, l.needId, l.storyId, l.kind],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(
        `SELECT n.*, (SELECT count(*) FROM project_need_stories l
                       WHERE l.project_id=n.project_id AND l.need_id=n.id
                         AND l.kind='declared') AS stories
           FROM project_needs n WHERE n.project_id=$1 ORDER BY n.number`,
        [projectId],
      );
      return rows.map((row) => rowToNeed(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT (SELECT count(*) FROM project_needs WHERE project_id=$1)                     AS needs,
                (SELECT count(*) FROM project_need_stories WHERE project_id=$1)              AS links,
                (SELECT count(*) FROM project_needs WHERE project_id=$1 AND priority='must') AS must`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { needs: Number(r["needs"]), links: Number(r["links"]), must: Number(r["must"]) };
    },

    async withoutStory(projectId) {
      const { rows } = await pg.query(
        `SELECT n.* FROM project_needs n
          WHERE n.project_id=$1 AND NOT EXISTS (
            SELECT 1 FROM project_need_stories l
             WHERE l.project_id=n.project_id AND l.need_id=n.id AND l.kind='declared')
          ORDER BY n.number`,
        [projectId],
      );
      return rows.map((row) => rowToNeed(row as Row));
    },

    async danglingNeeds(projectId) {
      const { rows } = await pg.query(
        `SELECT DISTINCT l.need_id, l.story_id, l.kind FROM project_need_stories l
          LEFT JOIN project_needs n ON n.project_id=l.project_id AND n.id=l.need_id
         WHERE l.project_id=$1 AND n.id IS NULL ORDER BY l.need_id, l.story_id`,
        [projectId],
      );
      return rows.map((row) => linkOf(row as Row));
    },

    async citedButNotDeclared(projectId) {
      const { rows } = await pg.query(
        `SELECT c.need_id, c.story_id, c.kind FROM project_need_stories c
          WHERE c.project_id=$1 AND c.kind='cited' AND NOT EXISTS (
            SELECT 1 FROM project_need_stories d
             WHERE d.project_id=c.project_id AND d.story_id=c.story_id
               AND d.need_id=c.need_id AND d.kind='declared')
          ORDER BY c.story_id, c.need_id`,
        [projectId],
      );
      return rows.map((row) => linkOf(row as Row));
    },

    close: () => pg.close(),
  };
}
