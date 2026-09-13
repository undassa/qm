import type React from "react";
import { type Lang, say } from "./say";
import { useEffect, useState } from "react";
import { ago } from "./live";

/**
 * Отметка свежести: когда данные получены и кнопка спросить сейчас.
 *
 * Стоит на каждой странице раздела. Без неё «живая» страница неотличима от
 * замершей — а замершая выглядит убедительнее, потому что не мигает.
 */
export function Live({ at, again,
  lang,
}: { at: number | null; again: () => void;
  lang: Lang;
}): React.JSX.Element {
  const [, tick] = useState(0);
  // Отметка стареет сама, даже когда данные не менялись.
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, []);
  return (
    <button type="button" className="live-mark" onClick={again} title={say(lang, "lv.ask")}>
      <i className={at === null ? "dot cold" : "dot"} />
      <span data-live-at={at ?? ""}>{ago(at, undefined, lang)}</span>
    </button>
  );
}
