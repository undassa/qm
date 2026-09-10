import { html, nothing, type TemplateResult } from "lit";

export interface ProcessGate {
  phase: string;
  item: string;
  kind: string;
  state: string;
  violations: number;
  signedBy: string | null;
  signedAt: number | null;
  staleDocuments: string[];
}

/**
 * Конвейер харнеса из `00-frame/document-plan.md` §3:
 * Ф0 рамка ─G0─► Ф1 требования ─G1─► Ф2 проект ─G2─► Ф3 тесты ─G3─► Ф4 генерация ─G4─► Ф5 выпуск
 *
 * Гейт — не собрание, а проверяемый список: пока он не пройден, следующая фаза не начинается.
 */
export const PHASES: { id: string; name: string; gate: string | null }[] = [
  { id: "Ф0", name: "рамка", gate: "G0" },
  { id: "Ф1", name: "требования", gate: "G1" },
  { id: "Ф2", name: "проект", gate: "G2" },
  { id: "Ф3", name: "тесты", gate: "G3" },
  { id: "Ф4", name: "генерация", gate: "G4" },
  { id: "Ф5", name: "выпуск", gate: null },
];

export type GateVerdict = "passed" | "failed" | "reopened" | "open" | "empty";

/** Гейт пройден, только когда пройдены все его пункты; неподписанное — не «пройдено». */
export function verdictOf(items: readonly ProcessGate[]): GateVerdict {
  if (!items.length) return "empty";
  if (items.some((i) => i.state === "failed" || i.state === "refused")) return "failed";
  // Подпись под изменившимся документом гейт не держит: правка переоткрывает его.
  if (items.some((i) => i.staleDocuments.length)) return "reopened";
  return items.every((i) => i.state === "passed") ? "passed" : "open";
}

/** Первая фаза, чей гейт не пройден: дальше конвейер не идёт. */
export function blockingPhase(gates: readonly ProcessGate[]): string | null {
  for (const phase of PHASES) {
    if (!phase.gate) continue;
    const verdict = verdictOf(gates.filter((g) => g.phase === phase.gate));
    if (verdict !== "passed") return phase.id;
  }
  return null;
}

const VERDICT_LABEL: Record<GateVerdict, string> = {
  passed: "пройден",
  reopened: "переоткрыт правкой",
  failed: "не пройден",
  open: "ждёт подписи",
  empty: "не объявлен",
};

function gateTpl(gate: string, items: ProcessGate[], blocking: boolean, actions: ProcessActions): TemplateResult {
  const verdict = verdictOf(items);
  return html`
    <div class="process-gate ${verdict} ${blocking ? "is-blocking" : ""}">
      <div class="process-gate-head">
        <b>${gate}</b>
        <span>${VERDICT_LABEL[verdict]}</span>
        <button type="button" class="process-read" @click=${() => actions.onRead(gate)}>читать</button>
      </div>
      <ul>
        ${items.map(
          (item) => html`
            <li class="${item.state}">
              <span class="process-item-mark" aria-hidden="true">${item.state === "passed" ? "✓" : "•"}</span>
              <span class="process-item-text">
                ${item.item}${item.violations ? html` — ${item.violations}` : nothing}
                ${item.signedBy ? html`<span class="process-signed">подписал ${item.signedBy}</span>` : nothing}
              </span>
              ${
                item.kind === "signed"
                  ? html`<button
                      type="button"
                      class="process-sign"
                      ?disabled=${actions.busy}
                      @click=${() => actions.onSign(item.phase, item.item, item.state === "passed")}
                    >
                      ${item.state === "passed" ? "снять" : "подписать"}
                    </button>`
                  : nothing
              }
            </li>
          `,
        )}
      </ul>
    </div>
  `;
}

export interface ProcessActions {
  /** Открыть документы фазы: подписывают прочитанное, а не название пункта. */
  onRead: (phase: string) => void;
  /** Что делать, когда пункт гейта требует работы — например завести трек проверок. */
  onStartTests: () => void;
  /** Подписать или снять подпись: подписной пункт закрывает человек. */
  onSign: (phase: string, item: string, revoke: boolean) => void;
  busy: boolean;
  testTasks: number;
  devTasks: number;
}

export function processTpl(gates: readonly ProcessGate[], actions: ProcessActions): TemplateResult {
  const blocking = blockingPhase(gates);
  return html`
    <section class="process">
      <ol class="process-line" aria-label="Конвейер">
        ${PHASES.map((phase) => {
          const items = gates.filter((g) => g.phase === phase.gate);
          const verdict = phase.gate ? verdictOf(items) : "empty";
          const reached = blocking === null || PHASES.findIndex((p) => p.id === blocking) >= PHASES.indexOf(phase);
          return html`
            <li
              class="process-phase ${verdict} ${phase.id === blocking ? "is-blocking" : ""} ${reached ? "" : "ahead"}"
            >
              <span class="process-phase-id">${phase.id}</span>
              <span class="process-phase-name">${phase.name}</span>
              ${phase.gate ? html`<span class="process-phase-gate">${phase.gate}</span>` : nothing}
            </li>
          `;
        })}
      </ol>

      ${
        blocking
          ? html`<p class="process-note">
              Конвейер стоит на <b>${blocking}</b>. Пока гейт не пройден, следующая фаза не начинается — это не
              соглашение, а правило харнеса.
            </p>`
          : html`<p class="process-note">Все гейты пройдены.</p>`
      }

      <div class="process-gates">
        ${PHASES.filter((p) => p.gate).map((phase) =>
          gateTpl(
            phase.gate!,
            gates.filter((g) => g.phase === phase.gate),
            phase.id === blocking,
            actions,
          ),
        )}
      </div>

      ${
        actions.testTasks === 0
          ? html`
              <div class="process-action">
                <div>
                  <b>Трека проверок нет.</b>
                  <p>
                    На ${actions.devTasks} задач разработки заведено ${actions.testTasks} задач проверок. Пока трек
                    пуст, гейт <b>G3</b> не пройти, а по правилу весь трек проверок предшествует всему коду — значит
                    стоит и генерация.
                  </p>
                </div>
                <button type="button" class="btn primary" @click=${actions.onStartTests}>
                  Запустить написание тестов
                </button>
              </div>
            `
          : nothing
      }
    </section>
  `;
}
