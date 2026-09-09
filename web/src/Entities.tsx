import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { loadEntityByName, loadIds, loadKinds, type Entity, type KindRow } from "./api";
import { plural } from "./findings";
import { Drawer } from "./Drawer";
import { Markdown } from "./Markdown";
import { Provenance } from "./Provenance";
import { useRows, type Slice } from "./Rows";

/**
 * Набор — сущностями, а не файлами.
 *
 * У проекта есть конституция, требование `FR-ORG-12`, решение `ADR-0157`. Адрес
 * был подробностью того, что набор лежал файлами; переезд в базу его отменил, и
 * страница спрашивает то же, что харнес: вид и имя.
 *
 * Вид без проекции показывается **прочерком, а не нулём**. Ноль читается как
 * «ничего нет», тогда как ответа просто нет — и по нулю страницу закроют,
 * решив, что смотреть нечего.
 */
const SLICES: Slice<KindRow>[] = [
  {
    key: "shape",
    title: "по устройству",
    groupOf: (k) => (k.single ? "одиночки" : k.shape === "inner" ? "объявлены внутри документа" : "сами документы"),
    sort: (a, b) => a.kind.localeCompare(b.kind),
  },
  { key: "count", title: "по числу", sort: (a, b) => (b.count ?? -1) - (a.count ?? -1) },
];

export function Entities({ projectId }: { projectId: string }): React.JSX.Element {
  const [kinds, setKinds] = useState<KindRow[] | null>(null);
  const [failed, setFailed] = useState("");
  const [openKind, setOpenKind] = useState<KindRow | null>(null);
  const [ids, setIds] = useState<string[] | null>(null);
  const [open, setOpen] = useState<Entity | null>(null);

  useEffect(() => {
    if (!projectId) return;
    setKinds(null);
    setFailed("");
    void loadKinds(projectId).then(setKinds).catch((e: unknown) => setFailed(String(e)));
  }, [projectId]);

  useEffect(() => {
    if (!openKind) return;
    setIds(null);
    setOpen(null);
    if (openKind.single) {
      void loadEntityByName(projectId, openKind.kind).then(setOpen);
      return;
    }
    void loadIds(projectId, openKind.kind).then(setIds);
  }, [projectId, openKind]);

  const textOf = useCallback((k: KindRow) => `${k.kind} ${k.shape}`, []);
  const rows = useRows(kinds ?? [], textOf, SLICES, setOpenKind);

  const flatIndex = useMemo(() => {
    const map = new Map<string, number>();
    let at = 0;
    for (const g of rows.groups) for (const k of g.items) map.set(k.kind, at++);
    return map;
  }, [rows.groups]);

  const dark = useMemo(() => (kinds ?? []).filter((k) => !k.projected).map((k) => k.kind), [kinds]);

  if (failed) return <p className="empty">Виды не читаются: <code>{failed}</code>.</p>;
  if (!kinds) return <p className="empty">Читаю виды…</p>;

  const total = kinds.reduce((n, k) => n + (k.count ?? 0), 0);

  return (
    <>
      <div className="head">
        <div>
          <h1>Сущности</h1>
          <div className="prov">вид и имя вместо адреса</div>
        </div>
        <span className="prov">
          <b>{kinds.length}</b> {plural(kinds.length, "вид", "вида", "видов")} · {total} сущностей
        </span>
      </div>

      <p className="note">
        Спрашивается тем же словарём, что и харнесом: <code>вид</code> и <code>имя</code>.{" "}
        <b>Прочерк вместо числа — «вида в базе нет»</b>, а не «ноль штук»: {dark.length} таких —{" "}
        {dark.map((k) => <code key={k}>{k}</code>).reduce<React.ReactNode[]>((a, n, i) => (i ? [...a, " · ", n] : [n]), [])}.
      </p>

      {rows.search}

      <div className="tl corp">
        {rows.groups.map((group) => (
          <section key={group.title || "все"} className="tl-step">
            <div className="tl-k">
              <b>{group.title}</b>
              <span>{group.items.length}</span>
            </div>
            <ul className="corp-list">
              {group.items.map((k) => (
                <li key={k.kind} {...rows.rowProps(flatIndex.get(k.kind) ?? -1)}>
                  <button type="button" onClick={() => setOpenKind(k)}>
                    <b>{k.kind}</b>
                    <i className="when">{k.single ? "одиночка" : k.shape === "inner" ? `в «${k.in}»` : ""}</i>
                    <i className={k.projected ? "" : "none"}>{k.projected ? k.count : "—"}</i>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>

      <Provenance source="project_documents и предметные таблицы" computed="числа по видам" />

      {openKind && !open ? (
        <Drawer title={openKind.kind} subtitle={`${openKind.count ?? "—"} сущностей`} onClose={() => setOpenKind(null)}>
          {ids ? (
            <ul className="corp-list">
              {ids.slice(0, 400).map((id) => (
                <li key={id}>
                  <button type="button" onClick={() => void loadEntityByName(projectId, openKind.kind, id).then(setOpen)}>
                    <b>{id}</b>
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty">Читаю имена…</p>
          )}
        </Drawer>
      ) : null}

      {open ? (
        <Drawer
          title={`${open.kind} ${open.id}`}
          subtitle={open.revision !== undefined ? `правка ${open.revision} · ${open.updatedBy ?? ""}` : "объявлена внутри документа"}
          onClose={() => {
            setOpen(null);
            setOpenKind(null);
          }}
        >
          {open.content !== undefined ? (
            <Markdown body={open.content} />
          ) : (
            // Внутренняя сущность приходит строкой предметной таблицы: то, что
            // видно здесь, посчитано пересборкой, а не взято из текста.
            <dl className="ent-row">
              {Object.entries(open.entity ?? {}).map(([field, value]) => (
                <div key={field}>
                  <dt>{field}</dt>
                  <dd data-field={field}>{String(value ?? "")}</dd>
                </div>
              ))}
            </dl>
          )}
        </Drawer>
      ) : null}
    </>
  );
}
