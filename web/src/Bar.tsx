import type React from "react";

/**
 * Полоска прогресса тремя числами.
 *
 * `unknown` не складывается ни в одну из сторон и рисуется своей полосой:
 * сложенный в «сделано» он врёт вверх, в «открыто» — вниз, а он не то и не
 * другое. Плитка, у которой отвечаемого нет, показывает не «0 %», а слова.
 */
export function Bar({ done, open, unknown }: { done: number; open: number; unknown: number }): React.JSX.Element {
  const all = Math.max(done + open + unknown, 1);
  return (
    <span className="bar" aria-hidden="true">
      <i className="bar-done" style={{ width: `${(done / all) * 100}%` }} />
      <i className="bar-open" style={{ width: `${(open / all) * 100}%` }} />
      <i className="bar-unknown" style={{ width: `${(unknown / all) * 100}%` }} />
    </span>
  );
}
