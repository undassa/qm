import type React from "react";

/**
 * Цепочка прослеживаемости — схемой, а не таблицей.
 *
 * Требование живёт не само: его просит потребность, показывает история,
 * закрывает проверка, делает задача. Порвана цепочка в одном месте — требование
 * не доказано целиком, и по таблице из четырёх колонок это не видно: глаз
 * читает четыре независимых числа, а не одну рвущуюся нить.
 *
 * Схема показывает ДВЕ вещи, которых в таблице нет. Первая — где именно рвётся:
 * ширина звена и есть число дошедших. Вторая — требования, не связанные НИ С
 * ЧЕМ: в таблице они рассыпаны по четырём колонкам нулями, а это один и тот же
 * десяток строк, и с ними надо разбираться отдельно от тех, у кого не хватает
 * одного звена.
 *
 * Числа здесь не хранятся и не считаются наперёд: они выведены из тех же строк,
 * что и таблица ниже. Второй копии счёта не существует — расходиться нечему.
 */

export interface Row {
  id: string;
  [k: string]: unknown;
}

const LINKS: { key: string; title: string; says: string }[] = [
  { key: "needs", title: "потребность", says: "кто и зачем этого просит" },
  { key: "stories", title: "история", says: "как это выглядит для человека" },
  { key: "checks", title: "проверка", says: "чем закрыто; без неё требование не доказано" },
  { key: "tasks", title: "задача", says: "кто это делает" },
];

const num = (r: Row, k: string): number => Number(r[k] ?? 0);

export function Chain({ rows, onPick }: { rows: Row[]; onPick?: (ids: string[]) => void }): React.JSX.Element {
  const total = rows.length;
  if (total === 0) return <p className="empty">Требований нет — цепочку строить не из чего.</p>;

  const links = LINKS.map((l) => {
    const has = rows.filter((r) => num(r, l.key) > 0);
    return { ...l, has: has.length, lost: rows.filter((r) => num(r, l.key) === 0) };
  });
  const whole = rows.filter((r) => LINKS.every((l) => num(r, l.key) > 0));
  const orphan = rows.filter((r) => LINKS.every((l) => num(r, l.key) === 0));

  return (
    <section className="chain">
      <div className="chain-rail">
        {links.map((l, n) => {
          const share = Math.round((l.has / total) * 100);
          return (
            <div key={l.key} className="chain-link">
              {n > 0 ? <span className="chain-arrow" aria-hidden="true" /> : null}
              <div className="chain-body">
                <div className="chain-t">{l.title}</div>
                <div className="chain-bar" title={`${l.has} из ${total}`}>
                  <span className="chain-fill" style={{ width: `${share}%` }} />
                </div>
                <div className="chain-n">
                  <b>{l.has}</b>
                  <span className="chain-of"> из {total}</span>
                </div>
                {l.lost.length > 0 ? (
                  <button
                    type="button"
                    className="chain-lost"
                    onClick={() => onPick?.(l.lost.map((r) => r.id))}
                    title="показать эти требования"
                  >
                    рвётся у {l.lost.length}
                  </button>
                ) : (
                  <div className="chain-ok">не рвётся</div>
                )}
                <div className="chain-s">{l.says}</div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="chain-sum">
        <div className="chain-card ok">
          <b>{whole.length}</b>
          <span>цепочка целая</span>
        </div>
        <div className={`chain-card${total - whole.length - orphan.length > 0 ? " warn" : ""}`}>
          <b>{total - whole.length - orphan.length}</b>
          <span>не хватает звена</span>
        </div>
        <button
          type="button"
          className={`chain-card${orphan.length > 0 ? " bad" : ""}`}
          onClick={() => onPick?.(orphan.map((r) => r.id))}
          disabled={orphan.length === 0}
        >
          <b>{orphan.length}</b>
          <span>не связаны ни с чем</span>
        </button>
      </div>
    </section>
  );
}
