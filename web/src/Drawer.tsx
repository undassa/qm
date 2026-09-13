import type React from "react";
import { type Lang, say } from "./say";
import { useEffect, useRef } from "react";
import { useFocusTrap } from "./focus";

/**
 * Дровер: предмет раскрывается сбоку, а список за ним остаётся на месте — не
 * теряется, куда ты вернёшься. Закрывается Escape и щелчком по подложке;
 * фокус уходит внутрь, держится в нём по Tab и возвращается туда, откуда
 * пришёл.
 */
export function Drawer({
  title,
  subtitle,
  onClose,
  children,
  lang,
}: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: React.ReactNode;
  lang: Lang;
}): React.JSX.Element {
  const panel = useRef<HTMLDivElement>(null);
  useFocusTrap(panel, true);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="drawer-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="drawer" role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={panel}>
        <header className="drawer-head">
          <div>
            <h2>{title}</h2>
            {subtitle ? <div className="prov">{subtitle}</div> : null}
          </div>
          <button type="button" className="drawer-close" onClick={onClose} aria-label={say(lang, "dr.close")}>
            ✕
          </button>
        </header>
        <div className="drawer-body">{children}</div>
      </div>
    </div>
  );
}
