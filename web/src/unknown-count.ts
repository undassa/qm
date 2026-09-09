import type { Tile } from "./api";

/**
 * Сколько всего мы не знаем — **одним счётом на весь интерфейс**.
 *
 * Считать это на каждой странице по-своему значит завести два числа на одну
 * вещь: экран «Где мы» показывал 1757, экран «Не знаем» — 1754, и оба выглядели
 * правдой. Одна вещь — одно место, где она считается.
 */
export function unknownTotal(tiles: readonly Tile[], statusesWithoutFact = 0): number {
  return tiles.reduce((n, t) => n + t.unknown, 0) + statusesWithoutFact;
}

/**
 * Что показать заголовком плитки.
 *
 * Процент считается от отвечаемого — это правило. Но когда неотвечаемого больше,
 * чем отвечаемого, «100 %» набранное из одного пункта заслоняет 1737 неизвестных
 * и читается как «всё сделано». Заголовком тогда идёт незнание: оно и есть
 * главное, что известно об этой плитке.
 */
export function headline(t: Tile): { text: string; dim: boolean } {
  const answerable = t.done + t.open;
  if (answerable === 0) return { text: "не измеряется", dim: true };
  if (t.unknown > answerable) return { text: `${t.unknown} неизвестно`, dim: true };
  return { text: `${t.percent} %`, dim: false };
}

/**
 * Заголовок задачи без повтора её имени.
 *
 * В наборе заголовок пишется как «M0-T12 · Подсказка пробуждения…», и рядом с
 * именем это читается «M0-T12 — M0-T12 · Подсказка…». Повтор мелкий, но именно
 * из таких интерфейс собирается недоделанным.
 */
export function titleOf(id: string, title: string): string {
  const trimmed = title.trim();
  if (!trimmed.startsWith(id)) return trimmed;
  return trimmed.slice(id.length).replace(/^\s*[·—-]\s*/, "") || trimmed;
}
