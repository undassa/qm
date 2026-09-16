import type React from "react";
import { useEffect, useRef, useState } from "react";
import { tool, write } from "./api";
import type { Lang } from "./say";

/**
 * Беседа над набором: спросить по ходу, разобрать документ, проверить замысел.
 *
 * Беседа — не прогон задачи. Прогон делает названную работу и кончается;
 * беседа думает вслух и живёт, пока её не закроют. Общая полоса сделала бы
 * «идёт» бессмысленным: у беседы нет ни задачи, ни попытки.
 *
 * Отвечает та сторона, что читает непрочитанное дверью `chat-inbox`. Пока она
 * не ответила, строка человека висит непрочитанной, и это видно.
 */
interface Thread {
  thread: string;
  title: string;
  state: string;
  at: number;
  said: number;
  waiting: number;
}
interface Said {
  at: number;
  side: string;
  text: string;
  read: boolean;
}

const когда = (at: number): string =>
  at > 0 ? new Date(at).toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" }) : "";

export function Talk({ projectId }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [threads, setThreads] = useState<Thread[]>([]);
  const [open_, setOpen] = useState<string>("");
  const [said, setSaid] = useState<Said[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [beef, setBeef] = useState("");
  const низ = useRef<HTMLDivElement | null>(null);

  const перечитать = (): void => {
    void tool<{ threads: Thread[] }>(projectId, "chat").then((r) => setThreads(r.threads ?? []));
  };
  const прочитать = (thread: string): void => {
    if (!thread) return;
    void tool<{ said: Said[] }>(projectId, "chat", { thread }).then((r) => setSaid(r.said ?? []));
  };

  useEffect(перечитать, [projectId]);
  useEffect(() => {
    if (!open_) return;
    прочитать(open_);
    // Ответ приходит не в этот вызов, а когда его напишет отвечающая сторона,
    // поэтому беседа перечитывается сама, пока открыта.
    const t = setInterval(() => {
      прочитать(open_);
      перечитать();
    }, 4000);
    return () => clearInterval(t);
  }, [open_, projectId]);
  useEffect(() => {
    низ.current?.scrollIntoView({ block: "end" });
  }, [said.length]);

  const завести = async (): Promise<void> => {
    setBusy(true);
    const r = await write(projectId, "chat-start", { title: "Беседа" });
    setBusy(false);
    if (!r.ok) {
      setBeef(r.why || "дверь не завела беседу");
      return;
    }
    const thread = (r.got as { thread?: string } | null)?.thread ?? "";
    перечитать();
    if (thread) setOpen(thread);
  };

  const сказать = async (): Promise<void> => {
    if (!draft.trim() || !open_) return;
    setBusy(true);
    setBeef("");
    const r = await write(projectId, "chat-say", { thread: open_, text: draft.trim() });
    setBusy(false);
    if (!r.ok) {
      setBeef(r.why || "дверь не приняла строку");
      return;
    }
    setDraft("");
    прочитать(open_);
    перечитать();
  };

  const ждут = threads.reduce((n, t) => n + t.waiting, 0);

  return (
    <section className="pu-band">
      <div className="pu-h">
        <h2>Беседа</h2>
        <span className={`pu-cnt${ждут ? " hot" : " calm"}`}>{ждут}</span>
        {ждут > 0 && <span className="pu-note">строк ждут ответа</span>}
        <button className="pu-do" type="button" disabled={busy} onClick={() => void завести()}>
          Новая беседа
        </button>
      </div>

      {threads.length > 0 && (
        <div className="pu-chips">
          {threads.map((t) => (
            <button
              key={t.thread}
              type="button"
              className={`pu-chip${open_ === t.thread ? " hot" : ""}`}
              onClick={() => setOpen(open_ === t.thread ? "" : t.thread)}
            >
              {t.title || "без названия"} <b>{t.said}</b>
              {t.waiting > 0 && <> · ждёт {t.waiting}</>}
            </button>
          ))}
        </div>
      )}

      {!open_ ? (
        <p className="pu-empty">
          <b>Беседа не открыта.</b> Здесь спрашивают по ходу и разбирают документы, не заводя задачу.
        </p>
      ) : (
        <div className="pu-talk">
          <div className="pu-said">
            {said.length === 0 ? (
              <p className="pu-empty">Пока ничего не сказано.</p>
            ) : (
              said.map((s) => (
                <p key={`${s.at}-${s.side}`} className={`pu-line ${s.side}`}>
                  <span className="pu-meta">
                    {s.side === "owner" ? "вы" : "набор"} · {когда(s.at)}
                    {s.side === "owner" && !s.read && <> · не прочитано</>}
                  </span>
                  <span className="pu-text">{s.text}</span>
                </p>
              ))
            )}
            <div ref={низ} />
          </div>
          <textarea
            className="pu-draft"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void сказать();
            }}
            placeholder="Вопрос или мысль. Enter с Ctrl — отправить"
            rows={3}
          />
          <div className="pu-act">
            <button className="pu-do key" type="button" disabled={busy || !draft.trim()} onClick={() => void сказать()}>
              {busy ? "Пишу…" : "Сказать"}
            </button>
            <span className="pu-hint">строка уходит дверью chat-say</span>
          </div>
          {beef && <p className="pu-beef">{beef}</p>}
        </div>
      )}
    </section>
  );
}
