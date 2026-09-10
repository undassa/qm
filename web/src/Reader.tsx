import type React from "react";
import { useEffect, useState } from "react";
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
export function Reader({ projectId }: { projectId: string }): React.JSX.Element {
  const [kinds, setKinds] = useState<KindRow[] | null>(null);
  const [kind, setKind] = useState<KindRow | null>(null);
  const [ids, setIds] = useState<string[] | null>(null);
  const [query, setQuery] = useState("");
  const [id, setId] = useState<string>("");
  const [entity, setEntity] = useState<Entity | null>(null);
  const [sections, setSections] = useState<Section[] | null>(null);
  /** Что открыто и что уже прочитано — по якорю раздела; `""` — документ без разделов. */
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [body, setBody] = useState<Map<string, DocBlock[]>>(new Map());
  const [links, setLinks] = useState<Backlink[]>([]);
  const [failed, setFailed] = useState("");

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
    void loadIds(projectId, kind.kind).then(setIds);
  }, [projectId, kind]);

  /**
   * Открытие сущности не тянет её текст.
   *
   * Документ набора бывает на сотню килобайт, и вываленный целиком он читается
   * не лучше, чем не открытый вовсе. Приходит **оглавление с весом** — сколько
   * в разделе блоков, знаков, таблиц, — а текст берётся разделом по требованию.
   */
  function show(k: KindRow, name?: string): void {
    setFailed("");
    setId(name ?? "");
    setOpen(new Set());
    setBody(new Map());
    setSections(null);
    setLinks([]);
    void loadEntityByName(projectId, k.kind, name, true)
      .then(setEntity)
      .catch((e: unknown) => setFailed(String(e)));
    // ВНУТРЕННИЙ вид разделов НЕ ИМЕЕТ. Проверка `TC-BUS-04` — строка таблицы
    // внутри `test-cases`, и спросить у неё разделы значит получить разделы
    // КОНТЕЙНЕРА: все триста шестьдесят проверок открывались одним и тем же
    // документом в 58 КБ, отличаясь только заголовком в списке слева.
    if (k.shape === "inner") setSections([]);
    else
      void loadSections(projectId, k.kind, name)
        .then((all) => {
          setSections(all);
          // Документ без заголовков разделить нечем — он и есть один раздел.
          if (!all.length) void read(k.kind, name ?? "", "");
          else if (all[0]) void read(k.kind, name ?? "", all[0].anchor);
        })
        .catch(() => setSections([]));
    void loadBacklinks(projectId, k.kind, name)
      .then((d) => setLinks(d.backlinks))
      .catch(() => setLinks([]));
  }

  /** Блоки раздела — собственные: вложенные подразделы открываются своими строками. */
  async function read(k: string, name: string, a: string): Promise<void> {
    setOpen((o) => new Set(o).add(a));
    if (body.has(a)) return;
    const d = await loadBlocks(projectId, k, name || undefined, a || undefined, true).catch(() => ({
      blocks: [] as DocBlock[],
    }));
    setBody((m) => new Map(m).set(a, d.blocks));
  }

  function toggle(a: string): void {
    if (!kind) return;
    if (open.has(a)) {
      setOpen((o) => {
        const n = new Set(o);
        n.delete(a);
        return n;
      });
      return;
    }
    void read(kind.kind, id, a);
  }

  /** Целиком — по явной просьбе, а не по умолчанию. */
  function all(): void {
    if (!kind || !sections) return;
    for (const s of sections) void read(kind.kind, id, s.anchor);
  }

  if (!kinds) return <p className="empty">Читаю виды…</p>;

  const shown = (ids ?? []).filter((x) => x.toLowerCase().includes(query.trim().toLowerCase()));
  const tables = (sections ?? []).reduce((n, s) => n + s.tables, 0);


  return (
    <>
      <div className="head">
        <div>
          <h1>Документы</h1>
          <div className="prov">вид · сущность · что написано</div>
        </div>
        <span className="prov">
          {kind ? (
            <>
              <b>{kind.kind}</b>
              {kind.single ? " · одиночка" : ` · ${kind.count ?? "—"}`}
            </>
          ) : null}
        </span>
      </div>

      <div className={`reader${kind?.single ? " solo" : ""}`}>
        <nav className="reader-kinds" aria-label="Виды">
          {kinds.map((k) => (
            <button
              type="button"
              key={k.kind}
              className={`rk${kind?.kind === k.kind ? " on" : ""}`}
              onClick={() => setKind(k)}
            >
              <span>{k.kind}</span>
              <i>{k.single ? "1" : (k.count ?? "—")}</i>
            </button>
          ))}
        </nav>

        {kind?.single ? null : (
        <div className="reader-list">
          {ids === null ? (
            <p className="empty">Читаю имена…</p>
          ) : (
            <>
              <input
                className="rows-search"
                type="search"
                value={query}
                placeholder="искать по имени…"
                onChange={(e) => setQuery(e.target.value)}
                aria-label="Поиск по имени"
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

        <article className="reader-doc">
          {failed ? (
            <p className="empty">
              Не читается: <code>{failed}</code>
            </p>
          ) : !entity ? (
            <p className="empty">Выберите сущность слева.</p>
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
                <p className="empty">Читаю оглавление…</p>
              ) : sections.length ? (
                <>
                  <div className="doc-sum">
                    <span>
                      {sections.length} {plural(sections.length, "раздел", "раздела", "разделов")} ·{" "}
                      {sections.reduce((n, s) => n + s.blocks, 0)} бл ·{" "}
                      {kb(sections.reduce((n, s) => n + s.chars, 0))}
                      {tables ? ` · ${tables} ${plural(tables, "таблица", "таблицы", "таблиц")}` : ""}
                    </span>
                    <button type="button" className="ghost" onClick={all}>
                      раскрыть всё
                    </button>
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
                          body.has(s.anchor) ? (
                            own(body.get(s.anchor)!).length ? (
                              <Blocks blocks={own(body.get(s.anchor)!)} />
                            ) : (
                              <p className="side-note">Собственного текста нет — только подразделы ниже.</p>
                            )
                          ) : (
                            <p className="empty">Читаю…</p>
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
                <Row row={entity.entity as Record<string, unknown>} />
              ) : (
                <p className="empty">{(entity as { why?: string }).why ?? "Текста нет."}</p>
              )}

              {/* Одна сущность может ссылаться дважды — это одна ссылающаяся
                  сущность, а не две. Число ссылок сохраняется рядом. */}
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
                          setKind(target);
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
function Row({ row }: { row: Record<string, unknown> }): React.JSX.Element {
  const where = String(row["entity_kind"] ?? "");
  const whereName = String(row["entity_name"] ?? "");
  const skip = new Set(["entity_kind", "entity_name"]);
  const fields = Object.entries(row).filter(([k, v]) => !skip.has(k) && String(v ?? "").trim() !== "");
  const empty = Object.entries(row).filter(([k, v]) => !skip.has(k) && String(v ?? "").trim() === "");
  return (
    <div className="row-view">
      {where ? (
        <p className="row-where">
          записано в <b>{where}</b>
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
