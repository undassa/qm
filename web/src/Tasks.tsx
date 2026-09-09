import type React from "react";
import { useEffect, useState } from "react";
import { tool } from "./api";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";
import { titleOf } from "./unknown-count";

/**
 * Задачи — доской волн.
 *
 * Волна отвечает на вопрос «что можно вести одновременно»: первая ничего не
 * ждёт, вторая ждёт только первую. Считает её сервер, а не эта страница: вывод
 * не пересчитывается дважды разными руками.
 *
 * Два этапа разделены **барьером**: весь трек проверок предшествует всему коду.
 * Это не украшение полосы, а правило, и оно написано словами поперёк доски.
 */
interface Card {
  id: string;
  milestone: string;
  title: string;
  kind: string;
  state: string;
  checks: number;
  requirements: number;
  preflight: string | null;
  wave: number;
  waits: number;
}
interface Next {
  task: { id: string; title: string } | null;
  why?: string;
  candidate?: string;
}

const PRE: Record<string, { mark: string; word: string }> = {
  ready: { mark: "✓", word: "предполёт: готова" },
  "ready-with-risks": { mark: "!", word: "предполёт: риски названы" },
  blocked: { mark: "×", word: "предполёт: заблокирована" },
};

export function Tasks({ projectId }: { projectId: string }): React.JSX.Element {
  const [cards, setCards] = useState<Card[] | null>(null);
  const [openRed, setOpenRed] = useState(0);
  const [next, setNext] = useState<Next | null>(null);
  const [queue, setQueue] = useState<{ count: number } | null>(null);

  // Доска волн живая: подача датчика меняет состояние задачи в течение волны,
  // и «что идёт прямо сейчас» ради этого и смотрят.
  const live = useLive(
    () => Promise.all([
      tool<{ cards: Card[]; openRed: number }>(projectId, "waves"),
      tool<Next>(projectId, "next-task"),
      tool<{ count: number }>(projectId, "preflight-queue"),
    ]).then(([w, n, q]) => ({ cards: w.cards, openRed: w.openRed, next: n, queue: q })),
    [projectId],
  );
  useEffect(() => {
    if (!live.data) return;
    setCards(live.data.cards);
    setOpenRed(live.data.openRed);
    setNext(live.data.next);
    setQueue(live.data.queue);
  }, [live.data]);

  if (!cards) return <p className="empty">Считаю волны…</p>;

  const red = cards.filter((c) => c.kind === "red");
  const dev = cards.filter((c) => c.kind !== "red");
  const closed = cards.filter((c) => c.state === "closed").length;

  // Проверки рёбер между собой не имеют — их ось это этап; у кода ось волна.
  const redColumns = group(red, (c) => c.milestone);
  // По волне, а не по паре «этап × волна»: дробление на 67 столбцов рассыпает
  // доску, а этап и так написан на каждой карточке.
  const devColumns = group(
    [...dev].sort((a, b) => a.wave - b.wave),
    (c) => `волна ${c.wave}`,
  );
  const milestones = group(cards, (c) => c.milestone).sort((a, b) => a[0].localeCompare(b[0]));

  const Card = ({ c, first }: { c: Card; first: boolean }): React.JSX.Element => {
    const done = c.state === "closed";
    const pre = c.preflight ? PRE[c.preflight] : undefined;
    // Ждущая задача — серая, а не тревожная: тревога значит «что-то не так», а
    // с ней всё так, она просто ждёт. Тревожный цвет остаётся заблокированному
    // предполёту, «брать сейчас» — выбранной. Так и у макета.
    const state = done ? "passed" : c.preflight === "blocked" ? "unknown" : first ? "at" : "wait";
    return (
      <li className={`card s-${state}${done ? " done" : ""}`}>
        <div className="id">
          <span>{c.id}</span>
          {pre ? (
            <b className="pre" title={pre.word}>
              {pre.mark}
            </b>
          ) : null}
        </div>
        <div className="t">{titleOf(c.id, c.title)}</div>
        <div className="bar">
          <i style={{ width: done ? "100%" : "0%" }} />
        </div>
        <div className="n">
          {c.kind === "red"
            ? `${c.checks} проверок · ${c.milestone}`
            : c.requirements
              ? `${c.requirements} требований · ${c.milestone}`
              : `собственных требований нет · ${c.milestone}`}
          {c.waits ? ` · ждёт ${c.waits}` : ""}
        </div>
      </li>
    );
  };

  const Board = ({ columns, code }: { columns: [string, Card[]][]; code: boolean }): React.JSX.Element => (
    <div className="waves">
      {columns.map(([name, items]) => {
        const done = items.filter((c) => c.state === "closed").length;
        const первая = !code && done === items.length ? false : items.every((c) => !c.waits);
        return (
          <section className={`col${первая && done < items.length ? " first" : ""}${code ? " repo" : ""}`} key={name}>
            <h2>
              {name}{" "}
              <em>
                {done}/{items.length}
              </em>
            </h2>
            <ul className="stack">
              {items.map((c) => (
                <Card c={c} first={next?.task?.id === c.id} key={c.id} />
              ))}
            </ul>
          </section>
        );
      })}
    </div>
  );

  return (
    <>
      <div className="head">
        <div>
          <h1>Задачи</h1>
          <div className="prov">что можно вести одновременно, каждое в своём дереве</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          проверки <b>{red.length}</b> · код <b>{dev.length}</b> · закрыто <b>{closed}</b>
        </span>
      </div>

      <div className="chips">
        {milestones.map(([m, items]) => {
          const r = items.filter((c) => c.kind === "red");
          const d = items.filter((c) => c.kind !== "red");
          const rDone = r.filter((c) => c.state === "closed").length;
          const dDone = d.filter((c) => c.state === "closed").length;
          const all = rDone + dDone === items.length;
          return (
            <span className={`chip${all ? " full" : rDone + dDone ? "" : " none"}`} key={m}>
              <b>{m}</b>
              <span>
                {r.length ? `проверки ${rDone}/${r.length}` : "проверок нет"} · код {dDone}/{d.length}
              </span>
            </span>
          );
        })}
      </div>

      {next?.task ? (
        <div className="now">
          <span className="now-k">брать</span>
          <span className="now-q">
            {next.task.id} · {titleOf(next.task.id, next.task.title)}
          </span>
          <span className="now-k">предполёт</span>
          <span className="now-why">
            {queue ? `ждут ${queue.count} задач — вердикта на текущей ревизии у них нет` : "…"}
          </span>
        </div>
      ) : (
        <p className="note">
          <b>Задача не выдана.</b> {next?.why} {next?.candidate ? `Кандидат — ${next.candidate}.` : null}
        </p>
      )}

      <div className="legend">
        <span>
          <i style={{ background: "var(--ready)" }} />
          брать сейчас — зависимости закрыты
        </span>
        <span>
          <i style={{ background: "var(--done)" }} />
          закрыта трейлером
        </span>
        <span>
          <i style={{ background: "var(--wait)" }} />
          ждёт задач из волн левее
        </span>
        <span>
          <i style={{ background: "var(--code)" }} />
          код: ждёт всего трека проверок
        </span>
      </div>

      <p className="stage">
        этап 1 · проверки — {red.length} задач в {redColumns.length} этапах
      </p>
      <Board columns={redColumns} code={false} />

      <div className="barrier">
        <span className="edge l" />
        <span className="txt">
          <b>Весь трек проверок предшествует всему коду.</b> Тест — исполнимая форма требования, и пока открыта
          хоть одна проверка, ни одна задача кода не берётся, сколько бы её собственные зависимости ни были
          закрыты. Сегодня открыто <b>{openRed}</b> из {red.length}.
        </span>
        <span className="edge" />
      </div>

      <p className="stage">
        этап 2 · код — {dev.length} задач, волн {Math.max(...dev.map((c) => c.wave), 0)}, вех{" "}
        {group(dev, (c) => c.milestone).length}
      </p>
      <Board columns={devColumns} code />

      <Provenance
        source="project_plan_tasks, project_plan_task_deps и task_milestone_dep"
        computed="волны, барьер и то, что брать"
      />
    </>
  );
}

/** Столбцы в порядке появления: порядок задач уже задан планом. */
function group(cards: Card[], key: (c: Card) => string): [string, Card[]][] {
  const map = new Map<string, Card[]>();
  for (const c of cards) map.set(key(c), [...(map.get(key(c)) ?? []), c]);
  return [...map.entries()];
}
