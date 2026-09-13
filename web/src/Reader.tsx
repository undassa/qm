import type React from "react";
import { type Lang, say } from "./say";
import { useEffect, useRef, useState } from "react";
import {
  loadBacklinks,
  loadEntityByName,
  loadIds,
  loadKinds,
  loadBlocks,
  loadSections,
  type Backlink,
  type Entity,
  type KindRow,
  type Section,
  tool,
} from "./api";
import { Blocks, type DocBlock } from "./Blocks";
import { Provenance } from "./Provenance";
import { plural } from "./findings";

/**
 * Тело раздела без его собственного заголовка: заголовок — это строка
 * оглавления, под которой тело и открылось, и второй раз он не нужен.
 * В наборе каждый заголовок объявлен разделом (1192 документа, расхождений 0),
 * поэтому внутри тела чужих заголовков не бывает.
 */
function own(blocks: DocBlock[]): DocBlock[] {
  return blocks[0]?.kind === "heading" ? blocks.slice(1) : blocks;
}

/** Знаки в килобайтах: «3,7 КБ» читается, «3790» — нет. */
function kb(chars: number): string {
  return chars < 1024 ? `${chars} зн` : `${(chars / 1024).toFixed(1).replace(".", ",")} КБ`;
}

/**
 * Читалка набора: вид → сущность → документ.
 *
 * Три колонки, и каждая отвечает на свой вопрос: что вообще есть · что есть
 * этого вида · что написано. Раздел открывается **запросом к серверу**, а не
 * прокруткой к найденному в тексте заголовку: якорь считает сервер, и своё
 * правило якоря здесь было бы вторым — разошлись бы молча.
 *
 * Обратные ссылки лежат рядом с текстом, а не на другой странице: «кто на это
 * ссылается» — часть чтения, а не отдельное занятие.
 */
export function Reader({
  projectId,
  want,
  lang,
}: {
  projectId: string;
  want?: { kind?: string; id?: string } | null;
  lang: Lang;
}): React.JSX.Element {
  const [kinds, setKinds] = useState<KindRow[] | null>(null);
  const [kind, setKind] = useState<KindRow | null>(null);
  const [ids, setIds] = useState<string[] | null>(null);
  const [query, setQuery] = useState("");
  const [видыОткрыты, setВидыОткрыты] = useState(false);
  const [id, setId] = useState<string>("");
  const [entity, setEntity] = useState<Entity | null>(null);
  const [sections, setSections] = useState<Section[] | null>(null);
  /** Что открыто и что уже прочитано — по якорю раздела; `""` — документ без разделов. */
  const [open, setOpen] = useState<Set<string>>(new Set());
  // Разделы, чьё тело не прочиталось: отказ — это не пустота.
  const [bad, setBad] = useState<Set<string>>(new Set());
  const [body, setBody] = useState<Map<string, DocBlock[]>>(new Map());
  const [links, setLinks] = useState<Backlink[]>([]);
  /** Куда ведёт сам документ. Входящие видны давно, исходящие — нет, и узнать
      их можно было только раскрыв все разделы и вычитав прозу. */
  const [out, setOut] = useState<Record<string, { id: string; kind: string; title?: string }[]> | null>(null);
  const [failed, setFailed] = useState("");
  /**
   * След перехода: куда щёлкали, оттуда можно вернуться.
   *
   * Переход по ссылке без возврата хуже отсутствия перехода: читатель уходит на
   * соседний документ и теряет то, ради чего читал. Кнопка браузера здесь не
   * помогает — адрес не менялся.
   */
  const [trail, setTrail] = useState<{ kind: KindRow; id: string }[]>([]);

  useEffect(() => {
    if (!projectId) return;
    void loadKinds(projectId).then((all) => {
      setKinds(all);
      setKind((k) => k ?? all.find((x) => x.kind === "constitution") ?? all[0] ?? null);
    });
  }, [projectId]);

  useEffect(() => {
    if (!kind) return;
    setIds(null);
    setId("");
    setEntity(null);
    setQuery("");
    if (kind.single) {
      setIds([]);
      show(kind, undefined);
      return;
    }
    void loadIds(projectId, kind.kind).then((names) => {
      setIds(names);
      // Первое имя открывается САМО. «Выберите сущность слева» — не ответ, а
      // отсрочка: выбрали вид, значит хотят его читать, и лишний щелчок здесь
      // ничего не уточняет.
      if (!id && names.length && kind) show(kind, names[0]);
    });
  }, [projectId, kind]);

  /**
   * Открытие сущности не тянет её текст.
   *
   * Документ набора бывает на сотню килобайт, и вываленный целиком он читается
   * не лучше, чем не открытый вовсе. Приходит **оглавление с весом** — сколько
   * в разделе блоков, знаков, таблиц, — а текст берётся разделом по требованию.
   */
  /**
   * Какой документ показан СЕЙЧАС. Ответы приходят вразнобой: читалка сперва
   * открывает одиночку по умолчанию, а следом — просимый ссылкой, и медленный
   * ответ первого перетирал оглавление второго. Дальше «раскрыть всё» слало
   * якоря ЧУЖОГО документа под нынешним родом и без имени — сервер отвечал
   * отказом на каждый, а страница показывала «собственного текста нет».
   */
  const показан = useRef("");
  const разбор = (метка: string): [string, string] => {
    const i = метка.indexOf("/");
    return i < 0 ? [метка, ""] : [метка.slice(0, i), метка.slice(i + 1)];
  };

  function show(k: KindRow, name?: string): void {
    const этот = `${k.kind}/${name ?? ""}`;
    показан.current = этот;
    setFailed("");
    setId(name ?? "");
    setOpen(new Set());
    setBody(new Map());
    setBad(new Set());
    setSections(null);
    setLinks([]);
    void loadEntityByName(projectId, k.kind, name, true)
      .then((e) => {
        if (показан.current === этот) setEntity(e);
      })
      .catch((e: unknown) => {
        if (показан.current === этот) setFailed(String(e));
      });
    // ВНУТРЕННИЙ вид разделов НЕ ИМЕЕТ. Проверка `TC-BUS-04` — строка таблицы
    // внутри `test-cases`, и спросить у неё разделы значит получить разделы
    // КОНТЕЙНЕРА: все триста шестьдесят проверок открывались одним и тем же
    // документом в 58 КБ, отличаясь только заголовком в списке слева.
    if (k.shape === "inner") setSections([]);
    else
      void loadSections(projectId, k.kind, name)
        .then((all) => {
          // Ответ по документу, с которого уже ушли, не показывается.
          if (показан.current !== этот) return;
          setSections(all);
          // Документ без заголовков разделить нечем — он и есть один раздел.
          if (!all.length) void read(k.kind, name ?? "", "");
          else if (all[0]) void read(k.kind, name ?? "", all[0].anchor);
        })
        .catch(() => {
          if (показан.current === этот) setSections([]);
        });
    setOut(null);
    void tool<{ sets?: Record<string, { id: string; kind: string; title?: string }[]> }>(
      projectId,
      "links-of",
      name ? { kind: k.kind, id: name } : { kind: k.kind },
    )
      .then((d) => {
        if (показан.current === этот) setOut(d.sets ?? {});
      })
      // Отказ двери — «наборы для рода не объявлены», и это не пустота.
      .catch(() => {
        if (показан.current === этот) setOut(null);
      });
    void loadBacklinks(projectId, k.kind, name)
      .then((d) => setLinks(d.backlinks))
      .catch(() => setLinks([]));
  }

  /**
   * Открыть то, что попросили снаружи — палитрой или ссылкой в адресе.
   *
   * Реагируем на СМЕНУ просьбы, а не на её наличие: иначе всякая перерисовка
   * возвращала бы читателя туда, откуда он уже ушёл.
   */
  const asked = want?.kind ? `${want.kind}/${want.id ?? ""}` : "";
  const [wasAsked, setWasAsked] = useState("");
  useEffect(() => {
    if (!asked || asked === wasAsked || !kinds) return;
    setWasAsked(asked);
    const row = kinds.find((k) => k.kind === want?.kind);
    if (!row) { setFailed(`вида «${want?.kind}» в наборе нет`); return; }
    setFailed("");
    setKind(row);
    void loadIds(projectId, row.kind).then(setIds).catch(() => setIds([]));
    show(row, want?.id);
  }, [asked, wasAsked, kinds, projectId, want]);

  /**
   * Переход по ссылке внутрь набора.
   *
   * Ссылки вида `decision:ADR-0138` разметка теперь отдаёт якорем с видом и
   * именем в данных. Ловим их здесь, у общего предка: вешать обработчик на
   * каждый абзац значило бы вешать его на тысячу абзацев.
   *
   * Ссылка на вид, которого в наборе нет, НЕ гасится молча: щёлкнувший должен
   * узнать, что документ обещает несуществующее, — это находка, а не пустота.
   */
  function follow(e: React.MouseEvent<HTMLElement>): void {
    const a = (e.target as HTMLElement).closest("a.go") as HTMLAnchorElement | null;
    if (!a) return;
    e.preventDefault();
    const want = a.dataset["kind"] ?? "";
    const name = a.dataset["id"] ?? "";
    const row = (kinds ?? []).find((k) => k.kind === want);
    if (!row) {
      setFailed(`ссылка ведёт в вид «${want}», которого в наборе нет`);
      return;
    }
    if (kind) setTrail((t) => [...t, { kind, id }]);
    setKind(row);
    void loadIds(projectId, row.kind).then(setIds).catch(() => setIds([]));
    show(row, name);
    document.querySelector(".reader-doc")?.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  /** Блоки раздела — собственные: вложенные подразделы открываются своими строками. */
  /**
   * Тело раздела.
   *
   * Отказ запроса ЗАПОМИНАЕТСЯ отдельно. Прежде он превращался в пустой список
   * блоков, а пустой список рисуется словами «собственного текста нет» — и
   * непрочитанный раздел был неотличим от пустого. Под нагрузкой так пропадал
   * весь текст документа, и страница уверенно показывала, что его нет.
   */
  async function read(k: string, name: string, a: string): Promise<void> {
    const этот = `${k}/${name}`;
    setOpen((o) => new Set(o).add(a));
    if (body.has(a)) return;
    setBad((b) => {
      if (!b.has(a)) return b;
      const n = new Set(b);
      n.delete(a);
      return n;
    });
    try {
      const d = await loadBlocks(projectId, k, name || undefined, a || undefined, true);
      if (показан.current === этот) setBody((m) => new Map(m).set(a, d.blocks));
    } catch {
      if (показан.current === этот) setBad((b) => new Set(b).add(a));
    }
  }

  function toggle(a: string): void {
    if (!kind) return;
    const [, имяДок] = разбор(показан.current);
    if (open.has(a)) {
      setOpen((o) => {
        const n = new Set(o);
        n.delete(a);
        return n;
      });
      return;
    }
    void read(kind.kind, имяДок, a);
  }

  /** Целиком — по явной просьбе, а не по умолчанию. */
  function all(): void {
    if (!kind || !sections) return;
    const [род, имя] = разбор(показан.current);
    if (род !== kind.kind) return;
    for (const s of sections) void read(kind.kind, имя, s.anchor);
  }

  if (!kinds) return <p className="empty">{say(lang, "rr.readingKinds")}</p>;

  const shown = (ids ?? []).filter((x) => x.toLowerCase().includes(query.trim().toLowerCase()));
  const tables = (sections ?? []).reduce((n, s) => n + s.tables, 0);


  return (
    <>
      <div className="head">
        <div>
          <h1>{say(lang, "rr.head")}</h1>
          <div className="prov">{say(lang, "rr.sub")}</div>
        </div>
        <span className="prov">
          {kind ? (
            <>
              <b>{kind.kind}</b>
              {kind.single ? ` · ${say(lang, "rr.single")}` : ` · ${kind.count ?? "—"}`}
            </>
          ) : null}
        </span>
      </div>

      <div className={`reader${kind?.single ? " solo" : ""}${видыОткрыты ? " open" : ""}`}>
        {/* Свёртка видов — для телефона: сорок видов столбиком не оставляли
            документу ни одной колонки, и текст шёл по десять знаков в строку.
            На широком экране кнопка спрятана стилем. */}
        <button type="button" className="rd-fold" onClick={() => setВидыОткрыты((v) => !v)}>
          <span>{kind?.kind ?? "вид"}</span>
          <b>{kind ? (kind.single ? "1" : (kind.count ?? "—")) : ""}</b>
          <i>{видыОткрыты ? "▴" : "▾"}</i>
        </button>
        <nav className="reader-kinds" aria-label={say(lang, "rr.kinds")}>
          {kinds.map((k) => (
            <button
              type="button"
              key={k.kind}
              className={`rk${kind?.kind === k.kind ? " on" : ""}${k.count === 0 ? " none" : ""}`}
              onClick={() => {
                setKind(k);
                setВидыОткрыты(false);
              }}
            >
              <span>{k.kind}</span>
              <i>{k.single ? "1" : (k.count ?? "—")}</i>
            </button>
          ))}
        </nav>

        {kind?.single ? null : (
        <div className="reader-list">
          {ids === null ? (
            <p className="empty">{say(lang, "rr.readingNames")}</p>
          ) : (
            <>
              <input
                className="rows-search"
                type="search"
                value={query}
                placeholder={say(lang, "rr.searchName")}
                onChange={(e) => setQuery(e.target.value)}
                aria-label={say(lang, "rr.searchNameAria")}
              />
              <div className="rk-list">
                {shown.slice(0, 300).map((name) => (
                  <button
                    type="button"
                    key={name}
                    className={`rk${id === name ? " on" : ""}`}
                    onClick={() => kind && show(kind, name)}
                  >
                    <span>{name}</span>
                  </button>
                ))}
                {shown.length > 300 ? (
                  <p className="side-note">ещё {shown.length - 300} — сузьте поиск</p>
                ) : null}
              </div>
            </>
          )}
        </div>
        )}

        <article className="reader-doc" onClick={follow}>
              {trail.length > 0 ? (
                <button
                  type="button"
                  className="trail-back"
                  onClick={(e) => {
                    e.stopPropagation();
                    const last = trail[trail.length - 1];
                    if (!last) return;
                    setTrail((t) => t.slice(0, -1));
                    setKind(last.kind);
                    void loadIds(projectId, last.kind.kind).then(setIds).catch(() => setIds([]));
                    show(last.kind, last.id);
                  }}
                >
                  ← назад к {trail[trail.length - 1]?.id || trail[trail.length - 1]?.kind.kind}
                </button>
              ) : null}
          {failed ? (
            <p className="empty">{say(lang, "rr.unreadable")}<code>{failed}</code>
            </p>
          ) : !entity ? (
            <p className="empty">{say(lang, "rr.pickLeft")}</p>
          ) : (
            <>
              <div className="doc-head">
                <b>
                  {entity.kind} {entity.id !== entity.kind ? entity.id : ""}
                </b>
                {entity.revision !== undefined ? (
                  <span className="prov">
                    правка {entity.revision} · {entity.updatedBy}
                  </span>
                ) : null}
              </div>

              {sections === null ? (
                <p className="empty">{say(lang, "rr.readingToc")}</p>
              ) : sections.length ? (
                <>
                  <div className="doc-sum">
                    <span>
                      {sections.length} {plural(sections.length, "раздел", "раздела", "разделов")} ·{" "}
                      {sections.reduce((n, s) => n + s.blocks, 0)} бл ·{" "}
                      {kb(sections.reduce((n, s) => n + s.chars, 0))}
                      {tables ? ` · ${tables} ${plural(tables, "таблица", "таблицы", "таблиц")}` : ""}
                    </span>
                    <button type="button" className="ghost" onClick={all}>{say(lang, "rr.openAll")}</button>
                  </div>
                  <div className="doc-secs">
                    {sections.map((s) => (
                      <section key={s.anchor} className={`sec l${Math.min(s.level, 4)}`}>
                        <button
                          type="button"
                          className={`sec-h${open.has(s.anchor) ? " on" : ""}`}
                          aria-expanded={open.has(s.anchor)}
                          onClick={() => toggle(s.anchor)}
                        >
                          <i className="caret">{open.has(s.anchor) ? "▾" : "▸"}</i>
                          <span className="sec-t">{s.title}</span>
                          {/* Вес раздела виден до того, как он открыт: читатель
                              решает, что грузить, зная сколько это. */}
                          <span className="sec-w">
                            {s.blocks} бл · {kb(s.chars)}
                            {s.tables ? ` · ⊞${s.tables}` : ""}
                            {s.code ? ` · {}${s.code}` : ""}
                          </span>
                        </button>
                        {open.has(s.anchor) ? (
                          bad.has(s.anchor) ? (
                            <p className="side-note warn">
                              Раздел не прочитался.{" "}
                              <button
                                type="button"
                                className="ghost"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  if (kind) void read(kind.kind, разбор(показан.current)[1], s.anchor);
                                }}
                              >{say(lang, "rr.again")}</button>
                            </p>
                          ) : body.has(s.anchor) ? (
                            own(body.get(s.anchor)!).length ? (
                              <Blocks blocks={own(body.get(s.anchor)!)} />
                            ) : (
                              <p className="side-note">{say(lang, "rr.noOwnText")}</p>
                            )
                          ) : (
                            <p className="empty">{say(lang, "rr.reading")}</p>
                          )
                        ) : null}
                      </section>
                    ))}
                  </div>
                </>
              ) : body.get("")?.length ? (
                // Документ без заголовков: делить нечего, показывается целиком.
                <Blocks blocks={body.get("")!} />
              ) : entity.entity ? (
                // Внутренняя сущность — строка предметной таблицы: текста у неё нет,
                // и подставлять его неоткуда. Зато есть место, где она записана, и
                // оно называется: без него строка висит в воздухе.
                <Row row={entity.entity as Record<string, unknown>} lang={lang} />
              ) : (
                <p className="empty">{(entity as { why?: string }).why ?? "Текста нет."}</p>
              )}

              {/* Одна сущность может ссылаться дважды — это одна ссылающаяся
                  сущность, а не две. Число ссылок сохраняется рядом. */}
              {out && Object.values(out).some((v) => v.length) ? (
                <div className="backs">
                  <h3 className="rows-group">{say(lang, "rr.pointsTo")}</h3>
                  <div className="backs-list">
                    {Object.entries(out)
                      .filter(([, v]) => v.length)
                      .flatMap(([, v]) => v)
                      .slice(0, 60)
                      .map((x) => (
                        <a
                          key={`${x.kind}/${x.id}`}
                          className="go back"
                          href={`?page=read&kind=${encodeURIComponent(x.kind)}&id=${encodeURIComponent(x.id)}`}
                          data-kind={x.kind}
                          data-id={x.id}
                          title={x.title ?? ""}
                        >
                          {x.id || x.kind}
                        </a>
                      ))}
                  </div>
                </div>
              ) : null}

              {links.length ? (
                <div className="backs">
                  <h3 className="rows-group">
                    На это ссылаются — {links.length} {plural(links.length, "раз", "раза", "раз")}
                  </h3>
                  <div className="backs-list">
                    {[...links.reduce((m, l) => m.set(l.from, (m.get(l.from) ?? 0) + 1), new Map<string, number>())]
                      .slice(0, 60)
                      .map(([from, times]) => ({ from, times, label: null as string | null }))
                      .map((l) => (
                      <button
                        type="button"
                        key={l.from}
                        className="back"
                        onClick={() => {
                          const [k, ...rest] = l.from.split(" ");
                          const target = kinds.find((x) => x.kind === k);
                          if (!target) return;
                          // Уход по обратной ссылке — такой же уход, как по
                          // прямой: без следа читатель теряет то, ради чего
                          // смотрел, кто на это ссылается.
                          if (kind) setTrail((t) => [...t, { kind, id }]);
                          setKind(target);
                          void loadIds(projectId, target.kind).then(setIds).catch(() => setIds([]));
                          const name = rest.join(" ");
                          setTimeout(() => show(target, name || undefined), 0);
                        }}
                        title={l.label ?? ""}
                      >
                        {l.from}
                        {l.times > 1 ? <i> ×{l.times}</i> : null}
                      </button>
                    ))}
                  </div>
                </div>
              ) : null}
            </>
          )}
        </article>
      </div>

      <Provenance source="project_documents и разбор его разделов" computed="оглавление и обратные ссылки" />
    </>
  );
}

/**
 * Строка предметной таблицы: поля крупно, место записи — отдельной строкой.
 *
 * `entity_kind` и `entity_name` — не данные строки, а её АДРЕС: где она
 * записана. В общем списке полей они читались как ещё два поля со странными
 * именами, и «где это лежит» приходилось выводить самому.
 *
 * Пустые поля названы, но не показаны значениями: «поля нет» и «поле пустое» —
 * разное, и молчание про второе читается как первое.
 */
function Row({ row, lang }: { row: Record<string, unknown>; lang: Lang }): React.JSX.Element {
  const where = String(row["entity_kind"] ?? "");
  const whereName = String(row["entity_name"] ?? "");
  const skip = new Set(["entity_kind", "entity_name"]);
  const fields = Object.entries(row).filter(([k, v]) => !skip.has(k) && String(v ?? "").trim() !== "");
  const empty = Object.entries(row).filter(([k, v]) => !skip.has(k) && String(v ?? "").trim() === "");
  return (
    <div className="row-view">
      {where ? (
        <p className="row-where">{say(lang, "rr.writtenIn")}<b>{where}</b>
          {whereName ? <> · {whereName}</> : null}
        </p>
      ) : null}
      <dl className="ent-row">
        {fields.map(([field, value]) => (
          <div key={field}>
            <dt>{field}</dt>
            <dd>{String(value ?? "")}</dd>
          </div>
        ))}
      </dl>
      {empty.length > 0 ? <p className="row-empty">пусто: {empty.map(([k]) => k).join(" · ")}</p> : null}
    </div>
  );
}
