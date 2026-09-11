import type React from "react";
import { useEffect, useMemo, useState } from "react";
import { tool } from "./api";
import { type Lang, kindName, say } from "./say";

/**
 * Корпус — всё, что знает проект.
 *
 * Шесть разделов — «Требования», «Доказательство», «Вопросы», «Архитектура»,
 * «Пользователь», «Правила» — читали ОДНУ дверь `summary kind=X` и различались
 * зашитым родом да порядком колонок. «Требования» и «Доказательство» брали
 * один и тот же род: разница была в том, какую колонку поставили первой.
 *
 * Здесь род выбирается, а не зашивается, — и тринадцать родов получают экран
 * разом вместо шести по одному. «Доказательство» становится сортировкой по
 * колонке `checks`, а не отдельной страницей.
 */
interface Summary {
  kind: string;
  count: number;
  columns: string[];
  numbers: string[];
  rows: Record<string, string | number>[];
}
interface KindRow { kind: string; count: number | null; shape: string; projection?: string }
interface Ent {
  id: string;
  kind: string;
  entity?: Record<string, unknown>;
  live?: { state: string; why?: string; through?: number; cause?: string };
  relations?: { links?: Record<string, number>; backlinks?: number };
  saidIn?: {
    count: number;
    definedIn?: { kind: string; name: string; section: number | null; title: string }[];
    where?: { kind: string; name: string; section: number | null; title: string; role: string }[];
  };
}

/** Колонки приходят от сервера английскими именами — подписываем их. */
const КОЛОНКА: Record<string, [string, string]> = {
  id: ["имя", "name"],
  kind: ["род", "kind"],
  area: ["область", "area"],
  priority: ["приоритет", "priority"],
  satisfied: ["удовлетворено", "satisfied"],
  checks: ["пров.", "chk"],
  stories: ["ист.", "sty"],
  tasks: ["зад.", "tsk"],
  needs: ["потр.", "need"],
  decisions: ["реш.", "dec"],
  alternatives: ["альт.", "alt"],
  consequences: ["следств.", "cons"],
  articles: ["ст.", "art"],
  journal: ["журн.", "log"],
  edits: ["правок", "edits"],
  links: ["связей", "links"],
  closes: ["закр.", "closes"],
  title: ["заголовок", "title"],
  state: ["состояние", "state"],
  answer: ["ответ", "answer"],
  text: ["текст", "text"],
  number: ["номер", "no."],
  status: ["состояние", "status"],
  answerState: ["ответ", "answer"],
  closedAt: ["закрыт", "closed"],
  gate: ["гейт", "gate"],
  date: ["дата", "date"],
  deciders: ["решали", "deciders"],
  persona: ["персона", "persona"],
  phase: ["фаза", "phase"],
  feature: ["фича", "feature"],
  milestone: ["этап", "milestone"],
  size: ["размер", "size"],
  spec: ["чем меряется", "spec"],
  requirement_id: ["требование", "requirement"],
  measured_by: ["чем меряется", "measured by"],
  crosscutting: ["сквозное", "crosscutting"],
  out_of_version: ["вне выпуска", "out of release"],
};
/** Колонки, ноль в которых значит «ничем не доказано», а не «мало». */
const ДОКАЗ = new Set(["checks", "stories", "tasks", "needs", "decisions"]);

/** Какого рода колонка: якорь · заголовок · число · свойство. */
const клеть = (c: string, numbers: string[]): string =>
  c === "id" ? "id" : c === "title" ? "t" : numbers.includes(c) ? "n" : "p";

const подпись = (l: Lang, c: string): string => (КОЛОНКА[c] ? (l === "en" ? КОЛОНКА[c][1] : КОЛОНКА[c][0]) : c);

export function Corpus({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [kinds, setKinds] = useState<KindRow[] | null>(null);
  const [kind, setKind] = useState<string>("requirement");
  const [sum, setSum] = useState<Summary | null>(null);
  const [pick, setPick] = useState<string>("");
  const [ent, setEnt] = useState<Ent | null>(null);
  const [q, setQ] = useState<string>("");
  const [by, setBy] = useState<string>("");
  /** Показывать все колонки или сводку. Десять колонок разом — сетка, а не список. */
  const [wide, setWide] = useState(false);

  useEffect(() => {
    if (!projectId) return;
    void tool<{ kinds: KindRow[] }>(projectId, "kinds").then((d) =>
      // Роды с ЗАПИСЯМИ и со своей таблицей. Прежде в списке стояли все
      // сорок шесть, включая одиночные документы `brs`, `cjm`, `concept` — по
      // одной штуке каждый. Это не роды записей, а документы, и место им в
      // «Документах»: `projection` их уже различает, надо было лишь прочесть.
      setKinds(
        (d.kinds ?? [])
          .filter((k) => (k.count ?? 0) > 0 && k.projection === "done")
          .sort((a, b) => (b.count ?? 0) - (a.count ?? 0)),
      ),
    );
  }, [projectId]);

  useEffect(() => {
    if (!projectId || !kind) return;
    setSum(null);
    setPick("");
    setBy("");
    void tool<Summary>(projectId, "summary", { kind }).then(setSum).catch(() => setSum(null));
  }, [projectId, kind]);

  useEffect(() => {
    if (!projectId || !pick) { setEnt(null); return; }
    void tool<Ent>(projectId, kind, { id: pick }).then(setEnt).catch(() => setEnt(null));
  }, [projectId, kind, pick]);

  /**
   * Колонки для показа. Сервер объявляет в `columns` только СВОЙСТВА рода —
   * имя и заголовок в перечень не входят, хотя в строке лежат. Отрисовав один
   * `columns`, я получил таблицу, которая показывает числа и не называет, о
   * чём они: ни имени записи, ни заголовка.
   *
   * Имя идёт первым всегда, заголовок — если он есть в строках.
   */
  const columns = useMemo(() => {
    const c = sum?.columns ?? [];
    const rs = sum?.rows ?? [];
    const first = rs[0] ?? {};
    const есть = (k: string): boolean => k in first && !c.includes(k);
    // КОЛОНКА С ОДНИМ ЗНАЧЕНИЕМ НЕ НЕСЁТ НИЧЕГО. У требований «род» был `FR` во
    // всех трёхстах шести строках и занимал место, которого не хватало
    // заголовку. Постоянные колонки уходят, а их значение показано над
    // таблицей — один раз, как и положено постоянному.
    const постоянные = c.filter(
      (k) => rs.length > 2 && new Set(rs.map((r) => String(r[k] ?? ""))).size === 1,
    );
    // ПУСТАЯ КОЛОНКА ЗАНИМАЕТ МЕСТО И НЕ ГОВОРИТ НИЧЕГО. «Удовлетворено» было
    // пусто у всех трёхсот шести строк и отнимало полторы сотни точек у
    // заголовка. Колонка, заполненная реже чем у пятой части записей, уходит
    // за переключатель — но не исчезает: её видно по «+N колонок».
    const редкие = c.filter(
      (k) =>
        rs.length > 10 &&
        rs.filter((r) => String(r[k] ?? "") !== "").length / rs.length < 0.2,
    );
    // По умолчанию — СВОДКА: имя, о чём запись, и свойства словами. Числовые
    // колонки уходят за переключатель: их десять, они почти все единицы, и
    // вместе они отнимают у заголовка ту ширину, ради которой строку читают.
    // Исключение одно — ноль доказательств: это не «мало», а «ничем», и
    // молчать о нём нельзя.
    const числа = sum?.numbers ?? [];
    const видно = (k: string): boolean =>
      wide || (!числа.includes(k) && !редкие.includes(k)) || k === "checks";
    return {
      show: [
        ...(есть("id") ? ["id"] : []),
        ...(есть("title") ? ["title"] : []),
        ...c.filter((k) => !постоянные.includes(k) && видно(k)),
      ],
      hidden: c.filter((k) => !постоянные.includes(k) && !видно(k)).length,
      same: постоянные.map((k) => [k, String(first[k] ?? "")] as [string, string]),
    };
  }, [sum, wide]);

  /**
   * Колонка, по которой записи идут вереницами: у требований это область —
   * пятнадцать `CFG`, потом одиннадцать `COR`. Значение показывается на первой
   * строке вереницы, дальше молчит, и на границе ложится линия.
   */
  /**
   * Колонки, идущие ВЕРЕНИЦАМИ: у требований это область — пятнадцать `CFG`,
   * потом одиннадцать `COR`. В них повтор молчит, и на границе ложится линия.
   *
   * Приоритет сюда НЕ попадает, и это важно: он чередуется, а не тянется, и
   * схлопнутый читается дырами — «О _ _ _ Ж» вместо «О О О О Ж». Схлопывать
   * можно повтор, но не чередование.
   */
  const вереницы = useMemo(() => {
    const rs = sum?.rows ?? [];
    const nums = new Set(sum?.numbers ?? []);
    if (rs.length < 12) return new Set<string>();
    const годные = (sum?.columns ?? []).filter((k) => {
      if (nums.has(k)) return false;
      const v = rs.map((r) => String(r[k] ?? ""));
      const разных = new Set(v).size;
      if (разных < 2 || разных > v.length / 4) return false;
      const скачков = v.filter((x, i) => i > 0 && x !== v[i - 1]).length;
      return скачков <= разных + 1;
    });
    return new Set(годные);
  }, [sum]);
  const группа = [...вереницы][0] ?? "";

  const rows = useMemo(() => {
    const r = sum?.rows ?? [];
    const ищем = q.trim().toLowerCase();
    const отобрано = ищем
      ? r.filter((x) => Object.values(x).some((v) => String(v).toLowerCase().includes(ищем)))
      : r;
    if (!by) return отобрано;
    const число = (sum?.numbers ?? []).includes(by);
    return [...отобрано].sort((a, b) =>
      число
        ? Number(b[by] ?? 0) - Number(a[by] ?? 0)
        : String(a[by] ?? "").localeCompare(String(b[by] ?? "")),
    );
  }, [sum, q, by]);

  if (!kinds) return <p className="empty">{say(lang, "co.reading")}</p>;

  return (
    <div className="corp">
      <nav className="co-kinds">
        <h2>{say(lang, "co.kinds")}</h2>
        <ul>
          {kinds.map((k) => (
            <li key={k.kind}>
              <button
                type="button"
                className={k.kind === kind ? "on" : ""}
                onClick={() => setKind(k.kind)}
              >
                <span>{kindName(lang, k.kind)}</span>
                <b>{k.count}</b>
              </button>
              <i style={{ transform: `scaleX(${(k.count ?? 0) / (kinds[0]?.count || 1)})` }} />
            </li>
          ))}
        </ul>
      </nav>

      <section className="co-main">
        {/* Раскладка одной записи вытесняет таблицу, а не приписывается к ней:
            человек смотрит либо на множество, либо на одну штуку. */}
        {ent ? (
          <One ent={ent} lang={lang} onBack={() => setPick("")} />
        ) : !sum ? (
          <p className="empty">{say(lang, "co.reading")}</p>
        ) : sum.rows.length === 0 ? (
          <p className="empty">{say(lang, "co.none")}</p>
        ) : (
          <>
            <div className="co-top">
              <input
                className="co-q"
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder={say(lang, "co.search")}
                type="search"
              />
              <span className="co-cnt">
                {rows.length} {say(lang, "co.rows")}
              </span>
              {columns.hidden > 0 && (
                <button className="co-wide" type="button" onClick={() => setWide(true)}>
                  + {columns.hidden} {say(lang, "co.cols")}
                </button>
              )}
              {wide && (
                <button className="co-wide" type="button" onClick={() => setWide(false)}>
                  {say(lang, "co.fold")}
                </button>
              )}
              {columns.same.length > 0 && (
                <span className="co-same">
                  {columns.same.map(([k, v]) => (
                    <span key={k}>
                      {подпись(lang, k)} <b>{v || "—"}</b>
                    </span>
                  ))}
                </span>
              )}
            </div>
            <div className="co-tbl">
              <table>
                <thead>
                  <tr>
                    {columns.show.map((c) => (
                      <th key={c} className={клеть(c, sum.numbers ?? [])}>
                        <button type="button" onClick={() => setBy(by === c ? "" : c)} className={by === c ? "on" : ""}>
                          {подпись(lang, c)}
                        </button>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.slice(0, 400).map((r, i, all) => (
                    <tr
                      key={String(r.id)}
                      onClick={() => setPick(String(r.id))}
                      /* Полоса начинается там, где меняется группа: пятнадцать
                         «CFG» подряд читаются как одна, а не как пятнадцать. */
                      className={
                        группа && i > 0 && String(all[i - 1]?.[группа] ?? "") !== String(r[группа] ?? "")
                          ? "brk"
                          : ""
                      }
                    >
                      {columns.show.map((c) => (
                        <td
                          key={c}
                          className={
                            клеть(c, sum.numbers ?? []) +
                            // Ноль доказательств — не «мало», а «ничем».
                            (ДОКАЗ.has(c) && Number(r[c] ?? 0) === 0 ? " zero" : "") +
                            ((sum.numbers ?? []).includes(c) && Number(r[c] ?? 0) === 1 ? " one" : "")
                          }
                        >
                          {c === "id" ? (
                            <span className="co-id">{String(r[c] ?? "")}</span>
                          ) : вереницы.has(c) &&
                            i > 0 &&
                            String(all[i - 1]?.[c] ?? "") === String(r[c] ?? "") ? (
                            // Повтор молчит: пятнадцать «CFG» подряд читаются
                            // как один, а не как пятнадцать.
                            ""
                          ) : (
                            String(r[c] ?? "")
                          )}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </section>
    </div>
  );
}

/** Раскладка одной записи: колонки, доказательство, кто на ней стоит, где написана. */
function One({ ent, lang, onBack }: { ent: Ent; lang: Lang; onBack: () => void }): React.JSX.Element {
  const e = ent.entity ?? {};
  // Служебные колонки наружу не идут: они про то, ОТКУДА запись, а не что в ней.
  const скрыть = new Set(["id", "entity_kind", "entity_name", "origin", "section_ord", "project_id"]);
  const поля = Object.entries(e).filter(([k, v]) => !скрыть.has(k) && v !== "" && v !== null);
  const длинное = поля.filter(([, v]) => typeof v === "string" && v.length > 60);
  const короткое = поля.filter(([, v]) => !(typeof v === "string" && v.length > 60));
  const жив = ent.live?.state ?? "";
  const где = ent.saidIn?.definedIn?.[0];
  const стоят = (ent.saidIn?.where ?? []).filter((w) => w.role !== "defines");

  return (
    <article className="co-one">
      <button className="co-back" type="button" onClick={onBack}>
        {say(lang, "co.back")}
      </button>
      <div className="co-one-top">
        <span className="co-one-id">{ent.id}</span>
        {короткое.map(([k, v]) => (
          <span key={k} className="co-pill">
            {подпись(lang, k)} {String(v)}
          </span>
        ))}
        {жив && (
          <span className={`co-pill ${жив === "reopened" ? "warn" : "done"}`}>
            {say(lang, жив === "reopened" ? "co.live.reopened" : "co.live.current")}
          </span>
        )}
      </div>

      {длинное.map(([k, v]) => (
        <p key={k} className="co-lead">
          {String(v)}
        </p>
      ))}

      <div className="co-grid">
        <div className="co-cell">
          <h5>{say(lang, "co.written")}</h5>
          {где ? (
            <ul>
              <li>
                <span className="co-where">
                  {где.kind}
                  {где.name ? `/${где.name}` : ""} §{где.section}
                </span>
              </li>
              <li>
                <span className="co-dim">{где.title}</span>
              </li>
            </ul>
          ) : (
            <p className="co-dim">{say(lang, "co.noproof")}</p>
          )}
        </div>

        <div className="co-cell">
          <h5>{say(lang, "co.rests")}</h5>
          {стоят.length === 0 ? (
            <p className="co-dim">{say(lang, "de.noOne")}</p>
          ) : (
            <ul>
              {стоят.slice(0, 10).map((w) => (
                <li key={`${w.kind}-${w.name}-${w.section}`}>
                  <span className="co-id">{w.name || w.kind}</span>
                  <span className="co-dim">{kindName(lang, w.kind)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        {ent.relations?.links && Object.keys(ent.relations.links).length > 0 && (
          <div className="co-cell">
            <h5>{say(lang, "co.proof")}</h5>
            <ul>
              {Object.entries(ent.relations.links).map(([k, n]) => (
                <li key={k}>
                  <span className="co-id">{n}</span>
                  <span className="co-dim">{подпись(lang, k)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="co-cell">
          <h5>{say(lang, "co.impact")}</h5>
          <ul>
            <li>
              <span className="co-id">{ent.saidIn?.count ?? 0}</span>
              <span className="co-dim">{say(lang, "co.linked")}</span>
            </li>
            {ent.relations?.backlinks != null && (
              <li>
                <span className="co-id">{ent.relations.backlinks}</span>
                <span className="co-dim">backlinks</span>
              </li>
            )}
          </ul>
        </div>
      </div>
    </article>
  );
}
