import type React from "react";
import { useState } from "react";
import { tool } from "./api";
import { Provenance } from "./Provenance";
import { Drawer } from "./Drawer";
import { Live } from "./Live";
import { useLive } from "./live";
import type { Phase } from "./Phases";

/**
 * Гейты — карточками: имя, способ и знак. Что пункт означает и кого он нашёл —
 * в дровере по щелчку: на доске это шум, а под рукой — нужно.
 *
 * Гейт отвечает на один вопрос: **можно ли идти на следующий этап**. Ответ
 * вычисляется целиком; подписей нет — за содержание отвечает автор документа, и
 * он назван у каждого.
 *
 * Ничего здесь не считается при открытии. Страница показывает СОХРАНЁННЫЙ замер
 * и время, когда он сделан; меряет сервер сам — при изменении набора. Оттого на
 * доске нет второго мнения: запись и есть замер, спорить не с чем.
 */
interface Item {
  item: string;
  kind: string;
  computed: string;
  violations?: number;
  detail?: string[];
  why?: string;
  means?: string;
  excepted?: number;
  exceptedNames?: string[];
  staleExceptions?: number;
}
interface Gate {
  gate: string;
  title?: string;
  computed: string;
  items: Item[];
  failedItems: number;
  openItems: number;
  /** Когда пункты этого гейта мерили в последний раз. Пусто — не мерили ни разу. */
  checkedAt?: number | null;
  /** Что о гейте говорит сам набор — и сходится ли это с замером. */
}

/** Слово, знак и тон — один словарь на карточку, чип и дровер. */
const WORD: Record<string, string> = {
  passed: "прошёл",
  failed: "провален",
  unknown: "мерить нечем",
  open: "открыт",
  held: "удержан",
};
const MARK: Record<string, string> = { passed: "✓", failed: "✗", unknown: "?", open: "◌", held: "▪" };
const TONE: Record<string, string> = { passed: "passed", failed: "unknown", unknown: "wait", open: "wait" };

/** Когда мерили — словами, а не отметкой времени: «час назад» читается сразу. */
function ago(at?: number | null): string {
  if (!at) return "ещё не мерили";
  const sec = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (sec < 60) return "измерено только что";
  if (sec < 3600) return `измерено ${Math.round(sec / 60)} мин назад`;
  if (sec < 86400) return `измерено ${Math.round(sec / 3600)} ч назад`;
  return `измерено ${Math.round(sec / 86400)} дн назад`;
}

export function Gates({ projectId }: { projectId: string }): React.JSX.Element {
  const live = useLive(
    () => Promise.all([
      tool<{ gates: Gate[] }>(projectId, "gate"),
      tool<{ phases: Phase[] }>(projectId, "phases"),
    ]).then(([g, p]) => ({ gates: g.gates, phases: p.phases })),
    [projectId],
  );
  const [open, setOpen] = useState<{ gate: Gate; item: Item } | null>(null);
  const gates = live.data?.gates ?? null;
  const phases = live.data?.phases ?? [];

  if (!gates) return <p className="empty">Считаю гейты…</p>;

  const items = gates.flatMap((g) => g.items);
  // Время замера — САМОЕ СТАРОЕ из гейтов, и не «последнее обновление». Показать
  // самое свежее значило бы прикрыть непосчитанный гейт посчитанным.
  const measured = gates.reduce<number | null>((old, g) => {
    if (!g.checkedAt) return old === undefined ? null : old;
    return old == null || g.checkedAt < old ? g.checkedAt : old;
  }, null);
  const passedAll = items.filter((i) => i.computed === "passed").length;
  const closes = (id: string): string => {
    const p = phases.find((x) => x.gate === id);
    return p ? `${p.phase} ${p.title}` : "";
  };

  return (
    <>
      <div className="head">
        <div>
          <h1>Гейты</h1>
          <div className="prov">можно ли идти дальше — вычисляется целиком, подписей нет</div>
        </div>
        <span className="prov">
          <Live at={live.at} again={live.again} />{" "}
          <b>{passedAll}</b> из <b>{items.length}</b> пунктов прошли
        </span>
      </div>

      <div className="chips">
        {gates.map((g) => {
          const passed = g.items.filter((i) => i.computed === "passed").length;
          return (
            <span className={`chip${passed === g.items.length ? " full" : g.failedItems ? " warn" : ""}`} key={g.gate}>
              <b>{MARK[g.computed] ?? "·"} {g.gate}</b>
              <span>
                {passed}/{g.items.length}
                {closes(g.gate) ? ` · ${closes(g.gate)}` : ""}
              </span>
            </span>
          );
        })}
      </div>

      <div className="legend">
        <span><i style={{ background: "var(--done)" }} />✓ прошёл — при последнем замере ничего не нашлось</span>
        <span><i style={{ background: "var(--warn)" }} />✗ провален — нашлось то, что он ищет</span>
        <span><i style={{ background: "var(--wait)" }} />? мерить нечем — способа не объявлено</span>
      </div>

      <div className="waves">
        {gates.map((g) => {
          const passed = g.items.filter((i) => i.computed === "passed").length;
          return (
            <section className={`col${g.failedItems ? " first" : ""}`} key={g.gate}>
              <h2>
                {MARK[g.computed] ?? "·"} {g.gate} <em>{WORD[g.computed] ?? g.computed}</em>
              </h2>
              <p className="says">
                {g.title ?? closes(g.gate) ?? ""}
                <br />
                <b>{passed} из {g.items.length}</b> пунктов прошли
              </p>
              <ul className="stack">
                {g.items.map((i) => (
                    <li
                      className={`card s-${TONE[i.computed] ?? "wait"}`}
                      key={i.item}
                      onClick={() => setOpen({ gate: g, item: i })}
                      title="открыть подробности"
                      style={{ cursor: "pointer" }}
                    >
                      <div className="id">
                        {/* Способ называют только когда он не обычный: запросом
                            меряют 38 пунктов из 42, и слово «запрос» на каждой
                            карточке — краска без сообщения. Подпись — редкость,
                            и её назвать стоит. */}
                        <span>{i.kind === "query" ? "" : i.kind === "signed" ? "подпись" : i.kind}</span>
                        <b className="pre" title={WORD[i.computed] ?? i.computed}>{MARK[i.computed] ?? "·"}</b>
                      </div>
                      {/* На карточке — имя, знак и исход. Что пункт означает,
                          лежит в дровере: доску читают глазами по знакам, а
                          объяснение нужно тогда, когда знак не устроил. */}
                      <div className="t" style={{ WebkitLineClamp: 3 }}>{i.item}</div>
                      <div className="n">
                        {WORD[i.computed] ?? i.computed}
                        {i.violations ? ` · нарушений ${i.violations}` : ""}
                        {i.excepted ? ` · прощено ${i.excepted}` : ""}
                      </div>
                    </li>
                ))}
              </ul>
            </section>
          );
        })}
      </div>

      {open ? (
        <Drawer
          title={open.item.item}
          subtitle={`${open.gate.gate} · ${WORD[open.item.computed] ?? open.item.computed}`}
          onClose={() => setOpen(null)}
        >
          <ItemDetail gate={open.gate} item={open.item} />
        </Drawer>
      ) : null}

      <div className="barrier">
        <span className="edge l" />
        <span className="txt">
          <b>Считается при изменении набора.</b> Правка документа, пересборка проекций или поданный
          факт — и сервер сам перемеряет все пункты, складывая результат рядом с ними. Страница его
          только показывает вместе со временем замера: {ago(measured)}.
        </span>
        <span className="edge" />
      </div>

      <Provenance source="project_gates" computed="замер сохранён при последнем изменении набора" />
    </>
  );
}

/**
 * Подробности пункта: чем меряют, что вышло, и кого именно нашли.
 *
 * Список нарушителей обрезан пятью — столько отдаёт сервер. Число рядом
 * названо целиком, чтобы обрезанный список не читался как весь.
 */
function ItemDetail({ gate, item }: { gate: Gate; item: Item }): React.JSX.Element {
  const total = item.violations ?? 0;
  const shown = item.detail ?? [];
  return (
    <>
      <p className="note">
        <b className="pre">{MARK[item.computed] ?? "·"}</b>{" "}
        {item.computed === "passed"
          ? "Проверка прошла"
          : item.computed === "failed"
            ? `Проверка не прошла — нашлось ${item.violations ?? 0}`
            : "Проверить нечем"}
      </p>

      {item.means ? (
        <>
          <h3>Что это значит</h3>
          <p>{item.means}</p>
        </>
      ) : null}

      {item.why ? (
        <>
          <h3>Почему не измерено</h3>
          <p className="warn">{item.why}</p>
        </>
      ) : null}

      {/* Чек-лист ЭТОГО правила: по строке на каждое найденное. Не список
          соседних правил — их видно на доске, а сюда приходят чинить вот это.
          Прощённые исключением стоят тут же и помечены: вычесть их молча
          значило бы показать дыру с разрешением как чистое место. */}
      <h3>Что нашла проверка</h3>
      {item.computed === "unknown" ? (
        <p className="warn">
          Проверять нечем — способа не объявлено. Это «не знаем», а не «всё хорошо».
        </p>
      ) : total === 0 && !item.excepted ? (
        <p>Ни одного нарушения. Проверено при последнем замере, {ago(gate.checkedAt)}.</p>
      ) : (
        <>
          <ul className="checklist">
            {shown.map((d, n) => (
              <li className="unknown" key={`v-${d}-${n}`}>
                <b className="pre">✗</b>
                <span><code>{d}</code></span>
                <em>чинить</em>
              </li>
            ))}
            {(item.exceptedNames ?? []).map((d, n) => (
              <li className="wait" key={`e-${d}-${n}`}>
                <b className="pre">○</b>
                <span><code>{d}</code></span>
                <em>прощено</em>
              </li>
            ))}
          </ul>
          {shown.length < total ? (
            <p className="warn">
              Показаны первые {shown.length} из {total}: остальные того же рода.
            </p>
          ) : null}
          {item.staleExceptions ? (
            <p className="warn">
              Исключений на то, чего уже нет: {item.staleExceptions}. Их пора снять.
            </p>
          ) : null}
        </>
      )}

    </>
  );
}
