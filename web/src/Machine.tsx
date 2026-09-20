import type React from "react";
import { type Lang, say } from "./say";
import { tool } from "./api";
import { ago, useLive } from "./live";

/**
 * Машина — состояние прибора, а не корпуса.
 *
 * Остальные страницы отвечают на вопросы о наборе: что написано, что готово,
 * чем доказано. Этой не было вовсе, и владелец про неё сказал прямо: «я просто
 * не вижу монитора». Состояние машины лежало в трёх местах — прогоны тестов в
 * базе, свежесть датчиков в базе, очередь в третьей ручке, — и чтобы его
 * собрать, надо было знать все три и спросить порознь.
 *
 * Показывается ровно то, что знают двери. Живость раннеров CI и контейнеров
 * сюда НЕ подмешивается: сервер о них не знает, и рисовать их зелёными по
 * умолчанию значило бы ровно тот вырожденный зелёный, который прибор и ловит.
 */
interface Tests {
  at: number;
  cleanAt: number;
  runs: number;
  counts: Record<string, number>;
  failed: string[];
}
interface Sensor {
  fact: string;
  about: string;
  lastAt: number | null;
  staleAfterMs: number | null;
  rows: number | null;
  silent: boolean;
  stale: boolean;
}
interface Sensors {
  sensors: Sensor[];
  undeclared: string[];
}
interface Run {
  task: string;
  agent: string | null;
  state: string;
  attempt: number;
  title: string;
}
interface Next {
  task?: { id: string; title: string } | null;
  why?: string;
}

/** Строка счёта: число и его имя, всегда рядом. */
function Count({ n, what, tone }: { n: number; what: string; tone?: string }): React.JSX.Element {
  return (
    <span className={`mx-count${tone ? ` ${tone}` : ""}`}>
      <b>{n}</b> {what}
    </span>
  );
}

export function Machine({
  projectId,
  lang,
}: {
  projectId: string;
  lang: Lang;
}): React.JSX.Element {
  const live = useLive(
    () =>
      Promise.all([
        tool<Tests>(projectId, "test-run"),
        tool<Sensors>(projectId, "sensors"),
        tool<{ runs: Run[] }>(projectId, "runs", { limit: 20 }),
        tool<Next>(projectId, "next-task"),
      ]).then(([t, s, r, n]) => ({ tests: t, sensors: s, runs: r.runs ?? [], next: n })),
    [projectId],
    undefined,
    lang,
  );

  if (!live.data) return <p className="empty">{say(lang, "mx.reading")}</p>;
  const { tests, sensors, runs, next } = live.data;

  const passed = tests.counts["passed"] ?? 0;
  const failed = tests.counts["failed"] ?? 0;
  // Датчик молчащий и датчик протухший — РАЗНОЕ. Первый не подавал ни разу:
  // о его предмете неизвестно вообще ничего. Второй подавал и замолчал: его
  // число лежит в базе и выглядит свежим, пока не спросишь срок.
  const silent = sensors.sensors.filter((s) => s.silent);
  const stale = sensors.sensors.filter((s) => s.stale);
  const working = runs.filter((r) => r.state === "running" || r.state === "working");
  const waiting = runs.filter((r) => r.state === "waiting");

  return (
    <div className="mx">
      <section className="mx-card">
        <h2>{say(lang, "mx.tests")}</h2>
        {tests.runs === 0 ? (
          <p className="empty">{say(lang, "mx.tests.never")}</p>
        ) : (
          <>
            <p className="mx-row">
              <Count n={passed} what={say(lang, "mx.passed")} tone="ok" />
              <Count n={failed} what={say(lang, "mx.failed")} tone={failed ? "bad" : ""} />
            </p>
            <p className="mx-note">
              {tests.cleanAt
                ? `${say(lang, "mx.clean")}: ${ago(tests.cleanAt, Date.now(), lang)}`
                : say(lang, "mx.clean.never")}
            </p>
            {tests.failed.length > 0 && (
              <ul className="mx-list">
                {tests.failed.map((f) => (
                  <li key={f} className="bad">
                    {f}
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </section>

      <section className="mx-card">
        <h2>{say(lang, "mx.sensors")}</h2>
        <p className="mx-row">
          <Count n={sensors.sensors.length - silent.length - stale.length} what={say(lang, "mx.fresh")} tone="ok" />
          <Count n={stale.length} what={say(lang, "mx.stale")} tone={stale.length ? "bad" : ""} />
          <Count n={silent.length} what={say(lang, "mx.silent")} tone={silent.length ? "bad" : ""} />
          <Count
            n={sensors.undeclared.length}
            what={say(lang, "mx.undeclared")}
            tone={sensors.undeclared.length ? "warn" : ""}
          />
        </p>
        {sensors.undeclared.length > 0 && (
          <p className="mx-note">{say(lang, "mx.undeclared.why")}</p>
        )}
        <ul className="mx-list">
          {[...stale, ...silent].slice(0, 12).map((s) => (
            <li key={s.fact} className="bad">
              <b>{s.fact}</b> — {s.silent ? say(lang, "mx.silent.one") : say(lang, "mx.stale.one")}
              {s.lastAt ? ` · ${ago(s.lastAt, Date.now(), lang)}` : ""}
            </li>
          ))}
          {sensors.undeclared.map((f) => (
            <li key={`u-${f}`} className="warn">
              <b>{f}</b> — {say(lang, "mx.undeclared.one")}
            </li>
          ))}
        </ul>
      </section>

      <section className="mx-card">
        <h2>{say(lang, "mx.queue")}</h2>
        {next.task ? (
          <p>
            <b>{next.task.id}</b> · {next.task.title}
          </p>
        ) : (
          <p className="mx-note bad">{next.why ?? say(lang, "mx.queue.stuck")}</p>
        )}
        <p className="mx-row">
          <Count n={working.length} what={say(lang, "mx.working")} />
          <Count n={waiting.length} what={say(lang, "mx.waiting")} tone={waiting.length ? "warn" : ""} />
        </p>
        <ul className="mx-list">
          {[...working, ...waiting].slice(0, 10).map((r) => (
            <li key={`${r.task}-${r.attempt}`}>
              <b>{r.task}</b> · {r.state}
              {r.agent ? ` · ${r.agent}` : ""} · {r.title}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
