import { html, nothing, type TemplateResult } from "lit";
import { GitBranch, Pencil, Plus, Trash2 } from "lucide";
import { api } from "./core-bridge";
import { errMessage } from "../../chassis/src/errors";
import { fieldSelect, icon } from "./ui";

export interface ProjectRepository {
  id: string;
  name: string;
  url: string;
  provider: string;
  baseBranch: string;
  credentialId: string | null;
  isDefault: boolean;
}

interface CredentialBrief {
  id: string;
  service?: string;
  kind?: string;
  envKey?: string;
  accountLabel?: string;
}

interface Draft {
  id: string | null;
  name: string;
  url: string;
  baseBranch: string;
  isDefault: boolean;
  credentialId: string;
}

const state = {
  projectId: "" as string,
  repositories: [] as ProjectRepository[],
  credentials: [] as CredentialBrief[],
  draft: null as Draft | null,
  notice: "" as string,
  loading: false,
  saving: false,
};

let redraw: () => void = () => undefined;

export function resetRepositories(): void {
  state.repositories = [];
  state.draft = null;
  state.notice = "";
}

export async function loadRepositories(projectId: string, onRedraw: () => void): Promise<void> {
  redraw = onRedraw;
  if (state.projectId === projectId && state.repositories.length) return;
  state.projectId = projectId;
  state.loading = true;
  resetRepositories();
  redraw();
  try {
    const [repos, keys] = await Promise.all([
      api<{ repositories: ProjectRepository[] }>(`/api/projects/${encodeURIComponent(projectId)}/repositories`),
      // Учётки берём из существующего keychain — второго места для секретов заводить не нужно.
      api<{ credentials: CredentialBrief[] }>("/api/keychain/credentials").catch(() => ({ credentials: [] })),
    ]);
    state.repositories = repos.repositories ?? [];
    state.credentials = keys.credentials ?? [];
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.loading = false;
    redraw();
  }
}

async function refresh(): Promise<void> {
  const r = await api<{ repositories: ProjectRepository[] }>(
    `/api/projects/${encodeURIComponent(state.projectId)}/repositories`,
  );
  state.repositories = r.repositories ?? [];
}

async function save(): Promise<void> {
  const draft = state.draft;
  if (!draft || state.saving) return;
  state.saving = true;
  state.notice = "";
  redraw();
  const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
  const body = JSON.stringify({
    name: draft.name,
    url: draft.url,
    baseBranch: draft.baseBranch,
    isDefault: draft.isDefault,
    credentialId: draft.credentialId || null,
  });
  try {
    if (draft.id)
      await api(`${base}/repository?repositoryId=${encodeURIComponent(draft.id)}`, { method: "PATCH", body });
    else await api(`${base}/repositories`, { method: "POST", body });
    await refresh();
    state.draft = null;
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.saving = false;
    redraw();
  }
}

async function remove(repository: ProjectRepository): Promise<void> {
  try {
    await api(
      `/api/projects/${encodeURIComponent(state.projectId)}/repository?repositoryId=${encodeURIComponent(repository.id)}`,
      { method: "DELETE" },
    );
    await refresh();
    if (state.draft?.id === repository.id) state.draft = null;
  } catch (error) {
    state.notice = errMessage(error);
  }
  redraw();
}

async function makeDefault(repository: ProjectRepository): Promise<void> {
  if (repository.isDefault) return;
  try {
    await api(
      `/api/projects/${encodeURIComponent(state.projectId)}/repository?repositoryId=${encodeURIComponent(repository.id)}`,
      { method: "PATCH", body: JSON.stringify({ isDefault: true }) },
    );
    await refresh();
  } catch (error) {
    state.notice = errMessage(error);
  }
  redraw();
}

function saveLabel(draft: Draft): string {
  if (state.saving) return "Сохраняю…";
  return draft.id ? "Сохранить" : "Привязать";
}

function formTpl(): TemplateResult {
  const draft = state.draft!;
  const field = (label: string, hint: string, control: TemplateResult) => html`
    <label class="agent-field">
      <span class="agent-field-label">${label}</span>
      ${control} ${hint ? html`<span class="agent-field-hint">${hint}</span>` : nothing}
    </label>
  `;
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
          "как репозиторий зовут в задачах",
          html`<input
            type="text"
            required
            maxlength="60"
            .value=${draft.name}
            @input=${(e: Event) => {
              draft.name = (e.target as HTMLInputElement).value;
            }}
          />`,
        )}
        ${field(
          "Базовая ветка",
          "от неё пойдут ветки задач",
          html`<input
            type="text"
            .value=${draft.baseBranch}
            @input=${(e: Event) => {
              draft.baseBranch = (e.target as HTMLInputElement).value;
            }}
          />`,
        )}
      </div>
      ${field(
        "Адрес",
        "https://, ssh:// или git@host:path — провайдер выведется сам",
        html`<input
          type="text"
          required
          maxlength="400"
          placeholder="https://github.com/…"
          .value=${draft.url}
          @input=${(e: Event) => {
            draft.url = (e.target as HTMLInputElement).value;
          }}
        />`,
      )}
      ${field(
        "Учётка",
        "приватному репозиторию нужен токен; заводится в разделе Keychain",
        fieldSelect({
          ariaLabel: "Учётка репозитория",
          value: draft.credentialId,
          onChange: (value) => {
            draft.credentialId = value;
          },
          options: [
            html`<option value="">без учётки — только публичный</option>`,
            ...state.credentials.map(
              (c) =>
                html`<option value=${c.id}>
                  ${c.envKey ?? c.service ?? c.id}${c.accountLabel ? ` · ${c.accountLabel}` : ""}
                </option>`,
            ),
          ],
        }),
      )}
      <label class="repo-default">
        <input
          type="checkbox"
          .checked=${draft.isDefault}
          @change=${(e: Event) => {
            draft.isDefault = (e.target as HTMLInputElement).checked;
          }}
        />
        <span>основной — сюда пойдут задачи, если не сказано иначе</span>
      </label>
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

function repositoryTpl(repository: ProjectRepository): TemplateResult {
  return html`
    <article class="repo-card">
      <div class="repo-head">
        <span class="agent-glyph" aria-hidden="true">${icon(GitBranch, 16)}</span>
        <div class="repo-titles">
          <h3>
            ${repository.name}
            ${repository.isDefault ? html`<span class="repo-default-badge">основной</span>` : nothing}
          </h3>
          <p class="repo-url">${repository.url}</p>
        </div>
        <div class="agent-card-actions">
          ${
            repository.isDefault
              ? nothing
              : html`<button type="button" class="btn" @click=${() => void makeDefault(repository)}>
                  Сделать основным
                </button>`
          }
          <button
            type="button"
            class="icon-btn subtle"
            aria-label=${`Настроить ${repository.name}`}
            @click=${() => {
              state.draft = {
                id: repository.id,
                name: repository.name,
                url: repository.url,
                baseBranch: repository.baseBranch,
                isDefault: repository.isDefault,
                credentialId: repository.credentialId ?? "",
              };
              redraw();
            }}
          >
            ${icon(Pencil, 15)}
          </button>
          <button
            type="button"
            class="icon-btn subtle"
            aria-label=${`Отвязать ${repository.name}`}
            @click=${() => void remove(repository)}
          >
            ${icon(Trash2, 15)}
          </button>
        </div>
      </div>
      <dl class="agent-facts">
        <div>
          <dt>Провайдер</dt>
          <dd>${repository.provider}</dd>
        </div>
        <div>
          <dt>Базовая ветка</dt>
          <dd>${repository.baseBranch}</dd>
        </div>
        <div>
          <dt>Учётка</dt>
          <dd>${repository.credentialId ?? html`<span class="agent-inherit">не задана</span>`}</dd>
        </div>
      </dl>
    </article>
  `;
}

function listTpl(): TemplateResult {
  if (state.loading) return html`<div class="empty compact">Читаю репозитории…</div>`;
  if (state.repositories.length) {
    return html`<div class="agents-list">${state.repositories.map((r) => repositoryTpl(r))}</div>`;
  }
  return html`<p class="context-inline-empty">
    Репозиториев нет. Пока их нет, задаче некуда писать код: прогон не сможет сделать ветку.
  </p>`;
}

export function repositoriesPanelTpl(): TemplateResult {
  const editing = state.draft !== null;
  return html`
    <section class="context-panel agents-panel">
      <div class="agents-head">
        <h2>Репозитории</h2>
        <span class="agents-count">${state.repositories.length}</span>
        <button
          type="button"
          class="btn primary"
          ?disabled=${editing}
          @click=${() => {
            state.draft = { id: null, name: "", url: "", baseBranch: "main", isDefault: false, credentialId: "" };
            redraw();
          }}
        >
          ${icon(Plus, 15)}<span>Привязать репозиторий</span>
        </button>
      </div>
      ${state.notice ? html`<div class="agents-notice">${state.notice}</div>` : nothing}
      ${editing ? formTpl() : nothing} ${listTpl()}
    </section>
  `;
}
