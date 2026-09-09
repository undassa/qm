import type { Finding, Row } from "./api";

/**
 * Находки читаются по весу, а не по величине: противоречие документа документу
 * или коду разбирают раньше, чем заявленное и забытое.
 */
const SOFT = new Set(["needs", "terms", "plan", "requirements"]);
const CODE = new Set(["dbTables"]);

export function tone(kind: string): "hard" | "soft" | "code" {
  if (CODE.has(kind)) return "code";
  return SOFT.has(kind) ? "soft" : "hard";
}

export function rank(findings: readonly Finding[]): Finding[] {
  const weight = (f: Finding) => (tone(f.kind) === "soft" ? 1 : 0);
  return [...findings].sort((a, b) => weight(a) - weight(b) || b.count - a.count);
}

/**
 * Чем строка себя называет. В находке решение зовётся `decisionId`, в перечне —
 * `id`; одним именем это не покрыть, поэтому опознаём по порядку.
 */
const ID_FIELDS = ["id", "decisionId", "storyId", "taskId", "name", "area"] as const;

export function rowKey(row: Row): string {
  for (const field of ID_FIELDS) {
    const value = row[field];
    if (typeof value === "string" && value) return value;
  }
  return "";
}

export function keysOf(detail: readonly unknown[] | undefined): Set<string> {
  const keys = new Set<string>();
  for (const row of detail ?? []) {
    if (row && typeof row === "object") {
      const key = rowKey(row as Row);
      if (key) keys.add(key);
    }
  }
  return keys;
}

/** Русское числительное согласуется: 1 запись, 2 записи, 5 записей, 11 записей. */
export function plural(n: number, one: string, few: string, many: string): string {
  const mod100 = Math.abs(n) % 100;
  const mod10 = mod100 % 10;
  if (mod100 >= 11 && mod100 <= 14) return many;
  if (mod10 === 1) return one;
  return mod10 >= 2 && mod10 <= 4 ? few : many;
}

/** Сколько дней прошло с календарной даты корпуса. Пусто — даты нет. */
export function ageInDays(date: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  const then = Date.parse(`${date}T00:00:00Z`);
  if (Number.isNaN(then)) return null;
  return Math.max(0, Math.floor((Date.now() - then) / 86_400_000));
}

/**
 * День правки из отметки времени в миллисекундах.
 *
 * Час здесь не показывается: корпус правят днями, а лишняя точность заставляет
 * читателя сверять минуты там, где вопрос был «сегодня или на той неделе».
 */
export function dayOf(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}
