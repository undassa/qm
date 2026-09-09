import type React from "react";
import { useEffect, useState } from "react";
import { tool, type Tile } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";
import { unknownTotal } from "./unknown-count";

/**
 * Чего мы не знаем.
 *
 * Единственная страница, которая должна расти в глазах, пока её не закроют.
 * Неизвестное здесь не сложено ни с «сделано», ни с «не сделано»: это третье, и
 * ровно оно решает, чему на других экранах можно верить.
 */
interface Claim { name: string; subject: string; claimed: number; fact: number | null; agrees: boolean | null; countsOver: string | null }
interface Status { name: string; hasFact: boolean; sourceCheck: string }
interface Due { due: { kind: string }[]; undeclared: string[]; note: string }

export function Unknown({ projectId }: { projectId: string }): React.JSX.Element {
  const [tiles, setTiles] = useState<Tile[] | null>(null);
  const [statuses, setStatuses] = useState<Status[] | null>(null);
  const [claims, setClaims] = useState<Claim[] | null>(null);
  const [due, setDue] = useState<Due | null>(null);

  const live = useLive(
    () => Promise.all([
      tool<{ tiles: Tile[] }>(projectId, "progress"),
      tool<{ statuses: Status[] }>(projectId, "statuses", { kind: "task" }),
      tool<{ claims: Claim[] }>(projectId, "claims"),
      tool<Due>(projectId, "kinds-due"),
    ]).then(([t, s, c, d]) => ({ tiles: t.tiles, statuses: s.statuses, claims: c.claims, due: d })),
    [projectId],
  );
  useEffect(() => {
    if (!live.data) return;
    setTiles(live.data.tiles);
    setStatuses(live.data.statuses);
    setClaims(live.data.claims);
    setDue(live.data.due);
  }, [live.data]);

  if (!tiles || !statuses || !claims || !due) return <p className="empty">Собираю неизвестное…</p>;

  const withoutMethod = tiles.filter((t) => t.unknown > 0);
  const withoutFact = statuses.filter((s) => !s.hasFact || s.sourceCheck);
  const unmeasured = claims.filter((c) => c.agrees === null);

  return (
    <>
      <div className="head">
        <div>
          <h1>Чего мы не знаем</h1>
          <div className="prov">не «нет», а «нечем ответить» — это разные вещи</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          <b className="tile-u">{unknownTotal(tiles, withoutFact.length)}</b> без способа проверки
        </span>
      </div>

      <p className="note">
        Пустой ответ — факт. <b>Отсутствие способа — не факт, а пробел</b>, и он обязан быть виден. Всё, что ниже, сегодня
        не проверяется ничем; пока это так, зелёное рядом значит меньше, чем кажется.
      </p>

      <h3 className="rows-group">Пункты без способа проверки</h3>
      <div className="rungs">
        {withoutMethod.map((t) => (
          <div className="rung" key={t.tile}>
            <span className="rung-n">{t.unknown}</span>
            <span className="rung-q">{t.tile}</span>
            <span className="rung-o">{t.says}</span>
            <span className="rung-s unknown">нечем мерить</span>
          </div>
        ))}
      </div>

      <h3 className="rows-group">Статусы, факт которых никто не записывает</h3>
      <div className="rungs">
        {withoutFact.map((s) => (
          <div className="rung" key={s.name}>
            <span className="rung-n">·</span>
            <span className="rung-q">{s.name}</span>
            <span className="rung-o">{s.hasFact ? "запрос есть, источник пуст" : "запроса нет"}</span>
            <span className="rung-s unknown">неизвестно</span>
          </div>
        ))}
      </div>

      <h3 className="rows-group">Виды, которым таблица положена, но их не разложили</h3>
      <div className="rungs">
        {due.due.map((d) => (
          <div className="rung" key={d.kind}>
            <span className="rung-n">·</span>
            <span className="rung-q">{d.kind}</span>
            <span className="rung-o">объявлено «положена»</span>
            <span className="rung-s unknown">не разложен</span>
          </div>
        ))}
        {due.undeclared.length ? (
          <div className="rung">
            <span className="rung-n">{due.undeclared.length}</span>
            <span className="rung-q">видов, у которых это не объявлено вовсе</span>
            <span className="rung-o">{due.undeclared.slice(0, 6).join(" · ")}…</span>
            <span className="rung-s unknown">дефект плана</span>
          </div>
        ) : null}
      </div>

      {unmeasured.length ? (
        <>
          <h3 className="rows-group">Заявленные числа, которые не с чем сверить</h3>
          <div className="rungs">
            {unmeasured.map((c) => (
              <div className="rung" key={`${c.name}:${c.subject}`}>
                <span className="rung-n">{c.claimed}</span>
                <span className="rung-q">{c.name} · {c.subject}</span>
                <span className="rung-o">над чем считано — не объявлено</span>
                <span className="rung-s unknown">не сверяется</span>
              </div>
            ))}
          </div>
        </>
      ) : null}

      <Provenance source="kind_status, readiness_item, раскладки видов и заявленных чисел" computed="всё неизвестное разом" />
    </>
  );
}
