import type React from "react";
import { type Lang, say } from "./say";
import { useState } from "react";
import { tool } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Готовность: можно ли верить числам, где проект стоит, что его держит.
 *
 * Порядок разделов — порядок вопросов, которые задают вслух.
 *
 * ПЕРВЫЙ вопрос не «сколько прошло», а «чего стоит это число». Страница
 * показывала «94 из 111 прошли» и молчала о том, что накануне семьдесят восемь
 * из этих правил не роняли НИ РАЗУ: зелёное у них значило только, что запрос
 * ничего не вернул. Выглядела страница при этом ровно так же. Поэтому наверху
 * стоит полоса доверия, и она красная, пока хоть одно правило не проверено
 * сломом.
 *
 * ВТОРОЙ — «что не прошло». Прежде показывались пункты ОДНОЙ выбранной фазы, а
 * провалы жили в других: полоса говорила «Ф4 · 135 нарушений», и чтобы их
 * увидеть, надо было догадаться щёлкнуть. Теперь непройденное собрано в один
 * список по всем гейтам разом, тяжёлое сверху; фаза остаётся отбором, а не
 * условием видимости.
 *
 * У каждой находки есть КЛЮЧ, которым она адресуется. Он приходит из самого
 * запроса пункта, и рядом с ним стоит готовая команда: объявить исключение
 * можно, не выясняя ключ перебором.
 */

interface Item {
  item: string;
  kind: string;
  computed: string;
  phase?: string;
  /** Роняли ли пункт пробой. `false` — не роняли ни разу, и зелёное у него ничего не значит. */
  probeRuns?: boolean | null;
  probe?: string;
  query?: string;
  id?: string;
  violations?: number;
  detail?: string[];
  why?: string;
  means?: string;
  excepted?: number;
  /** Чем адресуется находка этого правила. `null` — правило исключений не читает. */
  exceptionKey?: string | null;
  /** Объявленные исключения, под которые сегодня ничего не подходит. */
  staleExceptions?: number;
  staleExceptionNames?: string[];
}
interface Gate {
  gate: string;
  title?: string;
  computed: string;
  items: Item[];
  checkedAt?: number | null;
}
interface Phase {
  phase: string;
  title: string;
  gate?: string;
  gateState?: string;
}
interface Step {
  ord: number;
  question: string;
  owner?: string;
  touches?: string;
}
interface Tile {
  tile: string;
  done: number;
  open: number;
  unknown: number;
  percent?: number;
  says?: string;
}
interface PipeStep {
  status: string;
  title?: string;
  reached?: number;
  unknown?: number;
}
interface NextStep {
  at?: { ord?: number; question?: string; detail?: string[]; run?: string; owner?: string } | null;
  says?: string;
}

/** Тон состояния — цвет, а не слово: слово живёт в общем словаре. */
const TONE: Record<string, string> = {
  passed: "ok",
  failed: "bad",
  unknown: "dim",
};

const held = (computed: string) => computed === "passed";

/** Считает пункты гейта по состояниям — это и есть содержимое полоски фазы. */
function tally(g?: Gate): { passed: number; failed: number; other: number; total: number; violations: number } {
  const items = g?.items ?? [];
  let passed = 0, failed = 0, other = 0, violations = 0;
  for (const i of items) {
    if (held(i.computed)) passed++;
    else if (i.computed === "failed") { failed++; violations += i.violations ?? 0; }
    else other++;
  }
  return { passed, failed, other, total: items.length, violations };
}

export function Readiness({
  projectId,
  lang,
  onFind,
}: {
  projectId: string;
  lang: Lang;
  onFind?: ((q: string) => void) | undefined;
}): React.JSX.Element {
  const [phaseOnly, setPhaseOnly] = useState<string | null>(null);
  const [shown, setShown] = useState<string | null>(null);
  const [showPassed, setShowPassed] = useState(false);

  const live = useLive(
    () =>
      Promise.all([
        tool<{ gates: Gate[]; stale?: boolean; why?: string }>(projectId, "gate"),
        tool<{ phases: Phase[] }>(projectId, "phases"),
        tool<{ steps: Step[] }>(projectId, "process-state"),
        tool<NextStep>(projectId, "next-step"),
        tool<{ tiles: Tile[] }>(projectId, "progress"),
        tool<{ pipeline: PipeStep[] }>(projectId, "pipeline"),
      ]).then(([g, p, s, n, pr, pi]) => ({
        gates: g.gates ?? [],
        stale: g.stale === true,
        staleWhy: g.why ?? "",
        phases: p.phases ?? [],
        steps: s.steps ?? [],
        next: n,
        tiles: pr.tiles ?? [],
        pipeline: pi.pipeline ?? [],
      })),
    [projectId],
    undefined,
    lang,
  );

  if (!live.data) return <p className="empty">{say(lang, "rd.reading")}</p>;
  const { gates, phases, steps, next, tiles, pipeline, stale, staleWhy } = live.data;
  const byGate = new Map(gates.map((g) => [g.gate, g]));
  const gateOfPhase = new Map(phases.filter((p) => p.gate).map((p) => [p.gate as string, p]));

  // Фаза, на которой стоим, — ПЕРВАЯ, чей гейт не пройден. Не «текущая по
  // порядку»: пройденная фаза позади независимо от того, чем занят человек.
  const here = phases.find((p) => p.gateState !== "passed")?.phase ?? phases[phases.length - 1]?.phase;

  // Пункты ВСЕХ гейтов в одном списке. Гейт `corpus` фазы не имеет и держит
  // любую — прятать его в отдельный раздел значило бы делать вид, что он
  // касается чего-то другого.
  const all: (Item & { gate: string })[] = gates.flatMap((g) =>
    g.items.map((i) => ({ ...i, gate: g.gate })),
  );
  const passed = all.filter((i) => held(i.computed)).length;
  const violations = all.reduce((n, i) => n + (i.violations ?? 0), 0);

  // Полоса доверия. Правило, которое ни разу не уронили, зелёным быть не может:
  // его никто не проверял. Считается только то, у чего проба вообще
  // предусмотрена, — у подписных пунктов её и не бывает.
  const probed = all.filter((i) => i.probeRuns !== null && i.probeRuns !== undefined);
  const never = probed.filter((i) => i.probeRuns === false);
  const staleEx = all.reduce((n, i) => n + (i.staleExceptions ?? 0), 0);
  const noKey = all.filter((i) => i.computed === "failed" && !i.exceptionKey).length;

  const shownItems = all
    .filter((i) => (phaseOnly ? gateOfPhase.get(i.gate)?.phase === phaseOnly || (phaseOnly === "corpus" && !gateOfPhase.has(i.gate)) : true))
    .filter((i) => (showPassed ? true : !held(i.computed)))
    .sort(
      (a, b) =>
        (a.computed === "failed" ? 0 : held(a.computed) ? 2 : 1) -
          (b.computed === "failed" ? 0 : held(b.computed) ? 2 : 1) ||
        (b.violations ?? 0) - (a.violations ?? 0) ||
        a.item.localeCompare(b.item),
    );
  const hiddenPassed = all.filter(
    (i) => held(i.computed) &&
      (phaseOnly ? gateOfPhase.get(i.gate)?.phase === phaseOnly || (phaseOnly === "corpus" && !gateOfPhase.has(i.gate)) : true),
  ).length;

  return (
    <>
      <div className="head">
        <div>
          <h1>{say(lang, "rd.head")}</h1>
          <div className="prov">{say(lang, "rd.sub")}</div>
        </div>
        <span className="prov">
          <Live lang={lang} at={live.at} again={live.again} />{" "}
          <b>{passed}</b> {say(lang, "rd.of")} <b>{all.length}</b> {say(lang, "rd.itemsPassed")}
          {violations > 0 ? (
            <> · <b className="bad-n">{violations}</b> {say(lang, "rd.violations")}</>
          ) : null}
        </span>
      </div>

      {/* Числа, посчитанные по недособранным проекциям, читаются как настоящие.
          Слово об этом стоит ПЕРЕД ними и не прячет их: спрятать значило бы
          потерять и то, что всё-таки посчиталось. */}
      {stale ? <p className="side-note warn stale">{staleWhy}</p> : null}

      {/* ── Чему здесь верить ───────────────────────────────────────────── */}
      <div className={`trust${never.length > 0 ? " bad" : ""}`}>
        <div className="trust-main">
          {never.length > 0 ? (
            <>
              <b className="bad-n">{never.length}</b> из {probed.length} правил не роняли ни разу —{" "}
              <b>зелёное у них ничего не значит</b>
            </>
          ) : (
            <>{say(lang, "rd.all")}<b>{probed.length}</b> правил роняются подсадкой: каждое проверено сломом, а не
              тем, что запрос ничего не вернул
            </>
          )}
        </div>
        <div className="trust-side">
          {staleEx > 0 ? (
            <span title="исключение объявлено, а находки под него сегодня нет">
              <b>{staleEx}</b> исключений впустую
            </span>
          ) : null}
          {noKey > 0 ? (
            <span title="правило не соединяется с таблицей исключений: объявить исключение по нему нельзя">
              <b>{noKey}</b> красных без ключа исключения
            </span>
          ) : null}
          {staleEx === 0 && noKey === 0 ? <span className="ok">исключения все действующие</span> : null}
        </div>
      </div>

      {/* ── Полоса фаз ──────────────────────────────────────────────────── */}
      <div className="rail">
        {phases.map((p) => {
          const t = tally(p.gate ? byGate.get(p.gate) : undefined);
          const isHere = p.phase === here;
          const isOn = p.phase === phaseOnly;
          return (
            <button
              key={p.phase}
              type="button"
              className={`rail-seg${isOn ? " on" : ""}${isHere ? " here" : ""} ${TONE[p.gateState ?? "unknown"] ?? "dim"}`}
              onClick={() => { setPhaseOnly(isOn ? null : p.phase); setShown(null); }}
              title={isOn ? "показать все фазы" : "оставить только эту фазу"}
            >
              <span className="rail-n">{p.phase}</span>
              <span className="rail-t">{p.title}</span>
              <span className="rail-g">{p.gate ?? "—"}</span>
              {t.total > 0 ? (
                <span className="rail-bar" title={`${t.passed} прошло · ${t.failed} провалено · ${t.other} прочих`}>
                  <span className="rail-ok" style={{ flexGrow: t.passed || 0.001 }} />
                  <span className="rail-bad" style={{ flexGrow: t.failed || 0.001 }} />
                  <span className="rail-dim" style={{ flexGrow: t.other || 0.001 }} />
                </span>
              ) : (
                <span className="rail-bar empty-bar" title="пунктов нет" />
              )}
              <span className="rail-s">
                {t.total > 0 ? `${t.passed}/${t.total}` : "пунктов нет"}
                {t.failed > 0 ? <em> · {t.violations} нарушений</em> : null}
              </span>
              {isHere ? <span className="rail-here">{say(lang, "rd.here")}</span> : null}
            </button>
          );
        })}
      </div>

      {/* ── Что держит прямо сейчас ─────────────────────────────────────── */}
      {next?.at ? (
        <section className="holds">
          <div className="holds-k">{say(lang, "rd.holdsNow")}</div>
          <div className="holds-body">
            <div className="holds-q">
              <b>Ступень {next.at.ord}</b> · {next.at.question ?? "—"}
            </div>
            {next.at.run ? (
              <div className="holds-run">{say(lang, "rd.run")}<code>{next.at.run}</code>
                {next.at.owner ? <span className="holds-who"> · {next.at.owner}</span> : null}
              </div>
            ) : null}
            {(next.at.detail ?? []).length > 0 ? (
              <ul className="holds-list">
                {(next.at.detail ?? []).slice(0, 6).map((d, i) => (
                  <li key={i}><Found text={String(d)} onFind={onFind} /></li>
                ))}
                {(next.at.detail ?? []).length > 6 ? (
                  <li className="holds-more">…и ещё {(next.at.detail ?? []).length - 6}</li>
                ) : null}
              </ul>
            ) : null}
          </div>
        </section>
      ) : next?.says ? (
        <p className="note">{next.says}</p>
      ) : null}

      {/* ── Что не прошло: все гейты разом ──────────────────────────────── */}
      <h2 className="pick-h">{say(lang, "rd.failedHead")}<span className="pick-g">
          {phaseOnly ? `только ${phaseOnly}` : "все фазы разом"} · тяжёлое сверху
        </span>
      </h2>

      <div className="tabs">
        <button type="button" className={phaseOnly === null ? "on" : ""} onClick={() => setPhaseOnly(null)}>{say(lang, "rd.allPhases")}</button>
        {here ? (
          <button type="button" className={phaseOnly === here ? "on" : ""} onClick={() => setPhaseOnly(here)}>
            где стоим · {here}
          </button>
        ) : null}
        <button type="button" className={phaseOnly === "corpus" ? "on" : ""} onClick={() => setPhaseOnly("corpus")}>
          порядок в наборе
        </button>
        {hiddenPassed > 0 ? (
          <button type="button" className={`tab-fold${showPassed ? " on" : ""}`} onClick={() => setShowPassed(!showPassed)}>
            {showPassed ? "▾" : "▸"} {hiddenPassed} пройденных
          </button>
        ) : null}
      </div>

      {shownItems.length === 0 ? (
        <p className="empty ok-note">{say(lang, "rd.allPassed")}</p>
      ) : (
        <ul className="items">
          {shownItems.map((i) => (
            <ItemRow
              lang={lang}
              key={`${i.gate}·${i.item}`}
              item={i}
              phase={gateOfPhase.get(i.gate)?.phase ?? i.gate}
              open={shown === `${i.gate}·${i.item}`}
              onShow={() => setShown(shown === `${i.gate}·${i.item}` ? null : `${i.gate}·${i.item}`)}
              onFind={onFind}
            />
          ))}
        </ul>
      )}

      {/* ── Лестница входа ──────────────────────────────────────────────── */}
      <Ladder lang={lang} steps={steps} at={next?.at?.ord} />

      {/* ── Сколько прошли — три числа, никогда одно ────────────────────── */}
      {tiles.length > 0 ? (
        <>
          <h2 className="pick-h">{say(lang, "rd.bySubject")}<span className="pick-g">третье число — «не отвечается», и складывать его некуда</span>
          </h2>
          <div className="tilestrip">
            {tiles.map((t) => (
              <div key={t.tile} className={`ts${t.unknown > t.done + t.open ? " ts-mostly-unknown" : ""}`}>
                <div className="ts-t">{t.tile}</div>
                <div className="ts-n">
                  <b>{t.done}</b>
                  <span>из {t.done + t.open + t.unknown}</span>
                </div>
                <div className="ts-bar" title={`${t.done} сделано · ${t.open} открыто · ${t.unknown} не отвечается`}>
                  <span className="ts-done" style={{ flexGrow: t.done || 0.001 }} />
                  <span className="ts-open" style={{ flexGrow: t.open || 0.001 }} />
                  <span className="ts-unk" style={{ flexGrow: t.unknown || 0.001 }} />
                </div>
                <div className="ts-s">
                  {t.unknown > 0 ? <em>{t.unknown} не отвечается</em> : t.says ?? `${t.percent ?? 0} %`}
                </div>
              </div>
            ))}
          </div>
        </>
      ) : null}

      {/* ── Конвейер задач ──────────────────────────────────────────────── */}
      {pipeline.length > 0 ? (
        <section className="pipe">
          <h2 className="pick-h">{say(lang, "rd.pipeline")}<span className="pick-g">каждая ступень — своё доказательство, а не отметка</span>
          </h2>
          <ol className="pipe-rows">
            {pipeline.map((s2, n) => (
              <li key={`${s2.status}-${n}`} className="pipe-row">
                <span className="pipe-n">{s2.reached ?? 0}</span>
                <span className="pipe-t">{s2.status}</span>
                <span className="pipe-s">{s2.title ?? ""}</span>
                <span className="pipe-u">
                  {(s2.unknown ?? 0) > 0 ? <em>{s2.unknown} не отвечается</em> : null}
                </span>
              </li>
            ))}
          </ol>
        </section>
      ) : null}
    </>
  );
}

/**
 * Пункт гейта: состояние, находки и ЧЕМ ИХ АДРЕСОВАТЬ.
 *
 * Ключ исключения приходит из самого запроса пункта. Прежде его выясняли
 * перебором: `exception-set` требовал `entityId`, и чем он должен быть, не было
 * сказано нигде. `null` значит «правило исключений не читает» — это другое, чем
 * «читает, ключ такой-то», и путать нельзя.
 */
function ItemRow({
  item: i,
  phase,
  open,
  lang,
  onShow,
  onFind,
}: {
  item: Item & { gate: string };
  phase: string;
  open: boolean;
  lang: Lang;
  onShow: () => void;
  onFind?: ((q: string) => void) | undefined;
}): React.JSX.Element {
  const [showQuery, setShowQuery] = useState(false);
  const has = (i.detail ?? []).length > 0;
  return (
    <li className={`item ${TONE[i.computed] ?? "dim"}${open ? " open" : ""}`}>
      <button type="button" className="item-head" onClick={onShow} aria-expanded={open}>
        <span className="item-dot" aria-hidden="true" />
        <span className="item-ph">{phase}</span>
        <span className="item-t">{i.item}</span>
        <span className="item-s">
          {i.probeRuns === false ? (
            <b className="never" title="пробу не удалось исполнить: правило ни разу не роняли">не роняли</b>
          ) : null}
          {i.probeRuns === false ? " · " : null}
          {say(lang, `gate.${i.computed}`)}
          {(i.violations ?? 0) > 0 ? <b> · {i.violations}</b> : null}
          {(i.excepted ?? 0) > 0 ? <em> · {i.excepted} с причиной</em> : null}
        </span>
        <span className="item-caret" aria-hidden="true">{open ? "▾" : "▸"}</span>
      </button>
      {open ? (
        <div className="item-body">
          {i.probeRuns === false ? (
            <p className="item-why never-why">
              Пробу этого пункта исполнить нельзя — она записана прозой, а не запросом. Значит правило
              не роняли ни разу, и его зелёное ничего не доказывает.
              {i.probe ? <>{say(lang, "rd.written")}<code>{i.probe}</code></> : null}
            </p>
          ) : null}
          {i.means ? <p className="item-why">{i.means}</p> : null}
          {i.why && !i.means ? <p className="item-why">{i.why}</p> : null}

          {(i.staleExceptions ?? 0) > 0 ? (
            <p className="item-why never-why">
              {i.staleExceptions} исключений объявлено впустую: находки, ради которой их записали,
              сегодня нет. {(i.staleExceptionNames ?? []).slice(0, 4).join(" · ")}
            </p>
          ) : null}

          {has ? (
            <ul className="item-finds">
              {(i.detail ?? []).slice(0, 40).map((d, n) => (
                <li key={n}><Found text={String(d)} onFind={onFind} /></li>
              ))}
              {(i.detail ?? []).length > 40 ? (
                <li className="holds-more">…и ещё {(i.detail ?? []).length - 40}</li>
              ) : null}
            </ul>
          ) : (
            <p className="note">{say(lang, "rd.noFindings")}</p>
          )}

          {/* Чем адресуется находка — рядом с находкой, а не в чужой памяти. */}
          <div className="item-key">
            {i.exceptionKey ? (
              <>
                <span className="item-k">{say(lang, "rd.exKey")}</span>
                <code>{i.exceptionKey}</code>
                <span className="item-hint">{say(lang, "rd.declare")}<code>mh call exception-set rule={i.id ?? i.item} entityId=«ключ» reason=…</code>
                </span>
              </>
            ) : (
              <span className="item-hint">
                Правило не соединяется с таблицей исключений — объявить исключение по нему нельзя.
              </span>
            )}
            {i.query ? (
              <button type="button" className="item-q" onClick={() => setShowQuery(!showQuery)}>
                {showQuery ? "▾" : "▸"} запрос правила
              </button>
            ) : null}
          </div>
          {showQuery && i.query ? <pre className="item-sql">{i.query}</pre> : null}
        </div>
      ) : null}
    </li>
  );
}

/**
 * Лестница входа: тринадцать ступеней, две стороны, одна текущая.
 *
 * Прежде ступени стояли плитками в строку, и текст обрезался многоточием:
 * «каждый объявленный датчик репозитория по…». Ступень — это ВОПРОС, и вопрос,
 * обрезанный на середине, перестаёт быть вопросом. Здесь строки: места по
 * ширине хватает всем, и порядок читается сверху вниз, как он и работает.
 */
function Ladder({ steps, at, lang }: { steps: Step[]; at: number | undefined; lang: Lang }): React.JSX.Element | null {
  if (steps.length === 0) return null;
  // Сторона названа в `touches`: `corpus` или `repository`. Слова здесь свои,
  // и подставлять их наугад нельзя — ступень, попавшая не на ту сторону, врёт
  // о порядке работы.
  const side = (s: Step) => (s.touches === "repository" ? "репозиторий" : "набор");
  const groups: [string, Step[]][] = [
    ["сторона набора", steps.filter((s) => side(s) === "набор")],
    ["сторона репозитория", steps.filter((s) => side(s) === "репозиторий")],
  ];
  return (
    <section className="rungs-wrap">
      <h2 className="pick-h">{say(lang, "rd.ladder")}<span className="pick-g">{steps.length} ступеней · сперва набор, потом репозиторий</span>
      </h2>
      {groups.map(([name, group]) =>
        group.length === 0 ? null : (
          <div key={name} className="rung-side">
            <div className="rung-k">{name}</div>
            <ol className="rung-rows">
              {group
                .slice()
                .sort((a, b) => a.ord - b.ord)
                .map((s) => {
                  // Состояния у ступени нет: лестница ЛИНЕЙНА, и пройденное —
                  // это всё, что стоит до текущей. Второй записи о том же
                  // заводить не надо, она разойдётся.
                  const now = s.ord === at;
                  const done = at !== undefined && s.ord < at;
                  return (
                    <li key={s.ord} className={`rung-row${now ? " now" : ""}${done ? " done" : ""}`}>
                      <span className="rung-num">{s.ord}</span>
                      <span className="rung-q">{s.question}</span>
                      {s.owner ? <span className="rung-o">{s.owner}</span> : null}
                      {now ? <span className="rung-now">{say(lang, "rd.here")}</span> : null}
                    </li>
                  );
                })}
            </ol>
          </div>
        ),
      )}
    </section>
  );
}

/**
 * Находка со ссылками на названные в ней сущности.
 *
 * Находка называет виновника именем: `US-ESC-06`, `FR-SHF-04`, `M0-T12`. До сих
 * пор это был просто текст, и путь от красного пункта до документа шёл через
 * память и поиск руками.
 *
 * Какому ВИДУ принадлежит имя, здесь не решается. Приставка `US-` значит
 * историю только потому, что так договорился этот набор; другой договорится
 * иначе, и зашитая сюда таблица приставок стала бы умолчанием в коде — тем
 * самым, которого мы избегаем везде. Поэтому щелчок ищет имя тем же взвешенным
 * поиском, что и палитра: точное имя перевешивает всё остальное.
 */
const NAMED = /\b([A-Z][A-Z0-9]{0,7}(?:-[A-Z0-9]{1,9}){1,3})\b/g;

function Found({ text, onFind }: { text: string; onFind?: ((q: string) => void) | undefined }): React.JSX.Element {
  if (!onFind) return <>{text}</>;
  const parts: React.ReactNode[] = [];
  let at = 0;
  for (const m of text.matchAll(NAMED)) {
    const i = m.index ?? 0;
    if (i > at) parts.push(text.slice(at, i));
    const name = m[1] as string;
    parts.push(
      <button key={`${i}-${name}`} type="button" className="found" onClick={() => onFind(name)}>
        {name}
      </button>,
    );
    at = i + name.length;
  }
  if (at < text.length) parts.push(text.slice(at));
  return <>{parts}</>;
}
