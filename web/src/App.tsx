import type React from "react";
import { useEffect, useState } from "react";
import { loadProjects, tool, type Project } from "./api";
import { chooseProject } from "./project-address";
import { ProjectPicker } from "./Projects";
import { Entities } from "./Entities";
import { Reader } from "./Reader";
import { Requirements } from "./Requirements";
import { Rules } from "./Rules";
import { Users } from "./Users";
import { Readiness } from "./Readiness";
import { Palette, type Jump } from "./Palette";
import { Together } from "./Together";

/**
 * Обстановка вида: то, что раздел получает от приложения.
 *
 * Реестр разделов — константа модуля, и состояния приложения ему не видно.
 * Передавать по доводу на каждую нужду значит менять подпись всякий раз, как
 * разделу понадобится ещё одна; один объект меняется дополнением поля.
 */
interface Ctx {
  /** Что попросили открыть — палитрой или ссылкой. */
  want: Jump | null;
  /** Найти имя и открыть его: тем же взвешенным поиском, что и палитра. */
  onFind: (q: string) => void;
}
import { Process } from "./Process";
import { Tasks } from "./Tasks";
import { Gates } from "./Gates";
import { Decisions } from "./Decisions";
import { Proof } from "./Proof";
import { Questions } from "./Questions";
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
const PAGES: { page: string; title: string; note: string; view: (id: string, ctx: Ctx) => React.JSX.Element }[] = [
  { page: "where", title: "Готовность", note: "фазы и гейты", view: (id, ctx) => <Readiness projectId={id} onFind={ctx.onFind} /> },
  { page: "tasks", title: "Задачи", note: "конвейер", view: (id) => <Tasks projectId={id} /> },
  { page: "unknown", title: "Не знаем", note: "пробелы", view: (id, ctx) => <Unknown projectId={id} onFind={ctx.onFind} /> },
  // Раздел — это тип ресурса, а не папка: у требования свои колонки, свои
  // фильтры и свои дыры, и общей таблицей документов их не показать.
  { page: "rules", title: "Правила", note: "конституция", view: (id, ctx) => <Rules projectId={id} onFind={ctx.onFind} /> },
  { page: "requirements", title: "Требования", note: "и доказательства", view: (id) => <Requirements projectId={id} /> },
  { page: "users", title: "Пользователь", note: "путь и истории", view: (id) => <Users projectId={id} /> },
  { page: "read", title: "Документы", note: "читать", view: (id, ctx) => <Reader projectId={id} want={ctx.want} /> },
  { page: "decisions", title: "Архитектура", note: "и отвергнутое", view: (id) => <Decisions projectId={id} /> },
  { page: "proof", title: "Доказательство", note: "чем закрыто", view: (id) => <Proof projectId={id} /> },
  { page: "questions", title: "Вопросы", note: "чем закрыт каждый", view: (id) => <Questions projectId={id} /> },
  { page: "entities", title: "Сущности", note: "виды", view: (id) => <Entities projectId={id} /> },
  // Раздел БЕЗ проекта: он про то, что у проектов общее. `projectId` ему не
  // нужен — он спрашивает всех.
  { page: "together", title: "Вместе", note: "проекты рядом", view: () => <Together /> },
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
  /** Палитра и то, куда она попросила перейти. */
  const [pal, setPal] = useState(false);
  const [jump, setJump] = useState<Jump | null>(null);

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

  // Cmd+K на маке, Ctrl+K везде: одна привычка, выученная в других
  // инструментах, здесь работает без обучения.
  useEffect(() => {
    const key = (e: KeyboardEvent): void => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPal((v) => !v);
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  // Найти имя и открыть его: тот же взвешенный поиск, что у палитры. Раздел,
  // нашедший виновника, не обязан знать, какому виду тот принадлежит.
  const find = (q: string): void => {
    if (!project) return;
    void tool<{ hits: { kind: string; name: string }[] }>(project.projectId, "search", { query: q, limit: 5 })
      .then((d) => {
        const hit = (d.hits ?? [])[0];
        if (!hit) return;
        setPage("read");
        go({ page: "read" });
        setJump({ page: "read", kind: hit.kind, id: hit.name });
      })
      .catch(() => undefined);
  };
  const ctx = { want: jump, onFind: find };

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

      <main className="main">{project ? view.view(project.projectId, ctx) : <p className="empty">Выбираю проект…</p>}</main>

      {pal && project ? (
        <Palette
          projectId={project.projectId}
          sections={PAGES.map((p) => ({ page: p.page, title: p.title, note: p.note }))}
          onClose={() => setPal(false)}
          onGo={(j) => {
            if (j.page && j.page !== page) { setPage(j.page); go({ page: j.page }); }
            if (j.kind && j.id) setJump(j);
          }}
        />
      ) : null}
    </div>
  );
}
