import type React from "react";
import { useEffect, useState } from "react";
import { loadLinks, loadEntityByName, loadSummary, type LinkItem, type SummaryRow } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";
import { loadRetired } from "./api";

/**
 * Правила: конституция и словарь.
 *
 * Главный вопрос раздела — «чему это противоречит и кто на это опирается».
 * Поэтому первая колонка после имени — **число ссылающихся документов**: статья,
 * на которую не сослался никто, либо мертва, либо объявлена не там, где нужна.
 * Вес правила виден до того, как его открыли.
 */
export function Rules({ projectId }: { projectId: string }): React.JSX.Element {
  const [tab, setTab] = useState<"article" | "term">("article");
  const live = useLive(() => loadSummary(projectId, tab).then((d) => d.rows), [projectId, tab]);
  const rows = live.data;
  const [chosenId, setChosenId] = useState<string>("");
  const [body, setBody] = useState<string | null>(null);
  const [cited, setCited] = useState<LinkItem[] | null>(null);
  const [retiredWords, setRetiredWords] = useState<string[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!projectId) return;
    void loadRetired(projectId).then(setRetiredWords).catch(() => setRetiredWords([]));
  }, [projectId]);

  const chosen = (rows ?? []).find((r) => r.id === chosenId) ?? null;
  const setChosen = (r: SummaryRow | null) => setChosenId(r?.id ?? "");

  useEffect(() => {
    if (!chosen) return;
    setBody(null);
    setCited(null);
    if (tab === "article") {
      void loadEntityByName(projectId, "article", chosen.id)
        .then((e) => setBody(String((e as { entity?: { body?: string } }).entity?.body ?? "")))
        .catch(() => setBody(""));
    } else {
      setBody(String(chosen["meaning"] ?? ""));
    }
    // У статьи ссылки объявлены таблицей, у термина — только упоминанием.
    void loadLinks(projectId, tab === "article" ? "article" : "term", chosen.id)
      .then((d) => setCited(d.sets["citedBy"] ?? []))
      .catch(() => setCited([]));
  }, [projectId, tab, chosenId]);

  if (!rows) return <p className="empty">Читаю правила…</p>;
  const q = query.trim().toLowerCase();
  const shown = rows.filter((r) => !q || r.id.toLowerCase().includes(q) || String(r["title"]).toLowerCase().includes(q));
  // Снятое слово в словаре не лежит — его оттуда убрали. Поэтому снятые
  // показываются своей полкой, а не колонкой, которая всегда пуста.
  const retired = retiredWords.length;

  return (
    <>
      <div className="head">
        <div>
          <h1>Правила</h1>
          <div className="prov">конституция и словарь · чему это противоречит и кто на это опирается</div>
        </div>
        <div className="three">
          <Live at={live.at} again={live.again} />
          {tab === "article" ? (
            <>
              <span className="t-done"><b>{rows.length}</b> статей</span>
              <span className="t-open"><b>{rows.reduce((n, r) => n + Number(r["citedBy"]), 0)}</b> ссылок на них</span>
            </>
          ) : (
            <>
              <span className="t-done"><b>{rows.length}</b> терминов</span>
              <span className="t-unknown"><b>{retired}</b> слов снято</span>
            </>
          )}
        </div>
      </div>

      <div className="rows-bar">
        <button type="button" className={`gap${tab === "article" ? " on" : ""}`} onClick={() => setTab("article")}>
          <b>{tab === "article" ? rows.length : 17}</b><span>статей конституции</span>
        </button>
        <button type="button" className={`gap${tab === "term" ? " on" : ""}`} onClick={() => setTab("term")}>
          <b>{tab === "term" ? rows.length : 42}</b><span>термина словаря</span>
        </button>
        {retired ? (
          <span className="retired-shelf" title="слова, которые набор снял: встреченные в свежем тексте — находка">
            снято: {retiredWords.join(" · ")}
          </span>
        ) : null}
        <input
          className="rows-search"
          type="search"
          value={query}
          placeholder="искать…"
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Поиск правила"
        />
      </div>

      <div className={`split${chosen ? " open" : ""}`}>
        <div className="scroll-x">
          <table className="rows">
            <thead>
              <tr>
                <th>{tab === "article" ? "№" : "имя"}</th>
                <th>{tab === "article" ? "статья" : "термин"}</th>
                {tab === "term" ? <th>область</th> : null}
                <th className="n">{tab === "article" ? "ссылок" : "упоминаний"}</th>
                {tab === "article" ? <th className="n">знаков</th> : <th>снято</th>}
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr
                  key={r.id}
                  className={`row${chosen?.id === r.id ? " on" : ""}`}
                  onClick={() => setChosen(chosen?.id === r.id ? null : r)}
                >
                  <td className="k">{r.id}</td>
                  <td className="t">{String(r["title"])}</td>
                  {tab === "term" ? <td className="k dim">{String(r["area"] ?? "")}</td> : null}
                  <td className={`n${Number(r[tab === "article" ? "citedBy" : "mentions"]) === 0 ? " hole" : ""}`}>
                    {Number(r[tab === "article" ? "citedBy" : "mentions"])}
                  </td>
                  {tab === "article" ? (
                    <td className="n">{Number(r["chars"])}</td>
                  ) : (
                    <td className="k dim">{String(r["retired"] ?? "")}</td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {chosen ? (
          <aside className="panel">
            <div className="panel-head">
              <b>{tab === "article" ? `Статья ${chosen.id}` : chosen.id}</b>
              <button type="button" className="ghost" onClick={() => setChosen(null)}>закрыть</button>
            </div>
            <p className="panel-text">{String(chosen["title"])}</p>
            {body === null ? <p className="empty">Читаю…</p> : <pre className="rule-body">{body}</pre>}
            <div className="panel-set">
              <h4>
                на это опираются <i>{cited?.length ?? "…"}</i>
              </h4>
              {cited === null ? null : cited.length ? (
                <ul>
                  {[...new Set(cited.map((b) => b.id))].slice(0, 40).map((from) => (
                    <li key={from}><code>{from}</code></li>
                  ))}
                </ul>
              ) : (
                <p className="none">никто — правило либо мертво, либо объявлено не там</p>
              )}
            </div>
          </aside>
        ) : null}
      </div>

      <Provenance
        source="project_articles, project_article_references, project_terms"
        computed="вес правила — числом ссылающихся документов, а не мнением"
      />
    </>
  );
}
