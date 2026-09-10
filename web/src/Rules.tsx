import type React from "react";
import { useEffect, useState } from "react";
import { loadLinks, loadEntityByName, loadSummary, loadRetired, type LinkItem } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";
import { Markdown } from "./Markdown";

/**
 * Правила: конституция и словарь.
 *
 * Главный вопрос раздела — «чему это противоречит и кто на это опирается».
 * Прежде статья открывалась в узкой колонке сбоку: конституция — главный текст
 * проекта, а места ей отводилось меньше, чем таблице её заголовков. Теперь
 * строка раскрывается на месте и во всю ширину, а текст размечен, а не показан
 * сырым.
 *
 * Вес правила — числом ссылающихся документов, и он виден ПОЛОСКОЙ. Статья с
 * восемьюдесятью тремя ссылками и статья с одной стояли одинаковыми числами в
 * колонке; разница между ними — это разница между правилом, которым живут, и
 * правилом, о котором забыли, и её надо видеть не читая.
 */

interface Row {
  id: string;
  [k: string]: unknown;
}

export function Rules({ projectId, onFind }: { projectId: string; onFind?: (q: string) => void }): React.JSX.Element {
  const [tab, setTab] = useState<"article" | "term">("article");
  const live = useLive(() => loadSummary(projectId, tab).then((d) => d.rows), [projectId, tab]);
  const rows = live.data as Row[] | null;
  const [open, setOpen] = useState<string>("");
  const [body, setBody] = useState<string | null>(null);
  const [cited, setCited] = useState<LinkItem[] | null>(null);
  const [retiredWords, setRetiredWords] = useState<string[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!projectId) return;
    void loadRetired(projectId).then(setRetiredWords).catch(() => setRetiredWords([]));
  }, [projectId]);

  const chosen = (rows ?? []).find((r) => r.id === open) ?? null;

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
  }, [projectId, tab, open, chosen]);

  if (!rows) return <p className="empty">Читаю правила…</p>;

  const weightKey = tab === "article" ? "citedBy" : "mentions";
  const q = query.trim().toLowerCase();
  const shown = rows.filter(
    (r) => !q || r.id.toLowerCase().includes(q) || String(r["title"] ?? "").toLowerCase().includes(q),
  );
  const heaviest = Math.max(1, ...rows.map((r) => Number(r[weightKey] ?? 0)));
  const total = rows.reduce((n, r) => n + Number(r[weightKey] ?? 0), 0);
  const dead = rows.filter((r) => Number(r[weightKey] ?? 0) === 0);

  return (
    <>
      <div className="head">
        <div>
          <h1>Правила</h1>
          <div className="prov">конституция и словарь · чему это противоречит и кто на это опирается</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          <b>{rows.length}</b> {tab === "article" ? "статей" : "терминов"} · <b>{total}</b>{" "}
          {tab === "article" ? "ссылок на них" : "упоминаний"}
          {dead.length > 0 ? <> · <b className="bad-n">{dead.length}</b> без единой</> : null}
        </span>
      </div>

      <div className="tabs">
        <button type="button" className={tab === "article" ? "on" : ""} onClick={() => { setTab("article"); setOpen(""); }}>
          конституция
        </button>
        <button type="button" className={tab === "term" ? "on" : ""} onClick={() => { setTab("term"); setOpen(""); }}>
          словарь
        </button>
        <input
          className="rows-search"
          type="search"
          value={query}
          placeholder="искать по имени или заголовку…"
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Поиск правила"
        />
      </div>

      {/*
        Правило, на которое не сослался никто, — не «просто редкое»: оно либо
        мертво, либо объявлено не там, где нужно. Число стоит наверху, а не
        выводится читателем из просмотра всех строк.
      */}
      {dead.length > 0 ? (
        <p className="lede">
          Без единой ссылки — <b>{dead.length}</b>: {dead.slice(0, 8).map((r) => r.id).join(" · ")}
          {dead.length > 8 ? ` …и ещё ${dead.length - 8}` : ""}. Такое правило либо мертво, либо объявлено
          не там, где нужно.
        </p>
      ) : null}

      <ul className="arts">
        {shown.map((r) => {
          const weight = Number(r[weightKey] ?? 0);
          const isOpen = open === r.id;
          return (
            <li key={r.id} className={`art-row${isOpen ? " open" : ""}${weight === 0 ? " dead" : ""}`}>
              <button
                type="button"
                className="art-head"
                onClick={() => setOpen(isOpen ? "" : r.id)}
                aria-expanded={isOpen}
              >
                <span className="art-num">{r.id}</span>
                <span className="art-title">{String(r["title"] ?? "")}</span>
                {/* Колонка области занимает место ВСЕГДА. Пропущенная в режиме
                    конституции, она сдвигала сетку на одну ячейку, и полоска
                    веса схлопывалась в ноль — вес переставал быть виден. */}
                <span className="art-area">{tab === "term" ? String(r["area"] ?? "") : ""}</span>
                <span className="art-w" title={`${weight} ${tab === "article" ? "ссылок" : "упоминаний"}`}>
                  <span className="art-fill" style={{ width: `${Math.round((weight / heaviest) * 100)}%` }} />
                </span>
                <span className="art-n">{weight}</span>
                <span className="art-caret" aria-hidden="true">{isOpen ? "▾" : "▸"}</span>
              </button>

              {isOpen ? (
                <div className="art-open">
                  {body === null ? (
                    <p className="empty">Читаю…</p>
                  ) : body.trim() ? (
                    <div className="art-text">
                      <Markdown body={body} />
                    </div>
                  ) : (
                    <p className="note">Текста нет — объявлен только заголовок.</p>
                  )}

                  <div className="art-cited">
                    <div className="art-k">на это опираются · {cited?.length ?? "…"}</div>
                    {cited === null ? null : cited.length ? (
                      <div className="art-chips">
                        {[...new Set(cited.map((b) => b.id))]
                          .filter((from) => from.trim() !== "")
                          .slice(0, 60)
                          .map((from) => (
                          <button
                            key={from}
                            type="button"
                            className="art-chip"
                            onClick={() => onFind?.(from)}
                            title="открыть документ"
                            >
                              {from}
                            </button>
                          ))}
                      </div>
                    ) : (
                      <p className="note bad-n">
                        Никто. Правило либо мертво, либо объявлено не там, где нужно.
                      </p>
                    )}
                  </div>
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>

      {/*
        Снятое слово в словаре НЕ ЛЕЖИТ — его оттуда убрали. Поэтому снятые
        стоят своей полкой под словарём, а не колонкой, которая всегда пуста, и
        не строкой в панели кнопок, где им нечего объяснять.
      */}
      {tab === "term" && retiredWords.length > 0 ? (
        <section className="retired">
          <h2 className="pick-h">
            Снятые слова
            <span className="pick-g">{retiredWords.length} · встреченное в свежем тексте — находка</span>
          </h2>
          <div className="art-chips">
            {retiredWords.map((w) => (
              <span key={w} className="art-chip off">{w}</span>
            ))}
          </div>
        </section>
      ) : null}

      <Provenance
        source="project_articles, project_article_references, project_terms"
        computed="вес правила — числом ссылающихся документов, а не мнением"
      />
    </>
  );
}
