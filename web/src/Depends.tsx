import type React from "react";
import { useEffect, useState } from "react";
import { tool } from "./api";
import { type Lang, domainName, kindName, say } from "./say";

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
  domain?: string;
}
interface DomEdge { from: string; to: string; links: number; pairs: number }
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

/** Род зовётся так, как его зовёт общий словарь: второго перечня не нужно. */
const зовут = (l: Lang, k: string): string => kindName(l, k);


export function Depends({ projectId, lang }: { projectId: string; lang: Lang }): React.JSX.Element {
  const [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[]; domains?: DomEdge[] } | null>(null);
  /** Карта областей или карта родов. Области крупнее и отвечают «как устроено». */
  const [fold, setFold] = useState(true);
  const [kind, setKind] = useState<string>("");
  const [id, setId] = useState<string>("");
  const [hit, setHit] = useState<{ touched: number; reopens: number; items: Hit[] } | null>(null);
  const [ids, setIds] = useState<string[]>([]);

  useEffect(() => {
    if (!projectId) return;
    void tool<{ nodes: Node[]; edges: Edge[]; domains?: DomEdge[] }>(projectId, "links-graph").then(setGraph);
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

  if (!graph) return <p className="empty">{say(lang, "dp.reading")}</p>;

  const max = Math.max(1, ...graph.edges.map((e) => e.links));
  const домМакс = Math.max(1, ...(graph.domains ?? []).map((e) => e.links));
  const стоящие = graph.edges.filter((e) => e.to === kind);
  const крупнейший = Math.max(1, ...graph.nodes.map((n) => n.count));

  return (
    <div className="depends">
      <section className="dep-map">
        <div className="dep-h">
          <h2>{say(lang, "de.skeleton")}</h2>
          {/* Две крупности одной карты: области отвечают «как устроен проект»,
              роды — «что на чём стоит». Семнадцать рёбер против восьми. */}
          <span className="dep-fold">
            <button type="button" className={fold ? "on" : ""} onClick={() => setFold(true)}>
              {say(lang, "de.byDomain")}
            </button>
            <button type="button" className={!fold ? "on" : ""} onClick={() => setFold(false)}>
              {say(lang, "de.byKind")}
            </button>
          </span>
        </div>
        <p className="dep-note">
          <b>А → Б</b> {say(lang, "de.rule")}
        </p>
        {/* Свёрнуто по источнику: прежде «этап» стоял в списке пять раз, и
            семнадцать строк читались перечнем, а не картой. Теперь у каждого
            рода одна строка, а на чём он стоит — веером вправо. */}
        {fold ? (
          <ul className="dep-edges">
            {(graph.domains ?? [])
              .slice()
              .sort((a, b) => b.links - a.links)
              .map((e) => (
                <li key={`${e.from}-${e.to}`} className="dep-row dom">
                  <span className="dep-node dep-src">{domainName(lang, e.from) || "—"}</span>
                  <span className="dep-fan">
                    <span className="dep-dst" title={`${e.pairs}`}>
                      <i style={{ height: `${1 + Math.round((e.links / домМакс) * 4)}px` }} />
                      <span>{domainName(lang, e.to) || "—"}</span>
                      <b>{e.links}</b>
                    </span>
                  </span>
                </li>
              ))}
          </ul>
        ) : (
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
                  {зовут(lang, from)}
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
                        <span>{зовут(lang, e.to)}</span>
                        <b>{e.links}</b>
                      </button>
                    ))}
                </span>
              </li>
            ))}
        </ul>
        )}
      </section>

      <section className="dep-kinds">
        <h2>{say(lang, "dp.kinds")}</h2>
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
                {зовут(lang, n.kind)}
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
          {graph.nodes.filter((n) => n.reopens).map((n) => зовут(lang, n.kind)).join(" · ") || "ни один"}
        </p>
      </section>

      <section className="dep-what">
        <h2>{say(lang, "dp.willReopen")}</h2>
        {!kind && <p className="empty">{say(lang, "dp.pickKind")}</p>}
        {kind && (
          <>
            <p className="dep-note">{say(lang, "dp.on")}<b>{зовут(lang, kind)}</b> стоят:{" "}
              {стоящие.length
                ? стоящие.map((e) => зовут(lang, e.from)).join(" · ")
                : "никто — правка не переоткроет ничего"}
            </p>
            <select
              className="dep-pick"
              value={id}
              onChange={(e) => setId(e.target.value)}
            >
              <option value="">{say(lang, "dp.pickRecord")}</option>
              {ids.slice(0, 500).map((x) => (
                <option key={x} value={x}>{x}</option>
              ))}
            </select>
          </>
        )}
        {hit && (
          <div className="dep-hit">
            <p className="dep-sum">{say(lang, "dp.touched")}<b>{hit.touched}</b> · из них переоткроется{" "}
              <b className={hit.reopens > 0 ? "bad" : ""}>{hit.reopens}</b>
            </p>
            <ul className="dep-hits">
              {hit.items.map((h) => (
                <li key={`${h.kind}-${h.id}`} className={h.reopens ? "will" : "wont"}>
                  <span className="dep-depth">шаг {h.depth}</span>
                  <span className="dep-kind">{зовут(lang, h.kind)}</span>
                  <span className="dep-id">{h.id}</span>
                  <span className="dep-mark">{h.reopens ? "переоткроется" : "связано"}</span>
                </li>
              ))}
            </ul>
            {hit.touched === 0 && (
              <p className="empty">{say(lang, "dp.nobody")}</p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
