import type React from "react";
import { useEffect, useState } from "react";
import { tool, write } from "./api";
import type { Lang } from "./say";

/**
 * Очередь решений владельца и кнопки проверок.
 *
 * Заявка сессии на изменение харнеса и просьба подтвердить коммит лежат в одной
 * очереди: человек открывает пульт с одним вопросом — «что от меня ждут», — и
 * два места для ответа означали бы, что половина ожиданий не видна.
 *
 * Решение идёт ТОЛЬКО С ДОВОДОМ: очередь без доводов через неделю неотличима от
 * списка «почему-то отклонено».
 */
interface Ask {
  id: number;
  project: string;
  kind: string;
  title: string;
  body: string;
  askedBy: string;
  at: number;
  runId: string;
  state: string;
  why: string;
  decidedBy: string;
  decidedAt: number;
}

const РЕШЕНИЯ: Record<string, { state: string; label: string }[]> = {
  request: [
    { state: "taken", label: "В работу" },
    { state: "declined", label: "Отклонить" },
  ],
  approval: [
    { state: "approved", label: "Подтвердить" },
    { state: "rejected", label: "Отказать" },
  ],
};

const когда = (at: number): string =>
  at > 0 ? new Date(at).toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" }) : "";

export function Asks({ projectId }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [asks, setAsks] = useState<Ask[]>([]);
  const [open_, setOpen] = useState<number>(0);
  const [why, setWhy] = useState("");
  const [busy, setBusy] = useState(false);
  const [beef, setBeef] = useState("");
  const [ran, setRan] = useState("");

  const перечитать = (): void => {
    void tool<{ asks: Ask[] }>(projectId, "asks", { state: "open", limit: 40 }).then((r) => setAsks(r.asks ?? []));
  };
  useEffect(перечитать, [projectId]);

  const решить = async (a: Ask, state: string): Promise<void> => {
    if (!why.trim()) return;
    setBusy(true);
    setBeef("");
    const r = await write(projectId, "ask-decide", { ask: a.id, state, why: why.trim() });
    setBusy(false);
    if (!r.ok) {
      setBeef(r.why || "дверь не приняла решение");
      return;
    }
    setOpen(0);
    setWhy("");
    перечитать();
  };

  /** Проверка на весь набор: ответ двери — одной строкой, чтобы было видно, что она прошла. */
  const проверить = async (door: string, имя: string): Promise<void> => {
    setRan(`${имя}: идёт…`);
    const r = await write(projectId, door, {});
    setRan(r.ok ? `${имя}: готово` : `${имя}: ${r.why || "отказ"}`);
    перечитать();
  };

  return (
    <>
      <section className="pu-band">
        <div className="pu-h">
          <h2>Решения</h2>
          <span className={`pu-cnt${asks.length ? " hot" : " calm"}`}>{asks.length}</span>
          {asks.length > 0 && <span className="pu-note">заявки сессий и подтверждения коммитов</span>}
        </div>
        {asks.length === 0 ? (
          <p className="pu-empty">
            <b>Ничего не ждёт.</b> Заявки на изменение харнеса и просьбы подтвердить коммит появятся здесь.
          </p>
        ) : (
          <ul className="pu-asks">
            {asks.map((a) => (
              <li key={a.id} className={open_ === a.id ? "open" : ""}>
                <span className="pu-id">#{a.id}</span>
                <button
                  className="pu-do key"
                  type="button"
                  onClick={() => {
                    setOpen(open_ === a.id ? 0 : a.id);
                    setWhy("");
                    setBeef("");
                  }}
                >
                  {open_ === a.id ? "Свернуть" : "Решить"}
                </button>
                <span className="pu-t">{a.title}</span>
                <span className="pu-meta">
                  {a.kind === "approval" ? "подтверждение" : "заявка"}
                  {a.askedBy && <> · {a.askedBy}</>}
                  {a.at > 0 && <> · {когда(a.at)}</>}
                </span>
                {open_ === a.id && (
                  <div className="pu-form">
                    {a.body && <pre className="pu-body">{a.body}</pre>}
                    <textarea
                      className="pu-draft"
                      value={why}
                      onChange={(e) => setWhy(e.target.value)}
                      placeholder="Довод — он остаётся в очереди и читается спустя месяц"
                      rows={3}
                      autoFocus
                    />
                    <div className="pu-act">
                      {(РЕШЕНИЯ[a.kind] ?? РЕШЕНИЯ.request!).map((р) => (
                        <button
                          key={р.state}
                          className="pu-do key"
                          type="button"
                          disabled={busy || !why.trim()}
                          onClick={() => void решить(a, р.state)}
                        >
                          {busy ? "Пишу…" : р.label}
                        </button>
                      ))}
                      <span className="pu-hint">решение уходит дверью ask-decide</span>
                    </div>
                    {beef && <p className="pu-beef">{beef}</p>}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="pu-band">
        <div className="pu-h">
          <h2>Проверки</h2>
          {ran && <span className="pu-note">{ran}</span>}
        </div>
        <div className="pu-act">
          <button className="pu-do" type="button" onClick={() => void проверить("reproject", "Пересборка")}>
            Пересобрать набор
          </button>
          <button className="pu-do" type="button" onClick={() => void проверить("gate-measure", "Пересчёт гейта")}>
            Пересчитать гейт
          </button>
          <button className="pu-do" type="button" onClick={() => void проверить("gate-selftest", "Самотест")}>
            Самотест проб
          </button>
        </div>
      </section>
    </>
  );
}
