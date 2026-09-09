import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Живой запрос: страница обновляет себя сама.
 *
 * Набор правят и человек, и агент, и датчик харнеса — правка приходит каждые
 * несколько секунд. Страница, загруженная один раз, показывает прошлое и ничем
 * об этом не говорит: перезагрузку человек делает, только когда уже
 * заподозрил. Поэтому данные перезапрашиваются сами, а рядом стоит **когда**
 * они получены — цифра, по которой видно, что показано не вчерашнее.
 *
 * Три правила:
 *
 *   1. **Скрытая вкладка не спрашивает.** Смысла нет, а нагрузка есть;
 *   2. **Возврат на вкладку спрашивает сразу** — человек вернулся посмотреть;
 *   3. **Отказ не стирает показанное.** Сервер перезапускают на каждой выкатке;
 *      мигать пустотой из-за секундного отказа — хуже, чем показать прежнее с
 *      честной отметкой времени.
 */
export interface Live<T> {
  data: T | null;
  /** Когда данные получены. `null` — ещё ни разу. */
  at: number | null;
  failed: string;
  /** Спросить сейчас: кнопка «обновить» и всё, что меняет данные. */
  again: () => void;
}

export function useLive<T>(load: () => Promise<T>, deps: unknown[], everyMs = 15000): Live<T> {
  const [data, setData] = useState<T | null>(null);
  const [at, setAt] = useState<number | null>(null);
  const [failed, setFailed] = useState("");
  // Загрузчик пересоздаётся на каждый проход; в интервале живёт последний.
  const latest = useRef(load);
  latest.current = load;

  const again = useCallback(() => {
    void latest
      .current()
      .then((d) => {
        setData(d);
        setAt(Date.now());
        setFailed("");
      })
      .catch((e: unknown) => setFailed(String(e)));
  }, []);

  useEffect(() => {
    setData(null);
    setAt(null);
    again();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") again();
    }, everyMs);
    const onShow = () => {
      if (document.visibilityState === "visible") again();
    };
    document.addEventListener("visibilitychange", onShow);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onShow);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { data, at, failed, again };
}

/** «7 сек назад» — короче и честнее, чем «только что». */
export function ago(at: number | null, now = Date.now()): string {
  if (at === null) return "ещё не читали";
  const sec = Math.max(0, Math.round((now - at) / 1000));
  if (sec < 60) return `${sec} сек назад`;
  const min = Math.round(sec / 60);
  return min < 60 ? `${min} мин назад` : `${Math.round(min / 60)} ч назад`;
}
