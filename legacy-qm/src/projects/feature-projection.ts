import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Функциональная область — не папка, а способ сгруппировать истории: документ
 * `10-intent/functional/<область>.md` перечисляет свои истории разделами вида
 * «US-MON-01 · …». Отсюда проверяемо и обратное: история, не попавшая ни в одну
 * область, никем не заявлена как работа.
 */
export interface ProjectFeature {
  id: string;
  title: string;
  path: string;
  /** Сколько историй область объявляет своими. */
  stories: number;
}

export interface FeatureStoryLink {
  featureId: string;
  storyId: string;
}

export interface FeatureProjection {
  features: ProjectFeature[];
  links: FeatureStoryLink[];
}

const FEATURE_PATH = /^10-intent\/functional\/([^/]+)\.md$/;
// Заголовки разделов хранятся без разметки, поэтому обратных кавычек в них нет.
const STORY_IN_TITLE = /^\s*`?(US-[A-Z0-9]+-\d+)`?\b/;

export interface FeatureSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  sectionTitlesOf: (path: string) => readonly string[];
}

export function projectFeatures(source: FeatureSource): FeatureProjection {
  const features: ProjectFeature[] = [];
  const links: FeatureStoryLink[] = [];
  const seen = new Set<string>();

  for (const path of [...source.paths].sort()) {
    const match = FEATURE_PATH.exec(path);
    if (!match || match[1] === "README") continue;
    const id = match[1]!;
    let stories = 0;
    for (const title of source.sectionTitlesOf(path)) {
      const storyId = STORY_IN_TITLE.exec(title)?.[1];
      if (!storyId) continue;
      const key = `${id} ${storyId}`;
      if (seen.has(key)) continue;
      seen.add(key);
      links.push({ featureId: id, storyId });
      stories += 1;
    }
    features.push({ id, title: source.titleOf(path), path, stories });
  }

  return { features, links };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_features(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    stories INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE TABLE IF NOT EXISTS project_feature_stories(
    project_id TEXT NOT NULL, feature_id TEXT NOT NULL, story_id TEXT NOT NULL,
    PRIMARY KEY (project_id, feature_id, story_id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_feature_stories_by_story
    ON project_feature_stories(project_id, story_id)`,
];

type Row = Record<string, unknown>;

export interface FeatureStore {
  replace(projectId: string, projection: FeatureProjection): Promise<void>;
  /** Со сводкой: сколько историй области уже сделано и сколько в работе. */
  list(projectId: string): Promise<(ProjectFeature & { done: number; inProgress: number })[]>;
  counts(projectId: string): Promise<{ features: number; links: number }>;
  /** Область называет историю, которой нет. */
  danglingStories(projectId: string): Promise<FeatureStoryLink[]>;
  /** История, не заявленная ни одной областью: работа без заказчика. */
  unclaimedStories(projectId: string): Promise<{ id: string; title: string }[]>;
  close(): Promise<void>;
}

export function createPostgresFeatureStore(connectionString: string): FeatureStore {
  const pg = createPgPool(connectionString, SCHEMA);
  const linkRows = (rows: unknown[]) =>
    rows.map((row) => {
      const r = row as Row;
      return { featureId: String(r["feature_id"]), storyId: String(r["story_id"]) };
    });

  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_feature_stories WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_features WHERE project_id=$1`, [projectId]);
        for (const f of projection.features) {
          await client.query(
            `INSERT INTO project_features(project_id, id, title, path, stories) VALUES ($1,$2,$3,$4,$5)`,
            [projectId, f.id, f.title, f.path, f.stories],
          );
        }
        for (const l of projection.links) {
          await client.query(
            `INSERT INTO project_feature_stories(project_id, feature_id, story_id) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING`,
            [projectId, l.featureId, l.storyId],
          );
        }
      });
    },

    async list(projectId) {
      // Состояние истории выводится из задач, поэтому сводка области считается тем же путём.
      const { rows } = await pg.query(
        `WITH задачи AS (
           SELECT sr.story_id, t.id AS task_id, t.state
             FROM project_story_requirements sr
             JOIN task_requirement tr
               ON tr.project_id=sr.project_id AND tr.requirement_id=sr.requirement_id
             JOIN project_plan_tasks t ON t.project_id=sr.project_id AND t.id=tr.task_id
            WHERE sr.project_id=$1
            GROUP BY sr.story_id, t.id, t.state),
         свод AS (
           SELECT story_id, count(*) AS всего, count(*) FILTER (WHERE state='closed') AS закрыто
             FROM задачи GROUP BY story_id),
         истории AS (
           SELECT l.feature_id,
                  count(*) FILTER (WHERE c.всего > 0 AND c.закрыто = c.всего)          AS сделано,
                  count(*) FILTER (WHERE c.всего > 0 AND c.закрыто > 0
                                     AND c.закрыто < c.всего)                          AS в_работе
             FROM project_feature_stories l
             LEFT JOIN свод c ON c.story_id = l.story_id
            WHERE l.project_id=$1 GROUP BY l.feature_id)
         SELECT f.*, coalesce(i.сделано, 0) AS done, coalesce(i.в_работе, 0) AS in_progress
           FROM project_features f
           LEFT JOIN истории i ON i.feature_id = f.id
          WHERE f.project_id=$1 ORDER BY f.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          id: String(r["id"]),
          title: String(r["title"]),
          path: String(r["path"]),
          stories: Number(r["stories"] ?? 0),
          done: Number(r["done"] ?? 0),
          inProgress: Number(r["in_progress"] ?? 0),
        };
      });
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT (SELECT count(*) FROM project_features WHERE project_id=$1)        AS features,
                (SELECT count(*) FROM project_feature_stories WHERE project_id=$1) AS links`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { features: Number(r["features"]), links: Number(r["links"]) };
    },

    async danglingStories(projectId) {
      const { rows } = await pg.query(
        `SELECT l.feature_id, l.story_id FROM project_feature_stories l
          LEFT JOIN project_stories s ON s.project_id=l.project_id AND s.id=l.story_id
         WHERE l.project_id=$1 AND s.id IS NULL ORDER BY l.story_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    async unclaimedStories(projectId) {
      const { rows } = await pg.query(
        `SELECT s.id, s.title FROM project_stories s
          WHERE s.project_id=$1 AND NOT EXISTS (
            SELECT 1 FROM project_feature_stories l
             WHERE l.project_id=s.project_id AND l.story_id=s.id)
          ORDER BY s.id`,
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
