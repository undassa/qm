import type React from "react";
import { tool } from "./api";
import { useLive } from "./live";
import { type Lang, say } from "./say";

/**
 * Версии — как идёт работа по каждой.
 *
 * Версия — единица работы (решение владельца 2026-10-07): в ней сперва все
 * требования, потом все тесты, потом весь код, и каждая несёт ценность сама по
 * себе. Вопрос «как идут дела» поэтому задаётся о версии, а не о плане целиком:
 * сто сорок задач одной полосой не говорят, близок ли следующий выпуск.
 *
 * Всё считает дверь `versions` — место в конвейере тем же видом, по которому
 * судит выдача, — и экран ничего не выводит сам: выведенное здесь разошлось бы
 * с тем, что выдаёт `next-task`.
 */
interface VTask {
  id: string;
  title: string;
  kind: string;
  state: string;
  preflight: "ready" | "blocked" | null;
}
interface Version {
  id: string;
  title: string | null;
  state: "open" | "closed" | "planned";
  place: "open" | "next" | "later" | "closed" | null;
  phase: "prepare" | "tests" | "code" | "release" | "closed";
  tests: { total: number; closed: number };
  code: { total: number; closed: number };
  requirements: number;
  preflight: { ready: number; blocked: number; none: number };
  milestones: { id: string; title: string; tasks: VTask[] }[];
}

function Meter({ label, done, total }: { label: string; done: number; total: number }): React.JSX.Element {
  return (
    <div className="vr-meter">
      <span className="vr-ml">
        {label} <span className="num">{done}/{total}</span>
      </span>
      <span className="bar" role="img" aria-label={`${label} ${done}/${total}`}>
        <i className="bar-done" style={{ width: total ? `${(100 * done) / total}%` : "0" }} />
      </span>
    </div>
  );
}

export function Versions({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const live = useLive(() => tool<{ versions: Version[] }>(projectId, "versions"), [projectId], undefined, lang);
  if (!live.data) return <p className="empty">{live.failed || say(lang, "vr.reading")}</p>;
  const versions = live.data.versions ?? [];
  if (versions.length === 0) return <p className="empty">{say(lang, "vr.none")}</p>;

  return (
    <div className="vr">
      {live.failed ? <p className="side-note warn">{live.failed}</p> : null}
      {versions.map((v) => (
        // Раскрывается родным <details>: клавиатура, чтение с экрана и узкий
        // экран работают без своего кода.
        <details key={v.id} className={`vr-card${v.place === "open" ? " open-v" : ""}${v.state === "closed" ? " dim" : ""}`}>
          <summary>
            <span className="vr-top">
              <b className="vr-id">{v.id}</b>
              <span className={`vr-phase ${v.phase}`}>{say(lang, `vr.phase.${v.phase}`)}</span>
              {v.place === "open" ? <span className="vr-place">{say(lang, "vr.open")}</span> : null}
              {v.place === "next" ? <span className="vr-place next">{say(lang, "vr.next")}</span> : null}
            </span>
            <span className="vr-t">{v.title ?? <em>{say(lang, "vr.noTitle")}</em>}</span>
            <span className="vr-meters">
              <Meter label={say(lang, "vr.tests")} done={v.tests.closed} total={v.tests.total} />
              <Meter label={say(lang, "vr.code")} done={v.code.closed} total={v.code.total} />
            </span>
            <span className="vr-facts">
              <span><span className="num">{v.requirements}</span> {say(lang, "vr.reqs")}</span>
              {v.preflight.blocked > 0 ? (
                <span className="bad"><span className="num">{v.preflight.blocked}</span> {say(lang, "vr.blocked")}</span>
              ) : null}
            </span>
          </summary>
          {v.milestones.map((m) => (
            <section key={m.id} className="vr-ms">
              <h3><span className="vr-id">{m.id}</span> {m.title}</h3>
              {m.tasks.length === 0 ? (
                <p className="wk-note">{say(lang, "vr.noTasks")}</p>
              ) : (
                <ul className="vr-tasks">
                  {m.tasks.map((t) => (
                    <li key={t.id} className={t.state === "closed" ? "dim" : ""}>
                      <span className="vr-id">{t.id}</span>
                      <span className="vr-tt">{t.title}</span>
                      <span className="vr-kind">{t.kind === "red" ? say(lang, "vr.kindRed") : t.kind}</span>
                      <span className={`vr-st${t.preflight === "blocked" ? " bad" : ""}`}>
                        {t.state}{t.preflight === "blocked" ? ` · ${say(lang, "vr.blockedOne")}` : ""}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          ))}
        </details>
      ))}
    </div>
  );
}
