import type React from "react";
import { useState } from "react";
import { tool } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Готовность: где проект стоит, что его держит и чем именно.
 *
 * Прежде это были три раздела — «Где мы», «Ступени» и «Гейты», — и каждый
 * отвечал на свою треть одного вопроса. Человек, увидевший красный гейт, шёл
 * в другой раздел искать ступень, а оттуда в третий за находками; связь между
 * ними держалась у него в голове и рвалась при первом отвлечении.
 *
 * Здесь один спуск сверху вниз: полоса фаз → что держит сейчас → пункты
 * выбранной фазы → находки пункта. Каждый следующий уровень объясняет
 * предыдущий, и ни один не требует помнить, что было на другой странице.
 */

interface Item {
  item: string;
  kind: string;
  computed: string;
  violations?: number;
  detail?: string[];
  why?: string;
  means?: string;
  excepted?: number;
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
  documents?: { present: number; declared: number; absent: number } | null;
  tasks?: { total: number; closed: number; open: number } | null;
}
interface Step {
  ord: number;
  question: string;
  state?: string;
  owner?: string;
  owner_kind?: string;
  touches?: string;
  why?: string;
}
interface NextStep {
  at?: { ord?: number; question?: string; detail?: string[]; run?: string; owner?: string; side?: string } | null;
  says?: string;
}

/** Слово и тон состояния — один словарь на всю страницу. */
const WORD: Record<string, string> = {
  passed: "пройден",
  failed: "провален",
  unknown: "мерить нечем",
  waived: "снят с этого проекта",
  unsigned: "без подписи",
};
const TONE: Record<string, string> = {
  passed: "ok",
  failed: "bad",
  unknown: "dim",
  waived: "dim",
  unsigned: "dim",
};

/** Считает пункты гейта по состояниям — это и есть содержимое полоски фазы. */
function tally(g?: Gate): { passed: number; failed: number; other: number; total: number; violations: number } {
  const items = g?.items ?? [];
  let passed = 0, failed = 0, other = 0, violations = 0;
  for (const i of items) {
    if (i.computed === "passed") passed++;
    else if (i.computed === "failed") { failed++; violations += i.violations ?? 0; }
    else other++;
  }
  return { passed, failed, other, total: items.length, violations };
}

export function Readiness({ projectId }: { projectId: string }): React.JSX.Element {
  const [open, setOpen] = useState<string | null>(null);
  const [shown, setShown] = useState<string | null>(null);

  const live = useLive(
    () =>
      Promise.all([
        tool<{ gates: Gate[] }>(projectId, "gate"),
        tool<{ phases: Phase[] }>(projectId, "phases"),
        tool<{ steps: Step[] }>(projectId, "process-state"),
        tool<NextStep>(projectId, "next-step"),
      ]).then(([g, p, s, n]) => ({
        gates: g.gates ?? [],
        phases: p.phases ?? [],
        steps: s.steps ?? [],
        next: n,
      })),
    [projectId],
  );

  if (!live.data) return <p className="empty">Считаю готовность…</p>;
  const { gates, phases, steps, next } = live.data;
  const byGate = new Map(gates.map((g) => [g.gate, g]));

  // Фаза, на которой стоим, — ПЕРВАЯ, чей гейт не пройден. Не «текущая по
  // порядку»: пройденная фаза позади независимо от того, чем занят человек.
  const here = phases.find((p) => p.gateState !== "passed")?.phase ?? phases[phases.length - 1]?.phase;
  const chosen = open ?? here ?? phases[0]?.phase ?? "";
  const chosenPhase = phases.find((p) => p.phase === chosen);
  const chosenGate = chosenPhase?.gate ? byGate.get(chosenPhase.gate) : undefined;

  // `corpus` — гейт без фазы: он про набор целиком и держит любую фазу.
  const loose = gates.filter((g) => !phases.some((p) => p.gate === g.gate));
  const all = gates.flatMap((g) => g.items);
  const allItems = all.length;
  const allPassed = all.filter((i) => i.computed === "passed").length;
  const allViolations = all.reduce((n, i) => n + (i.violations ?? 0), 0);

  return (
    <>
      <div className="head">
        <div>
          <h1>Готовность</h1>
          <div className="prov">где стоим · что держит · что не прошло · кого нашли</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          <b>{allPassed}</b> из <b>{allItems}</b> пунктов прошли
          {allViolations > 0 ? <> · <b className="bad-n">{allViolations}</b> нарушений</> : null}
        </span>
      </div>

      {/* ── Полоса фаз ──────────────────────────────────────────────────── */}
      <div className="rail">
        {phases.map((p) => {
          const t = tally(p.gate ? byGate.get(p.gate) : undefined);
          const isHere = p.phase === here;
          const isOpen = p.phase === chosen;
          return (
            <button
              key={p.phase}
              type="button"
              className={`rail-seg${isOpen ? " on" : ""}${isHere ? " here" : ""} ${TONE[p.gateState ?? "unknown"] ?? "dim"}`}
              onClick={() => { setOpen(p.phase); setShown(null); }}
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
              {isHere ? <span className="rail-here">здесь</span> : null}
            </button>
          );
        })}
      </div>

      {/* ── Что держит прямо сейчас ─────────────────────────────────────── */}
      {next?.at ? (
        <section className="holds">
          <div className="holds-k">держит сейчас</div>
          <div className="holds-body">
            <div className="holds-q">
              <b>Ступень {next.at.ord}</b> · {next.at.question ?? "—"}
            </div>
            {next.at.run ? (
              <div className="holds-run">
                запустить <code>{next.at.run}</code>
                {next.at.owner ? <span className="holds-who"> · {next.at.owner}</span> : null}
              </div>
            ) : null}
            {(next.at.detail ?? []).length > 0 ? (
              <ul className="holds-list">
                {(next.at.detail ?? []).slice(0, 6).map((d, i) => (
                  <li key={i}>{d}</li>
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

      {/* ── Пункты выбранной фазы ───────────────────────────────────────── */}
      <h2 className="pick-h">
        {chosenPhase ? `${chosenPhase.phase} · ${chosenPhase.title}` : chosen}
        {chosenPhase?.gate ? <span className="pick-g">гейт {chosenPhase.gate}</span> : null}
      </h2>
      <ItemList gate={chosenGate} shown={shown} onShow={setShown} />

      {/* ── Ступени лестницы, относящиеся к этой стороне ────────────────── */}
      <Ladder steps={steps} at={next?.at?.ord} />

      {/* ── Гейт без фазы ───────────────────────────────────────────────── */}
      {loose.map((g) => (
        <section key={g.gate} className="loose">
          <h2 className="pick-h">
            Порядок в наборе
            <span className="pick-g">
              гейт {g.gate} · держит любую фазу · {tally(g).passed}/{tally(g).total}
            </span>
          </h2>
          {g.title ? <p className="lede">{g.title}</p> : null}
          <ItemList gate={g} shown={shown} onShow={setShown} />
        </section>
      ))}
    </>
  );
}

/** Пункты гейта: непройденные сверху, находки — по щелчку. */
function ItemList({
  gate,
  shown,
  onShow,
}: {
  gate: Gate | undefined;
  shown: string | null;
  onShow: (v: string | null) => void;
}): React.JSX.Element {
  const [showAll, setShowAll] = useState(false);
  if (!gate) return <p className="empty">У этой фазы гейта нет.</p>;
  if (gate.items.length === 0) return <p className="empty">Пунктов не объявлено — проверять нечем.</p>;
  // Сперва то, что держит: провален, потом неизвестное, потом прошедшее.
  const order = (i: Item) => (i.computed === "failed" ? 0 : i.computed === "passed" ? 2 : 1);
  const items = [...gate.items].sort(
    (a, b) => order(a) - order(b) || (b.violations ?? 0) - (a.violations ?? 0),
  );
  // Пройденное СВЁРНУТО. Пункт, который прошёл, внимания не требует, а места
  // занимает столько же, сколько провал: на гейте из двадцати шести пунктов
  // семнадцать зелёных отжимали красные за край экрана.
  const held = items.filter((i) => i.computed !== "passed");
  const done = items.length - held.length;
  const list = showAll ? items : held;
  return (
    <>
    {done > 0 ? (
      <button type="button" className="fold" onClick={() => setShowAll(!showAll)}>
        {showAll ? "▾" : "▸"} {done} {done === 1 ? "пройденный пункт" : "пройденных пунктов"}
      </button>
    ) : null}
    {held.length === 0 && !showAll ? <p className="note ok-note">Всё пройдено.</p> : null}
    <ul className="items">
      {list.map((i) => {
        const isOpen = shown === gate.gate + i.item;
        const has = (i.detail ?? []).length > 0;
        return (
          <li key={i.item} className={`item ${TONE[i.computed] ?? "dim"}${isOpen ? " open" : ""}`}>
            <button
              type="button"
              className="item-head"
              onClick={() => onShow(isOpen ? null : gate.gate + i.item)}
              aria-expanded={isOpen}
            >
              <span className="item-dot" aria-hidden="true" />
              <span className="item-t">{i.item}</span>
              <span className="item-s">
                {WORD[i.computed] ?? i.computed}
                {(i.violations ?? 0) > 0 ? <b> · {i.violations}</b> : null}
                {(i.excepted ?? 0) > 0 ? <em> · {i.excepted} с причиной</em> : null}
              </span>
              {has ? <span className="item-caret" aria-hidden="true">{isOpen ? "▾" : "▸"}</span> : null}
            </button>
            {isOpen ? (
              <div className="item-body">
                {i.means ? <p className="item-why">{i.means}</p> : null}
                {i.why && !i.means ? <p className="item-why">{i.why}</p> : null}
                {has ? (
                  <ul className="item-finds">
                    {(i.detail ?? []).slice(0, 40).map((d, n) => (
                      <li key={n}>{d}</li>
                    ))}
                    {(i.detail ?? []).length > 40 ? (
                      <li className="holds-more">…и ещё {(i.detail ?? []).length - 40}</li>
                    ) : null}
                  </ul>
                ) : (
                  <p className="note">Находок нет.</p>
                )}
              </div>
            ) : null}
          </li>
        );
      })}
    </ul>
    </>
  );
}

/** Лестница входа: тринадцать ступеней, две стороны, одна текущая. */
function Ladder({ steps, at }: { steps: Step[]; at: number | undefined }): React.JSX.Element | null {
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
      <h2 className="pick-h">
        Лестница входа
        <span className="pick-g">{steps.length} ступеней · сперва набор, потом репозиторий</span>
      </h2>
      {groups.map(([name, group]) =>
        group.length === 0 ? null : (
          <div key={name} className="rung-side">
            <div className="rung-k">{name}</div>
            <ol className="rung-strip">
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
                    <li
                      key={s.ord}
                      className={`rung-chip${now ? " now" : ""}${done ? " done" : ""}`}
                      title={s.question}
                    >
                      <span className="rung-num">{s.ord}</span>
                      <span className="rung-q">{s.question}</span>
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
