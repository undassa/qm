import type React from "react";
import { useEffect, useState } from "react";
import { tool, type NextStep, type Tile } from "./api";
import { Bar } from "./Bar";
import { headline, unknownTotal } from "./unknown-count";
import { Phases } from "./Phases";
import { Provenance } from "./Provenance";
import { Live } from "./Live";
import { useLive } from "./live";

/**
 * Где мы сейчас.
 *
 * Экран отвечает на четыре вопроса, и в этом порядке: где мы · что запустить ·
 * что мешает · чего мы не знаем. Последнее стоит на видном месте, а не в
 * подвале: это единственная плитка, которая должна расти в глазах, пока её не
 * закроют.
 */
interface Pipeline {
  total: number;
  pipeline: { status: string; title: string; reached: number | null; unknown: number; why?: string }[];
}

export function Where({ projectId }: { projectId: string }): React.JSX.Element {
  const [step, setStep] = useState<NextStep | null>(null);
  const [tiles, setTiles] = useState<Tile[] | null>(null);
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);
  const [failed, setFailed] = useState("");

  const live = useLive(
    () => Promise.all([
      tool<NextStep>(projectId, "next-step"),
      tool<{ tiles: Tile[] }>(projectId, "progress"),
      tool<Pipeline>(projectId, "pipeline"),
    ]).then(([s, p, pl]) => ({ step: s, tiles: p.tiles, pipeline: pl })),
    [projectId],
  );
  useEffect(() => {
    if (!live.data) return;
    setStep(live.data.step);
    setTiles(live.data.tiles);
    setPipeline(live.data.pipeline);
    setFailed(live.failed);
  }, [live.data, live.failed]);

  if (failed) return <p className="empty">Сервер не отвечает: <code>{failed}</code>.</p>;
  if (!step || !tiles || !pipeline) return <p className="empty">Смотрю, где мы…</p>;

  const at = step.at;
  // Статусы конвейера, факт которых никто не пишет, — то же незнание, что и в
  // плитках, и считается тем же счётом.
  const unknown = unknownTotal(tiles, pipeline.pipeline.filter((s) => s.reached === null).length);

  return (
    <>
      <div className="head">
        <div>
          <h1>Где мы</h1>
          <div className="prov">первая невыполненная ступень, и что она держит</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} /> процесс {step.process}
          {/* Положение СОХРАНЕНО, а не посчитано на этот заход. Пока работник
              считает заново, страница показывает прошлое — и говорит об этом,
              иначе агент примет устаревшее за нынешнее. */}
          {step.stale ? <b className="disagree"> · набор изменился, пересчёт идёт</b> : null}
        </span>
      </div>

      <Phases projectId={projectId} />

      {at ? (
        <div className="now">
          <span className="now-k">ступень</span>
          <span className="now-q">
            {at.ord} · {at.question}
          </span>
          <span className="now-k">запустить</span>
          <span className="now-run">
            {at.owner ? `${at.ownerKind === "agent" ? "субагент" : "скилл"} ${at.owner}` : "владельца нет: ступень закрывает человек"}
          </span>
          {at.why ? (
            <>
              <span className="now-k">почему</span>
              <span className="now-why">{at.why}</span>
            </>
          ) : null}
          {/* Что именно держит ступень. Без этого «здесь мы» называет номер и
              молчит о причине: идти чинить некуда. */}
          {at.detail?.length ? (
            <>
              <span className="now-k">держит</span>
              <span className="now-why">
                {at.detail.slice(0, 5).map((d) => (
                  <code key={d} className="held">{d}</code>
                ))}
                {(at.violations ?? 0) > at.detail.length
                  ? ` — и ещё ${(at.violations ?? 0) - at.detail.length}`
                  : ""}
              </span>
            </>
          ) : null}
          <span className="now-k">сторона</span>
          <span className="now-why">
            {at.touches === "corpus" ? "читает набор" : "пишет в репозиторий"}
            {step.corpusPhaseOpen && at.touches === "repository"
              ? " — и удержана: фаза набора ещё открыта"
              : null}
          </span>
        </div>
      ) : (
        <p className="note">Невыполненных ступеней нет.</p>
      )}

      <p className="note">
        Пройдено <b>{step.passed.length}</b> · пропущено с причиной <b>{step.skipped.length}</b> ·{" "}
        <b className="tile-u">не отвечается {step.unanswerable.length}</b>. Три списка, а не один: слипшись, они дают
        зелень, которая ничего не мерит.
      </p>

      <div className="tiles">
        <div className="tile">
          <div className="tile-t">чего мы не знаем</div>
          <div className="tile-n tile-u">{unknown}</div>
          <div className="tile-s">пунктов и статусов без способа проверки</div>
        </div>
        {tiles.map((t) => (
          <div className="tile" key={t.tile}>
            <div className="tile-t">{t.tile}</div>
            <div className={`tile-n${headline(t).dim ? " dim" : ""}`}>{headline(t).text}</div>
            <Bar done={t.done} open={t.open} unknown={t.unknown} />
            <div className="tile-s">
              {t.done} · {t.open}
              {t.unknown ? <span className="tile-u"> · {t.unknown} неизвестно</span> : null}
            </div>
          </div>
        ))}
      </div>

      <h3 className="rows-group">Конвейер задач — {pipeline.total}</h3>
      <div className="rungs">
        {pipeline.pipeline.map((s) => (
          <div className="rung" key={s.status}>
            <span className="rung-n">{s.reached === null ? "—" : s.reached}</span>
            <span className="rung-q">{s.status}</span>
            <span className="rung-o">{s.title}</span>
            <span className={`rung-s ${s.reached === null ? "unknown" : "passed"}`}>
              {s.reached === null ? "не записывается" : "считается"}
            </span>
          </div>
        ))}
      </div>

      <Provenance
        source="harness_process_step, kind_status и предметные таблицы"
        computed="состояние ступени и конвейер"
      />
    </>
  );
}
