import type React from "react";
import { useEffect, useState } from "react";
import { loadProjects, tool, type Project } from "./api";
import { chooseProject } from "./project-address";
import { ProjectPicker } from "./Projects";
import { Depends } from "./Depends";
import { Console } from "./Console";
import { Corpus } from "./Corpus";
import { Work } from "./Work";
import { type Lang, langNow, setLang, say } from "./say";
import { Reader } from "./Reader";
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
  /** Открыть другой раздел: пульт отсылает к вопросам и зависимостям. */
  onGo: (page: string) => void;
  /** Язык подписей. Проза харнеса не переводится: это слова проекта. */
  lang: Lang;
}
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
const PAGES: { page: string; key: string; view: (id: string, ctx: Ctx) => React.JSX.Element }[] = [
  { page: "pult", key: "nav.pult", view: (id, ctx) => <Console projectId={id} onGo={ctx.onGo} /> },
  { page: "where", key: "nav.where", view: (id, ctx) => <Readiness projectId={id} onFind={ctx.onFind} /> },
  { page: "tasks", key: "nav.tasks", view: (id, ctx) => <Work projectId={id} lang={ctx.lang} /> },
  { page: "unknown", key: "nav.unknown", view: (id, ctx) => <Unknown projectId={id} onFind={ctx.onFind} /> },
  // Раздел — это тип ресурса, а не папка: у требования свои колонки, свои
  // фильтры и свои дыры, и общей таблицей документов их не показать.
  { page: "read", key: "nav.read", view: (id, ctx) => <Reader projectId={id} want={ctx.want} /> },
  { page: "corpus", key: "nav.corpus", view: (id, ctx) => <Corpus projectId={id} lang={ctx.lang} /> },
  { page: "depends", key: "nav.depends", view: (id, ctx) => <Depends projectId={id} lang={ctx.lang} /> },
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
  const [page, setPage] = useState<string>(asked("page") ?? "pult");
  const [lang, setLangState] = useState<Lang>(langNow());
  useEffect(() => {
    const слушать = (): void => setLangState(langNow());
    window.addEventListener("mh-lang", слушать);
    return () => window.removeEventListener("mh-lang", слушать);
  }, []);
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
      setPage(asked("page") ?? "pult");
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
  const ctx = { want: jump, onFind: find, onGo: setPage, lang };

  const view = PAGES.find((p) => p.page === page) ?? PAGES[0]!;

  return (
    <div className="app">
      <nav className="side" aria-label="Экраны">
        <ProjectPicker
            projects={projects}
            current={project}
            onPick={(p) => go({ project: p })}
            onAll={() => {
              setProject(null);
              setUnknown("");
              // Раздел уходит вместе с проектом: он про ОДИН проект, и
              // оставшийся в адресе `page` читался бы как неполная ссылка —
              // а человек сам выбрал сводку, ссылка тут ни при чём.
              const url = new URL(window.location.href);
              url.searchParams.delete("project");
              url.searchParams.delete("page");
              window.history.pushState({}, "", url);
            }}
          />

        {unknown ? (
          <p className="side-note warn">
            В ссылке назван проект <code>{unknown}</code> — здесь такого нет. Открыт{" "}
            <b>{project?.name ?? "первый из списка"}</b>.
          </p>
        ) : null}

        {/* Разделы — про ВЫБРАННЫЙ проект. Без выбора они бессмысленны: раздел
            «Задачи» без проекта не про что. Вместо них — возврат к сводке. */}
        {project
          ? PAGES.map((p) => (
              <button
                key={p.page}
                type="button"
                className={`ent top${p.page === page ? " on" : ""}`}
                onClick={() => go({ page: p.page })}
              >
                <span className="ent-n">{say(lang, p.key)}</span>
                <span className="ent-c">{say(lang, `${p.key}.note`)}</span>
              </button>
            ))
          : null}

          {/* Язык подписей. Проза харнеса — причины отказов, доводы правил —
              не переводится: это слова, которыми владеет проект. */}
          <div className="lang">
            <button type="button" className={lang === "ru" ? "on" : ""} onClick={() => setLang("ru")}>РУ</button>
            <button type="button" className={lang === "en" ? "on" : ""} onClick={() => setLang("en")}>EN</button>
          </div>
      </nav>

      <main className="main">
        {projects.length === 0 ? (
          <p className="empty">Читаю проекты…</p>
        ) : project ? (
          view.view(project.projectId, ctx)
        ) : (
          // Проект не выбран — показывается сводка по всем. Первый вопрос при
          // открытии не «что у myack», а «что у нас вообще»: подставлять на него
          // чужие числа под именем, которого человек не выбирал, значит отвечать
          // не на тот вопрос.
          <>
            {/* Адрес, просящий раздел без проекта, НЕПОЛОН. Молча показать
                сводку значит ответить не на тот вопрос: человек шёл по ссылке
                в «Задачи», и ему надо сказать, чего в ссылке не хватило. */}
            {asked("page") ? (
              <p className="side-note warn">
                В ссылке назван раздел <code>{asked("page")}</code>, но не назван проект. Разделы
                показывают ОДИН проект: выберите его ниже, и раздел откроется.
              </p>
            ) : null}
            <Together onPick={(p) => go({ project: p, page: asked("page") ?? page })} />
          </>
        )}
      </main>

      {pal && project ? (
        <Palette
          projectId={project.projectId}
          sections={PAGES.map((p) => ({ page: p.page, title: say(lang, p.key), note: say(lang, `${p.key}.note`) }))}
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
