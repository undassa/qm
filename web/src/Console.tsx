import type React from "react";
import { useEffect, useState } from "react";
import { tool, write } from "./api";

/**
 * Пульт — главный экран. Человек приходит с тремя вопросами, и ни один из них
 * не «покажи документы»: что делают агенты, ждут ли меня, что поехало от
 * последних правок.
 *
 * Прежде ответы собирались руками из трёх ручек, а работа агентов не
 * отдавалась вовсе: `project_task_runs` знает, кто над чем сидит, и дверей к
 * нему не было ни одной.
 *
 * Раскладка одна на телефон и на стол. Различаются не размеры, а СТРОЕНИЕ
 * строки: на узком имя и суть встают в две строки, действие поднимается к
 * имени, причина переоткрытия получает свою строку. Это решает вёрстка, а не
 * второй набор разметки.
 */
interface Run {
  task: string;
  agent: string | null;
  state: string;
  attempt: number;
  title: string;
  workspace: string;
  branch: string;
}
interface Ask { id: string; title: string; since: string }
interface Moved { kind: string; id: string; cause: string; why: string; depth: number }
interface Console_ {
  working: Run[];
  asks: Ask[];
  moved: Moved[];
  movedByKind: { kind: string; count: number }[];
}
interface Stage { phase: string; title: string; gate: string; gateState: string; violations: number }
interface At { ord: number; question: string; owner: string; state: string; violations: number; first?: { run?: string } }
interface Step { stage?: Stage; at?: At; progress?: { met: number; of: number; left: number }; passed?: number[]; skipped?: { ord: number }[] }

const СЛОВО: Record<string, string> = {
  requirement: "требование", check: "проверка", story: "история", task: "задача",
  "red-task": "красная задача", milestone: "этап", question: "вопрос", screen: "экран",
  need: "потребность", decision: "решение", rationale: "рассуждение",
  "lint-rule": "правило кода", assertion: "утверждение",
};
const зовут = (k: string): string => СЛОВО[k] ?? k;

/** «1 нарушение · 2 нарушения · 5 нарушений» — иначе счёт читается как опечатка. */
function счёт(n: number, одно: string, двух: string, много: string): string {
  const с = Math.abs(n) % 100;
  const д = с % 10;
  if (с > 10 && с < 20) return `${n} ${много}`;
  if (д > 1 && д < 5) return `${n} ${двух}`;
  if (д === 1) return `${n} ${одно}`;
  return `${n} ${много}`;
}

/** Состояние прогона — словом, понятным человеку, а не машинным ярлыком. */
const ХОД: Record<string, string> = {
  provisioning: "готовит место",
  running: "работает",
  review: "на ревью",
  blocked: "встал",
  failed: "упал",
};

export function Console({ projectId, onGo }: { projectId: string; onGo?: (page: string) => void }): React.JSX.Element {
  const [c, setC] = useState<Console_ | null>(null);
  const [step, setStep] = useState<Step | null>(null);
  /** Какой вопрос отвечают прямо сейчас, что набрано и чем ответила дверь. */
  const [open_, setOpen] = useState<string>("");
  const [draft, setDraft] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [beef, setBeef] = useState<string>("");

  const перечитать = (): void => {
    void tool<Console_>(projectId, "console").then(setC);
  };

  const ответить = async (a: Ask): Promise<void> => {
    if (!draft.trim()) return;
    setBusy(true);
    setBeef("");
    // Состояние `decided` — «решено владельцем»: ответ записан, но закрывает
    // вопрос решение, а не сам факт ответа.
    const r = await write(projectId, "question-add", {
      id: a.id, title: a.title, state: "decided", answer: draft.trim(),
    });
    setBusy(false);
    if (!r.ok) { setBeef(r.why || "дверь не приняла ответ"); return; }
    setOpen(""); setDraft(""); перечитать();
  };

  useEffect(() => {
    if (!projectId) return;
    void tool<Console_>(projectId, "console").then(setC);
    void tool<Step>(projectId, "next-step").then(setStep);
  }, [projectId]);

  if (!c || !step) return <p className="empty">Читаю пульт…</p>;

  const st = step.stage;
  const at = step.at;
  const pr = step.progress;
  const passed = new Set(step.passed ?? []);
  const skipped = new Set((step.skipped ?? []).map((s) => s.ord));
  const всего = pr?.of ?? 13;

  return (
    <div className="pult">
      {/* Где мы — одна фраза, которую читают за секунду и уходят, если всё идёт. */}
      <header className="pu-head">
        {st && (
          <p className="pu-say">
            {st.phase} {st.title} · гейт{" "}
            <b className={st.gateState === "passed" ? "ok" : "st"}>
              {st.gate} {st.gateState === "passed" ? "зелен" : "красен"}
            </b>
            {st.violations > 0 && <>, {счёт(st.violations, "нарушение", "нарушения", "нарушений")}</>}
            {at && <> · ступень {at.ord} из {всего}</>}
            {at?.owner && <> · делает <span className="who">{at.owner}</span></>}
          </p>
        )}
        <div className="pu-sub">
          {pr && <span>пройдено <span className="num">{pr.met}</span> из <span className="num">{pr.of}</span></span>}
          {at?.question && <span className="pu-q">{at.question}</span>}
          {at?.first?.run && <code>{at.first.run}</code>}
        </div>
        <div className="pu-rung" aria-hidden="true">
          {Array.from({ length: всего }, (_, i) => (
            <i
              key={i}
              className={
                skipped.has(i) ? "skip" : passed.has(i) ? "done" : at && at.ord === i ? "at" : ""
              }
            />
          ))}
        </div>
      </header>

      <section className="pu-band">
        <div className="pu-h">
          <h2>Делают агенты</h2>
          <span className={`pu-cnt${c.working.length ? "" : " calm"}`}>{c.working.length}</span>
        </div>
        {c.working.length === 0 ? (
          <p className="pu-empty">Никто не занят: очередь задач пуста либо прогоны завершены.</p>
        ) : (
          <ul className="pu-runs">
            {c.working.map((r) => (
              <li key={r.task}>
                <span className="pu-id">{r.task}</span>
                <span className="pu-state">{ХОД[r.state] ?? r.state}</span>
                <span className="pu-t">{r.title || "—"}</span>
                <span className="pu-meta">
                  попытка {r.attempt}
                  {r.branch && <> · {r.branch}</>}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* От тебя ждут — единственное, что без человека не сдвинется. */}
      <section className="pu-band">
        <div className="pu-h">
          <h2>От тебя ждут</h2>
          <span className={`pu-cnt${c.asks.length ? " hot" : " calm"}`}>{c.asks.length}</span>
          {c.asks.length > 0 && <span className="pu-note">без ответа конвейер стоит</span>}
        </div>
        {c.asks.length === 0 ? (
          <p className="pu-empty">
            <b>Ничего не ждут.</b> Вопросов владельцу не отдано — конвейер идёт сам.
          </p>
        ) : (
          <ul className="pu-asks">
            {c.asks.slice(0, 12).map((a) => (
              <li key={a.id} className={open_ === a.id ? "open" : ""}>
                <span className="pu-id">{a.id}</span>
                <button
                  className="pu-do key"
                  type="button"
                  onClick={() => { setOpen(open_ === a.id ? "" : a.id); setDraft(""); setBeef(""); }}
                >
                  {open_ === a.id ? "Свернуть" : "Ответить"}
                </button>
                <span className="pu-t">{a.title}</span>
                {open_ === a.id && (
                  <div className="pu-form">
                    <textarea
                      className="pu-draft"
                      value={draft}
                      onChange={(e) => setDraft(e.target.value)}
                      placeholder="Ответ владельца — он попадёт в вопрос и закроет ступень"
                      rows={4}
                      autoFocus
                    />
                    <div className="pu-act">
                      <button
                        className="pu-do key"
                        type="button"
                        disabled={busy || !draft.trim()}
                        onClick={() => void ответить(a)}
                      >
                        {busy ? "Пишу…" : "Записать ответ"}
                      </button>
                      <span className="pu-hint">запись идёт дверью харнеса, как и всё прочее</span>
                    </div>
                    {beef && <p className="pu-beef">{beef}</p>}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
        {c.asks.length > 12 && (
          <button className="pu-more" type="button" onClick={() => onGo?.("questions")}>
            ещё {c.asks.length - 12} →
          </button>
        )}
      </section>

      <section className="pu-band">
        <div className="pu-h">
          <h2>Что поехало</h2>
          <span className={`pu-cnt${c.moved.length ? " hot" : " calm"}`}>{c.moved.length}</span>
          {c.moved.length > 0 && <span className="pu-note">стояли на том, что правили после них</span>}
        </div>
        {c.moved.length === 0 ? (
          <p className="pu-empty">
            <b>Ничего не поехало.</b> Ни одна запись не стоит на том, что изменили позже.
          </p>
        ) : (
          <>
            <ul className="pu-moved">
              {c.moved.slice(0, 8).map((m) => (
                <li key={`${m.kind}-${m.id}`}>
                  <span className="pu-kind">{зовут(m.kind)}</span>
                  <span className="pu-id">{m.id}</span>
                  <span className="pu-why">{m.why}</span>
                </li>
              ))}
            </ul>
            <div className="pu-chips">
              {c.movedByKind.map((k) => (
                <span key={k.kind} className="pu-chip hot">
                  {зовут(k.kind)} <b>{k.count}</b>
                </span>
              ))}
              <button className="pu-more" type="button" onClick={() => onGo?.("depends")}>
                вся цепочка →
              </button>
            </div>
          </>
        )}
      </section>
    </div>
  );
}
