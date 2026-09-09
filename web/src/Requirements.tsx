import type React from "react";
import { useEffect, useMemo, useState } from "react";
import { loadLinks, loadSummary, type LinkItem, type SummaryRow } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";
import { Provenance } from "./Provenance";

/**
 * Раздел требований: что я обязан сделать, чем это доказано и откуда взялось.
 *
 * Страница строится на ОДНОЙ ручке сводки — той же, что будет у решений и
 * историй. Свой запрос на каждый раздел разошёлся бы с соседним молча, как уже
 * разошлись две таблицы связей задача→требование.
 *
 * Дыра здесь — не подвал, а первая полка: требование без проверки заявлено, но
 * ничем не доказывается, и это главное, что страница обязана показать. Матрица,
 * в которой одни зелёные строки, не мерит покрытие — она его изображает.
 */
type Hole = { key: string; label: string; test: (r: SummaryRow) => boolean };

const HOLES: Hole[] = [
  { key: "no-check", label: "без единой проверки", test: (r) => Number(r["checks"]) === 0 },
  { key: "no-story", label: "не названо ни одной историей", test: (r) => Number(r["stories"]) === 0 },
  { key: "no-task", label: "не взято ни одной задачей", test: (r) => Number(r["tasks"]) === 0 },
  { key: "no-need", label: "без родителя-потребности", test: (r) => r["kind"] === "FR" && Number(r["needs"]) === 0 },
];

const NUM: Record<string, string> = {
  checks: "проверок", stories: "историй", tasks: "задач", needs: "потребн.",
};

export function Requirements({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(() => loadSummary(projectId, "requirement").then((d) => d.rows), [projectId]);
  const rows = live.data;
  const failed = live.failed;
  const [area, setArea] = useState("");
  const [prio, setPrio] = useState("");
  const [shape, setShape] = useState("");
  const [hole, setHole] = useState("");
  const [query, setQuery] = useState("");
  // Выбор хранится ИМЕНЕМ: строки перезапрашиваются, и сохранённый объект стал
  // бы прошлым — панель показывала бы вчерашние числа рядом со свежей таблицей.
  const [chosenId, setChosenId] = useState<string>("");
  const [sets, setSets] = useState<Record<string, LinkItem[]> | null>(null);
  const chosen = (rows ?? []).find((r) => r.id === chosenId) ?? null;
  const setChosen = (r: SummaryRow | null) => setChosenId(r?.id ?? "");

  useEffect(() => {
    if (!chosenId) return;
    setSets(null);
    void loadLinks(projectId, "requirement", chosenId)
      .then((d) => setSets(d.sets))
      .catch(() => setSets({}));
  }, [projectId, chosenId]);

  const areas = useMemo(
    () => [...new Set((rows ?? []).map((r) => String(r["area"])).filter(Boolean))].sort(),
    [rows],
  );
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (rows ?? []).filter(
      (r) =>
        (!area || r["area"] === area) &&
        (!prio || r["priority"] === prio) &&
        (!shape || r["kind"] === shape) &&
        (!hole || HOLES.find((h) => h.key === hole)?.test(r)) &&
        (!q || r.id.toLowerCase().includes(q) || String(r["title"]).toLowerCase().includes(q)),
    );
  }, [rows, area, prio, shape, hole, query]);

  if (failed) return <p className="empty">Не читается: <code>{failed}</code></p>;
  if (!rows) return <p className="empty">Читаю требования…</p>;

  const covered = rows.filter((r) => Number(r["checks"]) > 0).length;
  const open = rows.length - covered;

  return (
    <>
      <div className="head">
        <div>
          <h1>Требования</h1>
          <div className="prov">что обязано быть · чем доказано · откуда выросло</div>
        </div>
        {/* Три числа, никогда одно. Неизвестного здесь ноль — и это сказано,
            а не подразумевается: покрытие считается по всем требованиям. */}
        <div className="three">
          <Live at={live.at} again={live.again} />
          <span className="t-done"><b>{covered}</b> покрыто</span>
          <span className="t-open"><b>{open}</b> без проверки</span>
          <span className="t-unknown"><b>0</b> не отвечается</span>
        </div>
      </div>

      <div className="holes-shelf">
        {HOLES.map((h) => {
          const n = rows.filter(h.test).length;
          return (
            <button
              type="button"
              key={h.key}
              className={`gap${hole === h.key ? " on" : ""}${n === 0 ? " zero" : ""}`}
              onClick={() => setHole(hole === h.key ? "" : h.key)}
            >
              <b>{n}</b>
              <span>{h.label}</span>
            </button>
          );
        })}
      </div>

      <div className="rows-bar">
        <input
          className="rows-search"
          type="search"
          value={query}
          placeholder="имя или текст…"
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Поиск требования"
        />
        <Pick value={shape} set={setShape} all={["FR", "NFR"]} label="вид" />
        <Pick value={prio} set={setPrio} all={["О", "Ж", "П"]} label="приоритет" />
        <Pick value={area} set={setArea} all={areas} label="подсистема" />
        <span className="prov">{shown.length} из {rows.length}</span>
      </div>

      <div className={`split${chosen ? " open" : ""}`}>
        <div className="scroll-x">
          <table className="rows">
            <thead>
              <tr>
                <th>имя</th><th>что обязано</th><th>подс.</th><th>пр.</th>
                {["checks", "stories", "tasks", "needs"].map((n) => <th key={n} className="n">{NUM[n]}</th>)}
              </tr>
            </thead>
            <tbody>
              {shown.slice(0, 400).map((r) => (
                <tr
                  key={r.id}
                  className={`row${chosen?.id === r.id ? " on" : ""}`}
                  onClick={() => setChosen(chosen?.id === r.id ? null : r)}
                >
                  <td className="k">{r.id}</td>
                  <td className="t">{String(r["title"])}</td>
                  <td className="k dim">{String(r["area"])}</td>
                  <td className="k dim">{String(r["priority"])}</td>
                  {["checks", "stories", "tasks", "needs"].map((n) => (
                    <td key={n} className={`n${Number(r[n]) === 0 ? " hole" : ""}`}>{Number(r[n])}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {shown.length > 400 ? <p className="side-note">ещё {shown.length - 400} — сузьте отбор</p> : null}
        </div>

        {chosen ? (
          <aside className="panel">
            <div className="panel-head">
              <b>{chosen.id}</b>
              <button type="button" className="ghost" onClick={() => setChosen(null)}>закрыть</button>
            </div>
            <p className="panel-text">{String(chosen["title"])}</p>
            <div className="panel-fields">
              <span>{String(chosen["kind"])}</span>
              <span>{String(chosen["area"])}</span>
              <span>приоритет {String(chosen["priority"]) || "—"}</span>
              {chosen["satisfied"] ? <span className="done">{String(chosen["satisfied"])}</span> : null}
            </div>
            {sets === null ? (
              <p className="empty">Читаю связи…</p>
            ) : (
              <div className="panel-sets">
                {[
                  ["needs", "выросло из"],
                  ["checks", "доказывается"],
                  ["stories", "названо историями"],
                  ["tasks", "взято задачами"],
                  ["decisions", "затронуто решениями"],
                ].map(([key, label]) => {
                  const list = sets[key!] ?? [];
                  return (
                    <div key={key} className="panel-set">
                      <h4>
                        {label} <i>{list.length}</i>
                      </h4>
                      {/* Пустота называется словом: «нечем» и «не смотрели» —
                          разные ответы, и первый должен быть виден. */}
                      {list.length ? (
                        <ul>
                          {list.map((l) => (
                            <li key={l.kind + l.id}>
                              <code>{l.id}</code> {without(l.id, l.title)}
                            </li>
                          ))}
                        </ul>
                      ) : (
                        <p className="none">ни одного</p>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </aside>
        ) : null}
      </div>

      <Provenance
        source="project_requirements, project_checks, связи историй и задач"
        computed="покрытие и дыры — запросом, ни одна колонка не заполняется руками"
      />
    </>
  );
}

/**
 * Заголовок без повторённого имени: у истории и задачи заголовок в наборе
 * начинается с собственного имени, и рядом со ссылкой оно вышло бы дважды.
 */
function without(id: string, title: string): string {
  const t = title.trim();
  return t.startsWith(id) ? t.slice(id.length).replace(/^\s*[·—-]\s*/, "") : t;
}

function Pick({ value, set, all, label }: {
  value: string; set: (v: string) => void; all: string[]; label: string;
}): React.JSX.Element {
  return (
    <select className="pick" value={value} onChange={(e) => set(e.target.value)} aria-label={label}>
      <option value="">{label}: любая</option>
      {all.map((a) => <option key={a} value={a}>{a}</option>)}
    </select>
  );
}
