import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/** Решение живёт в каталоге по своему состоянию: `accepted/` или `superseded/`. */
export type DecisionStatus = "accepted" | "superseded" | "template";

export interface ProjectDecision {
  id: string;
  number: number;
  title: string;
  path: string;
  status: DecisionStatus;
  /** Что написано в поле «Статус» — оно может расходиться с каталогом, и это находка. */
  statusText: string;
  date: string;
  deciders: string;
  /** Разделы решения — по колонке на раздел: у ADR их состав закрыт и одинаков. */
  context: string;
  decision: string;
  consequences: string;
}

/**
 * Корпус двуязычен: ранние решения описаны английскими полями, поздние — русскими,
 * и одноимённые поля значат разное. Русское «Связано» перечисляет требования,
 * английское `Related` — другие решения. Свалить их в один вид связи значит объявить
 * ADR-0001 несуществующим требованием, то есть выдумать нарушение гейта.
 */
export type DecisionLinkKind =
  "refines" | "related" | "relates-to-decision" | "supersedes" | "closes" | "amends-article";

export interface DecisionLink {
  decisionId: string;
  kind: DecisionLinkKind;
  target: string;
}

/**
 * Отвергнутый вариант — строка, а не абзац в тексте решения.
 *
 * Это сердцевина ADR: решение без цены отвергнутого — не решение, а объявление. Строкой
 * оно становится запрашиваемым: «у каких решений вариантов нет» — соединение, а не чтение
 * ста семидесяти документов, и правило гейта «ADR без альтернатив» перестаёт быть числом,
 * которое кто-то поддерживает руками.
 */
export interface DecisionAlternative {
  decisionId: string;
  ord: number;
  /** Имя варианта: жирный зачин абзаца — «Оставить как есть», «Вариант 2 — …». */
  title: string;
  /** Остальное: чем платим и почему отвергнут. */
  body: string;
}

export interface DecisionProjection {
  decisions: ProjectDecision[];
  links: DecisionLink[];
  alternatives: DecisionAlternative[];
}

const DECISION_PATH = /^30-design\/decisions\/([a-z]+)\/(\d{3,4})-[^/]+\.md$/;
const TEMPLATE = /^30-design\/decisions\/0*-?template\.md$|\/0000-template\.md$/;
const DATE = /(\d{4}-\d{2}-\d{2})/;
const ADR_REFERENCE = /ADR-(\d{3,4})/g;
const REQUIREMENT_REFERENCE = /\b((?:FR|NFR|TC)-[A-Z0-9]+(?:-\d+[a-z]?)?)\b/g;
const QUESTION_REFERENCE = /\b(Q-\d+)\b/g;
// «Amends = Constitution Article 4» — правка статьи конституции, а не другого решения.
const ARTICLE_REFERENCE = /Article\s+(\d+)/g;

export function statusOfFolder(folder: string): DecisionStatus {
  if (folder === "accepted") return "accepted";
  return folder === "superseded" ? "superseded" : "template";
}

export interface DecisionSource {
  paths: readonly string[];
  titleOf: (path: string) => string;
  fieldsOf: (path: string) => ReadonlyMap<string, string>;
  /** Заголовки разделов документа — корпус двуязычен, и имя раздела бывает любым из пары. */
  sectionTitlesOf?: (path: string) => readonly string[];
  /** Тело раздела по заголовку. */
  sectionBodyOf?: (path: string, title: string) => string;
}

/** Пара имён одного раздела: ранние решения писаны по-английски, поздние по-русски. */
const SECTION_NAMES = {
  context: ["Контекст", "Context"],
  decision: ["Решение", "Decision"],
  consequences: ["Последствия", "Consequences"],
  alternatives: ["Отвергнутые варианты", "Отвергнутые варианты и их цена", "Alternatives considered", "Alternatives"],
} as const;

/**
 * Тело раздела по любому из его имён, и имя ищется НАЧАЛОМ заголовка: корпус пишет
 * «Отвергнутые варианты и их цена», «Последствия, измеренные», — заголовок несёт уточнение.
 */
function bodyOf(source: DecisionSource, path: string, names: readonly string[]): string {
  const titles = source.sectionTitlesOf?.(path) ?? [];
  for (const name of names) {
    const found = titles.find((t) => t === name) ?? titles.find((t) => t.startsWith(name));
    if (found) return (source.sectionBodyOf?.(path, found) ?? "").trim();
  }
  return "";
}

/**
 * Разбор раздела отвергнутых на варианты. Зачин варианта — жирный, и форм у него ДВЕ, обе
 * живые в наборе: ранние решения пишут списком («- **Stay on Rust.** Fully viable…»),
 * поздние — абзацем («**Оставить как есть.** Ноль правок…»). Разбор одной формы находил
 * варианты у 151 решения из 171 и объявлял безальтернативными шестнадцать, у которых раздел
 * есть и полон.
 *
 * Единица — АБЗАЦ, а не строка, и зачин ищется с флагом `s`: у `ADR-0157` зачин варианта
 * «Подмодуль или соседний репозиторий: набор версионируется, но…» растянут на две строки, и
 * построчный разбор терял вариант целиком, приклеивая его к предыдущему. Абзац без зачина
 * принадлежит предыдущему варианту: это его продолжение — «Вернёмся, если…» — а не
 * безымянный вариант.
 */
export function splitAlternatives(decisionId: string, body: string): DecisionAlternative[] {
  const out: DecisionAlternative[] = [];

  const open = (title: string, rest: string) => {
    const clean = title.replace(/\s+/g, " ").replace(/[.:;,]\s*$/, "").trim();
    if (!clean) return;
    out.push({ decisionId, ord: out.length, title: clean, body: rest.replace(/^[.:—-]\s*/, "").trim() });
  };
  const append = (text: string) => {
    const last = out[out.length - 1];
    if (last) last.body = `${last.body}\n\n${text}`.trim();
  };
  /** Жирный зачин абзаца; `s` обязателен — зачин бывает длиннее строки. */
  const lead = (text: string): [string, string] | null => {
    const m = /^\*\*(.+?)\*\*/s.exec(text.trim());
    return m ? [m[1]!, text.trim().slice(m[0].length)] : null;
  };

  for (const chunk of body.split(/\n\s*\n/)) {
    const text = chunk.trim();
    if (!text) continue;

    // Форма списком — ранние решения: «- **Stay on Rust.** Fully viable…».
    if (/^\s*[-*+]\s+\*\*/.test(text)) {
      let item = "";
      const flush = () => {
        const piece = item.trim();
        item = "";
        if (!piece) return;
        const head = lead(piece.replace(/^\s*[-*+]\s+/, ""));
        if (head) open(head[0], head[1]);
        else append(piece);
      };
      for (const line of text.split("\n")) {
        if (/^\s*[-*+]\s+/.test(line)) flush();
        item += `${line}\n`;
      }
      flush();
      continue;
    }

    // Форма абзацем — поздние решения: «**Оставить как есть.** Ноль правок…».
    const head = lead(text);
    if (head) open(head[0], head[1]);
    else append(text);
  }
  return out;
}

export function projectDecisions(source: DecisionSource): DecisionProjection {
  const decisions: ProjectDecision[] = [];
  const links: DecisionLink[] = [];
  const alternatives: DecisionAlternative[] = [];
  const seen = new Set<string>();

  for (const path of [...source.paths].sort()) {
    if (TEMPLATE.test(path)) continue;
    const match = DECISION_PATH.exec(path);
    if (!match) continue;
    const number = Number(match[2]);
    const id = `ADR-${match[2]}`;
    const fields = source.fieldsOf(path);
    const statusText = (fields.get("Статус") ?? fields.get("Status") ?? "").trim();

    decisions.push({
      id,
      number,
      title: source.titleOf(path),
      path,
      status: statusOfFolder(match[1]!),
      statusText: statusText,
      // Дата ищется и в самом «Статусе»: `ADR-0164` пишет «Статус: принято 2026-09-05» и
      // отдельного поля даты не имеет. Пустая дата у подписанного решения — потеря, а не
      // особенность записи.
      date: DATE.exec(fields.get("Дата") ?? fields.get("Date") ?? "")?.[1] ?? DATE.exec(statusText)?.[1] ?? "",
      // Тем же правилом решающие: у `ADR-0164` они в поле «Решает», а не «Решают».
      deciders: (fields.get("Решают") ?? fields.get("Решает") ?? fields.get("Deciders") ?? "").trim(),
      context: bodyOf(source, path, SECTION_NAMES.context),
      decision: bodyOf(source, path, SECTION_NAMES.decision),
      consequences: bodyOf(source, path, SECTION_NAMES.consequences),
    });
    alternatives.push(...splitAlternatives(id, bodyOf(source, path, SECTION_NAMES.alternatives)));

    const add = (kind: DecisionLink["kind"], target: string) => {
      const key = `${id} ${kind} ${target}`;
      if (target === id || seen.has(key)) return;
      seen.add(key);
      links.push({ decisionId: id, kind, target });
    };
    const amends = fields.get("Amends") ?? "";
    for (const m of (fields.get("Уточняет") ?? "").matchAll(ADR_REFERENCE)) add("refines", `ADR-${m[1]}`);
    for (const m of amends.matchAll(ADR_REFERENCE)) add("refines", `ADR-${m[1]}`);
    for (const m of amends.matchAll(ARTICLE_REFERENCE)) add("amends-article", m[1]!);
    for (const m of (fields.get("Связано") ?? "").matchAll(REQUIREMENT_REFERENCE)) add("related", m[1]!);
    for (const m of (fields.get("Related") ?? "").matchAll(ADR_REFERENCE)) add("relates-to-decision", `ADR-${m[1]}`);
    for (const m of (fields.get("Supersedes") ?? "").matchAll(ADR_REFERENCE)) add("supersedes", `ADR-${m[1]}`);
    for (const m of (fields.get("Закрывает") ?? "").matchAll(QUESTION_REFERENCE)) add("closes", m[1]!);
  }

  return { decisions: decisions.sort((a, b) => a.number - b.number), links, alternatives };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_decisions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('accepted','superseded','template')),
    status_text TEXT NOT NULL DEFAULT '', date TEXT NOT NULL DEFAULT '',
    deciders TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  )`,
  // Разделы решения — колонками, а не блобом: их состав у ADR закрыт и одинаков (170 из 171
  // несут все три), поэтому «решения, у которых не записаны последствия» должно быть
  // запросом, а не обходом каталога.
  `ALTER TABLE project_decisions ADD COLUMN IF NOT EXISTS context TEXT NOT NULL DEFAULT ''`,
  `ALTER TABLE project_decisions ADD COLUMN IF NOT EXISTS decision TEXT NOT NULL DEFAULT ''`,
  `ALTER TABLE project_decisions ADD COLUMN IF NOT EXISTS consequences TEXT NOT NULL DEFAULT ''`,
  `CREATE TABLE IF NOT EXISTS project_decision_alternatives(
    project_id TEXT NOT NULL, decision_id TEXT NOT NULL, ord INTEGER NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL,
    PRIMARY KEY (project_id, decision_id, ord)
  )`,
  `CREATE INDEX IF NOT EXISTS project_decision_alternatives_by_decision
    ON project_decision_alternatives(project_id, decision_id)`,
  `CREATE TABLE IF NOT EXISTS project_decision_links(
    project_id TEXT NOT NULL, decision_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN
      ('refines','related','relates-to-decision','supersedes','closes','amends-article')),
    target TEXT NOT NULL,
    PRIMARY KEY (project_id, decision_id, kind, target)
  )`,
  // Виды связей добавлялись после первой выкладки: CREATE TABLE IF NOT EXISTS
  // существующее ограничение не трогает, поэтому его пересоздаём явно.
  `ALTER TABLE project_decision_links DROP CONSTRAINT IF EXISTS project_decision_links_kind_check`,
  `ALTER TABLE project_decision_links ADD CONSTRAINT project_decision_links_kind_check
     CHECK (kind IN ('refines','related','relates-to-decision','supersedes','closes','amends-article'))`,
  `CREATE INDEX IF NOT EXISTS project_decision_links_by_target
    ON project_decision_links(project_id, target)`,
];

type Row = Record<string, unknown>;

function rowToDecision(row: Row): ProjectDecision {
  return {
    id: String(row["id"]),
    number: Number(row["number"]),
    title: String(row["title"]),
    path: String(row["path"]),
    status: String(row["status"]) as DecisionStatus,
    statusText: String(row["status_text"] ?? ""),
    date: String(row["date"] ?? ""),
    deciders: String(row["deciders"] ?? ""),
  };
}

export interface DecisionStore {
  replace(projectId: string, projection: DecisionProjection): Promise<void>;
  list(projectId: string): Promise<ProjectDecision[]>;
  /** Все связи решений: по ним видно, что решение закрывает и что отменяет. */
  listLinks(projectId: string): Promise<DecisionLink[]>;
  counts(projectId: string): Promise<{ total: number; accepted: number; superseded: number }>;
  /** Ссылка «Уточняет» на решение, которого нет. */
  danglingRefinements(projectId: string): Promise<DecisionLink[]>;
  /** Ссылка «Связано» на требование, которого нет в наборе требований. */
  danglingRequirements(projectId: string): Promise<DecisionLink[]>;
  /**
   * То же, но только из действующих решений. У заменённого решения ссылка на
   * удалённое требование — след истории; у принятого — обещание, которого больше нет.
   */
  danglingRequirementsInForce(projectId: string): Promise<DecisionLink[]>;
  /** «Закрывает Q-nn», где такого вопроса нет: решение закрывает пустоту. */
  danglingQuestions(projectId: string): Promise<DecisionLink[]>;
  /** Решение объявляет вопрос закрытым, а сам вопрос всё ещё открыт. */
  closedButOpenQuestions(projectId: string): Promise<DecisionLink[]>;
  /** Правка статьи конституции, которой нет. */
  danglingArticles(projectId: string): Promise<DecisionLink[]>;
  /** Какие решения правили какую статью: у статьи должна быть видна её история. */
  amendedArticles(projectId: string): Promise<DecisionLink[]>;
  close(): Promise<void>;
}

export function createPostgresDecisionStore(connectionString: string): DecisionStore {
  const pg = createPgPool(connectionString, SCHEMA);
  const linkRows = (rows: unknown[]) =>
    rows.map((row) => {
      const r = row as Row;
      return {
        decisionId: String(r["decision_id"]),
        kind: String(r["kind"]) as DecisionLink["kind"],
        target: String(r["target"]),
      };
    });

  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_decision_links WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_decision_alternatives WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_decisions WHERE project_id=$1`, [projectId]);
        for (const d of projection.decisions) {
          await client.query(
            `INSERT INTO project_decisions(project_id, id, number, title, path, status, status_text, date, deciders,
                                           context, decision, consequences)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)`,
            [projectId, d.id, d.number, d.title, d.path, d.status, d.statusText, d.date, d.deciders,
             d.context, d.decision, d.consequences],
          );
        }
        for (const a of projection.alternatives) {
          await client.query(
            `INSERT INTO project_decision_alternatives(project_id, decision_id, ord, title, body)
             VALUES ($1,$2,$3,$4,$5)`,
            [projectId, a.decisionId, a.ord, a.title, a.body],
          );
        }
        for (const l of projection.links) {
          await client.query(
            `INSERT INTO project_decision_links(project_id, decision_id, kind, target) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING`,
            [projectId, l.decisionId, l.kind, l.target],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_decisions WHERE project_id=$1 ORDER BY number`, [
        projectId,
      ]);
      return rows.map((row) => rowToDecision(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT count(*) AS total,
                count(*) FILTER (WHERE status='accepted')   AS accepted,
                count(*) FILTER (WHERE status='superseded') AS superseded
           FROM project_decisions WHERE project_id=$1`,
        [projectId],
      );
      const r = rows[0] as Row;
      return { total: Number(r["total"]), accepted: Number(r["accepted"]), superseded: Number(r["superseded"]) };
    },

    async listLinks(projectId) {
      const { rows } = await pg.query(
        `SELECT decision_id, kind, target FROM project_decision_links
          WHERE project_id=$1 ORDER BY decision_id, kind, target`,
        [projectId],
      );
      return linkRows(rows);
    },

    async danglingRefinements(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
          LEFT JOIN project_decisions d ON d.project_id=l.project_id AND d.id=l.target
         WHERE l.project_id=$1 AND l.kind='refines' AND d.id IS NULL
         ORDER BY l.decision_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    async danglingRequirements(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
          LEFT JOIN project_requirements r ON r.project_id=l.project_id AND r.id=l.target
          LEFT JOIN project_checks c ON c.project_id=l.project_id AND c.id=l.target
         WHERE l.project_id=$1 AND l.kind='related' AND r.id IS NULL AND c.id IS NULL
         ORDER BY l.decision_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    async danglingRequirementsInForce(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
           JOIN project_decisions d ON d.project_id=l.project_id AND d.id=l.decision_id
           LEFT JOIN project_requirements r ON r.project_id=l.project_id AND r.id=l.target
           LEFT JOIN project_checks c ON c.project_id=l.project_id AND c.id=l.target
         WHERE l.project_id=$1 AND l.kind='related' AND d.status='accepted'
           AND r.id IS NULL AND c.id IS NULL
         ORDER BY l.decision_id, l.target`,
        [projectId],
      );
      return linkRows(rows);
    },

    async danglingQuestions(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
          LEFT JOIN project_questions q ON q.project_id=l.project_id AND q.id=l.target
         WHERE l.project_id=$1 AND l.kind='closes' AND q.id IS NULL
         ORDER BY l.decision_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    async closedButOpenQuestions(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
           JOIN project_questions q ON q.project_id=l.project_id AND q.id=l.target
         WHERE l.project_id=$1 AND l.kind='closes' AND q.state='open'
         ORDER BY l.target`,
        [projectId],
      );
      return linkRows(rows);
    },

    async amendedArticles(projectId) {
      const { rows } = await pg.query(
        `SELECT decision_id, kind, target FROM project_decision_links
          WHERE project_id=$1 AND kind='amends-article' ORDER BY target, decision_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    async danglingArticles(projectId) {
      const { rows } = await pg.query(
        `SELECT l.decision_id, l.kind, l.target FROM project_decision_links l
          LEFT JOIN project_articles a
                 ON a.project_id=l.project_id AND a.number=NULLIF(l.target,'')::int
         WHERE l.project_id=$1 AND l.kind='amends-article' AND a.number IS NULL
         ORDER BY l.decision_id`,
        [projectId],
      );
      return linkRows(rows);
    },

    close: () => pg.close(),
  };
}
