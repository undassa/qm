import type React from "react";
import { tool } from "./api";
import { Live } from "./Live";
import { useLive } from "./live";

interface Documents { present: number; declared: number; absent: number }
interface Tasks { total: number; closed: number; open: number }
interface Phase { phase: string; title: string; documents?: Documents | null; gate?: string; gateState?: string; tasks?: Tasks | null }
interface Step { status: string; title?: string; reached?: number; unknown?: number }
interface Progress { tile: string; done: number; open: number; unknown: number; percent?: number; says?: string }

/**
 * Движение: куда проект дошёл и что его держит.
 *
 * Три числа в каждой плитке, никогда одно: **сделано · открыто · не
 * отвечается**. Полоска, сложившая «не отвечается» в любую сторону, врёт — и
 * врёт в успокаивающую сторону, поэтому третье число стоит отдельной колонкой,
 * а не растворено в процентах.
 */
export function Movement({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(
    () => Promise.all([
      tool<{ phases: Phase[] }>(projectId, "phases"),
      tool<{ tiles: Progress[] }>(projectId, "progress"),
      tool<{ pipeline: Step[] }>(projectId, "pipeline"),
    ]).then(([ph, pr, pi]) => ({ phases: ph.phases ?? [], tiles: pr.tiles ?? [], pipeline: pi.pipeline ?? [] })),
    [projectId],
  );

  if (!live.data) return <p className="empty">Считаю движение…</p>;
  const { phases, tiles, pipeline } = live.data;

  return (
    <>
      <Live at={live.at} again={live.again} />
      <header className="head">
        <h1>Движение</h1>
        <p className="note">
          Три числа в каждой плитке, и третье — «не отвечается». Сложить его в любую сторону
          значит соврать в успокаивающую.
        </p>
      </header>

      <h2>Плитки</h2>
      <table className="rows">
        <thead><tr><th>Что</th><th>Сделано</th><th>Открыто</th><th>Не отвечается</th><th>Из отвечаемого</th></tr></thead>
        <tbody>
          {tiles.map((t) => (
            <tr key={t.tile}>
              <td>{t.tile}</td>
              <td>{t.done}</td>
              <td>{t.open}</td>
              <td className={t.unknown > 0 ? "warn" : ""}>{t.unknown}</td>
              <td>{t.says ?? (t.done + t.open > 0 ? `${t.percent ?? 0} %` : "пока не измеряется")}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2>Фазы</h2>
      <table className="rows">
        <thead><tr><th>Фаза</th><th>Документов</th><th>Не хватает</th><th>Гейт</th><th>Состояние гейта</th><th>Задач</th><th>Закрыто</th></tr></thead>
        <tbody>
          {phases.map((p) => (
            <tr key={p.phase}>
              <td><code>{p.phase}</code> {p.title}</td>
              {/* `documents` и `tasks` приходят ОБЪЕКТАМИ — «есть · объявлено · не
                  хватает» и «всего · закрыто · открыто», — а страница рисовала их как
                  числа. React не умеет показать объект и падает целиком: раздел не
                  открывался вовсе. Пустой прочерк тут законен: у фазы генерации нет
                  объявленных документов, а у фаз без задач нет задач. */}
              <td>{p.documents ? `${p.documents.present} из ${p.documents.declared}` : "—"}</td>
              <td className={p.documents && p.documents.absent > 0 ? "warn" : ""}>
                {p.documents ? p.documents.absent : "—"}
              </td>
              <td>{p.gate ?? "—"}</td>
              <td className={p.gateState && p.gateState !== "passed" ? "warn" : ""}>{p.gateState ?? "—"}</td>
              <td>{p.tasks ? p.tasks.total : "—"}</td>
              <td className={p.tasks && p.tasks.open > 0 ? "warn" : ""}>
                {p.tasks ? `${p.tasks.closed} закрыто, ${p.tasks.open} открыто` : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2>Конвейер задач</h2>
      <table className="rows">
        <thead><tr><th>Статус</th><th>Дошло задач</th><th>Не отвечается</th><th>Что это значит</th></tr></thead>
        <tbody>
          {pipeline.map((s, i) => (
            <tr key={`${s.status}-${i}`}>
              <td>{s.status}</td>
              <td>{s.reached ?? 0}</td>
              <td className={(s.unknown ?? 0) > 0 ? "warn" : ""}>{s.unknown ?? 0}</td>
              <td>{s.title ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
