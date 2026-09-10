import type React from "react";
import { useEffect, useRef, useState } from "react";
import type { Project } from "./api";
import { dayOf, plural } from "./findings";

/**
 * Выбор проекта.
 *
 * Прежняя кнопка перебирала проекты по кругу: щелчок — следующий. Пока их два,
 * это ещё как-то работает, но узнать, что вообще есть, нельзя было никогда —
 * список существовал только в голове у того, кто щёлкал. Здесь список видно, и
 * у каждого проекта написано, чем он наполнен.
 *
 * Проект без документов из списка не прячется: пустой набор — законное
 * состояние (в базу его ещё не переносили), и это ровно тот проект, в который
 * надо зайти и увидеть, что он пуст.
 */
export function ProjectPicker({
  projects,
  current,
  onAll,
  onPick,
}: {
  projects: Project[];
  current: Project | null;
  /** Вернуться к сводке: показать все проекты разом. */
  onAll?: () => void;
  onPick: (p: Project) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState(0);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setAt(Math.max(0, projects.findIndex((p) => p.projectId === current?.projectId)));
    const outside = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        return;
      }
      if (e.key === "ArrowDown" || e.key === "j") {
        e.preventDefault();
        setAt((i) => Math.min(i + 1, projects.length - 1));
      } else if (e.key === "ArrowUp" || e.key === "k") {
        e.preventDefault();
        setAt((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const pick = projects[at];
        if (pick) {
          onPick(pick);
          setOpen(false);
        }
      }
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", key);
    };
  }, [open, projects, current, at, onPick]);

  return (
    <div className="picker" ref={box}>
      <button
        type="button"
        className="place"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="listbox"
        title="Выбрать проект"
      >
        <span className="place-k">{projects.length > 1 ? `проект · ${projects.length}` : "проект"}</span>
        {/* «Все проекты» — законное состояние, а не «ещё не выбрал»: без выбора
            показывается сводка. Многоточие тут читалось как загрузка. */}
        <span className="place-n">{current?.name ?? (projects.length ? "все проекты" : "…")}</span>
        <span className="place-s">
          {current ? volume(current) : projects.length ? `${projects.length} · сводка` : "читаю список"}
        </span>
      </button>

      {open ? (
        <ul className="picks" role="listbox" aria-label="Проекты">
          {/* Возврат к сводке — первой строкой: уйдя в проект, вернуться было
              нечем, а сводка и есть ответ на «что у нас вообще». */}
          <li className={current ? "" : "at"}>
            <button
              type="button"
              role="option"
              aria-selected={!current}
              className={current ? "" : "on"}
              onClick={() => {
                onAll?.();
                setOpen(false);
              }}
            >
              <b>все проекты</b>
              <span>сводка</span>
            </button>
          </li>
          {projects.map((p, i) => (
            <li key={p.projectId} className={i === at ? "at" : ""}>
              <button
                type="button"
                role="option"
                aria-selected={p.projectId === current?.projectId}
                className={p.projectId === current?.projectId ? "on" : ""}
                onMouseEnter={() => setAt(i)}
                onClick={() => {
                  onPick(p);
                  setOpen(false);
                }}
              >
                <b>{p.name}</b>
                <span>{volume(p)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** Чем наполнен проект — одной строкой. */
function volume(p: Project): string {
  if (!p.documents) return "ни одного документа";
  const count = `${p.documents} ${plural(p.documents, "документ", "документа", "документов")}`;
  return p.updatedAt ? `${count} · ${dayOf(p.updatedAt)}` : count;
}
