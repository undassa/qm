import type React from "react";
import { useState } from "react";
import { loadSummary } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";
import { EntityDrawer } from "./EntityDrawer";

/**
 * Вопросы: чем закрыт каждый.
 *
 * Три состояния ответа, и третье обязано быть видно: **объявлен** · **искали и
 * не нашли** · **не сказано ничего**. Закрытый вопрос, о котором не сказано
 * ничего, закрыт по памяти — через месяц никто не назовёт, чем.
 */
export function Questions({ projectId, onFind }: { projectId: string; onFind?: (q: string) => void }): React.JSX.Element {
  const live = useLive(() => loadSummary(projectId, "question").then((d) => d.rows), [projectId]);
  const rows = live.data;
  const [chosenId, setChosenId] = useState<string>("");
  const [only, setOnly] = useState<"all" | "unsaid" | "open">("open");
  const chosen = (rows ?? []).find((r) => r.id === chosenId) ?? null;


  if (!rows) return <p className="empty">Читаю вопросы…</p>;
  const unsaid = rows.filter((r) => String(r["answerState"] ?? r["answer_state"] ?? "") === "unsaid").length;
  const open = rows.filter((r) => String(r["state"] ?? "") === "open").length;
  const shown = rows.filter((r) =>
    only === "all" ? true
    : only === "open" ? String(r["state"] ?? "") === "open"
    : String(r["answerState"] ?? r["answer_state"] ?? "") === "unsaid");
  // Есть ли гейт хоть у одного показанного: пустая колонка во всех трёхстах
  // семидесяти строках — обещание сведений, которых нет.
  const anyGate = shown.some((r) => String(r["gate"] ?? "").trim() !== "");

  return (
    <>
      <div className="head">
        <div>
          <h1>Вопросы</h1>
          <div className="prov">чем закрыт каждый</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} /> <b>{rows.length}</b> всего · открытых{" "}
          <b className={open > 0 ? "bad-n" : ""}>{open}</b> · без объявленного ответа{" "}
          <b className={unsaid > 0 ? "bad-n" : ""}>{unsaid}</b>
        </span>
      </div>

      <p className="lede">
        «Закрыт, о чьём ответе не сказано ничего» — не «нет ответа», а «ответ не объявлен»: это разные вещи.
      </p>

      <div className="tabs">
        {(["open", "unsaid", "all"] as const).map((k) => (
          <button key={k} type="button" className={only === k ? "on" : ""} onClick={() => setOnly(k)}>
            {k === "all" ? "все" : k === "open" ? "открытые" : "без объявленного ответа"}
          </button>
        ))}
      </div>

      <div>
        <table className="rows">
          <thead>
            <tr>
              <th>Вопрос</th>
              <th>Состояние</th>
              <th>Ответ</th>
              {anyGate ? <th>Гейт</th> : null}
            </tr>
          </thead>
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
                  {anyGate ? <td>{String(r["gate"] ?? "")}</td> : null}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    {chosen ? (
      <EntityDrawer
        projectId={projectId}
        kind="question"
        id={chosen.id}
        title={`Вопрос ${chosen.id}`}
        subtitle={String(chosen["title"] ?? "")}
        onClose={() => setChosenId("")}
        onFind={onFind}
      />
    ) : null}
    </>
  );
}
