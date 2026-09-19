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
  // Род сущности отдаёт сама дверь. Зеркало проверок — `red-task`, и звать
  // его `task` значило получить отказ и пустой дровер.
  kind?: string;
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
/** Карточка волны: где задача стоит по порядку и что с ней сейчас. */
interface WaveCard {
  id: string;
  title: string;
  milestone: string | null;
  kind: string;
  state: string;
  wave: number | null;
  inFlight: string | null;
}
interface Waves {
  cards: WaveCard[];
}
interface Board {
  total: number;
  mirrors: number;
  torn: number;
  columns: Column[];
  tasks: Task[];
}

/**
 * ДОСКА ПОКАЗЫВАЕТ ВСЕ КАРТОЧКИ, а не три из шестидесяти.
 *
 * Прежде в колонке рисовалось три, остальные сворачивались в «ещё 57». Так
 * колонки вставали вровень и доска влезала в экран — но доской это быть
 * переставало: где какая задача, по ней было не узнать, а именно за этим на
 * доску и смотрят. Владелец сказал прямо: «сделай обычную доску, как в Trello».
 *
 * Высоту держит не число карточек, а прокрутка внутри колонки: колонки остаются
 * вровень, а содержимое доступно целиком.
 */

type View = "list" | "board" | "steps";

/** Название задачи без её же имени в начале: «M1-T7 · Выдержка» → «Выдержка». */
function bare(title: string): string {
  return title.replace(/^[^·]+·\s*/, "");
}

export function Work({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [queue, setQueue] = useState<Queue | null>(null);
  const [waves, setWaves] = useState<Waves | null>(null);
  const [board, setBoard] = useState<Board | null>(null);
  const [view, setView] = useState<View>("list");
  const [open_, setOpen] = useState<string>("");
  const [lane, setLane] = useState<string>("");

  useEffect(() => {
    if (!projectId) return;
    void tool<Queue>(projectId, "preflight-queue").then(setQueue);
    void tool<Waves>(projectId, "waves").then(setWaves);
    void tool<Board>(projectId, "board").then(setBoard);
  }, [projectId]);

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

  // ХУК СТОИТ ДО РАННЕГО ВЫХОДА. Поставленный ниже, он вызывался не всегда:
  // первый проход уходил в «читаю…» с меньшим числом хуков, второй — с
  // большим, и React падал ошибкой 310 у владельца на экране. Сборка этого
  // не ловит: порядок хуков — правило времени выполнения.
  // ВОЛНЫ — НАСТОЯЩИЕ, из порядка зависимостей. Прежде здесь группировалось по
  // ВЕХАМ и звалось волнами, а показывалась свежесть предполёта: по такому
  // экрану не узнать, где работа, а за этим на него и смотрят.
  //
  // Зеркала не показываются: у них свой трек и свой порядок, а смешанные в один
  // ряд они удваивают каждую волну и прячут дев-задачи.
  const поВолнам = useMemo(() => {
    const m = new Map<number, WaveCard[]>();
    for (const c of waves?.cards ?? []) {
      if (c.kind === "red" || c.wave === null) continue;
      const было = m.get(c.wave) ?? [];
      было.push(c);
      m.set(c.wave, было);
    }
    return [...m.entries()].sort((a, b) => a[0] - b[0]);
  }, [waves]);

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
        {/* ХОД РАБОТЫ СТОИТ В ШАПКЕ, а не на одной из вкладок.
            Прежде здесь были только «78 задач · 78 зеркал · 0 рваных» — три
            числа, из которых ни одно не отвечает «как идут дела». Владелец
            смотрел в список, видел «60 в очереди предполёта» и читал это как
            «не готово ничего», хотя три задачи уже закрыты, а две пишутся
            прямо сейчас.
            Ступени берутся из объявления и в его порядке: набор волен звать их
            по-своему и завести седьмую — строка её покажет без правки здесь.
            Ступень, которой никто не достиг, не прячется: ноль — это ответ.
            «Неизвестно» отличается от нуля и написано словом. */}
        <p className="wk-say wk-flow">
          {board.columns.map((c, i) => (
            <span key={c.status} className={i === board.columns.length - 1 ? "wk-end" : ""}>
              {i > 0 && <span className="wk-arrow"> → </span>}
              {c.reached === null ? (
                <em title={c.title}>{say(lang, "wk.unmeasured")}</em>
              ) : (
                <span className="num">{c.reached}</span>
              )}{" "}
              <span title={c.title}>{c.status}</span>
            </span>
          ))}
          {" · "}
          <span className="num">{board.total}</span> {say(lang, "wk.devTasks")}
          {board.torn > 0 && (
            <>
              {" · "}
              <span className="bad">
                <span className="num">{board.torn}</span> {say(lang, "wk.torn")}
              </span>
            </>
          )}
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
              // Колонки РАВНЫЕ, как на любой доске. Прежде пустая сужалась до
              // 150px, чтобы «закрыта» не уехала за край, — но это лечило
              // симптом тесноты, а тесноту создавали свёрнутые карточки.
              gridTemplateColumns: колонкиДоски.map(() => "minmax(200px, 1fr)").join(" "),
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
                    <ul className="wk-cards">{свои.map(карточка)}</ul>
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
            {/* Ряд на волну, карточка на задачу. Цвет окантовки — состояние, и
                оно берётся у двери, а не выводится из вида: синяя — ведётся
                прямо сейчас (открыто рабочее дерево), зелёная — закрыта.
                Клик открывает дровер задачи, тот же, что и на доске. */}
            <ul className="wk-waves">
              {поВолнам.map(([н, список]) => (
                <li key={н}>
                  <span className="wk-wn">
                    {say(lang, "wk.wave")} {н}
                    <i>{список.filter((c) => c.state === "closed").length}/{список.length}</i>
                  </span>
                  <ul className="wk-wcards">
                    {список.map((c) => (
                      <li
                        key={c.id}
                        className={c.inFlight ? "in" : c.state === "closed" ? "done" : ""}
                      >
                        <button
                          type="button"
                          onClick={() => setOpen(c.id)}
                          title={c.inFlight ? `${say(lang, "wk.onBranch")} ${c.inFlight}` : bare(c.title)}
                        >
                          <span className="wk-id">{c.id}</span>
                          <span className="wk-bt">{bare(c.title)}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
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
                        <span className="wk-id">
                          {c.kind === "red-task" && (
                            <i className="wk-pair" title={say(lang, "wk.mirror")}>⇄</i>
                          )}
                          {c.task}
                        </span>
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
          lang={lang}
          projectId={projectId}
          kind={видно.find((c) => c.task === open_)?.kind ?? "task"}
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
