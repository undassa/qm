import type React from "react";
import { useState } from "react";
import { tool, type Tile } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Чего мы не знаем.
 *
 * Единственная страница, которая должна расти в глазах, пока её не закроют.
 * Неизвестное здесь не сложено ни с «сделано», ни с «не сделано»: это третье, и
 * ровно оно решает, чему на других экранах можно верить.
 *
 * Прежде это был плоский список из двадцати строк, и тысяча семьсот пунктов
 * готовности стояли в нём ОДНОЙ строкой рядом с двумя пунктами гейтов. Такой
 * список врёт соразмерностью: глаз читает двадцать равных бед вместо одной
 * большой и девятнадцати мелких, и начинать берутся не с той.
 *
 * Теперь у каждого рода незнания есть РАЗМЕР — полоской, — и сказано, ЧЕМ он
 * закрывается. Пробел без способа его закрыть — жалоба, а не работа.
 */

interface Claim {
  name: string;
  subject: string;
  claimed: number;
  agrees: boolean | null;
}
interface Status {
  name: string;
  hasFact: boolean;
  sourceCheck: string;
}
interface Due {
  due: { kind: string }[];
  undeclared: string[];
  note: string;
}
interface Gaps {
  items: number;
  withoutMethod: number;
  byOwner: { ownerKind: string; items: number; withoutMethod: number; owners: number }[];
  closedBy: string;
}

/** Род незнания: сколько, чем закрывается и что показать, когда раскроют. */
interface Kindof {
  key: string;
  title: string;
  count: number;
  says: string;
  closes: string;
  body: React.ReactNode;
}

export function Unknown({ projectId, onFind }: { projectId: string; onFind?: (q: string) => void }): React.JSX.Element {
  const [open, setOpen] = useState<string>("");

  const live = useLive(
    () =>
      Promise.all([
        tool<{ tiles: Tile[] }>(projectId, "progress"),
        tool<{ statuses: Status[] }>(projectId, "statuses", { kind: "task" }),
        tool<{ claims: Claim[] }>(projectId, "claims"),
        tool<Due>(projectId, "kinds-due"),
        tool<Gaps>(projectId, "readiness-gaps"),
      ]).then(([t, s, c, d, g]) => ({
        tiles: t.tiles ?? [],
        statuses: s.statuses ?? [],
        claims: c.claims ?? [],
        due: d,
        gaps: g,
      })),
    [projectId],
  );

  if (!live.data) return <p className="empty">Собираю неизвестное…</p>;
  const { tiles, statuses, claims, due, gaps } = live.data;

  const withoutFact = statuses.filter((s) => !s.hasFact || s.sourceCheck);
  const unmeasured = claims.filter((c) => c.agrees === null);
  // Пункты готовности считаются своей ручкой; в плитках они же стоят одним
  // числом, и брать их дважды значило бы удвоить главное незнание.
  const gateUnknown = tiles.find((t) => t.tile === "пункты гейтов")?.unknown ?? 0;

  const kinds: Kindof[] = [
    {
      key: "readiness",
      title: "пункты готовности без способа проверки",
      count: gaps.withoutMethod,
      says: "строка списка внутри задачи или документа: сказано, что должно быть сделано, и не сказано, чем это проверить",
      closes: `объявить способ: mh call ${gaps.closedBy}`,
      body: (
        <table className="rows u-tab">
          <thead>
            <tr>
              <th>владелец</th>
              <th className="n">пунктов</th>
              <th className="n">без способа</th>
              <th className="n">владельцев</th>
            </tr>
          </thead>
          <tbody>
            {gaps.byOwner.map((b) => (
              <tr key={b.ownerKind}>
                <td>{b.ownerKind}</td>
                <td className="n">{b.items}</td>
                <td className="n hole">{b.withoutMethod}</td>
                <td className="n">{b.owners}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ),
    },
    {
      key: "kinds",
      title: "виды без раскладки",
      count: due.due.length + due.undeclared.length,
      says: "виду положена таблица, а его не разложили — или про раскладку не сказано вовсе",
      closes: "разложить вид либо объявить, что таблица ему не положена",
      body: (
        <>
          {due.due.length > 0 ? (
            <div className="u-part">
              <div className="u-k">объявлено «положена», но не разложен · {due.due.length}</div>
              <div className="art-chips">
                {due.due.map((d) => (
                  <button key={d.kind} type="button" className="art-chip" onClick={() => onFind?.(d.kind)}>
                    {d.kind}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
          {due.undeclared.length > 0 ? (
            <div className="u-part">
              <div className="u-k">не объявлено вовсе · {due.undeclared.length}</div>
              <p className="note">{due.note}</p>
              <div className="art-chips">
                {due.undeclared.map((k) => (
                  <button key={k} type="button" className="art-chip" onClick={() => onFind?.(k)}>
                    {k}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
        </>
      ),
    },
    {
      key: "claims",
      title: "заявленные числа, которые не с чем сверить",
      count: unmeasured.length,
      says: "документ называет число, а над чем оно считано — не объявлено; сверить не с чем",
      closes: "объявить, над каким множеством считается число",
      body: (
        <table className="rows u-tab">
          <thead>
            <tr>
              <th>где сказано</th>
              <th>про что</th>
              <th className="n">заявлено</th>
            </tr>
          </thead>
          <tbody>
            {unmeasured.map((c) => (
              <tr key={`${c.name}:${c.subject}`}>
                <td>
                  <button type="button" className="found" onClick={() => onFind?.(c.name)}>
                    {c.name}
                  </button>
                </td>
                <td>{c.subject}</td>
                <td className="n">{c.claimed}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ),
    },
    {
      key: "statuses",
      title: "статусы, факт которых никто не записывает",
      count: withoutFact.length,
      says: "статус объявлен, а откуда о нём узнают — нет: задача может стоять в нём вечно, и это не будет замечено",
      closes: "назвать запрос, которым статус подтверждается",
      body: (
        <ul className="u-list">
          {withoutFact.map((s) => (
            <li key={s.name}>
              <b>{s.name}</b> — {s.hasFact ? "запрос есть, источник пуст" : "запроса нет вовсе"}
            </li>
          ))}
        </ul>
      ),
    },
    {
      key: "gates",
      title: "пункты гейтов, которые нечем мерить",
      count: gateUnknown,
      says: "пункт объявлен, а способа его посчитать нет — он не зелёный и не красный",
      closes: "объявить запрос пункту либо снять пункт с этого проекта с причиной",
      body: <p className="note">Эти пункты видны в «Готовности» — там же их состояние и причина.</p>,
    },
  ].filter((k) => k.count > 0);

  const total = kinds.reduce((n, k) => n + k.count, 0);
  const biggest = Math.max(1, ...kinds.map((k) => k.count));

  return (
    <>
      <div className="head">
        <div>
          <h1>Чего мы не знаем</h1>
          <div className="prov">не «нет», а «нечем ответить» — это разные вещи</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} /> <b className="bad-n">{total}</b> без способа проверки
        </span>
      </div>

      <p className="lede">
        Пустой ответ — факт. <b>Отсутствие способа — не факт, а пробел</b>, и он обязан быть виден. Всё, что
        ниже, сегодня не проверяется ничем; пока это так, зелёное рядом значит меньше, чем кажется.
      </p>

      {kinds.length === 0 ? (
        <p className="empty ok-note">Незакрытых пробелов нет: у каждого пункта есть способ проверки.</p>
      ) : null}

      {/*
        Главное незнание названо ОТДЕЛЬНОЙ ФРАЗОЙ, а не выведено читателем из
        сравнения полосок. Если одно превышает всё остальное вместе, это и есть
        ответ на «с чего начать».
      */}
      {kinds[0] && kinds[0].count > total - kinds[0].count ? (
        <p className="lede">
          Из {total} незнаний <b>{kinds[0].count}</b> — одно и то же: {kinds[0].title}. Остальные пять родов
          вместе дают {total - kinds[0].count}.
        </p>
      ) : null}

      <ul className="u-kinds">
        {kinds.map((k) => {
          const isOpen = open === k.key;
          return (
            <li key={k.key} className={`u-kind${isOpen ? " open" : ""}`}>
              <button
                type="button"
                className="u-head"
                onClick={() => setOpen(isOpen ? "" : k.key)}
                aria-expanded={isOpen}
              >
                <span className="u-n">{k.count}</span>
                <span className="u-t">{k.title}</span>
                <span className="u-bar" title={`${k.count} из ${total}`}>
                  <span className="u-fill" style={{ width: `${Math.round((k.count / biggest) * 100)}%` }} />
                </span>
                <span className="u-caret" aria-hidden="true">{isOpen ? "▾" : "▸"}</span>
              </button>
              {isOpen ? (
                <div className="u-open">
                  <p className="u-says">{k.says}</p>
                  {/* Чем закрывается — рядом с пробелом, а не в чужой памяти.
                      Пробел без способа его закрыть — жалоба, а не работа. */}
                  <p className="u-closes">
                    <span className="u-k">закрывается</span> {k.closes}
                  </p>
                  {k.body}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>

      <Provenance
        source="readiness_item, kind_status, раскладки видов и заявленных чисел"
        computed="всё неизвестное разом, размером и с указанием, чем закрывается"
      />
    </>
  );
}
