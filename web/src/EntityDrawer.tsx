import type React from "react";
import { useEffect, useState } from "react";
import { loadSections, loadBlocks, loadEntityByName, loadBacklinks } from "./api";
import { Blocks, type DocBlock } from "./Blocks";
import { Drawer } from "./Drawer";

/**
 * Дровер сущности: сам документ, а не пересказ его полей.
 *
 * Разделы показывали выбранную строку панелью сбоку: там лежали те же числа,
 * что и в таблице, плюс перечень связей. Чтобы прочитать, ЧТО В ДОКУМЕНТЕ
 * написано, надо было уйти в «Документы», вспомнить вид, найти имя. Между
 * вопросом «почему этот вопрос открыт» и ответом стояло три перехода.
 *
 * Здесь открывается документ: разделы, текст, обратные ссылки. Список за
 * дровером остаётся на месте — видно, куда вернёшься.
 *
 * У ВНУТРЕННЕЙ сущности документа нет: проверка или требование — строка внутри
 * чужого документа. Ей показываются поля и говорится, где она записана; выдать
 * за её текст текст контейнера значило бы соврать — так уже было в читалке, и
 * триста шестьдесят проверок открывались одним и тем же файлом.
 */

interface Section {
  anchor: string;
  title: string;
  level: number;
  blocks: number;
  chars: number;
}
interface Backlink {
  from: string;
}

export function EntityDrawer({
  projectId,
  kind,
  id,
  title,
  subtitle,
  inner,
  onClose,
  onFind,
}: {
  projectId: string;
  kind: string;
  id: string;
  title: string;
  subtitle?: string;
  /** Внутренняя сущность: у неё нет своего документа, только строка. */
  inner?: boolean;
  onClose: () => void;
  onFind?: (q: string) => void;
}): React.JSX.Element {
  const [sections, setSections] = useState<Section[] | null>(null);
  const [body, setBody] = useState<Map<string, DocBlock[]>>(new Map());
  const [fields, setFields] = useState<Record<string, unknown> | null>(null);
  const [links, setLinks] = useState<Backlink[]>([]);
  const [failed, setFailed] = useState("");

  useEffect(() => {
    setSections(null);
    setBody(new Map());
    setFields(null);
    setLinks([]);
    setFailed("");

    void loadEntityByName(projectId, kind, id, true)
      .then((e) => setFields(((e as { entity?: Record<string, unknown> }).entity ?? null)))
      .catch(() => setFields(null));

    if (inner) {
      setSections([]);
    } else {
      void loadSections(projectId, kind, id)
        .then((all) => {
          setSections(all as Section[]);
          // Первый раздел читается сразу: дровер, открывшийся оглавлением без
          // текста, требует ещё одного щелчка ради того, ради чего его открыли.
          const first = (all as Section[])[0];
          if (!all.length) void read("");
          else if (first) void read(first.anchor);
        })
        .catch((e: unknown) => {
          setSections([]);
          setFailed(String(e));
        });
    }

    void loadBacklinks(projectId, kind, id)
      .then((d) => setLinks(d.backlinks as Backlink[]))
      .catch(() => setLinks([]));

    async function read(anchor: string): Promise<void> {
      const d = await loadBlocks(projectId, kind, id || undefined, anchor || undefined, true).catch(() => ({
        blocks: [] as DocBlock[],
      }));
      setBody((m) => new Map(m).set(anchor, d.blocks));
    }
  }, [projectId, kind, id, inner]);

  async function open(anchor: string): Promise<void> {
    if (body.has(anchor)) return;
    const d = await loadBlocks(projectId, kind, id || undefined, anchor || undefined, true).catch(() => ({
      blocks: [] as DocBlock[],
    }));
    setBody((m) => new Map(m).set(anchor, d.blocks));
  }

  const where = String(fields?.["entity_kind"] ?? "");
  const skip = new Set(["entity_kind", "entity_name", "content", "body"]);
  const shownFields = Object.entries(fields ?? {}).filter(
    ([k, v]) => !skip.has(k) && String(v ?? "").trim() !== "",
  );

  return (
    <Drawer title={title} subtitle={subtitle ?? `${kind} · ${id}`} onClose={onClose}>
      {inner ? (
        <>
          {where ? (
            <p className="row-where">
              записано в <b>{where}</b>
            </p>
          ) : null}
          <dl className="ent-row">
            {shownFields.map(([f, v]) => (
              <div key={f}>
                <dt>{f}</dt>
                <dd>{String(v ?? "")}</dd>
              </div>
            ))}
          </dl>
        </>
      ) : sections === null ? (
        <p className="empty">Читаю документ…</p>
      ) : failed ? (
        <p className="empty">Не читается: <code>{failed}</code></p>
      ) : sections.length === 0 && !body.get("")?.length ? (
        <>
          <p className="note">Документа нет — только объявленные поля.</p>
          <dl className="ent-row">
            {shownFields.map(([f, v]) => (
              <div key={f}>
                <dt>{f}</dt>
                <dd>{String(v ?? "")}</dd>
              </div>
            ))}
          </dl>
        </>
      ) : sections.length === 0 ? (
        <Blocks blocks={body.get("") ?? []} />
      ) : (
        <div className="dr-secs">
          {sections.map((s) => (
            <section key={s.anchor} className={`sec l${Math.min(s.level, 4)}`}>
              <button
                type="button"
                className={`sec-h${body.has(s.anchor) ? " on" : ""}`}
                aria-expanded={body.has(s.anchor)}
                onClick={() => void open(s.anchor)}
              >
                <i className="caret">{body.has(s.anchor) ? "▾" : "▸"}</i>
                <span className="sec-t">{s.title}</span>
                <span className="sec-w">{s.blocks} бл</span>
              </button>
              {body.has(s.anchor) ? <Blocks blocks={body.get(s.anchor) ?? []} /> : null}
            </section>
          ))}
        </div>
      )}

      {links.length > 0 ? (
        <div className="drawer-facts">
          <h4>на это ссылаются · {links.length}</h4>
          <div className="art-chips">
            {[...new Set(links.map((l) => l.from))]
              .filter((f) => f.trim() !== "")
              .slice(0, 40)
              .map((from) => (
                <button key={from} type="button" className="art-chip" onClick={() => onFind?.(from)}>
                  {from}
                </button>
              ))}
          </div>
        </div>
      ) : null}
    </Drawer>
  );
}
