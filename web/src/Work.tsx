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

  /** Причины очереди с числами — дверь называет их словами, а не кодом. */
  const причины = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of queue?.queue ?? []) {
      const w = c.why ?? "";
      if (w) m.set(w, (m.get(w) ?? 0) + 1);
    }
    return [...m].sort((a, b) => b[1] - a[1]);
  }, [queue]);

  /** Ступени, в которых не стоит никто: пустые и неизмеримые порознь. */
  const пустые = useMemo(() => (board?.columns ?? []).filter((c) => !c.at), [board]);

  /**
   * Колонки доски: сперва занятые в объявленном порядке, затем пустые.
   *
   * Пустая ступень посреди лестницы разрезала поток: «в работе» и
   * «имплементирована» стояли между «проверена» и «закрыта», и глаз терял
   * дорогу между тремя четвертями работы и её концом. Порядок ВНУТРИ занятых
   * не трогается — это лестница, а не вкус; а то, что две вынесены, сказано
   * строкой под доской, с их настоящими местами.
   */
  const колонкиДоски = useMemo(() => {
    const все = board?.columns ?? [];
    return [...все.filter((c) => c.at), ...все.filter((c) => !c.at)];
  }, [board]);

  /** Настоящее место ступени в лестнице — чтобы вынос не соврал о порядке. */
  const местоВЛестнице = useMemo(() => {
    const m = new Map<string, number>();
    (board?.columns ?? []).forEach((c, i) => m.set(c.status, i + 1));
    return m;
  }, [board]);

  /** Пропуск один на всех? Тогда его можно назвать, а не пересчитывать глазами. */
  const единственныйПропуск = useMemo(() => {
    const н = new Set((board?.tasks ?? []).filter((t) => t.torn).map((t) => t.missed.join(" · ")));
    return н.size === 1 ? [...н][0] : "";
  }, [board]);
  const единственныйЭтап = useMemo(() => {
    const н = new Set((board?.tasks ?? []).filter((t) => t.torn).map((t) => t.milestone ?? "—"));
    return н.size === 1 ? [...н][0] : "";
  }, [board]);

  if (!queue || !board) return <p className="empty">{say(lang, "wk.reading")}</p>;

  // Незакрытых — это всего минус стоящие в последней ступени. Ступень берётся
  // из объявления (`terminal`), а не по имени: набор волен звать её иначе.
  const незакрытых =
    board.total - (board.columns[board.columns.length - 1]?.at ?? 0);

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
              gridTemplateColumns: колонкиДоски
                .map((c) => (c.at ? "minmax(186px, 1fr)" : "150px"))
                .join(" "),
            }}
          >
            {колонкиДоски.map((c) => {
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
          {/* Подпись стоит ПОД доской и собирается из данных, а не пишется
              прозой: «все шестнадцать в M0» — это замер, и если завтра
              появится семнадцатая в M3, строка обязана сказать другое. */}
          <p className="wk-gap">
            {board.torn === 0 ? (
              say(lang, "wk.noneTorn")
            ) : (
              <>
                {say(lang, "wk.tornFoot")} <b>{board.torn}</b> {say(lang, "wk.onWhole")}.
                {единственныйПропуск && (
                  <>
                    {" "}
                    {say(lang, "wk.allSame")} <b>{единственныйПропуск}</b>.
                  </>
                )}
                {единственныйЭтап && (
                  <>
                    {" "}
                    {say(lang, "wk.allIn")} <b>{единственныйЭтап}</b>.
                  </>
                )}
              </>
            )}
          </p>
          {пустые.length > 0 && (
            <p className="wk-gap">
              {say(lang, "wk.movedOut")}{" "}
              {пустые.map((c, i) => (
                <span key={c.status}>
                  {i > 0 && " · "}
                  <b>{c.status}</b> — {местоВЛестнице.get(c.status)}
                  {say(lang, "wk.nth")}
                </span>
              ))}
              .
            </p>
          )}
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
                  {m.total}
                  {m.torn > 0 && <span className="bad"> · {m.torn} ↯</span>}
                </span>
              </li>
            ))}
          </ul>
          <ul className="wk-key">
            {board.columns.map((c, i) => c.at
              ? (
                <li key={c.status}>
                  <i className={`st-${i}`} />
                  {c.status}
                </li>
              )
              : null)}
          </ul>
          {/* Ступени, в которых не стоит никто, сведены в одну строку — с
              цветом их не показать, а назвать надо: пропасть в лестнице и
              есть то, что надо видеть. «Пусто» и «неизмеримо» тут разные
              ответы, и они не смешиваются. */}
          {пустые.length > 0 && (
            <p className="wk-gap">
              {say(lang, "wk.unusedHead")}{" "}
              {пустые.map((c, i) => (
                <span key={c.status}>
                  {i > 0 && " · "}
                  <b>{c.status}</b>{" — "}
                  {c.at === null ? (
                    <em>{say(lang, "wk.unmeasured")}</em>
                  ) : (
                    say(lang, "wk.empty")
                  )}
                </span>
              ))}
            </p>
          )}
        </section>
      )}

      {view === "list" && (
        <>
          <section className="wk-band">
            {/* Счёт очереди стоит при волнах, а не отдельной строкой: волны и
                есть очередь, а крупная строка спорила с заголовком страницы. */}
            <div className="wk-h">
              <h2>{say(lang, "wk.waves")}</h2>
              <span className="wk-cnt">
                {queue.count} {say(lang, "wk.ofUnclosed")} {незакрытых}{" "}
                {say(lang, "wk.unclosed")}
              </span>
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
            </div>
            {видно.length === 0 ? (
              <p className="empty">{say(lang, "wk.none")}</p>
            ) : (
              <>
                {/* Столбцы названы: без шапки «128 → 129» и «протух» читались
                    как два не связанных знака. Причина берётся у двери
                    дословно — «свеж/протух» выбрасывало то, ЧЕМ задача
                    протухла, а причин три и они разные. */}
                <div className="wk-thead">
                  <span>{say(lang, "wk.colTask")}</span>
                  <span>{say(lang, "wk.colTitle")}</span>
                  <span>{say(lang, "wk.colWhy")}</span>
                  <span>{say(lang, "wk.colRev")}</span>
                </div>
                <ul className="wk-list">
                  {видно.map((c) => (
                    <li key={c.task} className={c.ready ? "" : "stale"}>
                      <button type="button" onClick={() => setOpen(c.task)}>
                        <span className="wk-id">{c.task}</span>
                        <span className="wk-t">{bare(c.title)}</span>
                        <span className="wk-state bad" title={c.why ?? ""}>{c.why ?? ""}</span>
                        <span className="wk-rev">
                          {c.seenAtRevision ?? "—"} → {c.revision ?? "?"}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
                <p className="wk-gap">
                  {say(lang, "wk.reasons")}{" "}
                  {причины.map(([почему, n], i) => (
                    <span key={почему}>
                      {i > 0 && " · "}
                      <b>{n}</b> — {почему}
                    </span>
                  ))}
                  . {say(lang, "wk.clickDrawer")}
                </p>
              </>
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
