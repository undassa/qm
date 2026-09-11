import type React from "react";
import { useEffect, useMemo, useState } from "react";
import { tool } from "./api";
import { EntityDrawer } from "./EntityDrawer";
import { type Lang, say } from "./say";

/**
 * Работа — один набор задач, три вида.
 *
 * Вид здесь настройка над данными, а не отдельная страница: список отвечает
 * «что делать сейчас», доска — «где работа встала», этапы — «где сменилась
 * привычка». Считает всё дверь `board`: ступени она берёт из `kind_status`,
 * так что седьмая ступень появится во всех трёх видах без правки этого файла.
 *
 * Ленты времени здесь нет намеренно. У задачи датирован только предполёт,
 * закрытие помечено коммитом, а не датой; ось времени пришлось бы дорисовать
 * домыслом.
 */
interface Card {
  task: string;
  title: string;
  ready: boolean;
  revision: number | null;
  seenAtRevision: number | null;
  why?: string;
}
interface Queue {
  queue: Card[];
  count: number;
  ready: number;
  stale: number;
  never: number;
}
interface Column {
  status: string;
  title: string;
  at: number | null;
  torn: number;
  reached: number | null;
  unknown: number;
  durable: boolean;
  why?: string;
}
interface Task {
  id: string;
  title: string;
  milestone: string | null;
  mirror: string | null;
  at: string | null;
  torn: boolean;
  missed: string[];
}
interface Board {
  total: number;
  mirrors: number;
  torn: number;
  columns: Column[];
  tasks: Task[];
}

type View = "list" | "board" | "steps";

/** Название задачи без её же имени в начале: «M1-T7 · Выдержка» → «Выдержка». */
function bare(title: string): string {
  return title.replace(/^[^·]+·\s*/, "");
}

export function Work({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [queue, setQueue] = useState<Queue | null>(null);
  const [board, setBoard] = useState<Board | null>(null);
  const [view, setView] = useState<View>("list");
  const [open_, setOpen] = useState<string>("");
  const [lane, setLane] = useState<string>("");

  useEffect(() => {
    if (!projectId) return;
    void tool<Queue>(projectId, "preflight-queue").then(setQueue);
    void tool<Board>(projectId, "board").then(setBoard);
  }, [projectId]);

  /** Волны очереди: задачи по этапу, с долей свежих вердиктов. */
  const волны = useMemo(() => {
    const m = new Map<string, { tasks: Card[]; ready: number }>();
    for (const c of queue?.queue ?? []) {
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

  /** Задачи по колонкам доски: ключ — ступень, в которой задача стоит. */
  const колонки = useMemo(() => {
    const m = new Map<string, Task[]>();
    for (const t of board?.tasks ?? []) {
      if (!t.at) continue;
      if (lane && t.milestone !== lane) continue;
      const было = m.get(t.at) ?? [];
      было.push(t);
      m.set(t.at, было);
    }
    return m;
  }, [board, lane]);

  /** Этапы: по каждому — сколько задач на какой ступени. */
  const этапы = useMemo(() => {
    const m = new Map<string, { total: number; at: Map<string, number>; torn: number }>();
    for (const t of board?.tasks ?? []) {
      const веха = t.milestone ?? "—";
      const было = m.get(веха) ?? { total: 0, at: new Map<string, number>(), torn: 0 };
      было.total += 1;
      if (t.torn) было.torn += 1;
      if (t.at) было.at.set(t.at, (было.at.get(t.at) ?? 0) + 1);
      m.set(веха, было);
    }
    return [...m].sort((a, b) => a[0].localeCompare(b[0]));
  }, [board]);

  if (!queue || !board) return <p className="empty">{say(lang, "wk.reading")}</p>;

  const порядок = board.columns.map((c) => c.status);
  // Пропуск отмечается значком, а не строкой под каждой карточкой: колонка
  // «проверена» повторяла бы «миновала проанализирована» шестьдесят три раза
  // подряд, и повторение читалось бы как фон, а не как находка. Полный список
  // миновавших ступеней — в подсказке и в дровере.
  const карточка = (t: Task): React.JSX.Element => (
    <li key={t.id} className={t.torn ? "torn" : ""}>
      <button
        type="button"
        onClick={() => setOpen(t.id)}
        title={t.torn ? `${say(lang, "wk.missed")} ${t.missed.join(" · ")}` : bare(t.title)}
      >
        <span className="wk-top">
          <span className="wk-id">{t.id}</span>
          {t.torn && <span className="wk-torn">↯</span>}
        </span>
        <span className="wk-bt">{bare(t.title)}</span>
      </button>
    </li>
  );

  return (
    <div className={view === "board" ? "work wide" : "work"}>
      <header className="wk-head">
        <div className="wk-views">
          {(["list", "board", "steps"] as View[]).map((v) => (
            <button
              key={v}
              type="button"
              className={view === v ? "on" : ""}
              onClick={() => setView(v)}
            >
              {say(lang, v === "list" ? "wk.viewList" : v === "board" ? "wk.viewBoard" : "wk.viewSteps")}
            </button>
          ))}
          {lane && (
            <button className="wk-all" type="button" onClick={() => setLane("")}>
              {lane} × {say(lang, "wk.allWaves")}
            </button>
          )}
        </div>
        <p className="wk-say">
          <span className="num">{board.total}</span> {say(lang, "wk.devTasks")} ·{" "}
          <span className="num">{board.mirrors}</span> {say(lang, "wk.mirrors")} ·{" "}
          <span className="bad">
            <span className="num">{board.torn}</span> {say(lang, "wk.torn")}
          </span>
        </p>
      </header>

      {view === "board" && (
        <section className="wk-band">
          <div className="wk-h">
            <h2>{say(lang, "wk.viewBoard")}</h2>
            <span className="wk-note">{say(lang, "wk.boardNote")}</span>
          </div>
          {/* Колонка, в которой никто не стоит, сужается: её содержимое —
              одна пояснительная строка, а не карточки. Иначе две пустые
              ступени съедали столько же места, сколько «закрыта», и та —
              самая говорящая — уезжала за край экрана. */}
          <div
            className="wk-board"
            style={{
              gridTemplateColumns: board.columns
                .map((c) => (c.at ? "minmax(186px, 1fr)" : "150px"))
                .join(" "),
            }}
          >
            {board.columns.map((c) => {
              const свои = колонки.get(c.status) ?? [];
              return (
                <div key={c.status} className="wk-col">
                  <div className={`wk-colh${c.at === 0 || c.at === null ? " empty" : ""}`}>
                    <b>{c.status}</b>
                    <i>
                      {c.at === null ? say(lang, "wk.unmeasured") : c.at}
                      {c.torn > 0 && <span className="torn"> · {c.torn} ↯</span>}
                    </i>
                  </div>
                  {c.at === null ? (
                    <p className="wk-why warn">{say(lang, "wk.noFact")}</p>
                  ) : !c.durable && свои.length === 0 ? (
                    <p className="wk-why">{say(lang, "wk.transient")}</p>
                  ) : свои.length === 0 ? (
                    <p className="wk-why">{say(lang, "wk.passed")}</p>
                  ) : (
                    <ul className="wk-cards">{свои.slice(0, 12).map(карточка)}</ul>
                  )}
                  {свои.length > 12 && (
                    <p className="wk-why">
                      {say(lang, "wk.more")} {свои.length - 12}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </section>
      )}

      {view === "steps" && (
        <section className="wk-band">
          <div className="wk-h">
            <h2>{say(lang, "wk.viewSteps")}</h2>
            <span className="wk-note">{say(lang, "wk.stepsNote")}</span>
          </div>
          <ul className="wk-tl">
            {этапы.map(([веха, m]) => (
              <li key={веха} className={lane === веха ? "on" : ""}>
                <button
                  type="button"
                  className="wk-lane"
                  onClick={() => {
                    setLane(lane === веха ? "" : веха);
                    setView("board");
                  }}
                >
                  {веха}
                </button>
                <span className="wk-track">
                  {порядок.map((st) => {
                    const n = m.at.get(st) ?? 0;
                    if (n === 0) return null;
                    return (
                      <i
                        key={st}
                        className={`st-${порядок.indexOf(st)}`}
                        style={{ width: `${(n / m.total) * 100}%` }}
                        title={`${st}: ${n}`}
                      />
                    );
                  })}
                </span>
                <span className="wk-cnt">
                  {m.torn > 0 ? <span className="bad">{m.torn} ↯ </span> : null}
                  {m.total}
                </span>
              </li>
            ))}
          </ul>
          <ul className="wk-key">
            {board.columns.map((c, i) => (
              // Ступень, в которой никто не стоит, показана погашенной, а не
              // убрана: пропасть в лестнице — это и есть то, что надо видеть.
              <li key={c.status} className={c.at ? "" : "off"}>
                <i className={`st-${i}`} />
                {c.status}
                {c.at === null && <em> · {say(lang, "wk.unmeasured")}</em>}
              </li>
            ))}
          </ul>
        </section>
      )}

      {view === "list" && (
        <>
          <section className="wk-band">
            <div className="wk-h">
              <h2>{say(lang, "wk.waves")}</h2>
              <span className="wk-note">{say(lang, "wk.waveNote")}</span>
            </div>
            <p className="wk-say">
              {say(lang, "wk.queue")} <span className="num">{queue.count}</span> ·{" "}
              {say(lang, "wk.fresh")} <span className="num ok">{queue.ready}</span> ·{" "}
              <span className="bad">
                {say(lang, "wk.stale")} <span className="num">{queue.stale}</span>
              </span>
              {queue.never > 0 && (
                <>
                  {" "}· {say(lang, "wk.never")} <span className="num">{queue.never}</span>
                </>
              )}
            </p>
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
            </div>
            {видно.length === 0 ? (
              <p className="empty">{say(lang, "wk.none")}</p>
            ) : (
              <ul className="wk-list">
                {видно.map((c) => (
                  <li key={c.task} className={c.ready ? "" : "stale"}>
                    <button type="button" onClick={() => setOpen(c.task)}>
                      <span className="wk-id">{c.task}</span>
                      <span className="wk-t">{bare(c.title)}</span>
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
        </>
      )}

      {open_ && (
        <EntityDrawer
          projectId={projectId}
          kind="task"
          id={open_}
          title={open_}
          subtitle={
            board.tasks.find((t) => t.id === open_)?.title ??
            видно.find((c) => c.task === open_)?.title ??
            ""
          }
          onClose={() => setOpen("")}
        />
      )}
    </div>
  );
}
