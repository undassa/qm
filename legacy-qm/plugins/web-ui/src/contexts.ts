import { html, nothing, render, type TemplateResult } from "lit";
import { unsafeHTML } from "lit/directives/unsafe-html.js";
import {
  ArrowLeft,
  Boxes,
  Check,
  ChevronDown,
  Folder,
  FolderPlus,
  Hash,
  ListFilter,
  Lock,
  Plus,
  RefreshCw,
  Search,
  User,
  UserPlus,
  Users,
  X,
  type IconNode,
} from "lucide";
import {
  api,
  isContinuable,
  sharedContextLabel,
  withBase,
  type CoreContext,
  type CoreProject,
  type CoreSession,
} from "./core-bridge";
import { UI_BASE } from "./deep-link";
import { errMessage } from "../../chassis/src/errors";
import { actionSnippet, closeFormMenus, fieldSelect, formatBytes, icon, initials, relTime, toggleFormMenu } from "./ui";
import { chainOf, layoutWaves } from "./task-waves";
import { agentsPanelTpl, loadAgents, resetAgents } from "./project-agents";
import { loadRepositories, repositoriesPanelTpl, resetRepositories } from "./project-repositories";
import { processTpl } from "./project-process";
import { appState, replacePanePreservingFocus, switchView, syncUrlFromState, renderSidebarTop } from "./shell";
import { projectIdOfScope, setActiveScope } from "./shell-state";
import { newChatDraftKey, saveDraft } from "./drafts";
import { openBlankInFocusedPane } from "./split";
import { keysOfDetail, type EntityRow } from "./project-entity-catalog";
import {
  chainTpl,
  discussionPrompt,
  findingsTpl as chainFindingsTpl,
  type ChainJoint,
  type ChainLink,
  type ProjectFinding,
} from "./project-chain";
import {
  documentsWorkspaceTpl,
  openEntityKind,
  labelDocumentHeadings,
  mountProjectDocuments,
  openDocumentAt,
  renderMarkdownHtml,
} from "./documents";
import { mainConversation } from "./conversations";
import { groupDmTitle, openSession, refreshSessions, sessionsState, slackLogo, surfaceOf } from "./sessions";
import { activityOf } from "./session-list";
import type { WebhookView } from "./webhooks";
import type { CronView } from "./crons";
import { cronRunSummary, cronRunSummaryTitle, cronScheduleSummary } from "./cron-format";
import { restoreDialogFocus } from "./dialog-focus";
import { ambientPolicySection, loadAmbientPolicy, resetAmbientPolicy } from "./ambient-policy";
import { contextModelSection, loadContextModel, resetContextModel } from "./context-model";
import { channelHeaderSection, loadChannelHeader, resetChannelHeader } from "./channel-header";

interface ScopeFile {
  id: string;
  name: string;
  mimetype: string;
  sizeBytes: number;
  createdAt: number;
  openable: boolean;
}
interface ScopeDeployment {
  id: string;
  name: string;
  status: string;
  permission: "read" | "write";
  currentVersion: number;
}
interface ScopeSkill {
  id: string;
  name: string;
  description: string;
  status: string;
}
interface ScopeResourcesView {
  files: ScopeFile[];
  webhooks: WebhookView[];
  crons: CronView[];
  deployments: ScopeDeployment[];
  skills: ScopeSkill[];
  manageable: boolean;
}

interface DirectoryMatch {
  principalId: string;
  displayName: string;
  type: string;
}

export type ProjectTab = "overview" | "process" | "documents" | "board" | "agents" | "settings";

const PROJECT_TABS: readonly ProjectTab[] = ["overview", "process", "documents", "board", "agents", "settings"];

export function isProjectTab(value: string): value is ProjectTab {
  return (PROJECT_TABS as readonly string[]).includes(value);
}

interface BoardTask {
  id: string;
  title: string;
  milestoneId: string;
  state: string;
  size: string;
  blockedBy: number;
  dependsOn: string[];
  kind: string;
  artifacts: string[];
}

interface TaskRun {
  id: string;
  taskId: string;
  agentId: string;
  state: string;
  attempt: number;
  createdAt: number;
  finishedAt: number | null;
}

interface TaskRunEvent {
  id: number;
  fromState: string | null;
  toState: string;
  reason: string;
  actor: string;
  at: number;
}

interface AgentBrief {
  id: string;
  name: string;
  enabled: boolean;
  concurrency: number;
}

interface TaskCheck {
  id: string;
  spec: string;
}

interface TaskRequirement {
  id: string;
  kind: string;
  text: string;
  satisfied: boolean;
  checks: TaskCheck[];
}

interface TaskNeighbour {
  id: string;
  title: string;
  state: string;
}

interface TaskDetail {
  id: string;
  title: string;
  path: string;
  milestoneId: string;
  kind: string;
  state: string;
  size: string;
  closingCommit: string | null;
  runPath: string | null;
  dependsOn: TaskNeighbour[];
  blocks: TaskNeighbour[];
  requirements: TaskRequirement[];
}

interface MilestoneProgress {
  id: string;
  title: string;
  path: string;
  ord: number;
  tasks: number;
  closed: number;
  claimed: number;
}

interface ProjectProgress {
  documents: number;
  version: string | null;
  milestones: MilestoneProgress[];
  plan: {
    versions: number;
    milestones: number;
    tasks: number;
    closed: number;
    claimed: number;
    ready: number;
    tests: number;
  } | null;
  proof: { requirements: number; functional: number; checks: number; covered: number; uncovered: number } | null;
  gates: {
    phase: string;
    item: string;
    kind: string;
    state: string;
    violations: number;
    signedBy: string | null;
    signedAt: number | null;
    staleDocuments: string[];
  }[];
}

export const contextsState = {
  list: [] as CoreContext[],
  loaded: false,
  loadedAt: 0,
  selected: null as string | null,
  resources: null as ScopeResourcesView | null,
  resourcesScope: null as string | null,
  resourcesLoading: false,
  resourcesNotice: "",
  progress: null as ProjectProgress | null,
  progressScope: null as string | null,
  progressLoading: false,
  tab: "overview" as ProjectTab,
  chainScope: null as string | null,
  chainLinks: [] as ChainLink[],
  chainJoints: [] as ChainJoint[],
  chainFindings: [] as ProjectFinding[],
  chainLoading: false,
  board: [] as BoardTask[],
  boardLoading: false,
  activeRuns: {} as Record<string, TaskRun>,
  agentList: [] as AgentBrief[],
  taskRuns: [] as TaskRun[],
  taskEvents: [] as TaskRunEvent[],
  taskNext: [] as string[],
  assignTo: "" as string,
  runBusy: false,
  runNotice: "" as string,
  signBusy: false,
  taskDoc: "" as string,
  taskDocLoading: false,
  taskRunDoc: "" as string,
  task: null as TaskDetail | null,
  taskLoading: "" as string,
  boardView: "waves" as "waves" | "columns",
  kindFilter: "all" as "all" | "dev" | "test",
  milestone: "" as string,
  milestoneDoc: null as { path: string; content: string } | null,
  milestoneLoading: false,
  milestoneError: "" as string,
  createOpen: false,
  createName: "",
  createSaving: false,
  createError: "",
  memberProjectId: null as string | null,
  memberQuery: "",
  memberMatches: [] as DirectoryMatch[],
  memberSearching: false,
  memberBusy: false,
  memberError: "",
  memberSearchedQuery: "",
  slackEditing: false,
  slackValue: "",
  slackBusy: false,
  slackError: "",
};

let contextsLoading = false;
let contextsNotice = "";
let memberSearchSeq = 0;
const MEMBER_SEARCH_DEBOUNCE_MS = 300;
let memberSearchTimer: ReturnType<typeof setTimeout> | undefined;

function cancelMemberSearchTimer(): void {
  if (memberSearchTimer !== undefined) {
    clearTimeout(memberSearchTimer);
    memberSearchTimer = undefined;
  }
}
let createProjectOpener: HTMLElement | null = null;
let createProjectSeq = 0;
let contextsResetSeq = 0;
let contextsFetchSeq = 0;
let contextsQuery = "";
let contextsWorkspaceFilter: "active" | "all" = "active";

async function fetchContexts(): Promise<CoreContext[]> {
  const fetchSeq = ++contextsFetchSeq;
  const result = await api<{ contexts: CoreContext[] }>("/api/contexts").catch((error: unknown) => {
    if (fetchSeq !== contextsFetchSeq) return null;
    throw error;
  });
  if (!result || fetchSeq !== contextsFetchSeq) return contextsState.list;
  contextsState.list = result.contexts ?? [];
  contextsState.loaded = true;
  contextsState.loadedAt = Date.now();
  return contextsState.list;
}

export function resetContextsState(): void {
  contextsState.list = [];
  contextsState.loaded = false;
  contextsState.loadedAt = 0;
  contextsState.selected = null;
  contextsState.resources = null;
  contextsState.resourcesScope = null;
  contextsState.resourcesLoading = false;
  contextsState.progress = null;
  contextsState.progressScope = null;
  contextsState.progressLoading = false;
  contextsState.resourcesNotice = "";
  contextsState.createOpen = false;
  contextsState.createName = "";
  contextsState.createSaving = false;
  contextsState.createError = "";
  contextsState.memberProjectId = null;
  contextsState.memberQuery = "";
  contextsState.memberMatches = [];
  contextsState.memberSearching = false;
  contextsState.memberBusy = false;
  contextsState.memberError = "";
  contextsState.memberSearchedQuery = "";
  contextsState.slackEditing = false;
  contextsState.slackValue = "";
  contextsState.slackBusy = false;
  contextsState.slackError = "";
  cancelMemberSearchTimer();
  contextsNotice = "";
  memberSearchSeq++;
  contextsFetchSeq++;
  createProjectSeq++;
  contextsResetSeq++;
  createProjectOpener = null;
  contextsQuery = "";
  contextsWorkspaceFilter = "active";
}

export async function renderContexts(): Promise<void> {
  if (appState.currentView !== "contexts") return;
  const seq = appState.viewRenderSeq;
  contextsNotice = "";
  contextsLoading = true;
  drawContexts();
  try {
    await Promise.all([fetchContexts(), refreshSessions({ silent: true })]);
    if (seq !== appState.viewRenderSeq || appState.currentView !== "contexts") return;
  } catch (e) {
    if (seq !== appState.viewRenderSeq || appState.currentView !== "contexts") return;
    contextsNotice = errMessage(e, "Failed to load contexts.");
  }
  contextsLoading = false;
  if (
    contextsState.selected &&
    contextsState.list.some((c) => c.scopeId === contextsState.selected) &&
    contextsState.resourcesScope !== contextsState.selected
  ) {
    void loadScopeResources(contextsState.selected);
    void loadAmbientPolicy(contextsState.selected, drawContexts);
    void loadContextModel(contextsState.selected, drawContexts);
    void loadChannelHeader(contextsState.selected, drawContexts);
  }
  drawContexts();
}

function contextMeta(c: CoreContext): { title: string; sub: string; glyph: IconNode } {
  if (c.project) {
    const memberCount = projectPeople(c).length;
    return {
      title: c.project.name,
      sub: `${memberCount} ${memberCount === 1 ? "member" : "members"}`,
      glyph: Folder,
    };
  }
  if (c.kind === "personal") {
    return { title: "Personal", sub: "Just you — your web chats and DMs with the agent live here.", glyph: User };
  }
  if (c.kind === "group") {
    return {
      title: sharedContextLabel(c.scopeId, c.name) ?? "Group DM",
      sub: "Shared with everyone in this group conversation.",
      glyph: Users,
    };
  }
  return {
    title: sharedContextLabel(c.scopeId, c.name) ?? "Channel",
    sub: "Shared with everyone in this channel.",
    glyph: Hash,
  };
}

export async function ensureContexts(force = false): Promise<CoreContext[]> {
  if (contextsState.loaded && !force) return contextsState.list;
  try {
    await fetchContexts();
  } catch {
    void 0;
  }
  return contextsState.list;
}

export function personalScopeId(): string | null {
  return contextsState.list.find((c) => c.kind === "personal")?.scopeId ?? null;
}

export function resolveProjectScope(contexts: readonly CoreContext[], slug: string): string | null {
  if (slug.startsWith("channel:") || slug.startsWith("group:")) {
    return contexts.some((context) => context.scopeId === slug) ? slug : null;
  }
  const normalized = slug.toLowerCase();
  const matches = contexts.filter((context) => {
    const match = /^personal:([^@]+)@/.exec(context.scopeId);
    return match?.[1]?.toLowerCase() === normalized;
  });
  return matches.length === 1 ? matches[0]!.scopeId : null;
}

function metaForScope(scopeId: string | null, fallbackName?: string | null): { title: string; glyph: IconNode } {
  const c = scopeId ? contextsState.list.find((x) => x.scopeId === scopeId) : undefined;
  if (c) {
    const { title, glyph } = contextMeta(c);
    return { title, glyph };
  }
  const shared = sharedContextLabel(scopeId, fallbackName ?? null);
  if (shared) return { title: shared, glyph: scopeId?.startsWith("group:") ? Users : Hash };
  if (scopeId?.startsWith("personal:") && scopeId !== personalScopeId())
    return { title: "Shared personal space", glyph: User };
  return { title: fallbackName?.trim() || "Personal", glyph: User };
}

export function scopeTitle(scopeId: string | null, fallbackName?: string | null): string {
  return metaForScope(scopeId, fallbackName).title;
}

export function scopeChip(scopeId: string | null, fallbackName?: string | null): TemplateResult {
  const { title, glyph } = metaForScope(scopeId, fallbackName);
  return html`<span class="scope-chip" title=${`In ${title}`}
    >${icon(glyph, 12)}<span>${title.replace(/^#/, "")}</span></span
  >`;
}

/**
 * Одно и то же меню областей читается двумя способами. «filter» — фильтр над
 * общими списками, как было. «place» — где ты находишься: проект перестаёт быть
 * подкраской чужого списка и становится местом, внутри которого идёт работа.
 */
export function scopeFilterControl(
  current: string | null,
  onSelect: (scopeId: string | null) => void,
  variant: "filter" | "place" = "filter",
): TemplateResult {
  const label = current ? metaForScope(current).title : "All contexts";
  const option = (scopeId: string | null, text: string, glyph: IconNode) => {
    const active = (current ?? null) === scopeId;
    return html`
      <button
        class="menu-option ${active ? "active" : ""}"
        type="button"
        role="menuitemradio"
        aria-checked=${active ? "true" : "false"}
        @click=${(e: Event) => {
          e.stopPropagation();
          closeFormMenus();
          onSelect(scopeId);
        }}
      >
        <span class="menu-option-label scope-option-label">${icon(glyph, 14)}<span>${text}</span></span>
        ${active ? icon(Check, 15) : nothing}
      </button>
    `;
  };
  const place = variant === "place";
  const glyph = current ? metaForScope(current).glyph : Boxes;
  return html`
    <div class="menu-control form-menu-control scope-filter ${place ? "scope-place" : ""}">
      <button class="menu-button" type="button" aria-haspopup="menu" aria-expanded="false" @click=${toggleFormMenu}>
        ${
          place
            ? html`${icon(glyph, 15)}
                <span class="menu-label">
                  <span class="scope-place-eyebrow">${current ? "project" : "no project"}</span>
                  <span class="scope-place-name">${current ? label : "Choose one"}</span>
                </span>
                ${icon(ChevronDown, 14)}`
            : html`${icon(ListFilter, 14)}<span class="menu-label">Filter by: ${label}</span>${icon(ChevronDown, 14)}`
        }
      </button>
      <div class="menu-popover" role="menu" hidden>
        <div class="menu-title">Filter by context</div>
        ${option(null, "All contexts", Boxes)}
        ${contextsState.list.map((c) => option(c.scopeId, contextMeta(c).title, contextMeta(c).glyph))}
      </div>
    </div>
  `;
}

function sessionsIn(scopeId: string): CoreSession[] {
  return sessionsState.list
    .filter((s) => s.scopeId === scopeId && !s.archived)
    .sort((a, b) => activityOf(b) - activityOf(a));
}

function drawContexts(): void {
  if (appState.currentView !== "contexts" || !appState.mainEl) return;
  const host = document.createElement("div");
  const fullBleed = contextsState.selected !== null && contextsState.tab === "documents";
  host.className = `pane contexts-pane${fullBleed ? " documents-fullbleed" : ""}`;
  const selected = contextsState.selected
    ? contextsState.list.find((c) => c.scopeId === contextsState.selected)
    : undefined;
  render(selected ? detailTpl(selected) : gridTpl(), host);
  replacePanePreservingFocus(host);
  const dialog = host.querySelector<HTMLDialogElement>(".project-dialog");
  if (dialog && !dialog.open) dialog.showModal();
  const drawer = host.querySelector<HTMLDialogElement>(".issue-drawer");
  if (drawer && !drawer.open) drawer.showModal();
  labelDocumentHeadings();
}

function gridTpl(): TemplateResult {
  const status = contextsNotice || (contextsLoading && contextsState.list.length === 0 ? "Loading projects…" : "");
  const q = contextsQuery.trim().toLowerCase();
  const matches = (context: CoreContext) => {
    const meta = contextMeta(context);
    return (
      (!q || `${meta.title} ${meta.sub}`.toLowerCase().includes(q)) &&
      (contextsWorkspaceFilter === "all" ||
        context.kind === "personal" ||
        Boolean(context.project) ||
        Boolean(context.sessionCount))
    );
  };
  const projects = contextsState.list.filter(matches);
  const groupOf = (context: CoreContext) => {
    if (context.kind === "personal") return "personal";
    return context.project ? "web" : "slack";
  };
  const groups = [
    { key: "personal", label: "Personal" },
    { key: "web", label: "Web" },
    { key: "slack", label: "Slack" },
  ]
    .map((g) => ({ ...g, items: projects.filter((context) => groupOf(context) === g.key) }))
    .filter((g) => g.items.length > 0);
  const projectsFiltered = Boolean(q);
  let projectList: TemplateResult | typeof nothing = nothing;
  if (projects.length)
    projectList = html`<div class="project-list">
      ${groups.map(
        (g) =>
          html`<section class="project-group">
            <div class="project-group-head">
              ${g.label} <span class="project-group-count">· ${g.items.length}</span>
            </div>
            ${g.items.map(contextRow)}
          </section>`,
      )}
    </div>`;
  else if (!contextsLoading) {
    projectList = html`<div class="empty compact project-empty">
      ${projectsFiltered ? "No projects match your search." : "No projects yet."}
    </div>`;
  }
  return html`
    <div class="project-grid-content">
      <div class="pane-head">
        <h1 class="pane-title">Projects</h1>
        <div class="project-head-actions">
          <button
            class="pane-refresh"
            type="button"
            aria-label="Refresh projects"
            title="Refresh projects"
            @click=${() => void renderContexts()}
          >
            ${icon(RefreshCw, 17)}
          </button>
          <button
            class="btn primary project-create-button"
            type="button"
            aria-label="New project"
            title="New project"
            @click=${openCreateProject}
          >
            ${icon(FolderPlus, 15)}<span>New project</span>
          </button>
        </div>
      </div>
      <div class="list-toolbar project-toolbar">
        <label class="list-search"
          ><span class="sr-only">Search projects</span
          ><input
            data-focus-key="contexts-search"
            type="search"
            aria-label="Search projects"
            placeholder="Search projects…"
            .value=${contextsQuery}
            @input=${(event: InputEvent) => {
              contextsQuery = (event.currentTarget as HTMLInputElement).value;
              drawContexts();
            }}
        /></label>
        <label class="list-select"
          ><span>Show</span>${fieldSelect({
            compact: true,
            value: contextsWorkspaceFilter,
            onChange: (value) => {
              contextsWorkspaceFilter = value as typeof contextsWorkspaceFilter;
              drawContexts();
            },
            options: [html`<option value="active">Active only</option>`, html`<option value="all">Everything</option>`],
          })}</label
        >
      </div>
      ${status ? html`<div class="status">${status}</div>` : nothing} ${projectList}
    </div>
    ${createProjectDialog()}
  `;
}

function contextRow(c: CoreContext): TemplateResult {
  const { title, sub, glyph } = contextMeta(c);
  const count = c.sessionCount === 1 ? "1 conversation" : `${c.sessionCount} conversations`;
  const meta = [c.project ? sub : "", count, c.lastActivityAt ? `active ${relTime(c.lastActivityAt)}` : ""]
    .filter(Boolean)
    .join(" · ");
  return html`
    <button class="context-row" type="button" title=${sub} @click=${() => selectContext(c.scopeId)}>
      <span class="context-glyph">${icon(glyph, 15)}</span>
      <span class="context-row-title">${title}</span>
      ${c.isPrivate ? html`<span class="context-lock" title="Private channel">${icon(Lock, 12)}</span>` : nothing}
      <span class="context-row-meta">${meta}</span>
    </button>
  `;
}

function detailTpl(c: CoreContext): TemplateResult {
  const { title, sub, glyph } = contextMeta(c);
  const sessions = sessionsIn(c.scopeId);
  const hasDocuments = contextsState.progressScope === c.scopeId && (contextsState.progress?.documents ?? 0) > 0;
  const tabbed = contextsState.progressScope === c.scopeId && contextsState.progress !== null;
  const completelyEmpty = sessions.length === 0 && scopeResourcesEmpty(c.scopeId) && !hasDocuments;
  return html`
    <div class="context-detail">
      <button class="context-back" type="button" @click=${() => selectContext(null)}>
        ${icon(ArrowLeft, 15)}<span>Projects</span>
      </button>
      <div class="context-detail-head">
        <span class="context-glyph large">${icon(glyph, 22)}</span>
        <div class="context-detail-titles">
          <h1 class="pane-title">
            ${title}
            ${c.isPrivate ? html`<span class="context-lock" title="Private channel">${icon(Lock, 14)}</span>` : nothing}
          </h1>
          <div class="context-sub">
            ${c.project ? sub : `${sub} The agent's files and memory here are separate from your other contexts.`}
          </div>
        </div>
        <div class="context-detail-actions">
          ${
            c.project
              ? html`<button class="btn context-add-member" type="button" @click=${() => toggleMemberPicker(c)}>
                  ${icon(UserPlus, 15)}<span>Add people</span>
                </button>`
              : nothing
          }
          <button class="btn primary context-new-chat" type="button" @click=${() => startChatIn(c)}>
            ${icon(Plus, 15)}<span>New chat</span>
          </button>
        </div>
      </div>
      <div class="context-workspace ${tabbed ? "" : "has-settings"}">
        <div class="context-workspace-main">
          ${
            completelyEmpty
              ? html`
                  <section class="context-panel context-project-empty">
                    <span class="context-glyph large" aria-hidden="true">${icon(glyph, 22)}</span>
                    <h2>This project is ready for work</h2>
                    <p>
                      Start a conversation with New chat. Files, automations, and other work created there will stay
                      scoped to this project.
                    </p>
                  </section>
                `
              : html`
                  ${projectTabsTpl(c.scopeId)}
                  ${contextsState.tab === "documents" ? projectDocumentsSection(c.scopeId) : nothing}
                  ${contextsState.tab === "board" ? projectBoardSection(c.scopeId, c) : nothing}
                  ${contextsState.tab === "agents" ? agentsPanelTpl() : nothing}
                  ${
                    contextsState.tab === "process"
                      ? processTpl(contextsState.progress?.gates ?? [], {
                          testTasks: contextsState.board.filter((task) => task.kind === "test").length,
                          devTasks: contextsState.board.filter((task) => task.kind !== "test").length,
                          // Вопросы и ответы уже умеет чат проекта — отдельного канала не заводим.
                          onStartTests: () => startChatIn(c),
                          busy: contextsState.signBusy,
                          onSign: (phase, item, revoke) => void signGate(c.scopeId, phase, item, revoke),
                          onRead: (phase) => void openPhaseReading(c.scopeId, phase),
                        })
                      : nothing
                  }
                  ${
                    contextsState.tab === "settings"
                      ? html`<div class="context-settings-tab">${contextSettingsTpl(c)}</div>`
                      : nothing
                  }
                  ${contextsState.tab === "overview" ? projectChainSection(c.scopeId) : nothing}
                  ${contextsState.tab === "overview" ? projectProgressSection(c.scopeId) : nothing}
                  ${
                    contextsState.tab === "overview"
                      ? html`<section
                          class="context-panel context-conversations"
                          aria-labelledby="context-conversations-title"
                        >
                          <div class="context-panel-heading">
                            <h2 class="context-panel-title" id="context-conversations-title">Conversations</h2>
                            ${sessions.length ? html`<span class="context-panel-count">${sessions.length}</span>` : nothing}
                          </div>
                          ${
                            sessions.length
                              ? html`<div class="context-session-list">
                                  ${sessions.map((s) => contextSessionRow(s))}
                                </div>`
                              : html`<div class="context-inline-empty">No conversations yet.</div>`
                          }
                        </section>`
                      : nothing
                  }
                  ${contextsState.tab === "overview" ? resourceSections(c.scopeId) : nothing}
                `
          }
        </div>
        ${
          tabbed
            ? nothing
            : html`<aside class="context-settings" aria-label=${c.project ? "Project settings" : "Context settings"}>
                ${contextSettingsTpl(c)}
              </aside>`
        }
      </div>
    </div>
  `;
}

function scopeResourcesEmpty(scopeId: string): boolean {
  const r = contextsState.resourcesScope === scopeId ? contextsState.resources : null;
  return Boolean(
    r &&
    r.files.length === 0 &&
    r.webhooks.length === 0 &&
    r.crons.length === 0 &&
    r.deployments.length === 0 &&
    r.skills.length === 0,
  );
}

function projectPeople(context: CoreContext): string[] {
  if (!context.project) return [];
  return [...new Set([context.project.ownerId, ...context.project.memberIds].filter(Boolean))];
}

function isProjectOwner(context: CoreContext): boolean {
  return context.project?.ownerId === appState.me?.user;
}

function memberLabel(context: CoreContext, principalId: string): string {
  if (principalId === appState.me?.user) return "You";
  return context.project?.members.find((member) => member.principalId === principalId)?.displayName || principalId;
}

function channelNameOptions(): string[] {
  return [
    ...new Set(
      contextsState.list
        .filter((c) => c.kind === "channel" && c.name)
        .map((c) => c.name!.replace(/^#/, ""))
        .sort(),
    ),
  ];
}

async function linkProjectSlackChannel(context: CoreContext): Promise<void> {
  const channel = contextsState.slackValue.trim().replace(/^#/, "");
  if (!context.project || contextsState.slackBusy || !channel) return;
  const resetSeq = contextsResetSeq;
  contextsState.slackBusy = true;
  contextsState.slackError = "";
  drawContexts();
  try {
    const response = await api(`/api/projects/${encodeURIComponent(context.project.id)}/slack-channel`, {
      method: "PUT",
      body: JSON.stringify({ channel }),
    });
    if (resetSeq !== contextsResetSeq) return;
    const project = projectFromResponse(response);
    if (!project) throw new Error("Core returned an invalid project");
    upsertProject(project);
    contextsState.slackEditing = false;
    contextsState.slackValue = "";
  } catch (error) {
    if (resetSeq !== contextsResetSeq) return;
    contextsState.slackError = errMessage(error, "Couldn't link that channel — you must be a member of it.");
  } finally {
    if (resetSeq === contextsResetSeq) {
      contextsState.slackBusy = false;
      drawContexts();
    }
  }
}

async function unlinkProjectSlackChannel(context: CoreContext): Promise<void> {
  const linked = context.project?.slackChannel;
  if (!context.project || !linked || contextsState.slackBusy) return;
  if (!window.confirm(`Unlink #${linked.channelName} from ${context.name || "this project"}?`)) return;
  const resetSeq = contextsResetSeq;
  contextsState.slackBusy = true;
  contextsState.slackError = "";
  drawContexts();
  try {
    const response = await api(`/api/projects/${encodeURIComponent(context.project.id)}/slack-channel`, {
      method: "DELETE",
    });
    if (resetSeq !== contextsResetSeq) return;
    const project = projectFromResponse(response);
    if (!project) throw new Error("Core returned an invalid project");
    upsertProject(project);
  } catch (error) {
    if (resetSeq !== contextsResetSeq) return;
    contextsState.slackError = errMessage(error, "Couldn't unlink the channel.");
  } finally {
    if (resetSeq === contextsResetSeq) {
      contextsState.slackBusy = false;
      drawContexts();
    }
  }
}

function projectSlackLinked(context: CoreContext): TemplateResult {
  const linked = context.project!.slackChannel!;
  return html`
    <div class="project-member-row">
      <span class="context-glyph" aria-hidden="true">${icon(Hash, 15)}</span>
      <span class="project-member-name">${linked.channelName}</span>
      <button
        class="project-icon-button danger"
        type="button"
        aria-label=${`Unlink #${linked.channelName}`}
        title=${`Unlink #${linked.channelName}`}
        ?disabled=${contextsState.slackBusy}
        @click=${() => void unlinkProjectSlackChannel(context)}
      >
        ${icon(X, 15)}
      </button>
    </div>
    <p class="context-hint">
      The agent posts this project's updates to #${linked.channelName}, and everyone in the channel is in the project.
    </p>
  `;
}

function projectSlackEditor(context: CoreContext): TemplateResult {
  const options = channelNameOptions();
  return html`
    <form
      class="project-slack-form"
      @submit=${(e: Event) => {
        e.preventDefault();
        void linkProjectSlackChannel(context);
      }}
    >
      <div class="project-member-search-row">
        ${icon(Hash, 16)}
        <input
          type="text"
          data-focus-key="project-slack-channel"
          autocomplete="off"
          maxlength="200"
          placeholder="channel name"
          list="project-slack-channels"
          aria-label="Slack channel to link"
          .value=${contextsState.slackValue}
          ?disabled=${contextsState.slackBusy}
          @input=${(e: Event) => {
            contextsState.slackValue = (e.target as HTMLInputElement).value;
          }}
        />
      </div>
      <datalist id="project-slack-channels">${options.map((name) => html`<option value=${name}></option>`)}</datalist>
      <div class="project-slack-actions">
        <button class="btn primary" type="submit" ?disabled=${contextsState.slackBusy}>Link</button>
        <button
          class="btn"
          type="button"
          ?disabled=${contextsState.slackBusy}
          @click=${() => {
            contextsState.slackEditing = false;
            contextsState.slackValue = "";
            contextsState.slackError = "";
            drawContexts();
          }}
        >
          Cancel
        </button>
      </div>
    </form>
  `;
}

function projectSlackIdle(): TemplateResult {
  return html`
    <button
      class="btn project-slack-link"
      type="button"
      ?disabled=${contextsState.slackBusy}
      @click=${() => {
        contextsState.slackEditing = true;
        contextsState.slackError = "";
        drawContexts();
      }}
    >
      ${icon(Hash, 15)}<span>Link a channel</span>
    </button>
    <p class="context-hint">
      Give this project a home channel on Slack — the agent will post updates there, and everyone in the channel joins
      the project.
    </p>
  `;
}

function projectSlackSection(context: CoreContext): TemplateResult {
  const project = context.project!;
  let body: TemplateResult;
  if (project.slackChannel) body = projectSlackLinked(context);
  else if (contextsState.slackEditing) body = projectSlackEditor(context);
  else body = projectSlackIdle();
  return html`
    <section class="context-panel project-slack" aria-labelledby="project-slack-title">
      <div class="context-panel-heading">
        <h2 class="context-panel-title" id="project-slack-title">Slack channel</h2>
      </div>
      ${body}
      ${contextsState.slackError ? html`<div class="project-member-status error" aria-live="polite">${contextsState.slackError}</div>` : nothing}
    </section>
  `;
}

function projectMembersSection(context: CoreContext): TemplateResult {
  const project = context.project!;
  const pickerOpen = contextsState.memberProjectId === project.id;
  return html`
    <section class="context-panel project-members" aria-labelledby="project-people-title">
      <div class="context-panel-heading">
        <h2 class="context-panel-title" id="project-people-title">People</h2>
        <span class="context-panel-count">${projectPeople(context).length}</span>
      </div>
      <div class="project-member-list">
        ${projectPeople(context).map((principalId) => {
          const label = memberLabel(context, principalId);
          const viaChannel = Boolean(project.members.find((member) => member.principalId === principalId)?.viaChannel);
          return html`
            <div class="project-member-row">
              <span class="project-member-avatar" aria-hidden="true">${initials(label)}</span>
              <span class="project-member-name">${label}</span>
              ${principalId === project.ownerId ? html`<span class="badge">Owner</span>` : nothing}
              ${
                viaChannel && project.slackChannel
                  ? html`<span class="badge" title="Joined via the linked Slack channel"
                      >#${project.slackChannel.channelName}</span
                    >`
                  : nothing
              }
              ${
                isProjectOwner(context) && principalId !== project.ownerId && !viaChannel
                  ? html`<button
                      class="project-icon-button danger"
                      type="button"
                      aria-label=${`Remove ${label}`}
                      title=${`Remove ${label}`}
                      ?disabled=${contextsState.memberSearching || contextsState.memberBusy}
                      @click=${() => void removeProjectMember(context, principalId)}
                    >
                      ${icon(X, 15)}
                    </button>`
                  : nothing
              }
            </div>
          `;
        })}
      </div>
      ${pickerOpen ? memberPicker(context) : nothing}
      ${contextsState.memberError ? html`<div class="project-member-status error" aria-live="polite">${contextsState.memberError}</div>` : nothing}
    </section>
  `;
}

function memberPicker(context: CoreContext): TemplateResult {
  const members = new Set(projectPeople(context));
  const matches = contextsState.memberMatches.filter((match) => !members.has(match.principalId)).slice(0, 8);
  const idle = !contextsState.memberSearching && !contextsState.memberBusy && !contextsState.memberError;
  let emptyNote = "";
  if (idle && contextsState.memberSearchedQuery && matches.length === 0) {
    emptyNote = contextsState.memberMatches.length
      ? "Everyone matching is already in this project."
      : `No matches for “${contextsState.memberSearchedQuery}”.`;
  }
  let memberStatus = emptyNote;
  if (contextsState.memberSearching) memberStatus = "Searching…";
  else if (contextsState.memberBusy) memberStatus = "Working…";
  return html`
    <form class="project-member-picker" @submit=${(event: SubmitEvent) => void searchProjectMembers(event, context)}>
      <label for="project-member-search">Add people</label>
      <div class="project-member-search-row">
        ${icon(Search, 16)}
        <input
          id="project-member-search"
          data-focus-key="project-member-search"
          name="query"
          type="search"
          autocomplete="off"
          maxlength="80"
          placeholder="Search by name or handle"
          .value=${contextsState.memberQuery}
          ?disabled=${contextsState.memberBusy}
          @input=${(event: InputEvent) => {
            contextsState.memberQuery = (event.currentTarget as HTMLInputElement).value;
            scheduleMemberSearch(context);
          }}
        />
        <button
          class="project-icon-button"
          type="submit"
          aria-label="Search"
          title="Search"
          ?disabled=${contextsState.memberSearching || contextsState.memberBusy}
        >
          ${icon(Search, 15)}
        </button>
        <button
          class="project-icon-button"
          type="button"
          aria-label="Close"
          title="Close"
          ?disabled=${contextsState.memberBusy}
          @click=${closeMemberPicker}
        >
          ${icon(X, 15)}
        </button>
      </div>
      <div class="project-member-results">
        ${matches.map(
          (match) => html`
            <button
              class="project-member-result"
              type="button"
              ?disabled=${contextsState.memberSearching || contextsState.memberBusy}
              @click=${() => void addProjectMember(context, match)}
            >
              <span class="project-member-avatar" aria-hidden="true">${initials(match.displayName)}</span>
              <span class="project-member-name">${match.displayName}</span>
              ${icon(Plus, 15)}
            </button>
          `,
        )}
      </div>
      <div class="project-member-status" aria-live="polite">${memberStatus}</div>
    </form>
  `;
}

async function loadProjectBoard(scopeId: string, force = false): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId || contextsState.boardLoading) return;
  if (!force && contextsState.board.length) return;
  contextsState.boardLoading = true;
  try {
    const base = `/api/projects/${encodeURIComponent(projectId)}`;
    const [board, runs, agents] = await Promise.all([
      api<{ tasks: BoardTask[] }>(`${base}/board`),
      api<{ runs: TaskRun[] }>(`${base}/task-runs`).catch(() => ({ runs: [] })),
      api<{ agents: AgentBrief[] }>(`${base}/agents`).catch(() => ({ agents: [] })),
    ]);
    contextsState.board = board.tasks ?? [];
    contextsState.activeRuns = Object.fromEntries((runs.runs ?? []).map((run) => [run.taskId, run]));
    contextsState.agentList = (agents.agents ?? []).filter((a) => a.enabled);
  } catch {
    contextsState.board = [];
    contextsState.activeRuns = {};
    contextsState.agentList = [];
    contextsState.taskRuns = [];
    contextsState.taskEvents = [];
    contextsState.taskNext = [];
    contextsState.runNotice = "";
    contextsState.taskDoc = "";
    contextsState.taskRunDoc = "";
  } finally {
    contextsState.boardLoading = false;
    drawContexts();
  }
}

function selectProjectTab(scopeId: string, tab: ProjectTab): void {
  contextsState.tab = tab;
  if (tab === "board" || tab === "process") void loadProjectBoard(scopeId);
  if (tab === "agents") {
    const projectId = projectIdOfScope(scopeId);
    if (projectId) void loadAgents(projectId, drawContexts);
  }
  if (tab === "settings") {
    const projectId = projectIdOfScope(scopeId);
    if (projectId) void loadRepositories(projectId, drawContexts);
  }
  drawContexts();
}

function contextSettingsTpl(c: CoreContext): TemplateResult {
  return html`
    ${c.project ? repositoriesPanelTpl() : nothing} ${c.project ? projectMembersSection(c) : nothing}
    ${c.project ? projectSlackSection(c) : nothing} ${contextModelSection(c.scopeId)} ${channelHeaderSection(c.scopeId)}
    ${ambientPolicySection(c.scopeId)}
  `;
}

function projectTabsTpl(scopeId: string): TemplateResult | typeof nothing {
  if (contextsState.progressScope !== scopeId || !contextsState.progress) return nothing;
  const p = contextsState.progress;
  const tab = (id: ProjectTab, label: string, count: number | null) => html`
    <button
      type="button"
      role="tab"
      aria-selected=${contextsState.tab === id ? "true" : "false"}
      class="project-tab ${contextsState.tab === id ? "active" : ""}"
      @click=${() => selectProjectTab(scopeId, id)}
    >
      ${label}${count === null ? nothing : html`<span class="project-tab-count">${count}</span>`}
    </button>
  `;
  return html`
    <div class="project-tabs" role="tablist" aria-label="Project views">
      ${tab("overview", "Overview", null)} ${tab("process", "Процесс", null)}
      ${p.documents ? tab("documents", "Documents", p.documents) : nothing}
      ${p.plan && p.plan.tasks ? tab("board", "Board", p.plan.tasks) : nothing} ${tab("agents", "Агенты", null)}
      ${tab("settings", "Settings", null)}
    </div>
  `;
}

function stageSummaryTpl(p: ProjectProgress, current: MilestoneProgress | null): TemplateResult | typeof nothing {
  // Название вехи уже начинается с её идентификатора — второй раз его печатать не нужно.
  if (current) return html` · сейчас <b>${current.title}</b> — ${current.closed}/${current.tasks}`;
  return p.milestones.length ? html` · все вехи закрыты` : nothing;
}

function stageState(m: MilestoneProgress, done: boolean, current: MilestoneProgress | null): string {
  if (done) return "done";
  return m.id === current?.id ? "current" : "ahead";
}

/** Веха, которую делают сейчас: первая незакрытая. */
function currentMilestone(p: ProjectProgress): MilestoneProgress | null {
  return p.milestones.find((m) => m.closed < m.tasks) ?? null;
}

function stageStripTpl(p: ProjectProgress): TemplateResult | typeof nothing {
  if (!p.milestones.length) return nothing;
  const current = currentMilestone(p);
  return html`
    <ol class="project-stages" aria-label="Milestones">
      ${p.milestones.map((m) => {
        const done = m.tasks > 0 && m.closed === m.tasks;
        const share = m.tasks ? Math.round((m.closed / m.tasks) * 100) : 0;
        const state = stageState(m, done, current);
        return html`
          <li class="project-stage ${state}" title=${`${m.title} — ${m.closed}/${m.tasks}`}>
            <span class="project-stage-bar"><span style=${`width:${share}%`}></span></span>
            <span class="project-stage-name">${m.id}</span>
            <span class="project-stage-count">${m.closed}/${m.tasks}</span>
          </li>
        `;
      })}
    </ol>
  `;
}

/**
 * В поле «Размер» у задач лежит не только длительность, но и пояснение с разметкой.
 * На карточке нужна оценка, а не абзац: берём первый отрезок и снимаем разметку.
 */
function shortSize(size: string): string {
  const head = (size.split(/[·→,;(]/)[0] ?? "").replace(/\*\*/g, "").replace(/`/g, "").trim();
  if (head.length <= 24) return head;
  // обрезаем по границе слова, иначе на карточке остаётся огрызок
  const cut = head.slice(0, 24);
  return `${cut.slice(0, cut.lastIndexOf(" ") + 1 || 24).trim()}…`;
}

/** Лозенги статуса как в Jira: короткое слово капсом, цвет по колонке процесса. */
const STATE_LOZENGE: Record<string, { label: string; tone: string }> = {
  not_started: { label: "К РАБОТЕ", tone: "todo" },
  claimed: { label: "В РАБОТЕ", tone: "progress" },
  closed: { label: "ГОТОВО", tone: "done" },
};

const BOARD_COLUMNS: { state: string; label: string }[] = [
  { state: "not_started", label: "К работе" },
  { state: "claimed", label: "В работе" },
  { state: "closed", label: "Готово" },
];

function lozengeTpl(state: string): TemplateResult {
  const l = STATE_LOZENGE[state] ?? { label: state.toUpperCase(), tone: "todo" };
  return html`<span class="lozenge ${l.tone}">${l.label}</span>`;
}

/** Название задачи уже начинается с её идентификатора — рядом с ключом он не нужен. */
function taskTitle(id: string, title: string): string {
  return title.replace(new RegExp(`^${id}\\s*[·:-]\\s*`), "");
}

async function openTask(scopeId: string, taskId: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId) return;
  contextsState.taskLoading = taskId;
  drawContexts();
  try {
    const r = await api<{ task: TaskDetail }>(
      `/api/projects/${encodeURIComponent(projectId)}/task?taskId=${encodeURIComponent(taskId)}`,
    );
    contextsState.task = r.task;
    const runs = await api<{ runs: TaskRun[]; events: TaskRunEvent[]; next: string[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/task-run?taskId=${encodeURIComponent(taskId)}`,
    ).catch(() => ({ runs: [], events: [], next: [] }));
    contextsState.taskRuns = runs.runs ?? [];
    contextsState.taskEvents = runs.events ?? [];
    contextsState.taskNext = runs.next ?? [];
    contextsState.runNotice = "";
    contextsState.taskDoc = "";
    contextsState.taskRunDoc = "";
    contextsState.taskDocLoading = true;
    drawContexts();
    // Описание и запись сессии читаются вместе: в дровере они стоят рядом, прыгать некуда.
    const read = (path: string) =>
      api<{ document: { content: string } }>(
        `/api/projects/${encodeURIComponent(projectId)}/document?path=${encodeURIComponent(path)}`,
      ).catch(() => null);
    const [doc, runDoc] = await Promise.all([read(r.task.path), r.task.runPath ? read(r.task.runPath) : null]);
    contextsState.taskDoc = doc?.document.content ?? "";
    contextsState.taskRunDoc = runDoc?.document.content ?? "";
    contextsState.taskDocLoading = false;
  } catch (e) {
    contextsState.task = null;
    contextsState.resourcesNotice = errMessage(e);
  } finally {
    contextsState.taskLoading = "";
    drawContexts();
  }
}

function gateTone(failed: number, unknown: number): string {
  if (failed) return "failed";
  return unknown ? "unknown" : "passed";
}

function gateStripTpl(): TemplateResult | typeof nothing {
  const gates = contextsState.progress?.gates ?? [];
  if (!gates.length) return nothing;
  const byPhase = new Map<string, { phase: string; item: string; state: string; violations: number }[]>();
  for (const gate of gates) {
    const bucket = byPhase.get(gate.phase) ?? [];
    bucket.push(gate);
    byPhase.set(gate.phase, bucket);
  }
  return html`
    <div class="board-gates" aria-label="Гейты">
      ${[...byPhase.entries()].map(([phase, items]) => {
        const failed = items.filter((g) => g.state === "failed" || g.state === "refused").length;
        const unknown = items.filter((g) => g.state === "unknown").length;
        return html`
          <span
            class="board-gate ${gateTone(failed, unknown)}"
            title=${items.map((g) => `${g.item} — ${g.state}`).join("\n")}
          >
            <b>${phase}</b>
            <span>${items.filter((g) => g.state === "passed").length}/${items.length}</span>
          </span>
        `;
      })}
    </div>
  `;
}

function cardTpl(scopeId: string, task: BoardTask): TemplateResult {
  const open = contextsState.task?.id === task.id;
  return html`
    <button
      type="button"
      class="jira-card ${open ? "is-open" : ""} ${task.blockedBy ? "is-blocked" : ""}"
      aria-current=${open ? "true" : "false"}
      @click=${() => void openTask(scopeId, task.id)}
    >
      <span class="jira-card-summary">${taskTitle(task.id, task.title)}</span>
      <span class="jira-card-foot">
        <span class="issue-key">${task.id}</span>
        ${task.kind === "test" ? html`<span class="kind-chip">проверки</span>` : nothing}
        ${
          contextsState.activeRuns[task.id]
            ? html`<span class="run-state ${runTone(contextsState.activeRuns[task.id]!.state)}"
                >${RUN_STATE_LABEL[contextsState.activeRuns[task.id]!.state]}</span
              >`
            : nothing
        }
        ${task.blockedBy ? html`<span class="jira-flag" title="Ждёт другие задачи">⚑ ${task.blockedBy}</span>` : nothing}
        ${shortSize(task.size) ? html`<span class="jira-estimate">${shortSize(task.size)}</span>` : nothing}
      </span>
    </button>
  `;
}

/** Веха, которую делают сейчас: первая незакрытая. На ней степпер стоит по умолчанию. */
function defaultMilestone(milestones: readonly MilestoneProgress[]): string {
  return (milestones.find((m) => m.closed < m.tasks) ?? milestones[milestones.length - 1])?.id ?? "";
}

function milestoneState(m: MilestoneProgress, selected: string, current: string): string {
  if (m.id === selected) return "selected";
  if (m.tasks > 0 && m.closed === m.tasks) return "done";
  return m.id === current ? "current" : "ahead";
}

async function selectMilestone(scopeId: string, milestones: readonly MilestoneProgress[], id: string): Promise<void> {
  contextsState.milestone = id;
  contextsState.milestoneDoc = null;
  drawContexts();
  const projectId = projectIdOfScope(scopeId);
  const milestone = milestones.find((m) => m.id === id);
  if (!projectId || !milestone) return;
  contextsState.milestoneLoading = true;
  try {
    const r = await api<{ document: { content: string } }>(
      `/api/projects/${encodeURIComponent(projectId)}/document?path=${encodeURIComponent(milestone.path)}`,
    );
    contextsState.milestoneDoc = { path: milestone.path, content: r.document.content };
    contextsState.milestoneError = "";
    resetAgents();
    resetRepositories();
  } catch (e) {
    contextsState.milestoneDoc = null;
    contextsState.milestoneError = errMessage(e);
  } finally {
    contextsState.milestoneLoading = false;
    drawContexts();
  }
}

function stepperTpl(scopeId: string, milestones: readonly MilestoneProgress[], selected: string): TemplateResult {
  const current = defaultMilestone(milestones);
  return html`
    <ol class="milestone-stepper" aria-label="Вехи">
      ${milestones.map((m) => {
        const state = milestoneState(m, selected, current);
        const done = m.tasks > 0 && m.closed === m.tasks;
        return html`
          <li class="milestone-step ${state}">
            <button
              type="button"
              aria-current=${m.id === selected ? "step" : "false"}
              title=${`${m.title} — ${m.closed}/${m.tasks}`}
              @click=${() => void selectMilestone(scopeId, milestones, m.id)}
            >
              <span class="milestone-step-mark" aria-hidden="true">${done ? "✓" : m.ord + 1}</span>
              <span class="milestone-step-id">${m.id}</span>
              <span class="milestone-step-count">${m.closed}/${m.tasks}</span>
            </button>
          </li>
        `;
      })}
    </ol>
  `;
}

function milestoneBodyTpl(doc: { path: string; content: string } | null): TemplateResult {
  if (contextsState.milestoneLoading) return html`<p class="context-inline-empty">Читаю описание…</p>`;
  if (!doc) return html`<p class="context-inline-empty">${contextsState.milestoneError || "Описание недоступно."}</p>`;
  return html`${unsafeHTML(renderMarkdownHtml(doc.content))}`;
}

function milestonePanelTpl(scopeId: string, milestone: MilestoneProgress): TemplateResult {
  const projectId = projectIdOfScope(scopeId);
  const doc = contextsState.milestoneDoc;
  return html`
    <section class="milestone-panel" aria-labelledby="milestone-panel-title">
      <div class="milestone-panel-head">
        <h3 id="milestone-panel-title">${milestone.title}</h3>
        <span class="milestone-panel-count">${milestone.closed}/${milestone.tasks}</span>
        <button
          type="button"
          class="btn"
          @click=${() => {
            if (projectId) void openDocumentAt(projectId, milestone.path, drawContexts);
            selectProjectTab(scopeId, "documents");
          }}
        >
          Открыть документ
        </button>
      </div>
      <div class="milestone-panel-body markdown">${milestoneBodyTpl(doc)}</div>
    </section>
  `;
}

function kindLabel(kind: "all" | "dev" | "test"): string {
  if (kind === "dev") return "разработка";
  return kind === "test" ? "проверки" : "все";
}

/** Фильтр по виду задачи применяется до всего остального — и к волнам, и к колонкам. */
function byKind(tasks: readonly BoardTask[]): BoardTask[] {
  if (contextsState.kindFilter === "all") return [...tasks];
  return tasks.filter((task) => task.kind === contextsState.kindFilter);
}

function waveStatus(task: BoardTask): string {
  if (task.state === "closed") return "closed";
  if (task.state === "claimed") return "claimed";
  return task.blockedBy ? "blocked" : "ready";
}

function waveChipTpl(scopeId: string, task: BoardTask, chain: Set<string> | null): TemplateResult {
  const status = waveStatus(task);
  const dimmed = chain && !chain.has(task.id);
  return html`
    <button
      type="button"
      class="wave-chip ${status} ${task.kind === "test" ? "is-test" : "is-code"} ${dimmed ? "dim" : ""} ${contextsState.task?.id === task.id ? "is-open" : ""}"
      title=${`${task.id} · ${taskTitle(task.id, task.title)}`}
      @click=${() => void openTask(scopeId, task.id)}
    >
      <span class="wave-chip-id">${task.id}</span>
    </button>
  `;
}

function waveTpl(scopeId: string, wave: BoardTask[], number: number, chain: Set<string> | null): TemplateResult {
  return html`
    <div class="wave" role="listitem">
      <div class="wave-head">
        <span>волна ${number}</span>
        <span class="wave-count">${wave.length}</span>
      </div>
      <div class="wave-chips">${wave.map((task) => waveChipTpl(scopeId, task, chain))}</div>
    </div>
  `;
}

function wavesTpl(scopeId: string, tasks: readonly BoardTask[]): TemplateResult {
  const layout = layoutWaves(tasks);
  const chain = contextsState.task ? chainOf(tasks, contextsState.task.id) : null;
  return html`
    <div class="waves-summary">
      <span><b>${layout.chainLength}</b> волн с учётом общих артефактов</span>
      <span><b>${layout.idealChain}</b> волн, если бы артефакты не мешали</span>
      <span><b>${layout.peak}</b> задач в самой широкой волне</span>
      <span><b>${layout.ready}</b> можно взять прямо сейчас</span>
      ${chain ? html`<span class="waves-chain-note">в цепочке выбранной — ${chain.size}</span>` : nothing}
    </div>
    ${layout.testWaves ? html`<p class="wave-stage">этап 1 · проверки — волны 1–${layout.testWaves}</p>` : nothing}
    <div class="waves" role="list">
      ${layout.waves.slice(0, layout.testWaves || layout.waves.length).map((wave, index) => waveTpl(scopeId, wave, index + 1, chain))}
    </div>
    ${
      layout.testWaves
        ? html`
            <div class="wave-barrier">
              <span class="wave-barrier-rule"></span>
              <span class="wave-barrier-text">
                <b>Весь трек проверок предшествует всему коду.</b> Правило сильнее пары: не проверка за своей задачей, а
                трек целиком. Пока открыта хоть одна проверка, ни одна задача кода в волну не берётся, сколько бы её
                собственные зависимости ни были закрыты.
              </span>
              <span class="wave-barrier-rule"></span>
            </div>
            <p class="wave-stage">
              этап 2 · код — волны ${layout.testWaves + 1}–${layout.waves.length}, начнутся после закрытия трека
            </p>
            <div class="waves" role="list">
              ${layout.waves.slice(layout.testWaves).map((wave, index) => waveTpl(scopeId, wave, layout.testWaves + index + 1, chain))}
            </div>
          `
        : nothing
    }
  `;
}

function columnsTpl(scopeId: string, tasks: BoardTask[]): TemplateResult {
  return html`
    <div class="board-columns">
      ${BOARD_COLUMNS.map((column) => {
        const inColumn = tasks.filter((t) => t.state === column.state);
        return html`
          <div class="jira-column" aria-label=${column.label}>
            <div class="jira-column-head">${column.label}<span>${inColumn.length}</span></div>
            ${
              inColumn.length
                ? inColumn.map((task) => cardTpl(scopeId, task))
                : html`<p class="jira-column-empty">пусто</p>`
            }
          </div>
        `;
      })}
    </div>
  `;
}

function linkedIssuesTpl(label: string, items: TaskNeighbour[], scopeId: string): TemplateResult | typeof nothing {
  if (!items.length) return nothing;
  return html`
    <div class="issue-block">
      <h4>${label}</h4>
      <ul class="issue-links">
        ${items.map(
          (n) => html`
            <li>
              <button type="button" @click=${() => void openTask(scopeId, n.id)}>
                <span class="issue-key">${n.id}</span>
                <span class="issue-link-summary">${taskTitle(n.id, n.title)}</span>
                ${lozengeTpl(n.state)}
              </button>
            </li>
          `,
        )}
      </ul>
    </div>
  `;
}

function detailRow(label: string, value: TemplateResult | string): TemplateResult {
  return html`
    <div class="issue-field">
      <dt>${label}</dt>
      <dd>${value}</dd>
    </div>
  `;
}

const RUN_STATE_LABEL: Record<string, string> = {
  queued: "в очереди",
  provisioning: "готовим копию",
  running: "в работе",
  awaiting_human: "нужен человек",
  testing: "проверки",
  review: "ревью",
  merging: "слияние",
  done: "готово",
  failed: "сорвалось",
  cancelled: "отменён",
};

/** Тон прогона: остановка ради человека и срыв должны бросаться в глаза. */
function runTone(state: string): string {
  if (state === "awaiting_human" || state === "failed") return "attention";
  if (state === "done") return "done";
  if (state === "queued" || state === "cancelled") return "idle";
  return "live";
}

function agentName(agentId: string): string {
  return contextsState.agentList.find((a) => a.id === agentId)?.name ?? "агент";
}

async function assignTask(scopeId: string, taskId: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  const agentId = contextsState.assignTo || contextsState.agentList[0]?.id;
  if (!projectId || !agentId || contextsState.runBusy) return;
  contextsState.runBusy = true;
  contextsState.runNotice = "";
  drawContexts();
  try {
    await api(`/api/projects/${encodeURIComponent(projectId)}/task-run`, {
      method: "POST",
      body: JSON.stringify({ taskId, agentId }),
    });
    await openTask(scopeId, taskId);
    void loadProjectBoard(scopeId, true);
  } catch (e) {
    contextsState.runNotice = errMessage(e);
  } finally {
    contextsState.runBusy = false;
    drawContexts();
  }
}

async function moveRun(scopeId: string, run: TaskRun, to: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId || contextsState.runBusy) return;
  contextsState.runBusy = true;
  contextsState.runNotice = "";
  drawContexts();
  try {
    await api(`/api/projects/${encodeURIComponent(projectId)}/task-run?runId=${encodeURIComponent(run.id)}`, {
      method: "PATCH",
      body: JSON.stringify({ to }),
    });
    await openTask(scopeId, run.taskId);
    void loadProjectBoard(scopeId, true);
  } catch (e) {
    contextsState.runNotice = errMessage(e);
  } finally {
    contextsState.runBusy = false;
    drawContexts();
  }
}

function timelineTpl(): TemplateResult | typeof nothing {
  if (!contextsState.taskEvents.length) return nothing;
  return html`
    <ol class="run-timeline">
      ${contextsState.taskEvents.map(
        (event) => html`
          <li>
            <span class="run-timeline-state">${RUN_STATE_LABEL[event.toState] ?? event.toState}</span>
            <span class="run-timeline-meta">
              ${event.actor}${event.reason ? html` · ${event.reason}` : nothing} · ${relTime(event.at)}
            </span>
          </li>
        `,
      )}
    </ol>
  `;
}

function descriptionTpl(): TemplateResult {
  if (contextsState.taskDocLoading) return html`<p class="context-inline-empty">Читаю описание…</p>`;
  if (!contextsState.taskDoc) return html`<p class="context-inline-empty">Описание недоступно.</p>`;
  return html`<div class="issue-description markdown">${unsafeHTML(renderMarkdownHtml(contextsState.taskDoc))}</div>`;
}

function executionTpl(scopeId: string, taskId: string): TemplateResult {
  const live = contextsState.taskRuns.find((r) => !["done", "failed", "cancelled"].includes(r.state));
  const past = contextsState.taskRuns.filter((r) => r !== live);
  return html`
    <div class="issue-block">
      <h4>
        Исполнение
        ${contextsState.taskRuns.length ? html`<span class="issue-count">попыток ${contextsState.taskRuns.length}</span>` : nothing}
      </h4>
      ${contextsState.runNotice ? html`<div class="run-notice">${contextsState.runNotice}</div>` : nothing}
      ${
        live
          ? html`
              <div class="run-current">
                <span class="run-state ${runTone(live.state)}">${RUN_STATE_LABEL[live.state] ?? live.state}</span>
                <span class="run-agent">${agentName(live.agentId)}</span>
                <span class="run-attempt">попытка ${live.attempt}</span>
              </div>
              ${timelineTpl()}
              <div class="run-actions">
                ${contextsState.taskNext.map(
                  (to) => html`
                    <button
                      type="button"
                      class="btn"
                      ?disabled=${contextsState.runBusy}
                      @click=${() => void moveRun(scopeId, live, to)}
                    >
                      ${RUN_STATE_LABEL[to] ?? to}
                    </button>
                  `,
                )}
              </div>
            `
          : html`
              ${
                contextsState.agentList.length
                  ? html`<div class="run-assign">
                      ${fieldSelect({
                        ariaLabel: "Кому назначить",
                        value: contextsState.assignTo || contextsState.agentList[0]!.id,
                        onChange: (value) => {
                          contextsState.assignTo = value;
                        },
                        options: contextsState.agentList.map(
                          (a) => html`<option value=${a.id}>${a.name} · до ${a.concurrency}</option>`,
                        ),
                      })}
                      <button
                        type="button"
                        class="btn primary"
                        ?disabled=${contextsState.runBusy}
                        @click=${() => void assignTask(scopeId, taskId)}
                      >
                        Назначить
                      </button>
                    </div>`
                  : html`<p class="context-inline-empty">Нет включённых агентов — заведите его во вкладке «Агенты».</p>`
              }
              ${past.length ? timelineTpl() : nothing}
            `
      }
    </div>
  `;
}

function issueViewTpl(scopeId: string, context: CoreContext): TemplateResult {
  if (contextsState.taskLoading) return html`<div class="empty compact">Читаю задачу…</div>`;
  const task = contextsState.task;
  if (!task) return html`<p class="context-inline-empty">Выберите задачу на доске.</p>`;
  const checks = task.requirements.reduce((n, r) => n + r.checks.length, 0);

  return html`
    <div class="issue-view">
      <nav class="issue-breadcrumb" aria-label="Путь">
        <span>${context.name ?? "проект"}</span><span>/</span><span>${task.milestoneId}</span><span>/</span>
        <span class="issue-key">${task.id}</span>
      </nav>
      <h3 class="issue-summary">${taskTitle(task.id, task.title)}</h3>

      <div class="issue-actions">
        <button type="button" class="btn primary" @click=${() => startChatIn(context)}>Обсудить</button>
      </div>

      <div class="issue-body">
        <div class="issue-main">
          <div class="issue-block">
            <h4>Описание</h4>
            ${descriptionTpl()}
          </div>
          ${executionTpl(scopeId, task.id)}
          ${
            task.runPath
              ? html`<div class="issue-block">
                  <h4>Сессия разработки</h4>
                  ${
                    contextsState.taskRunDoc
                      ? html`<div class="issue-description markdown">
                          ${unsafeHTML(renderMarkdownHtml(contextsState.taskRunDoc))}
                        </div>`
                      : html`<p class="context-inline-empty">Запись недоступна.</p>`
                  }
                </div>`
              : nothing
          }
          ${linkedIssuesTpl("Заблокирована", task.dependsOn, scopeId)}
          ${linkedIssuesTpl("Блокирует", task.blocks, scopeId)}
          <div class="issue-block">
            <h4>Требования и проверки <span class="issue-count">${task.requirements.length} · ${checks}</span></h4>
            ${
              task.requirements.length
                ? html`<ul class="issue-requirements">
                    ${task.requirements.map(
                      (r) => html`
                        <li>
                          <div class="issue-requirement-head">
                            <span class="issue-key">${r.id}</span>
                            <span class="issue-requirement-text">${r.text}</span>
                          </div>
                          ${
                            r.checks.length
                              ? html`<div class="issue-checks">
                                  ${r.checks.map((c) => html`<span class="issue-check" title=${c.spec}>${c.id}</span>`)}
                                </div>`
                              : html`<div class="issue-checks empty">проверок нет</div>`
                          }
                        </li>
                      `,
                    )}
                  </ul>`
                : html`<p class="context-inline-empty">Требования не указаны.</p>`
            }
          </div>
        </div>

        <aside class="issue-details" aria-label="Детали">
          <div class="issue-details-head">Детали</div>
          <dl>
            ${detailRow("Статус", lozengeTpl(task.state))} ${detailRow("Этап", task.milestoneId)}
            ${task.size ? detailRow("Оценка", shortSize(task.size)) : nothing}
            ${detailRow("Требований", String(task.requirements.length))} ${detailRow("Проверок", String(checks))}
            ${task.dependsOn.length ? detailRow("Ждёт", String(task.dependsOn.length)) : nothing}
            ${task.closingCommit ? detailRow("Коммит", html`<code>${task.closingCommit.slice(0, 10)}</code>`) : nothing}
            ${detailRow("Прогон", task.runPath ? "есть" : "не начинался")}
          </dl>
        </aside>
      </div>
    </div>
  `;
}

/** Подпись под пунктом гейта — действие человека, поэтому после неё сводка перечитывается. */
/** Открывает документы фазы в рабочем месте: подписывают прочитанное, а не название пункта. */
async function openPhaseReading(scopeId: string, phase: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId) return;
  try {
    const r = await api<{ documents: { path: string }[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/gate/reading?phase=${encodeURIComponent(phase)}`,
    );
    const first = r.documents[0];
    if (!first) {
      contextsState.resourcesNotice = `У ${phase} нет документов для чтения`;
      return drawContexts();
    }
    await openDocumentAt(projectId, first.path, drawContexts);
    selectProjectTab(scopeId, "documents");
  } catch (e) {
    contextsState.resourcesNotice = errMessage(e);
    drawContexts();
  }
}

async function signGate(scopeId: string, phase: string, item: string, revoke: boolean): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId || contextsState.signBusy) return;
  contextsState.signBusy = true;
  drawContexts();
  try {
    await api(`/api/projects/${encodeURIComponent(projectId)}/gate/sign`, {
      method: "POST",
      body: JSON.stringify({ phase, item, revoke }),
    });
    await loadProjectProgress(scopeId);
  } catch (e) {
    contextsState.resourcesNotice = errMessage(e);
  } finally {
    contextsState.signBusy = false;
    drawContexts();
  }
}

function closeIssueDrawer(): void {
  contextsState.task = null;
  contextsState.taskLoading = "";
  drawContexts();
}

/** Задача открывается дровером поверх доски: нативный dialog даёт Esc и ловушку фокуса. */
function issueDrawerTpl(scopeId: string, context: CoreContext): TemplateResult | typeof nothing {
  if (!contextsState.task && !contextsState.taskLoading) return nothing;
  return html`
    <dialog
      class="issue-drawer"
      aria-label="Задача"
      @close=${closeIssueDrawer}
      @click=${(event: MouseEvent) =>
        event.target === event.currentTarget && (event.currentTarget as HTMLDialogElement).close()}
    >
      <div class="issue-drawer-body">
        <button class="issue-drawer-close" type="button" aria-label="Закрыть" @click=${closeIssueDrawer}>
          ${icon(X, 16)}
        </button>
        ${issueViewTpl(scopeId, context)}
      </div>
    </dialog>
  `;
}

function projectBoardSection(scopeId: string, context: CoreContext): TemplateResult | typeof nothing {
  if (contextsState.progressScope !== scopeId) return nothing;
  if (contextsState.boardLoading && !contextsState.board.length)
    return html`<div class="empty compact">Читаю задачи…</div>`;
  const tasks = byKind(contextsState.board);
  if (!contextsState.board.length) return html`<div class="empty compact">В плане проекта пока нет задач.</div>`;
  const milestones = contextsState.progress?.milestones ?? [];
  const selected = contextsState.milestone || defaultMilestone(milestones);
  if (!contextsState.milestone && selected) void selectMilestone(scopeId, milestones, selected);
  const milestone = milestones.find((m) => m.id === selected);
  const inMilestone = tasks.filter((t) => t.milestoneId === selected);
  const waves = contextsState.boardView === "waves";
  return html`
    <div class="board-view">
      ${gateStripTpl()}
      <div class="board-view-switch" role="tablist" aria-label="Вид доски">
        ${(["waves", "columns"] as const).map(
          (view) => html`
            <button
              type="button"
              role="tab"
              aria-selected=${contextsState.boardView === view ? "true" : "false"}
              class="board-view-tab ${contextsState.boardView === view ? "active" : ""}"
              @click=${() => {
                contextsState.boardView = view;
                drawContexts();
              }}
            >
              ${view === "waves" ? "Волны" : "Колонки"}
            </button>
          `,
        )}
      </div>
      <div class="board-view-switch" role="tablist" aria-label="Вид задач">
        ${(["all", "dev", "test"] as const).map(
          (kind) => html`
            <button
              type="button"
              role="tab"
              aria-selected=${contextsState.kindFilter === kind ? "true" : "false"}
              class="board-view-tab ${contextsState.kindFilter === kind ? "active" : ""}"
              @click=${() => {
                contextsState.kindFilter = kind;
                drawContexts();
              }}
            >
              ${kindLabel(kind)}
            </button>
          `,
        )}
      </div>
      ${waves ? nothing : stepperTpl(scopeId, milestones, selected)}
      <div class="board-main">
        ${
          waves
            ? wavesTpl(scopeId, tasks)
            : html`${milestone ? milestonePanelTpl(scopeId, milestone) : nothing} ${columnsTpl(scopeId, inMilestone)}`
        }
      </div>
      ${issueDrawerTpl(scopeId, context)}
    </div>
  `;
}

function progressTile(label: string, value: string, hint: string): TemplateResult {
  return html`
    <div class="context-progress-tile">
      <span class="context-progress-value">${value}</span>
      <span class="context-progress-label">${label}</span>
      ${hint ? html`<span class="context-progress-hint">${hint}</span>` : nothing}
    </div>
  `;
}

/**
 * Хребет проекта грузится один раз на проект: цепочка и находки приходят двумя
 * запросами, но показываются вместе — по отдельности они не отвечают на вопрос
 * «где рвётся».
 */
async function loadChain(scopeId: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  if (!projectId || contextsState.chainScope === scopeId || contextsState.chainLoading) return;
  contextsState.chainLoading = true;
  try {
    const base = `/api/projects/${encodeURIComponent(projectId)}`;
    const [chain, found] = await Promise.all([
      api<{ links: ChainLink[]; joints: ChainJoint[] }>(`${base}/chain`).catch(() => ({ links: [], joints: [] })),
      api<{ findings: ProjectFinding[] }>(`${base}/entities/findings`).catch(() => ({ findings: [] })),
    ]);
    contextsState.chainScope = scopeId;
    contextsState.chainLinks = chain.links ?? [];
    contextsState.chainJoints = chain.joints ?? [];
    contextsState.chainFindings = found.findings ?? [];
  } finally {
    contextsState.chainLoading = false;
    drawContexts();
  }
}

function projectChainSection(scopeId: string): TemplateResult | typeof nothing {
  if (!projectIdOfScope(scopeId)) return nothing;
  if (contextsState.chainScope !== scopeId) {
    void loadChain(scopeId);
    return nothing;
  }
  if (!contextsState.chainLinks.length) return nothing;
  // Сумма по находкам — это записи, а не разрывы: восемь родов находок и 139 строк в них.
  const records = contextsState.chainFindings.reduce((n, f) => n + f.count, 0);
  return html`
    <section class="context-panel" aria-labelledby="context-chain-title">
      <div class="context-panel-heading context-resource-heading">
        <h2 class="context-panel-title" id="context-chain-title">Где рвётся</h2>
        <span class="context-panel-count">${contextsState.chainFindings.length}</span>
        <span class="context-resource-note">${records} записей · цепочка от потребности до прогона</span>
      </div>
      ${chainTpl(contextsState.chainLinks, contextsState.chainJoints)}
      ${chainFindingsTpl(
        contextsState.chainFindings,
        (finding) => {
          // Вид находки и есть вид сущности — ведём прямо в её список.
          contextsState.tab = "documents";
          syncUrlFromState();
          drawContexts();
          const projectId = projectIdOfScope(scopeId);
          if (!projectId) return;
          const keys = keysOfDetail((finding.detail ?? []) as EntityRow[]);
          void openEntityKind(
            projectId,
            finding.kind,
            drawContexts,
            keys.size ? { keys, label: finding.item, records: finding.count } : undefined,
          );
        },
        (finding) => {
          // Разговор идёт за предметом: просьба уже написана и названа точно,
          // а чат открывается в области этого проекта, а не в общей.
          saveDraft(newChatDraftKey(appState.me?.user), discussionPrompt(finding));
          openBlankInFocusedPane(scopeId);
        },
      )}
    </section>
  `;
}

function projectProgressSection(scopeId: string): TemplateResult | typeof nothing {
  if (contextsState.progressScope !== scopeId) return nothing;
  const p = contextsState.progress;
  if (!p || p.documents === 0) return nothing;

  const current = currentMilestone(p);
  const gatesPassed = p.gates.filter((g) => g.state === "passed").length;
  const failing = p.gates.filter((g) => g.state === "failed" || g.state === "refused");

  return html`
    <section class="context-panel context-progress" aria-labelledby="context-progress-title">
      <div class="context-panel-heading context-resource-heading">
        <h2 class="context-panel-title" id="context-progress-title">Progress</h2>
        <span class="context-progress-stage">
          ${p.version ? html`<b>${p.version}</b>` : nothing} ${stageSummaryTpl(p, current)}
        </span>
      </div>
      ${stageStripTpl(p)}
      <div class="context-progress-tiles">
        ${
          // Задачи и требования уже названы хребтом выше — повторять их здесь значит
          // заставлять сверять две записи об одном. Остаются гейты и готовность к взятию.
          p.plan && p.plan.tasks > 0
            ? progressTile(
                "Ready to take",
                String(p.plan.ready),
                `${p.plan.claimed} in flight · ${p.plan.tests} на проверки`,
              )
            : nothing
        }
        ${p.gates.length ? progressTile("Gates passing", `${gatesPassed}/${p.gates.length}`, "") : nothing}
      </div>
      ${
        failing.length
          ? html`<ul class="context-progress-failing">
              ${failing.map(
                (g) => html`<li><b>${g.phase}</b> ${g.item}${g.violations ? html` — ${g.violations}` : nothing}</li>`,
              )}
            </ul>`
          : nothing
      }
    </section>
  `;
}

function projectDocumentsSection(scopeId: string): TemplateResult | typeof nothing {
  if (contextsState.progressScope !== scopeId) return nothing;
  if ((contextsState.progress?.documents ?? 0) === 0) return nothing;
  // Без панельной обёртки: рабочее место занимает экран целиком, как в Obsidian.
  return html`<div class="context-documents-full">${documentsWorkspaceTpl()}</div>`;
}

function resourceSections(scopeId: string): TemplateResult | typeof nothing {
  if (contextsState.resourcesScope !== scopeId) return html``;
  if (contextsState.resourcesNotice) return html`<div class="status">${contextsState.resourcesNotice}</div>`;
  const r = contextsState.resources;
  if (!r) {
    return contextsState.resourcesLoading
      ? html`<div class="empty compact">Loading this context's files, webhooks, crons, apps and skills…</div>`
      : html``;
  }
  if (
    r.files.length === 0 &&
    r.webhooks.length === 0 &&
    r.crons.length === 0 &&
    r.deployments.length === 0 &&
    r.skills.length === 0
  ) {
    return nothing;
  }
  const manage = r.manageable;
  return html`
    ${r.files.length ? resourceGroup("Files", r.files.map(fileRow)) : nothing}
    ${
      r.skills.length
        ? resourceGroup(
            "Skills",
            r.skills.map((s) => skillRow(s, manage)),
          )
        : nothing
    }
    ${
      r.crons.length
        ? resourceGroup(
            "Crons",
            r.crons.map((c) => cronRow(c, manage)),
          )
        : nothing
    }
    ${r.webhooks.length ? resourceGroup("Webhooks", r.webhooks.map(webhookRow)) : nothing}
    ${r.deployments.length ? resourceGroup("Apps", r.deployments.map(deploymentRow)) : nothing}
  `;
}

const resourceBusy = new Set<string>();

async function manageCron(id: string, action: "enable" | "disable" | "delete"): Promise<void> {
  const key = `cron:${id}`;
  if (resourceBusy.has(key)) return;
  if (action === "delete" && !confirm("Delete this cron? This can't be undone.")) return;
  resourceBusy.add(key);
  drawContexts();
  try {
    if (action === "delete") await api(`/api/crons/${encodeURIComponent(id)}`, { method: "DELETE" });
    else await api(`/api/crons/${encodeURIComponent(id)}/${action}`, { method: "POST" });
    const scope = contextsState.resourcesScope;
    if (scope) await loadScopeResources(scope);
  } catch (e) {
    contextsState.resourcesNotice = errMessage(e, "Couldn't update that cron.");
  } finally {
    resourceBusy.delete(key);
    drawContexts();
  }
}

async function deleteScopeSkill(id: string): Promise<void> {
  const key = `skill:${id}`;
  if (resourceBusy.has(key)) return;
  if (!confirm("Delete this skill? This can't be undone.")) return;
  resourceBusy.add(key);
  drawContexts();
  try {
    await api(`/api/skills/${encodeURIComponent(id)}`, { method: "DELETE" });
    const scope = contextsState.resourcesScope;
    if (scope) await loadScopeResources(scope);
  } catch (e) {
    contextsState.resourcesNotice = errMessage(e, "Couldn't delete that skill.");
  } finally {
    resourceBusy.delete(key);
    drawContexts();
  }
}

function resourceGroup(label: string, rows: TemplateResult[]): TemplateResult {
  const view = label === "Apps" ? "deploys" : label.toLowerCase();
  const scope = contextsState.resourcesScope;
  const supportsScopeLink = view === "files" || view === "deploys";
  const href = `${UI_BASE}/${encodeURIComponent(view)}${scope && supportsScopeLink ? `?scope=${encodeURIComponent(scope)}` : ""}`;
  return html`
    <section class="context-panel context-resource-group">
      <div class="context-panel-heading context-resource-heading">
        <h2 class="context-panel-title">${label}</h2>
        <a href=${href}>View all</a>
      </div>
      <div class="context-session-list">${rows}</div>
    </section>
  `;
}

function fileRow(f: ScopeFile): TemplateResult {
  return html`
    <div class="context-session-row context-resource-row">
      <span class="context-session-title">${f.name}</span>
      <span class="context-session-meta">
        <span>${formatBytes(f.sizeBytes)}</span>
        <span>${relTime(f.createdAt)}</span>
        ${
          f.openable
            ? html`<a
                class="context-resource-link"
                href=${withBase(`/api/files/${encodeURIComponent(f.id)}/content`)}
                target="_blank"
                rel="noreferrer"
                >Open</a
              >`
            : nothing
        }
      </span>
    </div>
  `;
}

function webhookRow(w: WebhookView): TemplateResult {
  let lastRun = "never fired";
  if (w.lastError) lastRun = "error";
  else if (w.lastFiredAt) lastRun = relTime(w.lastFiredAt);
  return html`
    <div class="context-session-row context-resource-row">
      <span class="context-session-title">${actionSnippet(w.action)}</span>
      <span class="context-session-meta">
        <span class="badge">${w.verification.scheme}</span>
        <span class="badge">${w.enabled ? "enabled" : "disabled"}</span>
        <span>${lastRun}</span>
      </span>
    </div>
  `;
}

function cronRow(c: CronView, manage = false): TemplateResult {
  let status = "disabled";
  if (c.archived) status = "archived";
  else if (c.enabled) status = "enabled";
  const busy = resourceBusy.has(`cron:${c.id}`);
  return html`
    <div class="context-session-row context-resource-row">
      <span class="context-session-title">${c.title ?? actionSnippet(c.message ?? c.action ?? "")}</span>
      <span class="context-session-meta">
        <span class="badge">${cronScheduleSummary(c)}</span>
        <span class="badge">${status}</span>
        <span title=${cronRunSummaryTitle(c)}>${cronRunSummary(c)}</span>
        ${
          manage && !c.archived
            ? html`
                <button
                  class="context-resource-action"
                  type="button"
                  ?disabled=${busy}
                  @click=${() => void manageCron(c.id, c.enabled ? "disable" : "enable")}
                >
                  ${c.enabled ? "Disable" : "Enable"}
                </button>
                <button
                  class="context-resource-action danger"
                  type="button"
                  ?disabled=${busy}
                  @click=${() => void manageCron(c.id, "delete")}
                >
                  Delete
                </button>
              `
            : nothing
        }
      </span>
    </div>
  `;
}

function skillRow(s: ScopeSkill, manage = false): TemplateResult {
  const busy = resourceBusy.has(`skill:${s.id}`);
  return html`
    <div class="context-session-row context-resource-row">
      <span class="context-session-title">${s.name}</span>
      <span class="context-session-meta">
        ${s.description ? html`<span class="context-resource-desc">${s.description}</span>` : nothing}
        <span class="badge">${s.status}</span>
        ${
          manage
            ? html`<button
                class="context-resource-action danger"
                type="button"
                ?disabled=${busy}
                @click=${() => void deleteScopeSkill(s.id)}
              >
                Delete
              </button>`
            : nothing
        }
      </span>
    </div>
  `;
}

function deploymentRow(d: ScopeDeployment): TemplateResult {
  return html`
    <div class="context-session-row context-resource-row">
      <span class="context-session-title">${d.name}</span>
      <span class="context-session-meta">
        <span class="badge">v${d.currentVersion}</span>
        <span class="badge">${d.status}</span>
        <span class="badge">${d.permission === "write" ? "manage" : "read"}</span>
      </span>
    </div>
  `;
}

function openCreateProject(): void {
  createProjectOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  contextsState.createOpen = true;
  contextsState.createName = "";
  contextsState.createError = "";
  drawContexts();
  queueMicrotask(() => document.querySelector<HTMLInputElement>("#project-name")?.focus());
}

function closeCreateProject(): void {
  createProjectSeq++;
  contextsState.createOpen = false;
  contextsState.createSaving = false;
  contextsState.createName = "";
  contextsState.createError = "";
  drawContexts();
  queueMicrotask(() => {
    const target = createProjectOpener;
    createProjectOpener = null;
    restoreDialogFocus(target, () => document.querySelector<HTMLElement>(".project-create-button"));
  });
}

function createProjectDialog(): TemplateResult | typeof nothing {
  if (!contextsState.createOpen) return nothing;
  return html`
    <dialog
      class="project-dialog"
      aria-labelledby="project-dialog-title"
      @close=${closeCreateProject}
      @click=${(event: MouseEvent) => event.target === event.currentTarget && (event.currentTarget as HTMLDialogElement).close()}
    >
      <form @submit=${(event: SubmitEvent) => void createProject(event)}>
        <div class="project-dialog-head">
          <span class="context-glyph large">${icon(FolderPlus, 21)}</span>
          <div><h2 id="project-dialog-title">New project</h2></div>
          <button
            class="project-icon-button"
            type="button"
            aria-label="Close new project"
            title="Close"
            @click=${closeCreateProject}
          >
            ${icon(X, 16)}
          </button>
        </div>
        <label class="project-name-field" for="project-name">
          <span>Name</span>
          <input
            id="project-name"
            data-focus-key="project-name"
            name="name"
            maxlength="200"
            autocomplete="off"
            placeholder="launch cohort"
            .value=${contextsState.createName}
            ?disabled=${contextsState.createSaving}
            @input=${(event: InputEvent) => {
              contextsState.createName = (event.currentTarget as HTMLInputElement).value;
              contextsState.createError = "";
            }}
          />
        </label>
        <div class="form-error" aria-live="polite">${contextsState.createError}</div>
        <div class="project-dialog-actions">
          <button class="btn" type="button" @click=${closeCreateProject}>
            ${contextsState.createSaving ? "Close" : "Cancel"}
          </button>
          <button class="btn primary" type="submit" ?disabled=${contextsState.createSaving}>
            ${icon(FolderPlus, 15)}<span>${contextsState.createSaving ? "Creating…" : "Create project"}</span>
          </button>
        </div>
      </form>
    </dialog>
  `;
}

export function openProjectDetail(scopeId: string): void {
  switchView("contexts");
  selectContext(scopeId);
}

export async function renameProject(project: CoreProject, name: string): Promise<boolean> {
  try {
    const updated = projectFromResponse(
      await api(`/api/projects/${encodeURIComponent(project.id)}`, { method: "PATCH", body: JSON.stringify({ name }) }),
    );
    if (!updated) return false;
    upsertProject(updated);
    if (appState.currentView === "contexts") drawContexts();
    return true;
  } catch {
    return false;
  }
}

function projectFromResponse(response: unknown): CoreProject | null {
  const project = (response as { project?: CoreProject } | null)?.project;
  if (
    !project ||
    !project.id ||
    !project.name ||
    !project.ownerId ||
    !project.scopeId?.trim() ||
    !Array.isArray(project.memberIds) ||
    !Array.isArray(project.members) ||
    project.members.some((member) => !member?.principalId || !member.displayName)
  )
    return null;
  return project;
}

function upsertProject(project: CoreProject): CoreContext {
  const loaded = contextsState.loaded;
  contextsFetchSeq++;
  const scopeId = project.scopeId;
  const current = contextsState.list.find((context) => context.scopeId === scopeId);
  const next: CoreContext = {
    ...current,
    scopeId,
    kind: "group",
    name: project.name,
    sessionCount: current?.sessionCount ?? 0,
    lastActivityAt: current?.lastActivityAt ?? null,
    project,
  };
  contextsState.list = [next, ...contextsState.list.filter((context) => context.scopeId !== scopeId)];
  contextsState.loaded = loaded;
  if (loaded) contextsState.loadedAt = Date.now();
  return next;
}

async function createProject(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  if (contextsState.createSaving) return;
  const name = contextsState.createName.trim();
  if (!name) {
    contextsState.createError = "Enter a project name.";
    drawContexts();
    queueMicrotask(() => document.querySelector<HTMLInputElement>("#project-name")?.focus());
    return;
  }
  const seq = ++createProjectSeq;
  const resetSeq = contextsResetSeq;
  contextsState.createSaving = true;
  drawContexts();
  try {
    const project = projectFromResponse(await api("/api/projects", { method: "POST", body: JSON.stringify({ name }) }));
    if (resetSeq !== contextsResetSeq) return;
    if (!project) throw new Error("Core returned an invalid project");
    const loaded = contextsState.loaded;
    let context = upsertProject(project);
    if (!loaded) {
      await fetchContexts().catch(() => contextsState.list);
      if (resetSeq !== contextsResetSeq) return;
      context = upsertProject(project);
    }
    if (seq !== createProjectSeq) {
      if (appState.currentView === "contexts") drawContexts();
      return;
    }
    contextsState.createOpen = false;
    contextsState.createSaving = false;
    contextsState.createName = "";
    selectContext(context.scopeId);
  } catch (error) {
    if (seq !== createProjectSeq || resetSeq !== contextsResetSeq) return;
    contextsState.createSaving = false;
    contextsState.createError = errMessage(error, "Couldn't create that project.");
    drawContexts();
    queueMicrotask(() => document.querySelector<HTMLInputElement>("#project-name")?.focus());
  }
}

function toggleMemberPicker(context: CoreContext): void {
  memberSearchSeq++;
  cancelMemberSearchTimer();
  contextsState.memberProjectId =
    contextsState.memberProjectId === context.project?.id ? null : (context.project?.id ?? null);
  contextsState.memberQuery = "";
  contextsState.memberMatches = [];
  contextsState.memberSearching = false;
  contextsState.memberError = "";
  contextsState.memberSearchedQuery = "";
  contextsState.slackEditing = false;
  contextsState.slackValue = "";
  contextsState.slackBusy = false;
  contextsState.slackError = "";
  drawContexts();
  if (contextsState.memberProjectId)
    queueMicrotask(() => document.querySelector<HTMLInputElement>("#project-member-search")?.focus());
}

function closeMemberPicker(): void {
  memberSearchSeq++;
  cancelMemberSearchTimer();
  contextsState.memberProjectId = null;
  contextsState.memberQuery = "";
  contextsState.memberMatches = [];
  contextsState.memberSearching = false;
  contextsState.memberError = "";
  contextsState.memberSearchedQuery = "";
  contextsState.slackEditing = false;
  contextsState.slackValue = "";
  contextsState.slackBusy = false;
  contextsState.slackError = "";
  drawContexts();
}

function scheduleMemberSearch(context: CoreContext): void {
  cancelMemberSearchTimer();
  memberSearchSeq++;
  const hadVisibleState =
    contextsState.memberSearching || contextsState.memberError !== "" || contextsState.memberSearchedQuery !== "";
  contextsState.memberSearching = false;
  contextsState.memberError = "";
  contextsState.memberSearchedQuery = "";
  contextsState.slackEditing = false;
  contextsState.slackValue = "";
  contextsState.slackBusy = false;
  contextsState.slackError = "";
  const query = contextsState.memberQuery.trim();
  if (query.length < 2) {
    if (hadVisibleState || contextsState.memberMatches.length) {
      contextsState.memberMatches = [];
      drawContexts();
    }
    return;
  }
  memberSearchTimer = setTimeout(() => {
    memberSearchTimer = undefined;
    void runMemberSearch(context, query);
  }, MEMBER_SEARCH_DEBOUNCE_MS);
  if (hadVisibleState) drawContexts();
}

async function searchProjectMembers(event: SubmitEvent, context: CoreContext): Promise<void> {
  event.preventDefault();
  if (!context.project || contextsState.memberBusy) return;
  cancelMemberSearchTimer();
  const input = (event.currentTarget as HTMLFormElement).elements.namedItem("query") as HTMLInputElement | null;
  const query = input?.value.trim() ?? "";
  contextsState.memberQuery = query;
  contextsState.memberError = "";
  if (query.length < 2) {
    contextsState.memberMatches = [];
    contextsState.memberSearchedQuery = "";
    contextsState.memberError = "Enter at least two characters.";
    drawContexts();
    return;
  }
  await runMemberSearch(context, query);
}

async function runMemberSearch(context: CoreContext, query: string): Promise<void> {
  if (!context.project || contextsState.memberBusy) return;
  if (contextsState.memberProjectId !== context.project.id) return;
  const projectId = context.project.id;
  const searchSeq = ++memberSearchSeq;
  contextsState.memberSearching = true;
  drawContexts();
  try {
    const response = await api<{ matches?: DirectoryMatch[] }>(`/api/directory/resolve?q=${encodeURIComponent(query)}`);
    if (searchSeq !== memberSearchSeq || contextsState.memberProjectId !== projectId) return;
    contextsState.memberMatches = (response.matches ?? []).filter((match) => match.type === "internal");
    contextsState.memberSearchedQuery = query;
  } catch (error) {
    if (searchSeq !== memberSearchSeq || contextsState.memberProjectId !== projectId) return;
    contextsState.memberSearchedQuery = "";
    contextsState.memberError = errMessage(error, "Couldn't search for people.");
  } finally {
    if (searchSeq === memberSearchSeq) {
      contextsState.memberSearching = false;
      drawContexts();
    }
  }
}

async function addProjectMember(context: CoreContext, member: DirectoryMatch): Promise<void> {
  if (!context.project || contextsState.memberBusy) return;
  const resetSeq = contextsResetSeq;
  memberSearchSeq++;
  cancelMemberSearchTimer();
  contextsState.memberSearching = false;
  contextsState.memberBusy = true;
  contextsState.memberError = "";
  drawContexts();
  try {
    const response = await api(`/api/projects/${encodeURIComponent(context.project.id)}/members`, {
      method: "POST",
      body: JSON.stringify({ memberId: member.principalId }),
    });
    if (resetSeq !== contextsResetSeq) return;
    const project = projectFromResponse(response);
    if (!project) throw new Error("Core returned an invalid project");
    upsertProject(project);
    contextsState.memberQuery = "";
    contextsState.memberMatches = [];
    contextsState.memberSearchedQuery = "";
  } catch (error) {
    if (resetSeq !== contextsResetSeq) return;
    contextsState.memberError = errMessage(error, "Couldn't add that person.");
  } finally {
    if (resetSeq === contextsResetSeq) {
      contextsState.memberBusy = false;
      drawContexts();
    }
  }
}

async function removeProjectMember(context: CoreContext, principalId: string): Promise<void> {
  if (!context.project || contextsState.memberBusy) return;
  const label = memberLabel(context, principalId);
  if (!window.confirm(`Remove ${label} from ${context.name || "this project"}?`)) return;
  const resetSeq = contextsResetSeq;
  memberSearchSeq++;
  cancelMemberSearchTimer();
  contextsState.memberSearching = false;
  contextsState.memberBusy = true;
  contextsState.memberError = "";
  drawContexts();
  try {
    const response = await api(
      `/api/projects/${encodeURIComponent(context.project.id)}/members/${encodeURIComponent(principalId)}`,
      { method: "DELETE" },
    );
    if (resetSeq !== contextsResetSeq) return;
    const project = projectFromResponse(response);
    if (!project) throw new Error("Core returned an invalid project");
    upsertProject(project);
  } catch (error) {
    if (resetSeq !== contextsResetSeq) return;
    contextsState.memberError = errMessage(error, "Couldn't remove that person.");
  } finally {
    if (resetSeq === contextsResetSeq) {
      contextsState.memberBusy = false;
      drawContexts();
    }
  }
}

/** Сводка живёт отдельно от ресурсов: у проекта без плана её просто нет, и это не ошибка. */
async function loadProjectProgress(scopeId: string): Promise<void> {
  const projectId = projectIdOfScope(scopeId);
  contextsState.progress = null;
  contextsState.progressScope = scopeId;
  if (!projectId) return;
  void mountProjectDocuments(projectId, drawContexts);
  contextsState.progressLoading = true;
  const seq = appState.viewRenderSeq;
  const stale = () =>
    seq !== appState.viewRenderSeq || appState.currentView !== "contexts" || contextsState.selected !== scopeId;
  try {
    const r = await api<ProjectProgress>(`/api/projects/${encodeURIComponent(projectId)}/progress`);
    if (!stale()) contextsState.progress = r;
  } catch {
    // проект может не вести документов — тогда сводки нет, и панель не показывается
  } finally {
    if (!stale()) {
      contextsState.progressLoading = false;
      drawContexts();
    }
  }
}

async function loadScopeResources(scopeId: string): Promise<void> {
  void loadProjectProgress(scopeId);
  contextsState.resources = null;
  contextsState.resourcesScope = scopeId;
  contextsState.resourcesLoading = true;
  contextsState.resourcesNotice = "";
  drawContexts();
  const seq = appState.viewRenderSeq;
  const stale = () =>
    seq !== appState.viewRenderSeq || appState.currentView !== "contexts" || contextsState.selected !== scopeId;
  try {
    const r = await api<ScopeResourcesView>(`/api/scope-resources?scope=${encodeURIComponent(scopeId)}`);
    if (stale()) return;
    contextsState.resources = {
      files: r.files ?? [],
      webhooks: r.webhooks ?? [],
      crons: r.crons ?? [],
      deployments: r.deployments ?? [],
      skills: r.skills ?? [],
      manageable: r.manageable === true,
    };
  } catch (e) {
    if (stale()) return;
    contextsState.resourcesNotice = errMessage(e, "Failed to load this context's resources.");
  } finally {
    if (!stale()) {
      contextsState.resourcesLoading = false;
      drawContexts();
    }
  }
}

function contextSessionRow(s: CoreSession): TemplateResult {
  const surface = surfaceOf(s);
  const readOnly = !isContinuable(s, appState.me?.user ?? "");
  return html`
    <button class="context-session-row" type="button" @click=${() => void openFromContext(s)}>
      <span class="context-session-title">${groupDmTitle(s)}</span>
      <span class="context-session-meta">
        ${surface === "slack" ? html`<span class="surface surface-slack">${slackLogo(13)}</span>` : html`<span class="badge">${surface}</span>`}
        ${readOnly ? html`<span class="ro-lock" title="Read-only — replies happen on the original surface">${icon(Lock, 12)}</span>` : nothing}
        <span>${relTime(activityOf(s))}</span>
      </span>
    </button>
  `;
}

function selectContext(scopeId: string | null): void {
  memberSearchSeq++;
  cancelMemberSearchTimer();
  contextsState.memberProjectId = null;
  contextsState.memberQuery = "";
  contextsState.memberMatches = [];
  contextsState.memberSearching = false;
  contextsState.memberError = "";
  contextsState.memberSearchedQuery = "";
  contextsState.slackEditing = false;
  contextsState.slackValue = "";
  contextsState.slackBusy = false;
  contextsState.slackError = "";
  contextsState.selected = scopeId;
  // Войти в проект — значит оказаться внутри него: место в сайдбаре обязано
  // совпадать с тем, что открыто, иначе проект снова читается как фильтр.
  if (scopeId) setActiveScope(scopeId);
  contextsState.chainScope = null;
  contextsState.resources = null;
  contextsState.resourcesScope = null;
  contextsState.resourcesNotice = "";
  contextsState.resourcesLoading = false;
  resetAmbientPolicy();
  resetContextModel();
  resetChannelHeader();
  syncUrlFromState();
  drawContexts();
  renderSidebarTop();
  if (scopeId) {
    void loadScopeResources(scopeId);
    void loadAmbientPolicy(scopeId, drawContexts);
    void loadContextModel(scopeId, drawContexts);
    void loadChannelHeader(scopeId, drawContexts);
  }
}

function startChatIn(c: CoreContext): void {
  mainConversation().newChat(c.kind === "personal" ? undefined : { scopeId: c.scopeId, name: c.name });
}

async function openFromContext(s: CoreSession): Promise<void> {
  await openSession(s);
}
