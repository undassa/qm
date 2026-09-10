import type React from "react";
import { useEffect, useMemo, useState } from "react";
import { loadLinks, loadSummary, type LinkItem, type SummaryRow } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";
import { EntityDrawer } from "./EntityDrawer";

/**
 * Пользователь: путь, истории, области, экраны.
 *
 * Карта пути здесь не рисуется — она **собирается из историй**. Нарисованная
 * карта устаревает в тот же квартал и живёт отдельной жизнью от работы; эта
 * не может: колонка — фаза, карточка — история, а под карточкой видно то, чего
 * нет ни в одной рисовалке, — требования, экраны и задачи этой истории.
 *
 * Разнобой в написании фазы не сглаживается. Колонка берётся по ведущему
 * номеру, а сколько разных написаний за ней стоит — сказано числом: молча
 * слить «3 · разбор» и «3–4 · пейдж и разбор» значило бы придумать порядок,
 * которого набор не объявлял.
 */
const NO_PHASE = "фаза не разобрана";

/** Ведущий номер фазы: «3 · разбор» → «3», «4–5 · разбор и профилактика» → «4». */
function phaseOf(raw: string): string {
  const m = /^\s*(\d+)/.exec(raw.trim());
  return m ? m[1]! : NO_PHASE;
}

export function Users({ projectId, onFind }: { projectId: string; onFind?: (q: string) => void }): React.JSX.Element {
  const live = useLive(
    () => Promise.all([loadSummary(projectId, "story"), loadSummary(projectId, "feature")])
            .then(([s, f]) => ({ stories: s.rows, features: f.rows })),
    [projectId],
  );
  const stories = live.data?.stories ?? null;
  const features = live.data?.features ?? null;
  const [view, setView] = useState<"path" | "features">("path");
  const [persona, setPersona] = useState("");
  const [chosenId, setChosenId] = useState<string>("");
  const [sets, setSets] = useState<Record<string, LinkItem[]> | null>(null);
  const chosen = [...(stories ?? []), ...(features ?? [])].find((r) => r.id === chosenId) ?? null;
  const setChosen = (r: SummaryRow | null) => setChosenId(r?.id ?? "");

  useEffect(() => {
    if (!chosenId) return;
    setSets(null);
    const kind = view === "path" ? "story" : "feature";
    void loadLinks(projectId, kind, chosenId).then((d) => setSets(d.sets)).catch(() => setSets({}));
  }, [projectId, view, chosenId]);

  const personas = useMemo(
    () => [...new Set((stories ?? []).map((s) => String(s["persona"])).filter(Boolean))].sort(),
    [stories],
  );
  const columns = useMemo(() => {
    const by = new Map<string, SummaryRow[]>();
    for (const s of stories ?? []) {
      if (persona && s["persona"] !== persona) continue;
      const key = phaseOf(String(s["phase"] ?? ""));
      by.set(key, [...(by.get(key) ?? []), s]);
    }
    return [...by.entries()].sort((a, b) =>
      a[0] === NO_PHASE ? 1 : b[0] === NO_PHASE ? -1 : Number(a[0]) - Number(b[0]),
    );
  }, [stories, persona]);

  if (!stories || !features) return <p className="empty">Читаю истории…</p>;
  const spellings = new Set(stories.map((s) => String(s["phase"] ?? ""))).size;
  const noReq = stories.filter((s) => Number(s["requirements"]) === 0).length;
  const noScreen = stories.filter((s) => Number(s["screens"]) === 0).length;

  return (
    <>
      <div className="head">
        <div>
          <h1>Пользователь</h1>
          <div className="prov">путь · истории · области — собрано из историй, а не нарисовано</div>
        </div>
        <div className="three">
          <Live at={live.at} again={live.again} />
          <span className="t-done"><b>{stories.length}</b> историй</span>
          <span className="t-open"><b>{columns.length}</b> фаз пути</span>
          <span className="t-unknown"><b>{spellings}</b> написаний фазы</span>
        </div>
      </div>

      <div className="rows-bar">
        <button type="button" className={`gap${view === "path" ? " on" : ""}`} onClick={() => { setView("path"); setChosen(null); }}>
          <b>{stories.length}</b><span>путь по фазам</span>
        </button>
        <button type="button" className={`gap${view === "features" ? " on" : ""}`} onClick={() => { setView("features"); setChosen(null); }}>
          <b>{features.length}</b><span>функциональных областей</span>
        </button>
        <button type="button" className={`gap${noReq ? "" : " zero"}`} onClick={() => setPersona("")}>
          <b>{noReq}</b><span>историй без требований</span>
        </button>
        <button type="button" className="gap zero">
          <b>{noScreen}</b><span>историй без экрана</span>
        </button>
        {view === "path" ? (
          <select className="pick" value={persona} onChange={(e) => setPersona(e.target.value)} aria-label="персона">
            <option value="">персона: любая</option>
            {personas.map((p) => <option key={p} value={p}>{p}</option>)}
          </select>
        ) : null}
      </div>

      <div className={`split${chosen ? " open" : ""}`}>
        {view === "path" ? (
          <div className="board">
            {columns.map(([phase, list]) => (
              <section className="phase-col" key={phase}>
                <h3>
                  {phase === NO_PHASE ? NO_PHASE : `фаза ${phase}`} <i>{list.length}</i>
                </h3>
                {/* Написания внутри колонки показываются как есть: набор их
                    не сводил, и мы не сводим. */}
                <div className="phase-spellings">
                  {[...new Set(list.map((s) => String(s["phase"])))].slice(0, 4).join(" · ")}
                </div>
                {/* Колонка рисует часть: сто историй в одной колонке — это
                    экран за экраном мимо соседних, и сравнить их становится нельзя. */}
                {list.slice(0, 24).map((s) => (
                  <button
                    type="button"
                    key={s.id}
                    className={`story${chosen?.id === s.id ? " on" : ""}`}
                    onClick={() => setChosen(chosen?.id === s.id ? null : s)}
                  >
                    <b>{s.id}</b>
                    <span className="story-t">{String(s["title"]).replace(String(s.id), "").replace(/^\s*·\s*/, "")}</span>
                    <span className="story-n">
                      {String(s["persona"]) || "персона не названа"} ·{" "}
                      <i className={Number(s["requirements"]) === 0 ? "zero" : ""}>{Number(s["requirements"])} тр.</i> ·{" "}
                      <i className={Number(s["screens"]) === 0 ? "zero" : ""}>{Number(s["screens"])} эк.</i>
                    </span>
                  </button>
                ))}
                {list.length > 24 ? <div className="col-more">ещё {list.length - 24} — сузьте персоной</div> : null}
              </section>
            ))}
          </div>
        ) : (
          <div className="scroll-x">
            <table className="rows">
              <thead>
                <tr><th>область</th><th>что это</th><th className="n">историй</th><th className="n">требований</th><th className="n">покрыто</th></tr>
              </thead>
              <tbody>
                {features.map((f) => (
                  <tr key={f.id} className={`row${chosen?.id === f.id ? " on" : ""}`}
                      onClick={() => setChosen(chosen?.id === f.id ? null : f)}>
                    <td className="k">{f.id}</td>
                    <td className="t">{String(f["title"])}</td>
                    <td className="n">{Number(f["stories"])}</td>
                    <td className="n">{Number(f["requirements"])}</td>
                    <td className={`n${Number(f["covered"]) < Number(f["requirements"]) ? " hole" : ""}`}>
                      {Number(f["covered"])}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {chosen ? (
          <aside className="panel">
            <div className="panel-head">
              <b>{chosen.id}</b>
              <button type="button" className="ghost" onClick={() => setChosen(null)}>закрыть</button>
            </div>
            <p className="panel-text">{String(chosen["title"])}</p>
            {view === "path" ? (
              <div className="panel-fields">
                <span>{String(chosen["persona"]) || "персона не названа"}</span>
                <span>{String(chosen["phase"]) || "фаза не названа"}</span>
                {chosen["feature"] ? <span>{String(chosen["feature"])}</span> : null}
              </div>
            ) : null}
            {sets === null ? (
              <p className="empty">Читаю связи…</p>
            ) : (
              <div className="panel-sets">
                {Object.entries(sets).map(([key, list]) => (
                  <div key={key} className="panel-set">
                    <h4>{LABEL[key] ?? key} <i>{list.length}</i></h4>
                    {list.length ? (
                      <ul>
                        {list.map((l) => (
                          <li key={l.kind + l.id}><code>{l.id}</code> {l.title}</li>
                        ))}
                      </ul>
                    ) : (
                      <p className="none">ни одного</p>
                    )}
                  </div>
                ))}
              </div>
            )}
          </aside>
        ) : null}
      </div>

      <Provenance
        source="project_stories, project_features, связи требований и экранов"
        computed="колонка пути — по ведущему номеру фазы; сколько разных написаний за ней, сказано числом"
      />
    {chosen ? (
      <EntityDrawer
        projectId={projectId}
        kind="story"
        id={chosen.id}
        title={`История ${chosen.id}`}
        subtitle={String(chosen["title"] ?? "")}
        onClose={() => setChosenId("")}
        onFind={onFind}
      />
    ) : null}
    </>
  );
}

const LABEL: Record<string, string> = {
  requirements: "опирается на требования",
  screens: "показывается экранами",
  needs: "выросло из потребностей",
  features: "заявлено областями",
  stories: "истории области",
};
