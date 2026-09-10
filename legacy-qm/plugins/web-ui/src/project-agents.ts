import { html, nothing, type TemplateResult } from "lit";
import { Bot, Pencil, Plus, Trash2 } from "lucide";
import { api } from "./core-bridge";
import { errMessage } from "../../chassis/src/errors";
import { fieldSelect, icon, relTime } from "./ui";

export interface ProjectAgent {
  id: string;
  name: string;
  role: string;
  harness: string;
  model: string | null;
  sandbox: string | null;
  concurrency: number;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

interface Draft {
  id: string | null;
  name: string;
  role: string;
  harness: string;
  model: string;
  sandbox: string;
  concurrency: number;
  enabled: boolean;
}

const state = {
  projectId: "" as string,
  agents: [] as ProjectAgent[],
  harnesses: [] as string[],
  sandboxes: [] as string[],
  draft: null as Draft | null,
  notice: "" as string,
  loading: false,
  saving: false,
};

let redraw: () => void = () => undefined;

export function resetAgents(): void {
  state.agents = [];
  state.draft = null;
  state.notice = "";
}

function blankDraft(): Draft {
  return {
    id: null,
    name: "",
    role: "",
    harness: state.harnesses[0] ?? "claude",
    model: "",
    sandbox: "",
    concurrency: 1,
    enabled: true,
  };
}

function draftOf(agent: ProjectAgent): Draft {
  return {
    id: agent.id,
    name: agent.name,
    role: agent.role,
    harness: agent.harness,
    model: agent.model ?? "",
    sandbox: agent.sandbox ?? "",
    concurrency: agent.concurrency,
    enabled: agent.enabled,
  };
}

export async function loadAgents(projectId: string, onRedraw: () => void): Promise<void> {
  redraw = onRedraw;
  if (state.projectId === projectId && state.agents.length) return;
  state.projectId = projectId;
  state.loading = true;
  resetAgents();
  redraw();
  try {
    const r = await api<{ agents: ProjectAgent[]; harnesses: string[]; sandboxes: string[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/agents`,
    );
    state.agents = r.agents ?? [];
    state.harnesses = r.harnesses ?? [];
    state.sandboxes = r.sandboxes ?? [];
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.loading = false;
    redraw();
  }
}

async function refresh(): Promise<void> {
  const r = await api<{ agents: ProjectAgent[]; harnesses: string[]; sandboxes: string[] }>(
    `/api/projects/${encodeURIComponent(state.projectId)}/agents`,
  );
  state.agents = r.agents ?? [];
  state.harnesses = r.harnesses ?? [];
  state.sandboxes = r.sandboxes ?? [];
}

async function save(): Promise<void> {
  const draft = state.draft;
  if (!draft || state.saving) return;
  state.saving = true;
  state.notice = "";
  redraw();
  const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
  // Пустая строка означает «наследовать», поэтому она едет как null, а не как пустое значение.
  const body = JSON.stringify({
    name: draft.name,
    role: draft.role,
    harness: draft.harness,
    model: draft.model.trim() || null,
    sandbox: draft.sandbox || null,
    concurrency: draft.concurrency,
    enabled: draft.enabled,
  });
  try {
    if (draft.id) await api(`${base}/agent?agentId=${encodeURIComponent(draft.id)}`, { method: "PATCH", body });
    else await api(`${base}/agents`, { method: "POST", body });
    await refresh();
    state.draft = null;
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.saving = false;
    redraw();
  }
}

async function remove(agent: ProjectAgent): Promise<void> {
  try {
    await api(`/api/projects/${encodeURIComponent(state.projectId)}/agent?agentId=${encodeURIComponent(agent.id)}`, {
      method: "DELETE",
    });
    await refresh();
    if (state.draft?.id === agent.id) state.draft = null;
  } catch (error) {
    state.notice = errMessage(error);
  }
  redraw();
}

async function toggle(agent: ProjectAgent): Promise<void> {
  try {
    await api(`/api/projects/${encodeURIComponent(state.projectId)}/agent?agentId=${encodeURIComponent(agent.id)}`, {
      method: "PATCH",
      body: JSON.stringify({ enabled: !agent.enabled }),
    });
    await refresh();
  } catch (error) {
    state.notice = errMessage(error);
  }
  redraw();
}

function field(label: string, hint: string, control: TemplateResult): TemplateResult {
  return html`
    <label class="agent-field">
      <span class="agent-field-label">${label}</span>
      ${control} ${hint ? html`<span class="agent-field-hint">${hint}</span>` : nothing}
    </label>
  `;
}

function saveLabel(draft: Draft): string {
  if (state.saving) return "Сохраняю…";
  return draft.id ? "Сохранить" : "Завести";
}

function formTpl(): TemplateResult {
  const draft = state.draft!;
  return html`
    <form
      class="agent-form"
      @submit=${(event: SubmitEvent) => {
        event.preventDefault();
        void save();
      }}
    >
      <div class="agent-form-grid">
        ${field(
          "Имя",
          "как агента зовут при назначении",
          html`<input
            type="text"
            required
            maxlength="80"
            .value=${draft.name}
            @input=${(e: Event) => {
              draft.name = (e.target as HTMLInputElement).value;
            }}
          />`,
        )}
        ${field(
          "Харнес",
          "чем агент исполняется",
          fieldSelect({
            ariaLabel: "Харнес агента",
            value: draft.harness,
            onChange: (value) => {
              draft.harness = value;
            },
            options: state.harnesses.map((h) => html`<option value=${h}>${h}</option>`),
          }),
        )}
        ${field(
          "Модель",
          "пусто — модель проекта",
          html`<input
            type="text"
            placeholder="наследуется"
            .value=${draft.model}
            @input=${(e: Event) => {
              draft.model = (e.target as HTMLInputElement).value;
            }}
          />`,
        )}
        ${field(
          "Песочница",
          "пусто — по умолчанию",
          fieldSelect({
            ariaLabel: "Песочница агента",
            value: draft.sandbox,
            onChange: (value) => {
              draft.sandbox = value;
            },
            options: [
              html`<option value="">наследуется</option>`,
              ...state.sandboxes.map((s) => html`<option value=${s}>${s}</option>`),
            ],
          }),
        )}
        ${field(
          "Задач разом",
          `от 1 до 20`,
          html`<input
            type="number"
            min="1"
            max="20"
            .value=${String(draft.concurrency)}
            @input=${(e: Event) => {
              draft.concurrency = Number((e.target as HTMLInputElement).value);
            }}
          />`,
        )}
      </div>
      ${field(
        "Роль",
        "зачем этот агент — читают люди, не машина",
        html`<textarea
          rows="2"
          maxlength="500"
          .value=${draft.role}
          @input=${(e: Event) => {
            draft.role = (e.target as HTMLTextAreaElement).value;
          }}
        ></textarea>`,
      )}
      <div class="agent-form-actions">
        <button type="submit" class="btn primary" ?disabled=${state.saving}>${saveLabel(draft)}</button>
        <button
          type="button"
          class="btn"
          ?disabled=${state.saving}
          @click=${() => {
            state.draft = null;
            redraw();
          }}
        >
          Отмена
        </button>
      </div>
    </form>
  `;
}

function agentTpl(agent: ProjectAgent): TemplateResult {
  return html`
    <article class="agent-card ${agent.enabled ? "" : "is-off"}">
      <div class="agent-card-head">
        <span class="agent-glyph" aria-hidden="true">${icon(Bot, 17)}</span>
        <div class="agent-card-titles">
          <h3>${agent.name}</h3>
          ${agent.role ? html`<p class="agent-role">${agent.role}</p>` : nothing}
        </div>
        <div class="agent-card-actions">
          <button
            type="button"
            class="agent-toggle ${agent.enabled ? "on" : ""}"
            role="switch"
            aria-checked=${agent.enabled ? "true" : "false"}
            aria-label=${agent.enabled ? `Выключить ${agent.name}` : `Включить ${agent.name}`}
            @click=${() => void toggle(agent)}
          >
            <span></span>
          </button>
          <button
            type="button"
            class="icon-btn subtle"
            aria-label=${`Настроить ${agent.name}`}
            @click=${() => {
              state.draft = draftOf(agent);
              redraw();
            }}
          >
            ${icon(Pencil, 15)}
          </button>
          <button
            type="button"
            class="icon-btn subtle"
            aria-label=${`Удалить ${agent.name}`}
            @click=${() => void remove(agent)}
          >
            ${icon(Trash2, 15)}
          </button>
        </div>
      </div>
      <dl class="agent-facts">
        <div>
          <dt>Харнес</dt>
          <dd>${agent.harness}</dd>
        </div>
        <div>
          <dt>Модель</dt>
          <dd>${agent.model ?? html`<span class="agent-inherit">проекта</span>`}</dd>
        </div>
        <div>
          <dt>Песочница</dt>
          <dd>${agent.sandbox ?? html`<span class="agent-inherit">по умолчанию</span>`}</dd>
        </div>
        <div>
          <dt>Задач разом</dt>
          <dd>${agent.concurrency}</dd>
        </div>
        <div>
          <dt>Изменён</dt>
          <dd>${relTime(agent.updatedAt)}</dd>
        </div>
      </dl>
    </article>
  `;
}

function agentsListTpl(): TemplateResult {
  if (state.loading) return html`<div class="empty compact">Читаю агентов…</div>`;
  if (state.agents.length) return html`<div class="agents-list">${state.agents.map((agent) => agentTpl(agent))}</div>`;
  return html`<p class="context-inline-empty">
    Агентов пока нет. Агент — это кто и чем исполняет задачи проекта: харнес, модель, песочница и сколько задач он ведёт
    разом.
  </p>`;
}

export function agentsPanelTpl(): TemplateResult {
  const editing = state.draft !== null;
  return html`
    <section class="agents-panel">
      <div class="agents-head">
        <h2>Агенты</h2>
        <span class="agents-count">${state.agents.length}</span>
        <button
          type="button"
          class="btn primary"
          ?disabled=${editing}
          @click=${() => {
            state.draft = blankDraft();
            redraw();
          }}
        >
          ${icon(Plus, 15)}<span>Завести агента</span>
        </button>
      </div>
      ${state.notice ? html`<div class="agents-notice">${state.notice}</div>` : nothing}
      ${editing ? formTpl() : nothing} ${agentsListTpl()}
    </section>
  `;
}
