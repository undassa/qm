import type React from "react";
import { tool } from "./api";
import { useEffect, useState } from "react";

/**
 * Цепочка фаз — так, как её объявляет сам набор:
 * `Ф0 рамка ─G0─► Ф1 требования ─G1─► Ф2 проект ─G2─► Ф3 тесты ─G3─►
 * Ф4 генерация ─G4─► Ф5 выпуск`.
 *
 * У фазы три части, и **ни одна не складывается с другой**: документы, гейт и
 * задачи считаются порознь и порознь показываются. До этого в картине были
 * только вехи кода — трек проверок, на котором держится вся Ф3, не участвовал
 * вовсе.
 */
export interface Phase {
  phase: string;
  title: string;
  gate: string | null;
  gateState: string | null;
  documents: { present: number; absent: number; declared: number } | null;
  tasks: { closed: number; open: number; total: number } | null;
}

export function Phases({ projectId }: { projectId: string }): React.JSX.Element {
  const [phases, setPhases] = useState<Phase[] | null>(null);

  useEffect(() => {
    if (!projectId) return;
    void tool<{ phases: Phase[] }>(projectId, "phases").then((d) => setPhases(d.phases));
  }, [projectId]);

  if (!phases) return <p className="empty">Читаю фазы…</p>;

  return (
    <div className="phases">
      {phases.map((p, i) => {
        const done = p.tasks ? p.tasks.closed === p.tasks.total && p.tasks.total > 0 : false;
        const state = p.gateState === "passed" ? "passed" : p.gateState === "failed" ? "unknown" : "wait";
        return (
          <div className="" key={p.phase}>
            <div className={`phase s-${done ? "passed" : state}`}>
              <div className="phase-h">
                <b>{p.phase}</b> {p.title}
              </div>
              {p.documents ? (
                <div className="phase-l">
                  {/* Знаменатель — сколько документов ДОЛЖНО быть, а не сколько
                      строк в описи. Объявленное отсутствующим — решение с
                      причиной, и держать его в знаменателе значит вечно
                      показывать недописанным то, что дописывать не собирались. */}
                  документы <b>{p.documents.present}</b>/{p.documents.declared - p.documents.absent}
                  {p.documents.absent ? <span> · {p.documents.absent} объявлено нет</span> : null}
                </div>
              ) : null}
              {p.tasks ? (
                <div className="phase-l">
                  задачи <b>{p.tasks.closed}</b>/{p.tasks.total}
                </div>
              ) : null}
              {!p.documents && !p.tasks ? <div className="phase-l">своего счёта нет</div> : null}
            </div>
            {p.gate ? (
              <div className={`gatepin g-${state}`} title={`гейт ${p.gate}: ${p.gateState ?? "—"}`}>
                {p.gate}
              </div>
            ) : i < phases.length - 1 ? (
              <div className="gatepin g-none" title="гейта нет">
                —
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
