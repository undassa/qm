import type React from "react";
import { useEffect, useState } from "react";
import { tool, type NextStep } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Лестница входа — доской.
 *
 * Раскладка перенесена с «Волн»: полоса этапа, столбцы, карточка с цветной
 * кромкой слева и **барьер** — правило поперёк доски, а не подпись под ней.
 * Барьер здесь тот же по смыслу, что там: сперва набор, потом репозиторий.
 *
 * Цвет кромки не заменяет слова: состояние написано рядом. Цвет — чтобы
 * увидеть строй, слово — чтобы прочесть, что именно.
 */
interface Rung {
  ord: number;
  question: string;
  method_kind: string;
  owner_kind: string;
  owner: string;
  touches: string;
  answerable: string;
}

type State = "passed" | "skipped" | "unknown" | "held" | "at" | "wait";

const WORD: Record<State, string> = {
  passed: "пройдена",
  skipped: "пропущена",
  unknown: "нечем ответить",
  held: "удержана",
  at: "здесь мы",
  wait: "не пройдена",
};

const MARK: Record<State, string> = {
  passed: "✓",
  skipped: "–",
  unknown: "?",
  held: "⏸",
  at: "→",
  wait: "·",
};

export function Process({ projectId }: { projectId: string }): React.JSX.Element {
  const [step, setStep] = useState<NextStep | null>(null);
  const [rungs, setRungs] = useState<Rung[] | null>(null);

  const live = useLive(
    () => Promise.all([
      tool<NextStep>(projectId, "next-step"),
      tool<{ steps: Rung[] }>(projectId, "process-state"),
    ]).then(([s, r]) => ({ step: s, rungs: r.steps })),
    [projectId],
  );
  useEffect(() => {
    if (!live.data) return;
    setStep(live.data.step);
    setRungs(live.data.rungs);
  }, [live.data]);

  if (!step || !rungs) return <p className="empty">Читаю лестницу…</p>;

  const skipped = new Map(step.skipped.map((s) => [s.ord, s.why]));
  const unanswerable = new Map(step.unanswerable.map((s) => [s.ord, s.why]));
  const passed = new Set(step.passed);

  const stateOf = (r: Rung): State => {
    if (step.at?.ord === r.ord) return step.at.state === "held" ? "held" : "at";
    if (passed.has(r.ord)) return "passed";
    if (skipped.has(r.ord)) return "skipped";
    if (unanswerable.has(r.ord)) return "unknown";
    return "wait";
  };

  const corpus = rungs.filter((r) => r.touches === "corpus");
  const repo = rungs.filter((r) => r.touches === "repository");
  const withoutMethod = rungs.filter((r) => r.answerable === "unknown").length;

  const Card = ({ r }: { r: Rung }): React.JSX.Element => {
    const state = stateOf(r);
    // Причина показывается только когда добавляет к слову состояния. «Нечем
    // ответить» и «способ не объявлен» — одно и то же, и печатать обе строки
    // значит забивать карточку повтором.
    const why = skipped.get(r.ord) ?? "";
    return (
      <li className={`card s-${state} col${state === "at" ? " first" : ""}${r.touches === "repository" ? " repo" : ""}`}>
        <div className="id">
          <span>{r.ord} · {WORD[state]}</span>
          <b className="pre" title={WORD[state]}>{MARK[state]}</b>
        </div>
        <div className="t">{r.question}</div>
        {why ? <div className="t" style={{ color: "var(--muted)" }}>{why}</div> : null}
        <div className="n">
          {r.owner ? `${r.owner_kind === "agent" ? "субагент" : "скилл"} ${r.owner}` : "закрывает человек"}
        </div>
      </li>
    );
  };

  return (
    <>
      <div className="head">
        <div>
          <h1>Процесс</h1>
          <div className="prov">лестница входа: вопрос · чем отвечается · кто чинит</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          набор <b>{corpus.length}</b> ступеней · репозиторий <b>{repo.length}</b>
        </span>
      </div>

      <div className="chips">
        <span className={`chip${passed.size ? " full" : " none"}`}>
          <b>пройдено</b>
          <span>
            {passed.size}/{rungs.length}
          </span>
        </span>
        <span className={`chip${step.skipped.length ? "" : " none"}`}>
          <b>пропущено с причиной</b>
          <span>{step.skipped.length}</span>
        </span>
        <span className={`chip${step.unanswerable.length ? " warn" : " none"}`}>
          <b>нечем ответить</b>
          <span>{step.unanswerable.length}</span>
        </span>
        <span className={`chip${withoutMethod ? " warn" : " none"}`}>
          <b>без способа проверки</b>
          <span>
            {withoutMethod}/{rungs.length}
          </span>
        </span>
      </div>

      <div className="legend">
        <span>
          <i style={{ background: "var(--ready)" }} />
          здесь мы — эту ступень и закрывать
        </span>
        <span>
          <i style={{ background: "var(--done)" }} />
          пройдена
        </span>
        <span>
          <i style={{ background: "var(--line)" }} />
          пропущена с причиной — условие не в игре
        </span>
        <span>
          <i style={{ background: "var(--warn)" }} />
          нечем ответить — способ не объявлен
        </span>
        <span>
          <i style={{ background: "var(--code)" }} />
          удержана — пишет в репозиторий, а фаза набора открыта
        </span>
      </div>

      <p className="stage">
        сторона набора · ступени {corpus[0]?.ord ?? 0}–{corpus[corpus.length - 1]?.ord ?? 0}
      </p>
      <ul className="board">
        {corpus.map((r) => (
          <Card r={r} key={r.ord} />
        ))}
      </ul>

      <div className="barrier">
        <span className="edge l" />
        <span className="txt">
          <b>Сперва набор, потом репозиторий.</b> Ступени ниже пишут в дерево кода, и ни одна не берётся, пока
          открыта фаза набора: {step.corpusPhaseOpen ? "сейчас она открыта" : "сейчас она закрыта"}. Диспетчер
          отказывается их выдавать и называет причину, а не молчит.
        </span>
        <span className="edge" />
      </div>

      <p className="stage">
        сторона репозитория · ступени {repo[0]?.ord ?? 0}–{repo[repo.length - 1]?.ord ?? 0}
      </p>
      <ul className="board">
        {repo.map((r) => (
          <Card r={r} key={r.ord} />
        ))}
      </ul>

      <Provenance source="harness_process_step" computed="состояние каждой ступени" />
    </>
  );
}
