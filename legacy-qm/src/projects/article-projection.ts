import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Статья конституции — сущность, а не заголовок в тексте: на статьи ссылаются
 * из 314 документов, и без таблицы такую ссылку нечем проверить.
 */
export interface ConstitutionArticle {
  number: number;
  title: string;
  path: string;
  anchor: string;
  /** Тело статьи как есть — по нему её читают, не перечитывая весь документ. */
  body: string;
}

export interface ArticleReference {
  path: string;
  number: number;
}

export interface ArticleProjection {
  articles: ConstitutionArticle[];
  references: ArticleReference[];
}

const ARTICLE_HEADING = /^##\s+Article\s+(\d+)\s*[—–-]\s*(.+?)\s*$/;
const ARTICLE_REFERENCE = /Article\s+(\d+)/g;

export interface ArticleSource {
  /** Документ конституции: путь и содержимое. */
  constitution: { path: string; content: string } | null;
  /** Все документы проекта — по ним считаются ссылки. */
  documents: readonly { path: string; content: string }[];
}

/** Якорь статьи совпадает с якорем раздела документа, чтобы ссылка вела в нужное место. */
export function articleAnchor(number: number, title: string): string {
  return `article-${number}-${title}`
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

export function projectArticles(source: ArticleSource): ArticleProjection {
  const articles: ConstitutionArticle[] = [];

  if (source.constitution) {
    const { path, content } = source.constitution;
    const lines = content.split("\n");
    let current: { number: number; title: string; body: string[] } | null = null;
    const flush = () => {
      if (!current) return;
      articles.push({
        number: current.number,
        title: current.title,
        path,
        anchor: articleAnchor(current.number, current.title),
        body: current.body.join("\n").trim(),
      });
      current = null;
    };
    for (const line of lines) {
      const heading = ARTICLE_HEADING.exec(line);
      if (heading) {
        flush();
        current = { number: Number(heading[1]), title: heading[2]!, body: [] };
        continue;
      }
      // Тело статьи кончается следующим разделом ИЛИ тематическим разрывом.
      //
      // Одного заголовка мало, и это замерено: после Article 16 в конституции myack нет ни
      // одного `##`, дальше идёт `---` и история версий документа. Без разрыва тело статьи
      // забирало её целиком — 7133 символа вместо 1501, — и «что говорит статья 16»
      // отвечалось перечнем версий конституции.
      if (current && (/^##\s/.test(line) || /^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line))) flush();
      else if (current) current.body.push(line);
    }
    flush();
  }

  const references: ArticleReference[] = [];
  const seen = new Set<string>();
  for (const document of source.documents) {
    for (const match of document.content.matchAll(ARTICLE_REFERENCE)) {
      const number = Number(match[1]);
      const key = `${document.path}\0${number}`;
      if (seen.has(key)) continue;
      seen.add(key);
      references.push({ path: document.path, number });
    }
  }

  return { articles: articles.sort((a, b) => a.number - b.number), references };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_articles(
    project_id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL, anchor TEXT NOT NULL, body TEXT NOT NULL,
    PRIMARY KEY (project_id, number)
  )`,
  `CREATE TABLE IF NOT EXISTS project_article_references(
    project_id TEXT NOT NULL, path TEXT NOT NULL, number INTEGER NOT NULL,
    PRIMARY KEY (project_id, path, number)
  )`,
  `CREATE INDEX IF NOT EXISTS project_article_references_by_number
    ON project_article_references(project_id, number)`,
];

type Row = Record<string, unknown>;

export interface ArticleStore {
  replace(projectId: string, projection: ArticleProjection): Promise<void>;
  list(projectId: string): Promise<(ConstitutionArticle & { citedBy: number })[]>;
  /** Ссылки на статьи, которых нет — то, ради чего статьи стали таблицей. */
  danglingReferences(projectId: string): Promise<ArticleReference[]>;
  close(): Promise<void>;
}

export function createPostgresArticleStore(connectionString: string): ArticleStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_article_references WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_articles WHERE project_id=$1`, [projectId]);
        for (const article of projection.articles) {
          await client.query(
            `INSERT INTO project_articles(project_id, number, title, path, anchor, body)
             VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, article.number, article.title, article.path, article.anchor, article.body],
          );
        }
        for (const reference of projection.references) {
          await client.query(
            `INSERT INTO project_article_references(project_id, path, number) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING`,
            [projectId, reference.path, reference.number],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(
        `SELECT a.*, (SELECT count(*) FROM project_article_references r
                       WHERE r.project_id=a.project_id AND r.number=a.number) AS cited
           FROM project_articles a WHERE a.project_id=$1 ORDER BY a.number`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          number: Number(r["number"]),
          title: String(r["title"]),
          path: String(r["path"]),
          anchor: String(r["anchor"]),
          body: String(r["body"]),
          citedBy: Number(r["cited"]),
        };
      });
    },

    async danglingReferences(projectId) {
      const { rows } = await pg.query(
        `SELECT r.path, r.number FROM project_article_references r
          LEFT JOIN project_articles a ON a.project_id=r.project_id AND a.number=r.number
         WHERE r.project_id=$1 AND a.number IS NULL
         ORDER BY r.number, r.path`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return { path: String(r["path"]), number: Number(r["number"]) };
      });
    },

    close: () => pg.close(),
  };
}
