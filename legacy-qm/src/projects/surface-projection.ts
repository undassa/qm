import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Истории и экраны разбираются вместе: история называет экраны, которых касается,
 * и задача тоже. Пока экран — заголовок в документе, такую ссылку проверить нечем.
 */
export interface ProjectStory {
  id: string;
  title: string;
  path: string;
  /** Область пути: каталог, в котором лежит история. */
  area: string;
  persona: string;
  phase: string;
  feature: string;
}

export interface ProjectScreen {
  id: string;
  title: string;
  path: string;
  area: string;
}

/** Кто на какой экран ссылается: история или задача. */
export interface ScreenReference {
  source: string;
  sourceKind: "story" | "task";
  screenId: string;
}

/** На какие требования опирается история — из раздела, а не из поля. */
export interface StoryRequirement {
  storyId: string;
  requirementId: string;
}

export const STORY_REQUIREMENTS_SECTION = "Требования, на которые опирается";

/**
 * Первые ячейки строк таблицы, склеенные через перевод строки: место, где идентификатор
 * объявлен, а не помянут. Строки не-таблицы и разделитель `|---|` отбрасываются.
 */
function declaredColumn(body: string): string {
  const out: string[] = [];
  for (const line of body.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("|") || /^[\s|:-]+$/.test(trimmed)) continue;
    const first = trimmed.slice(1).split("|")[0];
    if (first !== undefined) out.push(first.trim());
  }
  return out.join("\n");
}
const REQUIREMENT_ID = /\b((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)\b/g;

export interface SurfaceProjection {
  stories: ProjectStory[];
  screens: ProjectScreen[];
  references: ScreenReference[];
  storyRequirements: StoryRequirement[];
}

const STORY_PATH = /^10-intent\/use-cases\/([^/]+)\/(US-[A-Z0-9]+-\d+)\.md$/;
const SCREEN_PATH = /^20-surface\/([^/]+)\/([^/]+)\.md$/;
// Заголовки секций хранятся без разметки: обратных кавычек в них уже нет.
const SCREEN_IN_TITLE = /^\s*`?(SCR-[A-Z0-9]+-\d+)`?\b/;
const SCREEN_ID = /\b(SCR-[A-Z0-9]+-\d+)\b/g;
const TASK_PATH = /^50-plan\/v\d+\/[MV]\d+\/([MV]\d+-T[0-9a-z]+)\.md$/;
// Элементы оболочки описаны разделами внутри одного документа, а не по документу на экран.
const COLLECTIVE_SURFACE = /^20-surface\/([^/]+)\.md$/;

export interface SurfaceSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  fieldsOf: (path: string) => ReadonlyMap<string, string>;
  /** Заголовки всех разделов: часть экранов описана не документом, а разделом внутри общего. */
  sectionTitlesOf: (path: string) => readonly string[];
  /**
   * Тело раздела по заголовку. Требования, на которые опирается история, объявлены
   * разделом, а не полем, и брать их из всего документа нельзя: соседние разделы
   * называют проверки и экраны, и они бы примешались.
   */
  sectionBodyOf?: (path: string, title: string) => string;
}

export function projectSurface(source: SurfaceSource): SurfaceProjection {
  const stories: ProjectStory[] = [];
  const screens: ProjectScreen[] = [];
  const references: ScreenReference[] = [];
  const storyRequirements: StoryRequirement[] = [];
  const seen = new Set<string>();
  const seenRequirement = new Set<string>();

  const cite = (source_: string, kind: ScreenReference["sourceKind"], text: string) => {
    for (const match of text.matchAll(SCREEN_ID)) {
      const key = `${source_} ${match[1]}`;
      if (seen.has(key)) continue;
      seen.add(key);
      references.push({ source: source_, sourceKind: kind, screenId: match[1]! });
    }
  };

  for (const path of [...source.paths].sort()) {
    const story = STORY_PATH.exec(path);
    if (story) {
      const fields = source.fieldsOf(path);
      stories.push({
        id: story[2]!,
        title: source.titleOf(path),
        path,
        area: story[1]!,
        persona: (fields.get("Персона") ?? "").trim(),
        phase: (fields.get("Фаза пути") ?? "").trim(),
        feature: (fields.get("Фича") ?? "").trim(),
      });
      cite(story[2]!, "story", fields.get("Экраны") ?? "");
      // Требование ОБЪЯВЛЕНО первой ячейкой строки таблицы; всё остальное в разделе —
      // упоминание. Разница не косметическая: `US-GAP-03` называет `FR-GAP-06`, а внутри
      // его описания стоит «Недостижимая цепочка (FR-ESC-05)», и обход всего тела заводил
      // историю на требование, которое она лишь поминает. Так же попались `US-INV-02` и
      // `US-SCH-03`. Ту же границу «объявлен против упомянут» набор проводит и сам.
      const body = declaredColumn(source.sectionBodyOf?.(path, STORY_REQUIREMENTS_SECTION) ?? "");
      for (const m of body.matchAll(REQUIREMENT_ID)) {
        const key = `${story[2]} ${m[1]}`;
        if (seenRequirement.has(key)) continue;
        seenRequirement.add(key);
        storyRequirements.push({ storyId: story[2]!, requirementId: m[1]! });
      }
      continue;
    }

    const screen = SCREEN_PATH.exec(path);
    if (screen && screen[2] !== "README") {
      // Идентификатор экрана стоит в заголовке, а не в имени файла.
      const id = SCREEN_IN_TITLE.exec(source.titleOf(path))?.[1];
      if (id) screens.push({ id, title: source.titleOf(path), path, area: screen[1]! });
      continue;
    }

    const collective = COLLECTIVE_SURFACE.exec(path);
    if (collective && collective[1] !== "README") {
      for (const title of source.sectionTitlesOf(path)) {
        const id = SCREEN_IN_TITLE.exec(title)?.[1];
        if (id) screens.push({ id, title, path, area: collective[1]! });
      }
      continue;
    }

    const task = TASK_PATH.exec(path);
    if (task) cite(task[1]!, "task", source.fieldsOf(path).get("Экраны") ?? "");
  }

  return {
    stories: stories.sort((a, b) => a.id.localeCompare(b.id)),
    screens: screens.sort((a, b) => a.id.localeCompare(b.id)),
    references,
    storyRequirements,
  };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_stories(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    area TEXT NOT NULL DEFAULT '', persona TEXT NOT NULL DEFAULT '',
    phase TEXT NOT NULL DEFAULT '', feature TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE TABLE IF NOT EXISTS project_screens(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    area TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE TABLE IF NOT EXISTS project_screen_references(
    project_id TEXT NOT NULL, source TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('story','task')), screen_id TEXT NOT NULL,
    PRIMARY KEY (project_id, source, screen_id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_screen_references_by_screen
    ON project_screen_references(project_id, screen_id)`,
  `CREATE TABLE IF NOT EXISTS project_story_requirements(
    project_id TEXT NOT NULL, story_id TEXT NOT NULL, requirement_id TEXT NOT NULL,
    PRIMARY KEY (project_id, story_id, requirement_id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_story_requirements_by_requirement
    ON project_story_requirements(project_id, requirement_id)`,
];

/**
 * Состояние истории не объявлено нигде — оно выводится. История опирается на
 * требования, требования называет задача, у задачи есть состояние. Поэтому:
 * сделана, когда все её требования взяты закрытыми задачами; в работе, когда
 * хоть одна взята; запланирована, когда задачи есть, но ни одна не начата;
 * не запланирована, когда ни одна задача её требований не называет. Отдельно —
 * история, которая не назвала требований вовсе: это пробел в самом корпусе,
 * а не в плане, и смешивать их значит прятать одну беду за другой.
 */
export type StoryState = "done" | "in-progress" | "planned" | "unplanned" | "no-requirements";

type Row = Record<string, unknown>;

/**
 * Задача связана с историей через требования: обе их называют. Прямой ссылки
 * «задача → история» в корпусе нет, и придумывать её нельзя.
 */
const STORY_STATE_SQL = `
  WITH задачи AS (
    SELECT sr.story_id, t.id AS task_id, t.state
      FROM project_story_requirements sr
      JOIN task_requirement tr
        ON tr.project_id = sr.project_id AND tr.requirement_id = sr.requirement_id
      JOIN project_plan_tasks t
        ON t.project_id = sr.project_id AND t.id = tr.task_id
     WHERE sr.project_id = $1
     GROUP BY sr.story_id, t.id, t.state
  ),
  свои AS (
    SELECT story_id, count(*) AS требований FROM project_story_requirements
     WHERE project_id = $1 GROUP BY story_id
  ),
  свод AS (
    SELECT story_id, count(*) AS всего,
           count(*) FILTER (WHERE state = 'closed')       AS закрыто,
           count(*) FILTER (WHERE state NOT IN ('closed','not_started')) AS в_работе
      FROM задачи GROUP BY story_id
  )
  SELECT s.*, coalesce(c.всего, 0) AS tasks, coalesce(c.закрыто, 0) AS done,
         CASE
           WHEN coalesce(r.требований, 0) = 0       THEN 'no-requirements'
           WHEN coalesce(c.всего, 0) = 0            THEN 'unplanned'
           WHEN c.закрыто = c.всего                 THEN 'done'
           WHEN coalesce(c.в_работе, 0) > 0 OR c.закрыто > 0 THEN 'in-progress'
           ELSE 'planned'
         END AS state
    FROM project_stories s
    LEFT JOIN свод c ON c.story_id = s.id
    LEFT JOIN свои r ON r.story_id = s.id
   WHERE s.project_id = $1`;

function rowToStory(row: Row): ProjectStory & { state: StoryState; tasks: number; done: number } {
  return {
    id: String(row["id"]),
    title: String(row["title"]),
    path: String(row["path"]),
    area: String(row["area"] ?? ""),
    persona: String(row["persona"] ?? ""),
    phase: String(row["phase"] ?? ""),
    feature: String(row["feature"] ?? ""),
    state: String(row["state"]) as StoryState,
    tasks: Number(row["tasks"] ?? 0),
    done: Number(row["done"] ?? 0),
  };
}

export interface SurfaceStore {
  replace(projectId: string, projection: SurfaceProjection): Promise<void>;
  counts(projectId: string): Promise<{ stories: number; screens: number; references: number }>;
  /** Ссылка на экран, которого нет ни в одном документе поверхности. */
  danglingScreens(projectId: string): Promise<ScreenReference[]>;
  /** Экраны, на которые не ссылается ни история, ни задача: нарисован и забыт. */
  orphanScreens(projectId: string): Promise<ProjectScreen[]>;
  listStories(projectId: string): Promise<(ProjectStory & { state: StoryState; tasks: number; done: number })[]>;
  /** Истории области с выведенным состоянием — для раскрытия области в каталоге. */
  storiesOfFeature(
    projectId: string,
    featureId: string,
  ): Promise<(ProjectStory & { state: StoryState; tasks: number; done: number })[]>;
  /** Экраны с числом ссылок: по нему видно, какой экран несёт вес, а какой забыт. */
  listScreens(projectId: string): Promise<(ProjectScreen & { citedBy: number })[]>;
  /** Кто ссылается на экран — истории и задачи вместе. */
  screenReferences(projectId: string, screenId: string): Promise<ScreenReference[]>;
  /** Все ссылки на экраны разом: страница поверхности показывает их вместе. */
  allScreenReferences(projectId: string): Promise<ScreenReference[]>;
  close(): Promise<void>;
}

export function createPostgresSurfaceStore(connectionString: string): SurfaceStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        for (const table of [
          "project_screen_references",
          "project_story_requirements",
          "project_stories",
          "project_screens",
        ]) {
          await client.query(`DELETE FROM ${table} WHERE project_id=$1`, [projectId]);
        }
        for (const s of projection.stories) {
          await client.query(
            `INSERT INTO project_stories(project_id, id, title, path, area, persona, phase, feature)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)`,
            [projectId, s.id, s.title, s.path, s.area, s.persona, s.phase, s.feature],
          );
        }
        for (const s of projection.screens) {
          await client.query(`INSERT INTO project_screens(project_id, id, title, path, area) VALUES ($1,$2,$3,$4,$5)`, [
            projectId,
            s.id,
            s.title,
            s.path,
            s.area,
          ]);
        }
        for (const sr of projection.storyRequirements) {
          await client.query(
            `INSERT INTO project_story_requirements(project_id, story_id, requirement_id) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING`,
            [projectId, sr.storyId, sr.requirementId],
          );
        }
        for (const r of projection.references) {
          await client.query(
            `INSERT INTO project_screen_references(project_id, source, source_kind, screen_id)
             VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING`,
            [projectId, r.source, r.sourceKind, r.screenId],
          );
        }
      });
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT (SELECT count(*) FROM project_stories WHERE project_id=$1)            AS stories,
                (SELECT count(*) FROM project_screens WHERE project_id=$1)            AS screens,
                (SELECT count(*) FROM project_screen_references WHERE project_id=$1)  AS refs`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { stories: Number(r["stories"]), screens: Number(r["screens"]), references: Number(r["refs"]) };
    },

    async danglingScreens(projectId) {
      const { rows } = await pg.query(
        `SELECT r.source, r.source_kind, r.screen_id FROM project_screen_references r
          LEFT JOIN project_screens s ON s.project_id=r.project_id AND s.id=r.screen_id
         WHERE r.project_id=$1 AND s.id IS NULL ORDER BY r.screen_id, r.source`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          source: String(r["source"]),
          sourceKind: String(r["source_kind"]) as ScreenReference["sourceKind"],
          screenId: String(r["screen_id"]),
        };
      });
    },

    async listStories(projectId) {
      const { rows } = await pg.query(`${STORY_STATE_SQL} ORDER BY s.id`, [projectId]);
      return rows.map((row) => rowToStory(row as Row));
    },

    async storiesOfFeature(projectId, featureId) {
      const { rows } = await pg.query(
        `${STORY_STATE_SQL} AND s.id IN (
           SELECT story_id FROM project_feature_stories
            WHERE project_id=$1 AND feature_id=$2)
         ORDER BY s.id`,
        [projectId, featureId],
      );
      return rows.map((row) => rowToStory(row as Row));
    },

    async listScreens(projectId) {
      const { rows } = await pg.query(
        `SELECT s.*, (SELECT count(*) FROM project_screen_references r
                       WHERE r.project_id=s.project_id AND r.screen_id=s.id) AS cited_by
           FROM project_screens s WHERE s.project_id=$1 ORDER BY s.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          id: String(r["id"]),
          title: String(r["title"]),
          path: String(r["path"]),
          area: String(r["area"] ?? ""),
          citedBy: Number(r["cited_by"] ?? 0),
        };
      });
    },

    async allScreenReferences(projectId) {
      const { rows } = await pg.query(
        `SELECT source, source_kind, screen_id FROM project_screen_references
          WHERE project_id=$1 ORDER BY screen_id, source_kind, source`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          source: String(r["source"]),
          sourceKind: String(r["source_kind"]) as ScreenReference["sourceKind"],
          screenId: String(r["screen_id"]),
        };
      });
    },

    async screenReferences(projectId, screenId) {
      const { rows } = await pg.query(
        `SELECT source, source_kind, screen_id FROM project_screen_references
          WHERE project_id=$1 AND screen_id=$2 ORDER BY source_kind, source`,
        [projectId, screenId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          source: String(r["source"]),
          sourceKind: String(r["source_kind"]) as ScreenReference["sourceKind"],
          screenId: String(r["screen_id"]),
        };
      });
    },

    async orphanScreens(projectId) {
      const { rows } = await pg.query(
        `SELECT s.* FROM project_screens s
          WHERE s.project_id=$1 AND NOT EXISTS (
            SELECT 1 FROM project_screen_references r
             WHERE r.project_id=s.project_id AND r.screen_id=s.id)
          ORDER BY s.id`,
        [projectId],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          id: String(r["id"]),
          title: String(r["title"]),
          path: String(r["path"]),
          area: String(r["area"]),
        };
      });
    },

    close: () => pg.close(),
  };
}
