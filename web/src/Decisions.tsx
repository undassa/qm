import type React from "react";
import { useEffect, useState } from "react";
import { loadEntityByName, loadLinks, loadSummary, type LinkItem } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Архитектура: решения и то, что они ОТВЕРГЛИ.
 *
 * Отвергнутый вариант — единственное место, где видно, что выбор вообще был.
 * Их 523, и до сих пор они не показывались нигде: решение читалось как
 * единственно возможное, а спор, который за ним стоял, исчезал. Поэтому
 * альтернативы здесь не подвал карточки, а её главная половина.
 */
export function Decisions({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(() => loadSummary(projectId, "decision").then((d) => d.rows), [projectId]);
  const rows = live.data;
  const [chosenId, setChosenId] = useState<string>("");
  const [entity, setEntity] = useState<Record<string, unknown> | null>(null);
  const [links, setLinks] = useState<Record<string, LinkItem[]> | null>(null);
  const [query, setQuery] = useState("");

  const chosen = (rows ?? []).find((r) => r.id === chosenId) ?? null;

  useEffect(() => {
    if (!chosen) return;
    setEntity(null);
    setLinks(null);
    void loadEntityByName(projectId, "decision", chosen.id)
      .then((e) => setEntity(e as unknown as Record<string, unknown>))
      .catch(() => setEntity({}));
    void loadLinks(projectId, "decision", chosen.id)
      .then((d) => setLinks(d.sets))
      .catch(() => setLinks({}));
  }, [projectId, chosenId]);

  if (!rows) return <p className="empty">Читаю решения…</p>;
  const q = query.trim().toLowerCase();
  const shown = rows.filter((r) => !q || r.id.toLowerCase().includes(q) || String(r.title).toLowerCase().includes(q));
  const noAlternatives = rows.filter((r) => Number(r["alternatives"] ?? 0) === 0).length;

  return (
    <>
      <Live at={live.at} again={live.again} />
      <header className="head">
        <h1>Архитектура</h1>
        <p className="note">
          Решений {rows.length}. Без единого отвергнутого варианта — <b>{noAlternatives}</b>:
          у такого решения по документу не видно, был ли выбор вообще.
        </p>
        <input type="search" placeholder="имя или заголовок" value={query} onChange={(e) => setQuery(e.target.value)} />
      </header>

      <div className="split">
        <table className="rows">
          <thead>
            <tr><th>Решение</th><th>Состояние</th><th>Отвергнуто</th><th>Связей</th></tr>
          </thead>
          <tbody>
            {shown.map((r) => (
              <tr key={r.id} className={r.id === chosenId ? "on" : ""} onClick={() => setChosenId(r.id)}>
                <td><code>{r.id}</code> {String(r.title)}</td>
                <td>{String(r["status"] ?? "")}</td>
                <td className={Number(r["alternatives"] ?? 0) === 0 ? "warn" : ""}>{String(r["alternatives"] ?? 0)}</td>
                <td>{String(r["links"] ?? 0)}</td>
              </tr>
            ))}
          </tbody>
        </table>

        <aside className="panel">
          {!chosen ? (
            <p className="empty">Выберите решение — справа встанут отвергнутые варианты.</p>
          ) : (
            <>
              <h2><code>{chosen.id}</code> {String(chosen.title)}</h2>
              <p className="note">
                {String(chosen["status"] ?? "")} · {String(chosen["date"] ?? "дата не названа")} ·
                решали: {String(chosen["deciders"] ?? "не названы")}
              </p>
              <h3>Отвергнутые варианты</h3>
              <Alternatives entity={entity} />
              <h3>Связи</h3>
              {!links ? <p className="empty">Читаю…</p> : (
                Object.entries(links).filter(([, v]) => v.length).map(([set, items]) => (
                  <p key={set}><b>{set}</b>: {items.map((i) => i.id).join(" · ")}</p>
                ))
              )}
            </>
          )}
        </aside>
      </div>
    </>
  );
}

/**
 * Отвергнутые приходят строками сущности. Пусто — это НЕ «вариантов не было»:
 * это «документ о них молчит», и сказано так прямо.
 */
function Alternatives({ entity }: { entity: Record<string, unknown> | null }): React.JSX.Element {
  if (!entity) return <p className="empty">Читаю…</p>;
  const text = String((entity as { content?: string }).content ?? "");
  const at = text.search(/^##\s+Отвергнутые варианты/m);
  if (at < 0) {
    return (
      <p className="empty warn">
        Раздела «Отвергнутые варианты» в документе нет. Это не «выбора не было» —
        это «о выборе не написано».
      </p>
    );
  }
  const rest = text.slice(at);
  const end = rest.slice(2).search(/^##\s/m);
  return <pre className="body">{(end < 0 ? rest : rest.slice(0, end + 2)).trim()}</pre>;
}
