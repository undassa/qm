import type React from "react";
import { useEffect, useState } from "react";
import { tool } from "./api";

/**
 * Зависимости набора — то, чего интерфейс не показывал никогда.
 *
 * Связи есть давно, но поодиночке их три тысячи, и окинуть взглядом нельзя.
 * Свёрнутые до родов, они умещаются в два десятка рёбер, и это карта того, как
 * устроен проект: задача стоит на требовании, требование на потребности, этап
 * на своих перечнях.
 *
 * Ребро «А → Б» читается «А стоит на Б», и направление здесь несущее: правка Б
 * переоткрывает А, а не наоборот. Стрелка нарисована, чтобы это было видно без
 * подписи.
 *
 * Вторая половина отвечает на вопрос, который прежде задавали только задним
 * числом: «что переоткроется, если изменить вот это». Обход идёт по тем же
 * связям и той же глубине, что и живой каскад, — но от предполагаемой правки.
 */
interface Node {
  kind: string;
  count: number;
  reopened: number;
  reopens: boolean;
}
interface Edge {
  from: string;
  to: string;
  links: number;
  sources: number;
}
interface Hit {
  kind: string;
  id: string;
  depth: number;
  reopens: boolean;
}

const СЛОВО: Record<string, string> = {
  requirement: "требование",
  check: "проверка",
  story: "история",
  task: "задача",
  "red-task": "красная задача",
  milestone: "этап",
  question: "вопрос",
  screen: "экран",
  need: "потребность",
  decision: "решение",
  rationale: "рассуждение",
  "lint-rule": "правило кода",
  assertion: "утверждение",
};
const зовут = (k: string): string => СЛОВО[k] ?? k;

export function Depends({ projectId }: { projectId: string }): React.JSX.Element {
  const [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[] } | null>(null);
  const [kind, setKind] = useState<string>("");
  const [id, setId] = useState<string>("");
  const [hit, setHit] = useState<{ touched: number; reopens: number; items: Hit[] } | null>(null);
  const [ids, setIds] = useState<string[]>([]);

  useEffect(() => {
    if (!projectId) return;
    void tool<{ nodes: Node[]; edges: Edge[] }>(projectId, "links-graph").then(setGraph);
  }, [projectId]);

  // Имена берутся у самого набора, а не набираются руками: род уже выбран, и
  // предлагать несуществующее имя — предлагать ошибку.
  useEffect(() => {
    if (!projectId || !kind) return;
    setIds([]);
    void tool<{ ids?: string[] }>(projectId, `${kind}-list`)
      .then((d) => setIds(d.ids ?? []))
      .catch(() => setIds([]));
  }, [projectId, kind]);

  useEffect(() => {
    if (!projectId || !kind || !id) {
      setHit(null);
      return;
    }
    void tool<{ touched: number; reopens: number; items: Hit[] }>(
      projectId,
      "impact",
      { kind, id },
    ).then(setHit);
  }, [projectId, kind, id]);

  if (!graph) return <p className="empty">Читаю связи…</p>;

  const max = Math.max(1, ...graph.edges.map((e) => e.links));
  const стоящие = graph.edges.filter((e) => e.to === kind);
  const крупнейший = Math.max(1, ...graph.nodes.map((n) => n.count));

  return (
    <div className="depends">
      <section className="dep-map">
        <h2>Скелет</h2>
        <p className="dep-note">
          <b>А → Б</b> значит «А стоит на Б»: правка Б переоткрывает А. Толщина — сколько связей.
        </p>
        {/* Свёрнуто по источнику: прежде «этап» стоял в списке пять раз, и
            семнадцать строк читались перечнем, а не картой. Теперь у каждого
            рода одна строка, а на чём он стоит — веером вправо. */}
        <ul className="dep-edges">
          {Array.from(
            graph.edges.reduce((m, e) => {
              const l = m.get(e.from) ?? [];
              l.push(e);
              m.set(e.from, l);
              return m;
            }, new Map<string, Edge[]>()),
          )
            .sort((a, b) => b[1].reduce((n, e) => n + e.links, 0) - a[1].reduce((n, e) => n + e.links, 0))
            .map(([from, outs]) => (
              <li key={from} className="dep-row">
                <button
                  className={`dep-node dep-src${kind === from ? " on" : ""}`}
                  onClick={() => { setKind(from); setId(""); }}
                  type="button"
                >
                  {зовут(from)}
                </button>
                <span className="dep-fan">
                  {outs
                    .slice()
                    .sort((a, b) => b.links - a.links)
                    .map((e) => (
                      <button
                        key={e.to}
                        className={`dep-dst${kind === e.to ? " on" : ""}`}
                        onClick={() => { setKind(e.to); setId(""); }}
                        type="button"
                        title={`${e.links} связей от ${e.sources} записей`}
                      >
                        <i style={{ height: `${1 + Math.round((e.links / max) * 4)}px` }} />
                        <span>{зовут(e.to)}</span>
                        <b>{e.links}</b>
                      </button>
                    ))}
                </span>
              </li>
            ))}
        </ul>
      </section>

      <section className="dep-kinds">
        <h2>Роды</h2>
        {/* Пометка стоит там, где есть что сказать. Прежде «не переоткрывается»
            повторялось шесть раз подряд и читалось как тревога, хотя это
            обычное состояние: у проверки и рассуждения состояния нет вовсе. */}
        <ul className="dep-list">
          {graph.nodes.map((n) => (
            <li key={n.kind} className={n.reopened > 0 ? "hot" : ""}>
              <button
                className={`dep-node${kind === n.kind ? " on" : ""}`}
                onClick={() => { setKind(n.kind); setId(""); }}
                type="button"
              >
                {зовут(n.kind)}
              </button>
              <span className="dep-bar" aria-hidden="true">
                <i style={{ width: `${Math.max(2, Math.round((n.count / крупнейший) * 100))}%` }} />
              </span>
              <span className="dep-count">{n.count}</span>
              {n.reopened > 0 && <span className="dep-hot">переоткрыто {n.reopened}</span>}
            </li>
          ))}
        </ul>
        <p className="dep-note dep-legend">
          Переоткрываются при правке связей:{" "}
          {graph.nodes.filter((n) => n.reopens).map((n) => зовут(n.kind)).join(" · ") || "ни один"}
        </p>
      </section>

      <section className="dep-what">
        <h2>Что переоткроется</h2>
        {!kind && <p className="empty">Выберите род на карте.</p>}
        {kind && (
          <>
            <p className="dep-note">
              На <b>{зовут(kind)}</b> стоят:{" "}
              {стоящие.length
                ? стоящие.map((e) => зовут(e.from)).join(" · ")
                : "никто — правка не переоткроет ничего"}
            </p>
            <select
              className="dep-pick"
              value={id}
              onChange={(e) => setId(e.target.value)}
            >
              <option value="">— выберите запись —</option>
              {ids.slice(0, 500).map((x) => (
                <option key={x} value={x}>{x}</option>
              ))}
            </select>
          </>
        )}
        {hit && (
          <div className="dep-hit">
            <p className="dep-sum">
              затронуто <b>{hit.touched}</b> · из них переоткроется{" "}
              <b className={hit.reopens > 0 ? "bad" : ""}>{hit.reopens}</b>
            </p>
            <ul className="dep-hits">
              {hit.items.map((h) => (
                <li key={`${h.kind}-${h.id}`} className={h.reopens ? "will" : "wont"}>
                  <span className="dep-depth">шаг {h.depth}</span>
                  <span className="dep-kind">{зовут(h.kind)}</span>
                  <span className="dep-id">{h.id}</span>
                  <span className="dep-mark">{h.reopens ? "переоткроется" : "связано"}</span>
                </li>
              ))}
            </ul>
            {hit.touched === 0 && (
              <p className="empty">На этой записи не стоит никто: правка никого не затронет.</p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
