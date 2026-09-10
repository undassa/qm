import type React from "react";
import { useState } from "react";

/**
 * Предел показа: длинный список рисуется частями.
 *
 * Триста строк требований — это десять тысяч точек по высоте. Найти в них
 * что-либо прокруткой нельзя, а браузер рисует их все и на каждой перерисовке
 * заново. Показывается первая часть; остальное — по просьбе.
 *
 * Число скрытых НАЗЫВАЕТСЯ. Молчаливое усечение неотличимо от «это всё», и
 * человек, увидевший шестьдесят строк из трёхсот, уходит с неверным счётом.
 */
export function useLimit(total: number, step = 60): { limit: number; more: React.JSX.Element | null } {
  const [limit, setLimit] = useState(step);
  const hidden = total - limit;
  return {
    limit,
    more:
      hidden > 0 ? (
        <button type="button" className="more-btn" onClick={() => setLimit((n) => n + step * 4)}>
          показать ещё · скрыто {hidden} из {total}
        </button>
      ) : null,
  };
}
