import type React from "react";
import { useEffect, useMemo, useState } from "react";
import { tool } from "./api";
import { EntityDrawer } from "./EntityDrawer";
import { type Lang, say } from "./say";

/**
 * Работа — лента времени и сами задачи.
 *
 * Страница показывала карточки, и по ним не было видно движения: задача
 * проходит шесть объявленных ступеней, а в записи у неё два состояния —
 * закрыта либо не начата. Единственное, что у задачи датировано, — предполёт,
 * и по нему видно волны: когда прошли и что успело протухнуть после.
 *
 * Лента честно неполна, и это сказано на ней самой: даты закрытия у задачи
 * нет, закрытие помечено коммитом. Показать её домыслом значило бы соврать о
 * том, чего харнес не мерил.
 */
interface Card {
  task: string;
  title: string;
  ready: boolean;
  revision: number | null;
  seenAtRevision: number | null;
  why?: string;
}

export function Work({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [queue, setQueue] = useState<{ queue: Card[]; count: number; ready: number; stale: number; never: number } | null>(null);
  const [flow, setFlow] = useState<{ total: number; pipeline: { status: string; reached: number | null }[] } | null>(null);
  const [open_, setOpen] = useState<string>("");
  const [lane, setLane] = useState<string>("");

  useEffect(() => {
    if (!projectId) return;
    void tool<{ queue: Card[]; count: number; ready: number; stale: number; never: number }>(
      projectId,
      "preflight-queue",
    ).then(setQueue);
    // Счёт берётся у `pipeline`, а не у сводки: `summary kind=task` отдаёт
    // ноль строк и ноль колонок — задача не попадает в общий разбор, и
    // страница показывала «закрыто 0 из 0» при девяноста девяти закрытых.
    void tool<{ total: number; pipeline: { status: string; reached: number | null }[] }>(
      projectId,
      "pipeline",
    ).then(setFlow);
  }, [projectId]);

  /** Волны: задачи, сгруппированные по этапу, с крайними датами предполёта. */
  const волны = useMemo(() => {
    const q = queue?.queue ?? [];
    const m = new Map<string, { tasks: Card[]; ready: number }>();
    for (const c of q) {
      // Этап читается из имени задачи — `M1-T7`. Это объявленный образец вида,
      // а не догадка: `^[mv][0-9]+-t[0-9]+[a-z]?$` с заглавными у этого набора.
      const веха = /^([MmVv]\d+)-/.exec(c.task)?.[1]?.toUpperCase() ?? "—";
      const было = m.get(веха) ?? { tasks: [], ready: 0 };
      было.tasks.push(c);
      if (c.ready) было.ready += 1;
      m.set(веха, было);
    }
    return [...m].sort((a, b) => a[0].localeCompare(b[0]));
  }, [queue]);

  const видно = useMemo(() => {
    const q = queue?.queue ?? [];
    return lane ? q.filter((c) => c.task.toUpperCase().startsWith(`${lane}-`)) : q;
  }, [queue, lane]);

  if (!queue) return <p className="empty">{say(lang, "wk.reading")}</p>;

  const всего = flow?.total ?? 0;
  const закрыто = flow?.pipeline?.find((s) => s.status === "закрыта")?.reached ?? 0;

  return (
    <div className="work">
      <header className="wk-head">
        <p className="wk-say">
          {say(lang, "wk.closed")} <span className="num">{закрыто}</span> {say(lang, "pu.of")}{" "}
          <span className="num">{всего}</span> · {say(lang, "wk.queue")}{" "}
          <span className="num">{queue.count}</span> · {say(lang, "wk.fresh")}{" "}
          <span className="num ok">{queue.ready}</span> ·{" "}
          <span className="bad">
            {say(lang, "wk.stale")} <span className="num">{queue.stale}</span>
          </span>
          {queue.never > 0 && (
            <>
              {" "}· {say(lang, "wk.never")} <span className="num">{queue.never}</span>
            </>
          )}
        </p>
      </header>

      {/* Лента: одна дорожка на волну. Полоса — доля свежих вердиктов в ней. */}
      <section className="wk-band">
        <div className="wk-h">
          <h2>{say(lang, "wk.waves")}</h2>
          <span className="wk-note">{say(lang, "wk.waveNote")}</span>
        </div>
        <ul className="wk-tl">
          {волны.map(([веха, w]) => {
            const доля = Math.round((w.ready / w.tasks.length) * 100);
            return (
              <li key={веха} className={lane === веха ? "on" : ""}>
                <button
                  type="button"
                  className="wk-lane"
                  onClick={() => setLane(lane === веха ? "" : веха)}
                >
                  {веха}
                </button>
                <span className="wk-track">
                  <i className="fresh" style={{ width: `${доля}%` }} />
                  <i className="stale" style={{ width: `${100 - доля}%` }} />
                </span>
                <span className="wk-cnt">
                  {w.ready}/{w.tasks.length}
                </span>
              </li>
            );
          })}
        </ul>
        <p className="wk-gap">{say(lang, "wk.honest")}</p>
      </section>

      <section className="wk-band">
        <div className="wk-h">
          <h2>{say(lang, "wk.tasks")}</h2>
          <span className="wk-cnt">{видно.length}</span>
          {lane && (
            <button className="wk-all" type="button" onClick={() => setLane("")}>
              {say(lang, "wk.allWaves")}
            </button>
          )}
        </div>
        {видно.length === 0 ? (
          <p className="empty">{say(lang, "wk.none")}</p>
        ) : (
          <ul className="wk-list">
            {видно.map((c) => (
              <li key={c.task} className={c.ready ? "" : "stale"}>
                <button type="button" onClick={() => setOpen(c.task)}>
                  <span className="wk-id">{c.task}</span>
                  <span className="wk-t">{c.title.replace(/^[^·]+·\s*/, "")}</span>
                  <span className="wk-state">
                    {c.ready ? say(lang, "wk.freshOne") : say(lang, "wk.staleOne")}
                  </span>
                  <span className="wk-rev">
                    {c.seenAtRevision == null
                      ? say(lang, "wk.neverOne")
                      : `${c.seenAtRevision} → ${c.revision ?? "?"}`}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {open_ && (
        <EntityDrawer
          projectId={projectId}
          kind="task"
          id={open_}
          title={open_}
          subtitle={видно.find((c) => c.task === open_)?.title ?? ""}
          onClose={() => setOpen("")}
        />
      )}
    </div>
  );
}
