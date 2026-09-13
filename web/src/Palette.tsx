import type React from "react";
import { type Lang, say } from "./say";
import { useEffect, useRef, useState } from "react";
import { useFocusTrap } from "./focus";
import { tool } from "./api";

/**
 * Палитра: одно поле, из которого достаётся что угодно.
 *
 * У приложения пятнадцать разделов, пятьдесят семь видов и больше тысячи
 * документов. Меню такого не держит: боковик показывает разделы, но чтобы
 * добраться до `ADR-0138`, надо помнить, что он в «Документах», найти там вид
 * `decision` среди пятидесяти семи и пролистать сто семьдесят одну строку.
 *
 * Здесь набирают имя. Разделы находятся мгновенно — их пятнадцать и они рядом;
 * сущности спрашиваются у сервера, потому что помнить тысячу имён на стороне
 * браузера значит держать вторую копию набора.
 *
 * Открывается по Cmd+K или Ctrl+K, закрывается Esc. Ходят стрелками, выбирают
 * Enter: рука не уходит с клавиатуры, ради чего палитра и нужна.
 */

interface Hit {
  kind: string;
  name: string;
  excerpt?: string;
}
export interface Jump {
  page?: string;
  kind?: string;
  id?: string;
}

interface Section {
  page: string;
  title: string;
  note: string;
}

export function Palette({
  projectId,
  sections,
  onGo,
  onClose,
  lang,
}: {
  projectId: string;
  sections: Section[];
  onGo: (j: Jump) => void;
  onClose: () => void;
  lang: Lang;
}): React.JSX.Element {
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [busy, setBusy] = useState(false);
  const [at, setAt] = useState(0);
  const box = useRef<HTMLInputElement>(null);
  const слой = useRef<HTMLDivElement | null>(null);
  useFocusTrap(слой, true);

  useEffect(() => {
    box.current?.focus();
    const выход = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", выход);
    return () => document.removeEventListener("keydown", выход);
  }, [onClose]);

  // Спрашиваем сервер не на каждую букву: набирающий имя делает восемь нажатий
  // за секунду, и восемь запросов на них — это восемь ответов не про то, что
  // сейчас в поле.
  useEffect(() => {
    const text = q.trim();
    if (text.length < 2) {
      setHits([]);
      return;
    }
    let dead = false;
    setBusy(true);
    const t = setTimeout(() => {
      void tool<{ hits: Hit[] }>(projectId, "search", { query: text, limit: 40 })
        .then((d) => {
          if (!dead) setHits((d.hits ?? []).slice(0, 40));
        })
        .catch(() => {
          if (!dead) setHits([]);
        })
        .finally(() => {
          if (!dead) setBusy(false);
        });
    }, 180);
    return () => {
      dead = true;
      clearTimeout(t);
    };
  }, [q, projectId]);

  const text = q.trim().toLowerCase();
  const pages = text
    ? sections.filter((s) => s.title.toLowerCase().includes(text) || s.note.toLowerCase().includes(text))
    : sections;
  const rows: { key: string; label: string; sub: string; jump: Jump }[] = [
    ...pages.map((s) => ({
      key: "p:" + s.page,
      label: s.title,
      sub: "раздел · " + s.note,
      jump: { page: s.page } as Jump,
    })),
    ...hits.map((h) => ({
      key: "e:" + h.kind + "/" + h.name,
      label: h.name || h.kind,
      sub: h.kind + (h.excerpt ? " · " + h.excerpt.replace(/\s+/g, " ").slice(0, 70) : ""),
      jump: { page: "read", kind: h.kind, id: h.name } as Jump,
    })),
  ];
  const here = Math.min(at, Math.max(rows.length - 1, 0));

  function keys(e: React.KeyboardEvent): void {
    if (e.key === "Escape") { onClose(); return; }
    if (e.key === "ArrowDown") { e.preventDefault(); setAt((n) => Math.min(n + 1, rows.length - 1)); return; }
    if (e.key === "ArrowUp") { e.preventDefault(); setAt((n) => Math.max(n - 1, 0)); return; }
    if (e.key === "Enter") {
      e.preventDefault();
      const row = rows[here];
      if (row) { onGo(row.jump); onClose(); }
    }
  }

  return (
    <div className="pal-scrim" onClick={onClose} role="presentation">
      <div
        className="pal"
        ref={слой}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={say(lang, "pl.head")}
      >
        <input
          ref={box}
          className="pal-in"
          value={q}
          onChange={(e) => { setQ(e.target.value); setAt(0); }}
          onKeyDown={keys}
          placeholder={say(lang, "pl.hint")}
          aria-label={say(lang, "pl.what")}
        />
        <div className="pal-list">
          {rows.length === 0 ? (
            <p className="pal-none">
              {busy ? "ищу…" : text.length < 2 ? "наберите хотя бы две буквы" : "ничего не нашлось"}
            </p>
          ) : (
            rows.map((r, n) => (
              <button
                key={r.key}
                type="button"
                className={`pal-row${n === here ? " on" : ""}`}
                onMouseEnter={() => setAt(n)}
                onClick={() => { onGo(r.jump); onClose(); }}
              >
                <span className="pal-l">{r.label}</span>
                <span className="pal-s">{r.sub}</span>
              </button>
            ))
          )}
        </div>
        <div className="pal-foot">
          <span>{say(lang, "pl.pick")}</span>
          <span>{say(lang, "pl.open")}</span>
          <span>{say(lang, "pl.esc")}</span>
          {busy ? <span className="pal-busy">{say(lang, "pl.searching")}</span> : null}
        </div>
      </div>
    </div>
  );
}
