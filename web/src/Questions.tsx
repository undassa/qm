import type React from "react";
import { useEffect, useState } from "react";
import { loadEntityByName, loadSummary } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Вопросы: чем закрыт каждый.
 *
 * Три состояния ответа, и третье обязано быть видно: **объявлен** · **искали и
 * не нашли** · **не сказано ничего**. Закрытый вопрос, о котором не сказано
 * ничего, закрыт по памяти — через месяц никто не назовёт, чем.
 */
export function Questions({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(() => loadSummary(projectId, "question").then((d) => d.rows), [projectId]);
  const rows = live.data;
  const [chosenId, setChosenId] = useState<string>("");
  const [body, setBody] = useState<string | null>(null);
  const [only, setOnly] = useState<"all" | "unsaid" | "open">("all");
  const chosen = (rows ?? []).find((r) => r.id === chosenId) ?? null;

  useEffect(() => {
    if (!chosen) return;
    setBody(null);
    void loadEntityByName(projectId, "question", chosen.id)
      .then((e) => setBody(String((e as { content?: string }).content ?? "")))
      .catch(() => setBody(""));
  }, [projectId, chosenId]);

  if (!rows) return <p className="empty">Читаю вопросы…</p>;
  const unsaid = rows.filter((r) => String(r["answerState"] ?? r["answer_state"] ?? "") === "unsaid").length;
  const open = rows.filter((r) => String(r["state"] ?? "") === "open").length;
  const shown = rows.filter((r) =>
    only === "all" ? true
    : only === "open" ? String(r["state"] ?? "") === "open"
    : String(r["answerState"] ?? r["answer_state"] ?? "") === "unsaid");

  return (
    <>
      <Live at={live.at} again={live.again} />
      <header className="head">
        <h1>Вопросы</h1>
        <p className="note">
          Всего {rows.length} · открытых <b>{open}</b> · закрытых, о чьём ответе не сказано ничего — <b>{unsaid}</b>.
          Последнее не «нет ответа», а «ответ не объявлен»: это разные вещи.
        </p>
        <div className="tabs">
          {(["all", "open", "unsaid"] as const).map((k) => (
            <button key={k} type="button" className={only === k ? "on" : ""} onClick={() => setOnly(k)}>
              {k === "all" ? "все" : k === "open" ? "открытые" : "без объявленного ответа"}
            </button>
          ))}
        </div>
      </header>

      <div className="split">
        <table className="rows">
          <thead><tr><th>Вопрос</th><th>Состояние</th><th>Ответ</th><th>Гейт</th></tr></thead>
          <tbody>
            {shown.map((r) => {
              const answer = String(r["answerState"] ?? r["answer_state"] ?? "");
              return (
                <tr key={r.id} className={r.id === chosenId ? "on" : ""} onClick={() => setChosenId(r.id)}>
                  <td><code>{r.id}</code> {String(r.title)}</td>
                  <td>{String(r["state"] ?? "")}</td>
                  <td className={answer === "unsaid" ? "warn" : ""}>
                    {answer === "answered" ? "объявлен" : answer === "searched" ? "искали, не нашли" : "не сказано"}
                  </td>
                  <td>{String(r["gate"] ?? "")}</td>
                </tr>
              );
            })}
          </tbody>
        </table>

        <aside className="panel">
          {!chosen ? <p className="empty">Выберите вопрос.</p> : (
            <>
              <h2><code>{chosen.id}</code> {String(chosen.title)}</h2>
              <p className="note">
                заведён {String(chosen["opened"] ?? chosen["opened_at"] ?? "—")} ·
                закрыт {String(chosen["closed"] ?? chosen["closed_at"] ?? "—")}
              </p>
              {body === null ? <p className="empty">Читаю…</p> : <pre className="body">{body}</pre>}
            </>
          )}
        </aside>
      </div>
    </>
  );
}
