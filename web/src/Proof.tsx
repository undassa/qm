import type React from "react";
import { useState } from "react";
import { loadSummary } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";
import { Chain } from "./Chain";

/**
 * Доказательство: чем закрыто каждое требование.
 *
 * Главная колонка — **проверок**. Требование без проверки не «почти готово»:
 * оно ничем не закрыто, и зелёный на нём означает только то, что никто не
 * смотрел. Ноль здесь показывается отдельно и первым.
 */
export function Proof({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(() => loadSummary(projectId, "requirement").then((d) => d.rows), [projectId]);
  const rows = live.data;
  const [only, setOnly] = useState<"all" | "bare">("bare");
  const [query, setQuery] = useState("");
  /** Имена, выбранные щелчком по схеме: показывается ровно этот десяток. */
  const [picked, setPicked] = useState<string[] | null>(null);

  if (!rows) return <p className="empty">Читаю требования…</p>;
  const bare = rows.filter((r) => Number(r["checks"] ?? 0) === 0);
  const q = query.trim().toLowerCase();
  const base = picked ? rows.filter((r) => picked.includes(r.id)) : only === "bare" ? bare : rows;
  const shown = base.filter(
    (r) => !q || r.id.toLowerCase().includes(q) || String(r.title).toLowerCase().includes(q),
  );

  return (
    <>
      <Live at={live.at} again={live.again} />
      <header className="head">
        <h1>Доказательство</h1>
        <p className="note">
          Требований {rows.length} · без единой проверки — <b>{bare.length}</b>.
          Требование без проверки ничем не закрыто, и зелёное на нём значит лишь, что никто не смотрел.
        </p>
        <div className="tabs">
          <button type="button" className={only === "bare" ? "on" : ""} onClick={() => { setOnly("bare"); setPicked(null); }}>без проверок</button>
          <button type="button" className={only === "all" ? "on" : ""} onClick={() => { setOnly("all"); setPicked(null); }}>все</button>
        </div>
        <input type="search" placeholder="имя или текст" value={query} onChange={(e) => setQuery(e.target.value)} />
      </header>

      <Chain rows={rows} onPick={(ids) => setPicked(ids)} />
      {picked ? (
        <button type="button" className="fold" onClick={() => setPicked(null)}>
          ✕ показаны {picked.length} выбранных схемой — снять отбор
        </button>
      ) : null}

      <table className="rows">
        <thead><tr><th>Требование</th><th>Вид</th><th>Проверок</th><th>Историй</th><th>Задач</th><th>Потребностей</th></tr></thead>
        <tbody>
          {shown.map((r) => (
            <tr key={r.id}>
              <td><code>{r.id}</code> {String(r.title)}</td>
              <td>{String(r["kind"] ?? "")}</td>
              <td className={Number(r["checks"] ?? 0) === 0 ? "warn" : ""}>{String(r["checks"] ?? 0)}</td>
              <td>{String(r["stories"] ?? 0)}</td>
              <td>{String(r["tasks"] ?? 0)}</td>
              <td>{String(r["needs"] ?? 0)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
