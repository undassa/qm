import type React from "react";
import { useEffect, useState } from "react";
import { loadProjects, type Project } from "./api";
import { chooseProject } from "./project-address";
import { ProjectPicker } from "./Projects";
import { Entities } from "./Entities";
import { Reader } from "./Reader";
import { Requirements } from "./Requirements";
import { Rules } from "./Rules";
import { Users } from "./Users";
import { Where } from "./Where";
import { Process } from "./Process";
import { Tasks } from "./Tasks";
import { Gates } from "./Gates";
import { Decisions } from "./Decisions";
import { Proof } from "./Proof";
import { Questions } from "./Questions";
import { Movement } from "./Movement";
import { Unknown } from "./Unknown";

/**
 * Оболочка.
 *
 * Экраны отвечают на четыре вопроса, и в этом порядке: **где мы · что запустить
 * · что мешает · чего мы не знаем**. Последний стоит отдельной страницей, а не в
 * подвале: пока он не закрыт, зелёное на остальных значит меньше, чем кажется.
 *
 * Столов восемь, и каждый отвечает на свой вопрос: где мы · ступени · задачи ·
 * гейты · не знаем · правила · требования · пользователь. К ним четыре, которых
 * не хватало: **движение** (куда дошли), **архитектура** (и что она отвергла —
 * 523 варианта, не показанных нигде), **доказательство** (чем закрыто каждое
 * требование) и **вопросы** (чем закрыт каждый).
 */
const PAGES: { page: string; title: string; note: string; view: (id: string) => React.JSX.Element }[] = [
  { page: "where", title: "Где мы", note: "процесс", view: (id) => <Where projectId={id} /> },
  { page: "process", title: "Ступени", note: "лестница", view: (id) => <Process projectId={id} /> },
  { page: "tasks", title: "Задачи", note: "конвейер", view: (id) => <Tasks projectId={id} /> },
  { page: "gates", title: "Гейты", note: "вычислено", view: (id) => <Gates projectId={id} /> },
  { page: "unknown", title: "Не знаем", note: "пробелы", view: (id) => <Unknown projectId={id} /> },
  // Раздел — это тип ресурса, а не папка: у требования свои колонки, свои
  // фильтры и свои дыры, и общей таблицей документов их не показать.
  { page: "rules", title: "Правила", note: "конституция", view: (id) => <Rules projectId={id} /> },
  { page: "requirements", title: "Требования", note: "и доказательства", view: (id) => <Requirements projectId={id} /> },
  { page: "users", title: "Пользователь", note: "путь и истории", view: (id) => <Users projectId={id} /> },
  { page: "read", title: "Документы", note: "читать", view: (id) => <Reader projectId={id} /> },
  { page: "movement", title: "Движение", note: "куда дошли", view: (id) => <Movement projectId={id} /> },
  { page: "decisions", title: "Архитектура", note: "и отвергнутое", view: (id) => <Decisions projectId={id} /> },
  { page: "proof", title: "Доказательство", note: "чем закрыто", view: (id) => <Proof projectId={id} /> },
  { page: "questions", title: "Вопросы", note: "чем закрыт каждый", view: (id) => <Questions projectId={id} /> },
  { page: "entities", title: "Сущности", note: "виды", view: (id) => <Entities projectId={id} /> },
];

/**
 * Проект и страница, названные в адресе.
 *
 * Адрес — единственное место, где выбор переживает перезагрузку и пересылается
 * другому человеку.
 */
const asked = (name: string): string | null => new URLSearchParams(window.location.search).get(name);

export function App(): React.JSX.Element {
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<Project | null>(null);
  const [page, setPage] = useState<string>(asked("page") ?? "where");
  const [unknown, setUnknown] = useState<string>("");

  useEffect(() => {
    void loadProjects().then((list) => {
      setProjects(list);
      const choice = chooseProject(list, asked("project"));
      setProject(choice.project);
      setUnknown(choice.unknown);
    });
  }, []);

  useEffect(() => {
    const back = (): void => {
      const choice = chooseProject(projects, asked("project"));
      if (choice.project) setProject(choice.project);
      setUnknown(choice.unknown);
      setPage(asked("page") ?? "where");
    };
    window.addEventListener("popstate", back);
    return () => window.removeEventListener("popstate", back);
  }, [projects]);

  const go = (next: { project?: Project; page?: string }): void => {
    const url = new URL(window.location.href);
    if (next.project) {
      setProject(next.project);
      setUnknown("");
      url.searchParams.set("project", next.project.projectId);
    }
    if (next.page) {
      setPage(next.page);
      url.searchParams.set("page", next.page);
    }
    window.history.pushState({}, "", url);
  };

  const view = PAGES.find((p) => p.page === page) ?? PAGES[0]!;

  return (
    <div className="app">
      <nav className="side" aria-label="Экраны">
        <ProjectPicker projects={projects} current={project} onPick={(p) => go({ project: p })} />

        {unknown ? (
          <p className="side-note warn">
            В ссылке назван проект <code>{unknown}</code> — здесь такого нет. Открыт{" "}
            <b>{project?.name ?? "первый из списка"}</b>.
          </p>
        ) : null}

        {PAGES.map((p) => (
          <button
            key={p.page}
            type="button"
            className={`ent top${p.page === page ? " on" : ""}`}
            onClick={() => go({ page: p.page })}
          >
            <span className="ent-n">{p.title}</span>
            <span className="ent-c">{p.note}</span>
          </button>
        ))}
      </nav>

      <main className="main">{project ? view.view(project.projectId) : <p className="empty">Выбираю проект…</p>}</main>
    </div>
  );
}
