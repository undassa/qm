import { html, nothing, type TemplateResult } from "lit";

/**
 * Каталог сущностей: проект состоит не из файлов, а из статей, вопросов, решений,
 * историй и экранов. У каждой сущности свои колонки — общий список сюда не годится,
 * потому что смотрят на разное: у вопроса важно состояние, у экрана — кто на него ссылается.
 */
export interface EntitySummary {
  kind: string;
  title: string;
  one: string;
  icon: string;
  source: string;
  count: number;
  /** Вид документа, который эта сущность вытесняет из сайдбара; пусто — ничего не вытесняет. */
  replaces: string;
}

export interface EntityFinding {
  kind: string;
  item: string;
  count: number;
}

export type EntityRow = Record<string, unknown>;

interface Column {
  key: string;
  label: string;
  /** Колонка идентификатора набирается моноширинным и не переносится. */
  code?: boolean;
  numeric?: boolean;
  /** Своё представление значения — когда колонка сводит несколько полей в одно. */
  text?: (row: EntityRow) => string;
  /** Дата не должна ломаться на две строки: «2026-\n07-27» читается как две даты. */
  nowrap?: boolean;
  chip?: (row: EntityRow) => string;
}

const COLUMNS: Record<string, Column[]> = {
  articles: [
    { key: "number", label: "№", numeric: true },
    { key: "title", label: "Статья" },
    { key: "citedBy", label: "Цитируют", numeric: true },
  ],
  questions: [
    { key: "id", label: "Вопрос", code: true },
    { key: "state", label: "Состояние", chip: (r) => String(r["state"] ?? "") },
    { key: "title", label: "Заголовок" },
    { key: "gate", label: "Гейт", code: true },
    { key: "closedAt", label: "Закрыт", nowrap: true },
  ],
  decisions: [
    { key: "id", label: "Решение", code: true },
    { key: "status", label: "Статус", chip: (r) => String(r["status"] ?? "") },
    { key: "title", label: "Заголовок" },
    { key: "date", label: "Дата", nowrap: true },
    { key: "deciders", label: "Решают" },
  ],
  stories: [
    { key: "id", label: "История", code: true },
    { key: "state", label: "Состояние", chip: (r) => String(r["state"] ?? "") },
    { key: "progress", label: "Задачи", numeric: true, text: (r) => `${r["done"] ?? 0} / ${r["tasks"] ?? 0}` },
    { key: "title", label: "Заголовок" },
    { key: "persona", label: "Персона" },
  ],
  screens: [
    { key: "id", label: "Экран", code: true },
    { key: "area", label: "Область" },
    { key: "title", label: "Заголовок" },
    { key: "citedBy", label: "Ссылок", numeric: true },
  ],
  needs: [
    { key: "id", label: "Потребность", code: true },
    { key: "priority", label: "Приоритет", chip: (r) => String(r["priority"] ?? "") },
    { key: "text", label: "Формулировка" },
    { key: "sides", label: "Сторона" },
    { key: "theme", label: "Тема" },
    { key: "stories", label: "Историй", numeric: true },
  ],
  requirements: [
    { key: "id", label: "Требование", code: true },
    { key: "kind", label: "Вид", chip: (r) => String(r["kind"] ?? "") },
    { key: "area", label: "Область" },
    { key: "text", label: "Формулировка" },
    { key: "checks", label: "Проверок", numeric: true },
  ],
  checks: [
    { key: "id", label: "Проверка", code: true },
    { key: "area", label: "Область" },
    { key: "requirementId", label: "Требование", code: true },
    { key: "spec", label: "Что проверяет" },
  ],
  features: [
    { key: "id", label: "Область", code: true },
    { key: "title", label: "Заголовок" },
    { key: "stories", label: "Историй", numeric: true },
    { key: "done", label: "Сделано", numeric: true, text: (r) => `${r["done"] ?? 0} / ${r["stories"] ?? 0}` },
    { key: "inProgress", label: "В работе", numeric: true },
  ],
  plan: [
    { key: "name", label: "Документ", code: true },
    { key: "claim", label: "Заявлено", chip: (r) => String(r["claim"] ?? "") },
    { key: "level", label: "Уровень" },
    { key: "contains", label: "Что содержит" },
    { key: "matches", label: "Найдено", numeric: true },
  ],
  dbTables: [
    { key: "name", label: "Таблица", code: true },
    { key: "migration", label: "Миграция", code: true },
    { key: "migrationFile", label: "Файл", code: true },
    { key: "columns", label: "Колонки" },
  ],
  risks: [
    { key: "id", label: "Риск", code: true },
    { key: "state", label: "Состояние", chip: (r) => String(r["state"] ?? "") },
    { key: "title", label: "Заголовок" },
    { key: "impact", label: "Влияние" },
    { key: "probability", label: "Вероятность" },
    { key: "owner", label: "Владелец" },
  ],
  terms: [
    { key: "id", label: "Идентификатор", code: true },
    { key: "term", label: "Термин" },
    { key: "meaning", label: "Что значит" },
    { key: "area", label: "Область" },
    { key: "used", label: "Документов", numeric: true },
  ],
  board: [
    { key: "taskId", label: "Задача", code: true },
    { key: "milestone", label: "Этап", code: true },
    { key: "state", label: "Состояние", chip: (r) => String(r["state"] ?? "") },
    { key: "commit", label: "Коммит", code: true },
  ],
  traceability: [
    { key: "area", label: "Подсистема", code: true },
    { key: "title", label: "Название" },
    { key: "requirements", label: "Требований", numeric: true },
    { key: "described", label: "Описано", numeric: true },
    { key: "inContract", label: "В контракте", numeric: true },
    { key: "inCode", label: "В коде" },
  ],
  runs: [
    { key: "id", label: "Прогон", code: true },
    { key: "milestone", label: "Этап", code: true },
    { key: "title", label: "Заголовок" },
    { key: "leftOpen", label: "Долг", chip: (r) => (r["leftOpen"] === true ? "оставлено" : "нет") },
    { key: "sections", label: "Разделов", numeric: true },
  ],
};

/**
 * Метка состояния зависит от сущности: `closed` — «закрыт» у вопроса и «закрыта»
 * у задачи, род разный. Общий плоский словарь тут не годится: одинаковый ключ
 * с разным смыслом молча побеждал по порядку записи.
 */
const CHIP_LABEL: Record<string, Record<string, string>> = {
  questions: { open: "открыт", decided: "решён", closed: "закрыт" },
  decisions: { accepted: "принято", superseded: "заменено", template: "шаблон" },
  board: { not_started: "не начата", claimed: "в работе", closed: "закрыта" },
  risks: { open: "открыт", accepted: "принят как цена", closed: "закрыт" },
  needs: { must: "обязательно", should: "желательно", later: "можно после", unknown: "не указан" },
  stories: {
    done: "сделана",
    "in-progress": "в работе",
    planned: "запланирована",
    unplanned: "не запланирована",
    "no-requirements": "без требований",
  },
  requirements: { FR: "функциональное", NFR: "нефункциональное" },
  plan: { present: "есть", absent: "нет", self: "этот файл", unknown: "не сказано" },
  runs: { оставлено: "оставлено открытым", нет: "нет" },
};

export function chipLabel(kind: string, value: string): string {
  return CHIP_LABEL[kind]?.[value] ?? value;
}

/** Внимания требуют открытые вопросы и заменённые решения — они и есть работа. */
const CHIP_ATTENTION = new Set(["open", "superseded", "оставлено", "unplanned", "no-requirements", "unknown"]);
/** Сделанное отмечаем отдельно: иначе «сделана» неотличима от «запланирована». */
const CHIP_DONE = new Set(["done", "closed"]);

export function columnsFor(kind: string): Column[] {
  return (
    COLUMNS[kind] ?? [
      { key: "id", label: "Идентификатор", code: true },
      { key: "title", label: "Заголовок" },
    ]
  );
}

/**
 * Заголовок в корпусе начинается с того же идентификатора, что стоит в соседней
 * колонке: «Q-01 — …», «SCR-CFG-01 · …». В таблице это удвоение, поэтому префикс снимается.
 */
export function titleWithoutId(id: unknown, title: string): string {
  if (typeof id !== "string" || !id) return title;
  if (!title.startsWith(id)) return title;
  return title.slice(id.length).replace(/^\s*[—–·:-]\s*/, "") || title;
}

function chipTone(value: string): string {
  if (CHIP_ATTENTION.has(value)) return "attention";
  return CHIP_DONE.has(value) ? "done" : "";
}

function cellTpl(kind: string, row: EntityRow, column: Column): TemplateResult {
  if (column.chip) {
    const value = column.chip(row);
    const tone = chipTone(value);
    return html`<td><span class="entity-chip ${tone}">${chipLabel(kind, value)}</span></td>`;
  }
  const raw = column.text ? column.text(row) : row[column.key];
  const plain = raw === null || raw === undefined ? "" : String(raw);
  const text = column.key === "title" ? titleWithoutId(row["id"], plain) : plain;
  const cls = [
    column.code ? "entity-code" : "",
    column.numeric ? "entity-num" : "",
    column.nowrap ? "entity-nowrap" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return html`<td class=${cls || nothing}>${text || html`<span class="entity-blank">—</span>`}</td>`;
}

export function entityTableTpl(
  kind: string,
  rows: readonly EntityRow[],
  open: (row: EntityRow) => void,
): TemplateResult {
  const columns = columnsFor(kind);
  return html`
    <div class="entity-table-scroll">
      <table class="entity-table">
        <thead>
          <tr>
            ${columns.map((c) => html`<th class=${c.numeric ? "entity-num" : nothing}>${c.label}</th>`)}
          </tr>
        </thead>
        <tbody>
          ${rows.map(
            (row) => html`
              <tr
                tabindex="0"
                @click=${() => open(row)}
                @keydown=${(e: KeyboardEvent) => e.key === "Enter" && open(row)}
              >
                ${columns.map((column) => cellTpl(kind, row, column))}
              </tr>
            `,
          )}
        </tbody>
      </table>
    </div>
  `;
}

export function findingsTpl(kind: string, findings: readonly EntityFinding[]): TemplateResult | typeof nothing {
  const mine = findings.filter((f) => f.kind === kind);
  if (!mine.length) return nothing;
  return html`
    <div class="entity-findings">
      ${mine.map((f) => html`<span class="entity-finding"><b>${f.count}</b> ${f.item}</span>`)}
    </div>
  `;
}

/**
 * Чем строка себя называет. У находок и у списков это разные поля — решение
 * зовётся `decisionId` в связи и `id` в перечне, — поэтому опознаём по порядку,
 * а не по одному имени.
 */
const ID_FIELDS = ["id", "decisionId", "storyId", "taskId", "name", "area"] as const;

export function rowKey(row: EntityRow): string {
  for (const field of ID_FIELDS) {
    const value = row[field];
    if (typeof value === "string" && value) return value;
  }
  return "";
}

/** Строки, названные находкой: список сужается до них, а не оставляет искать глазами. */
export function keysOfDetail(detail: readonly EntityRow[]): Set<string> {
  const keys = new Set<string>();
  for (const row of detail) {
    const key = rowKey(row);
    if (key) keys.add(key);
  }
  return keys;
}

/**
 * Русское числительное согласуется с существительным: 1 запись, 2 записи,
 * 5 записей, 11 записей, 21 запись. «31 записей» читается как небрежность.
 */
export function plural(n: number, one: string, few: string, many: string): string {
  const mod100 = Math.abs(n) % 100;
  const mod10 = mod100 % 10;
  if (mod100 >= 11 && mod100 <= 14) return many;
  if (mod10 === 1) return one;
  return mod10 >= 2 && mod10 <= 4 ? few : many;
}
