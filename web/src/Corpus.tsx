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
  checks: ["проверок", "checks"],
  stories: ["историй", "stories"],
  tasks: ["задач", "tasks"],
  needs: ["потребностей", "needs"],
  title: ["заголовок", "title"],
  state: ["состояние", "state"],
  answer: ["ответ", "answer"],
  text: ["текст", "text"],
  number: ["номер", "no."],
  status: ["состояние", "status"],
};
const подпись = (l: Lang, c: string): string => (КОЛОНКА[c] ? (l === "en" ? КОЛОНКА[c][1] : КОЛОНКА[c][0]) : c);

export function Corpus({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [kinds, setKinds] = useState<KindRow[] | null>(null);
  const [kind, setKind] = useState<string>("requirement");
  const [sum, setSum] = useState<Summary | null>(null);
  const [pick, setPick] = useState<string>("");
  const [ent, setEnt] = useState<Ent | null>(null);
  const [q, setQ] = useState<string>("");
  const [by, setBy] = useState<string>("");

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
            </div>
            <div className="co-tbl">
              <table>
                <thead>
                  <tr>
                    {sum.columns.map((c) => (
                      <th key={c} className={(sum.numbers ?? []).includes(c) ? "n" : ""}>
                        <button type="button" onClick={() => setBy(by === c ? "" : c)} className={by === c ? "on" : ""}>
                          {подпись(lang, c)}
                        </button>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.slice(0, 400).map((r) => (
                    <tr key={String(r.id)} onClick={() => setPick(String(r.id))}>
                      {sum.columns.map((c) => (
                        <td key={c} className={(sum.numbers ?? []).includes(c) ? "n" : ""}>
                          {c === "id" ? <span className="co-id">{String(r[c] ?? "")}</span> : String(r[c] ?? "")}
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
