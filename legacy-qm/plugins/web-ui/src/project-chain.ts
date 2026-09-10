import { html, nothing, type TemplateResult } from "lit";

/**
 * Хребет проекта. Работа не в описи сущностей, а в том, где цепочка
 * «потребность → история → требование → проверка → задача → прогон» рвётся.
 * Разрыв показан на своём стыке: видно и что сломано, и что дальше цело.
 */
export interface ChainLink {
  key: string;
  title: string;
  count: number;
  note: string;
}

export interface ChainJoint {
  from: string;
  to: string;
  /** Есть ли в данных связь, по которой стык вообще можно проверить. */
  checked: boolean;
  broken: number;
  what: string;
}

export interface ProjectFinding {
  kind: string;
  item: string;
  count: number;
  /** Строки, о которых находка: по ним список сужается до неё. */
  detail?: unknown[];
}

/** Зелёным помечается только проверенное: непроверяемый стык зелёным быть не может. */
export function jointState(joint: ChainJoint): "broken" | "clean" | "unchecked" {
  if (!joint.checked) return "unchecked";
  return joint.broken > 0 ? "broken" : "clean";
}

/**
 * Находки идут по весу: сперва противоречия документа документу, потом заявленное
 * и забытое. Внутри группы — по числу, потому что большее число дороже разбирать.
 */
const SOFT_KINDS = new Set(["needs", "terms", "plan", "requirements"]);
const CODE_KINDS = new Set(["dbTables"]);

export function findingTone(kind: string): "hard" | "soft" | "code" {
  if (CODE_KINDS.has(kind)) return "code";
  return SOFT_KINDS.has(kind) ? "soft" : "hard";
}

export function rankFindings(findings: readonly ProjectFinding[]): ProjectFinding[] {
  const weight = (f: ProjectFinding) => (findingTone(f.kind) === "soft" ? 1 : 0);
  return [...findings].sort((a, b) => weight(a) - weight(b) || b.count - a.count);
}

function linkTpl(link: ChainLink, clean: boolean): TemplateResult {
  return html`
    <div class="chain-link ${clean ? "is-clean" : ""}">
      <span class="chain-link-k">${link.title}</span>
      <span class="chain-link-n">${link.count}</span>
      <span class="chain-link-s">${link.note}</span>
    </div>
  `;
}

const JOINT_LABEL: Record<ReturnType<typeof jointState>, string> = {
  broken: "рвётся",
  clean: "сходится",
  unchecked: "не проверяется",
};

function jointTpl(joint: ChainJoint): TemplateResult {
  const state = jointState(joint);
  const label = state === "broken" ? `${joint.broken} ${JOINT_LABEL.broken}` : JOINT_LABEL[state];
  return html`
    <div class="chain-joint is-${state}">
      <span class="chain-wire"></span>
      <span class="chain-mark">${label}</span>
      <span class="chain-what">${joint.what}</span>
    </div>
  `;
}

export function chainTpl(links: readonly ChainLink[], joints: readonly ChainJoint[]): TemplateResult {
  const brokenAt = new Set(joints.filter((j) => jointState(j) === "broken").map((j) => j.to));
  return html`
    <div class="chain">
      ${links.map((link, i) => {
        const joint = joints[i - 1];
        const clean = !brokenAt.has(link.key) && (!joint || jointState(joint) === "clean");
        return html`${joint ? jointTpl(joint) : nothing}${linkTpl(link, clean)}`;
      })}
    </div>
  `;
}

/**
 * Разговор всегда о чём-то. Просьба формулируется за человека и называет находку
 * точно — иначе в пустом чате придётся заново объяснять, о чём речь.
 */
export function discussionPrompt(finding: ProjectFinding): string {
  return [
    `Разбери находку каталога: «${finding.item}» — ${finding.count} записей, набор «${finding.kind}».`,
    "Покажи, из чего она состоит, найди общую причину и предложи, что с этим делать.",
  ].join(" ");
}

export function findingsTpl(
  findings: readonly ProjectFinding[],
  open: (finding: ProjectFinding) => void,
  discuss?: (finding: ProjectFinding) => void,
): TemplateResult {
  const ranked = rankFindings(findings);
  if (!ranked.length) {
    return html`<p class="chain-empty">Разрывов не найдено — все проверки сходятся.</p>`;
  }
  return html`
    <div class="finds">
      ${ranked.map(
        (f) => html`
          <div class="find is-${findingTone(f.kind)}">
            <span class="find-n">${f.count}</span>
            <span class="find-body">
              <span class="find-t">${f.item}</span>
              <span class="find-m">${f.kind}</span>
            </span>
            <span class="find-actions">
              ${
                discuss
                  ? html`<button type="button" class="find-go" @click=${() => discuss(f)}>обсудить</button>`
                  : nothing
              }
              <button type="button" class="find-go is-open" @click=${() => open(f)}>открыть →</button>
            </span>
          </div>
        `,
      )}
    </div>
  `;
}
