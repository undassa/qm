import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

/**
 * Список, по которому работают, а не прокручивают.
 *
 * Что взято у Jira и ClickUp — и чего не взято.
 *
 * Взято: мгновенный поиск по набору и движение с клавиатуры (`j`/`k`, `Enter`,
 * `/`). Именно скорость хвалят в отзывах: рука не уходит в мышь, и список из
 * трёхсот строк перестаёт быть прокруткой.
 *
 * Не взято: конструктор представлений. Главная жалоба на ClickUp — не нехватка
 * настроек, а их избыток: «group by ужасен, я не хочу настраивать это сам». Кто
 * собрал страницу, тот и обязан был решить, как её группировать. Поэтому разрезы
 * здесь названы заранее и их два-три, а не «любое поле».
 */
export interface Slice<T> {
  key: string;
  title: string;
  /** Пусто — не группировать. */
  groupOf?: (item: T) => string;
  /** Порядок внутри разреза; без него — как пришло. */
  sort?: (a: T, b: T) => number;
}

export interface RowsView<T> {
  query: string;
  setQuery: (v: string) => void;
  slice: Slice<T>;
  setSlice: (s: Slice<T>) => void;
  /** Что показывать: сгруппировано, если у разреза есть `groupOf`. */
  groups: { title: string; items: T[] }[];
  shown: number;
  total: number;
  cursor: number;
  /** Ставится на строку: делает её управляемой с клавиатуры. */
  rowProps: (index: number) => { "data-row": number; className: string };
  search: React.JSX.Element;
}

/**
 * Поиск идёт по строке, которую страница сама объявляет текстом предмета:
 * искать по идентификатору и по существу — разные вещи, и обе нужны.
 */
export function useRows<T>(
  items: readonly T[],
  textOf: (item: T) => string,
  slices: readonly Slice<T>[],
  onOpen: (item: T) => void,
): RowsView<T> {
  const [query, setQuery] = useState("");
  const [slice, setSlice] = useState<Slice<T>>(slices[0]!);
  const [cursor, setCursor] = useState(-1);
  const box = useRef<HTMLInputElement>(null);

  const found = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [...items];
    // Слова ищутся все сразу и в любом порядке: «pag курсор» найдёт то же, что
    // «курсор pag». Точная фраза здесь никому не нужна.
    const words = q.split(/\s+/);
    return items.filter((item) => {
      const hay = textOf(item).toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  }, [items, textOf, query]);

  const groups = useMemo(() => {
    const sorted = slice.sort ? [...found].sort(slice.sort) : found;
    if (!slice.groupOf) return [{ title: "", items: sorted }];
    const map = new Map<string, T[]>();
    for (const item of sorted) {
      const key = slice.groupOf(item);
      map.set(key, [...(map.get(key) ?? []), item]);
    }
    return [...map.entries()].map(([title, group]) => ({ title, items: group }));
  }, [found, slice]);

  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);

  useEffect(() => setCursor(-1), [query, slice]);

  const onKey = useCallback(
    (e: KeyboardEvent) => {
      const typing = document.activeElement === box.current;
      if (e.key === "/" && !typing) {
        e.preventDefault();
        box.current?.focus();
        return;
      }
      if (typing && e.key === "Escape") {
        setQuery("");
        box.current?.blur();
        return;
      }
      if (typing) return;
      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setCursor((c) => Math.min(c + 1, flat.length - 1));
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(c - 1, 0));
      } else if (e.key === "Enter" && cursor >= 0 && flat[cursor]) {
        e.preventDefault();
        onOpen(flat[cursor]!);
      }
    },
    [flat, cursor, onOpen],
  );

  useEffect(() => {
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onKey]);

  useEffect(() => {
    if (cursor < 0) return;
    document.querySelector(`[data-row="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const search = (
    <div className="rows-bar">
      <input
        ref={box}
        className="rows-search"
        type="search"
        value={query}
        placeholder="искать по номеру и по смыслу…"
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Поиск"
      />
      {slices.length > 1 ? (
        <span className="rows-slices">
          {slices.map((s) => (
            <button
              key={s.key}
              type="button"
              className={s.key === slice.key ? "on" : ""}
              onClick={() => setSlice(s)}
            >
              {s.title}
            </button>
          ))}
        </span>
      ) : null}
      <span className="rows-count">
        {query ? (
          <>
            <b>{found.length}</b> из {items.length}
          </>
        ) : (
          <>
            <b>{items.length}</b>
          </>
        )}
      </span>
      <span className="rows-hint">
        <kbd>/</kbd> искать · <kbd>j</kbd>
        <kbd>k</kbd> ходить · <kbd>↵</kbd> открыть
      </span>
    </div>
  );

  let running = -1;
  const index = new Map<T, number>();
  for (const g of groups) for (const item of g.items) index.set(item, ++running);

  return {
    query,
    setQuery,
    slice,
    setSlice,
    groups,
    shown: found.length,
    total: items.length,
    cursor,
    rowProps: (i: number) => ({ "data-row": i, className: i === cursor ? " at" : "" }),
    search,
  };
}
