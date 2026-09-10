import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";
import { classifyAnswer, sectionOf, type AnswerKind, type AnswerVerdict } from "./question-answer.ts";

/**
 * Состояние вопроса объявлено в самом документе полем «Состояние», а не выводится
 * из наличия раздела «Ответ»: «решено» и «закрыт» — разные вещи, и признак закрытия
 * у части вопросов — исполнение, а не согласие.
 */
export type QuestionState = "open" | "decided" | "closed";

export interface ProjectQuestion {
  id: string;
  number: number;
  title: string;
  path: string;
  state: QuestionState;
  /** Что сказано в поле дословно — по нему видно, чем именно закрыт вопрос. */
  stateText: string;
  gate: string;
  openedAt: string;
  closedAt: string;
  /** Есть ли в документе раздел «Ответ» — отдельно от объявленного состояния. */
  hasAnswer: boolean;
}

const QUESTION_PATH = /^00-frame\/questions\/(Q-(\d+))\.md$/;
const DATE = /(\d{4}-\d{2}-\d{2})/;

/** Первое слово поля «Состояние» решает; неизвестное считаем открытым, а не закрытым. */
export function stateOfQuestion(field: string | undefined): { state: QuestionState; text: string } {
  const text = (field ?? "").trim();
  const head = text.split(/[\s·]/)[0]?.toLowerCase() ?? "";
  if (head.startsWith("закрыт")) return { state: "closed", text };
  if (head.startsWith("решено") || head.startsWith("решён")) return { state: "decided", text };
  return { state: "open", text };
}

export interface QuestionSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  fieldsOf: (path: string) => ReadonlyMap<string, string>;
  sectionTitlesOf: (path: string) => readonly string[];
}

/**
 * Заголовок раздела ответа.
 *
 * Точное равенство «Ответ» мерило уже, чем обещало: набор пишет «Ответ, 2026-09-06: дефект
 * есть, но не тот» — заголовок несёт дату и суть. Семь закрытых вопросов из 268 читались
 * как «решено, а ответа нет», и та же точная сверка доставала тело ответа для интерфейса,
 * то есть у этих семи ответ был не только не посчитан, но и не показан.
 */
export function isAnswerHeading(title: string): boolean {
  return /^Ответ(\s*[,:—-]|\s+\d|$)/.test(title.trim());
}

export function projectQuestions(source: QuestionSource): ProjectQuestion[] {
  const questions: ProjectQuestion[] = [];
  for (const path of [...source.paths].sort()) {
    const match = QUESTION_PATH.exec(path);
    if (!match) continue;
    const fields = source.fieldsOf(path);
    const { state, text } = stateOfQuestion(fields.get("Состояние"));
    questions.push({
      id: match[1]!,
      number: Number(match[2]),
      title: source.titleOf(path),
      path,
      state,
      stateText: text,
      gate: fields.get("Гейт") ?? "",
      openedAt: DATE.exec(fields.get("Заведён") ?? "")?.[1] ?? "",
      closedAt: DATE.exec(fields.get("Закрыт") ?? "")?.[1] ?? "",
      hasAnswer: source.sectionTitlesOf(path).some(isAnswerHeading),
    });
  }
  return questions.sort((a, b) => a.number - b.number);
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_questions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('open','decided','closed')),
    state_text TEXT NOT NULL DEFAULT '', gate TEXT NOT NULL DEFAULT '',
    opened_at TEXT NOT NULL DEFAULT '', closed_at TEXT NOT NULL DEFAULT '',
    has_answer BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (project_id, id)
  )`,
  `CREATE INDEX IF NOT EXISTS project_questions_by_state ON project_questions(project_id, state)`,
];

type Row = Record<string, unknown>;

/** Реестр пишет «не держит» одной из трёх фраз; всё прочее читается как «держит». */
const NOT_HOLDING = /гейтов этапа не держ|гейт этапа НЕ держ|не держит гейт/i;
const HOLDING = /держ(ит|ал)\s/i;
const LEVEL = /уровень (\d(?:[–-]\d)?)/i;

/** Наборы, в которых живут имена: ответ, назвавший имя, проверяется по ним. */
const NAMED_TABLES = [
  "project_requirements",
  "project_checks",
  "project_decisions",
  "project_plan_tasks",
  "project_stories",
  "project_screens",
  "project_needs",
  "project_features",
  "project_questions",
];

function rowToQuestion(row: Row): ProjectQuestion {
  return {
    id: String(row["id"]),
    number: Number(row["number"]),
    title: String(row["title"]),
    path: String(row["path"]),
    state: String(row["state"]) as QuestionState,
    stateText: String(row["state_text"] ?? ""),
    gate: String(row["gate"] ?? ""),
    openedAt: String(row["opened_at"] ?? ""),
    closedAt: String(row["closed_at"] ?? ""),
    hasAnswer: row["has_answer"] === true,
  };
}

export interface QuestionStore {
  replace(projectId: string, questions: readonly ProjectQuestion[]): Promise<void>;
  list(projectId: string, state?: QuestionState): Promise<ProjectQuestion[]>;
  counts(projectId: string): Promise<{ total: number; open: number; decided: number; closed: number }>;
  /** Решённые, но без раздела «Ответ»: решение есть, а записи ответа в документе нет. */
  decidedWithoutAnswer(projectId: string): Promise<ProjectQuestion[]>;
  /**
   * Чем закрыт каждый вопрос — по мерке самого реестра. Считается чтением, а не
   * хранится: правило живёт в README, и проекция, разошедшаяся с ним, соврала бы
   * тише, чем отсутствие числа.
   */
  answers(projectId: string): Promise<QuestionAnswer[]>;
  /**
   * Чем вопрос можно закрыть: всё, что корпус ведёт как место с именем. Ответ по
   * правилу реестра — ссылка на такой документ, и выбирать её удобнее из списка,
   * чем вспоминать путь.
   */
  carriers(projectId: string): Promise<{ id: string; kind: string; title: string; path: string }[]>;
  /**
   * Проверка черновика ответа той же меркой, что считает страницу. Второй
   * реализации быть не должно: разойдясь, они начнут спорить о том, закрыт ли
   * вопрос.
   */
  checkDraft(projectId: string, questionPath: string, draft: string): Promise<AnswerVerdict>;
  close(): Promise<void>;
}

export interface QuestionAnswer {
  id: string;
  number: number;
  title: string;
  path: string;
  state: QuestionState;
  /** Поле «Гейт» из шапки: что вопрос держит и почему сторож на него не краснеет. */
  gate: string;
  /**
   * Держит ли вопрос гейт. Берётся не из вольного чтения, а из фразы, которой
   * реестр это пишет: «гейтов этапа не держит» — явное «нет». Где не сказано ни
   * того, ни другого, так и говорим: неизвестно.
   */
  holds: "holds" | "no" | "unsaid";
  /** Уровень из шапки, если назван. */
  level: string;
  /** Когда заведён и когда закрыт, как их пишет сам документ. */
  openedAt: string;
  closedAt: string;
  kind: AnswerKind;
  /** Адреса или имена из ответа, которые никуда не ведут. */
  broken: string[];
}

export function createPostgresQuestionStore(connectionString: string): QuestionStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, questions) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_questions WHERE project_id=$1`, [projectId]);
        for (const q of questions) {
          await client.query(
            `INSERT INTO project_questions(project_id, id, number, title, path, state, state_text, gate, opened_at, closed_at, has_answer)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)`,
            [
              projectId,
              q.id,
              q.number,
              q.title,
              q.path,
              q.state,
              q.stateText,
              q.gate,
              q.openedAt,
              q.closedAt,
              q.hasAnswer,
            ],
          );
        }
      });
    },

    async list(projectId, state) {
      const { rows } = state
        ? await pg.query(`SELECT * FROM project_questions WHERE project_id=$1 AND state=$2 ORDER BY number`, [
            projectId,
            state,
          ])
        : await pg.query(`SELECT * FROM project_questions WHERE project_id=$1 ORDER BY number`, [projectId]);
      return rows.map((row) => rowToQuestion(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS total,
                count(*) FILTER (WHERE state='open')    AS open,
                count(*) FILTER (WHERE state='decided') AS decided,
                count(*) FILTER (WHERE state='closed')  AS closed
           FROM project_questions WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return {
        total: Number(r["total"]),
        open: Number(r["open"]),
        decided: Number(r["decided"]),
        closed: Number(r["closed"]),
      };
    },

    async decidedWithoutAnswer(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_questions
          WHERE project_id=$1 AND state <> 'open' AND has_answer = FALSE ORDER BY number`,
        [projectId],
      );
      return rows.map((row) => rowToQuestion(row as Row));
    },

    async answers(projectId) {
      // Имена набора собираются один раз: ответ, назвавший требование или
      // проверку, проверяется по тем же таблицам, что ведёт остальной разбор.
      const { rows } = await pg.query(
        `SELECT q.id, q.number, q.title, q.path, q.state, q.gate, q.opened_at, q.closed_at, d.content
           FROM project_questions q
           JOIN project_documents d ON d.project_id=q.project_id AND d.path=q.path
          WHERE q.project_id=$1 ORDER BY q.number`,
        [projectId],
      );
      const paths = await pg.query(`SELECT path FROM project_documents WHERE project_id=$1`, [projectId]);
      const documents = new Set(paths.rows.map((r) => String((r as Row)["path"])));

      const names = new Set<string>();
      for (const table of NAMED_TABLES) {
        try {
          const got = await pg.query(`SELECT id FROM ${table} WHERE project_id=$1`, [projectId]);
          for (const r of got.rows) names.add(String((r as Row)["id"]));
        } catch {
          // Проекции строятся отдельно; отсутствие таблицы — не ошибка чтения.
        }
      }

      return rows.map((row) => {
        const r = row as Row;
        const gate = String(r["gate"] ?? "");
        const path = String(r["path"]);
        const dir = path.slice(0, path.lastIndexOf("/"));
        const verdict = classifyAnswer({
          body: sectionOf(String(r["content"] ?? ""), "Ответ"),
          dir,
          hasDocument: (p) => documents.has(p),
          hasName: (n) => names.has(n),
        });
        return {
          id: String(r["id"]),
          number: Number(r["number"]),
          title: String(r["title"] ?? ""),
          path,
          state: String(r["state"]) as QuestionState,
          gate: gate,
          holds: NOT_HOLDING.test(gate) ? "no" : HOLDING.test(gate) ? "holds" : "unsaid",
          level: LEVEL.exec(gate)?.[1] ?? "",
          openedAt: String(r["opened_at"] ?? ""),
          closedAt: String(r["closed_at"] ?? ""),
          kind: verdict.kind,
          broken: verdict.broken,
        };
      });
    },

    async carriers(projectId) {
      const parts = [
        ["project_requirements", "требование", "left(text, 120)"],
        ["project_checks", "проверка", "left(spec, 120)"],
        ["project_decisions", "решение", "title"],
        ["project_plan_tasks", "задача", "title"],
        ["project_stories", "история", "title"],
        ["project_screens", "экран", "title"],
      ] as const;
      // Проектный документ — пятое место, которое реестр называет приземлением;
      // сущностью он не является, поэтому берётся из самих документов.
      const DESIGN = /^(30-design|10-intent)\/[^/]+\.md$/;
      const out: { id: string; kind: string; title: string; path: string }[] = [];
      for (const [table, kind, title] of parts) {
        try {
          const { rows } = await pg.query(
            `SELECT id, ${title} AS title, path FROM ${table} WHERE project_id=$1 ORDER BY id`,
            [projectId],
          );
          for (const row of rows) {
            const r = row as Row;
            out.push({ id: String(r["id"]), kind, title: String(r["title"] ?? ""), path: String(r["path"] ?? "") });
          }
        } catch {
          // Проекции строятся отдельно; отсутствие таблицы — не ошибка чтения.
        }
      }
      try {
        const { rows } = await pg.query(
          `SELECT path FROM project_documents WHERE project_id=$1 ORDER BY path`,
          [projectId],
        );
        for (const row of rows) {
          const path = String((row as Row)["path"]);
          if (!DESIGN.test(path)) continue;
          out.push({ id: path.replace(/^.*\//, ""), kind: "проектный документ", title: path, path });
        }
      } catch {
        // см. выше
      }
      return out;
    },

    async checkDraft(projectId, questionPath, draft) {
      const paths = await pg.query(`SELECT path FROM project_documents WHERE project_id=$1`, [projectId]);
      const documents = new Set(paths.rows.map((r) => String((r as Row)["path"])));
      const names = new Set<string>();
      for (const table of NAMED_TABLES) {
        try {
          const got = await pg.query(`SELECT id FROM ${table} WHERE project_id=$1`, [projectId]);
          for (const r of got.rows) names.add(String((r as Row)["id"]));
        } catch {
          // см. выше
        }
      }
      return classifyAnswer({
        body: draft,
        dir: questionPath.slice(0, questionPath.lastIndexOf("/")),
        hasDocument: (p) => documents.has(p),
        hasName: (n) => names.has(n),
      });
    },

    close: () => pg.close(),
  };
}
