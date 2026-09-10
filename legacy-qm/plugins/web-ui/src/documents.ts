import { html, nothing, render, type TemplateResult } from "lit";
import { linkEntities, type EntityRef } from "./entity-links.ts";
import {
  entityTableTpl,
  findingsTpl,
  type EntityFinding,
  type EntityRow,
  type EntitySummary,
  rowKey,
  plural,
} from "./project-entity-catalog.ts";
import { ChevronDown, ChevronRight, File, Folder, FolderOpen } from "lucide";
import { unsafeHTML } from "lit/directives/unsafe-html.js";
import { marked } from "marked";
import { api } from "./core-bridge";
import { errMessage } from "../../chassis/src/errors";
import { listPageTpl } from "./list-page";
import { appState, setActiveScope, scopeOfProjectId, projectIdOfScope } from "./shell";
import { installMarkdownSanitizer } from "./markdown-sanitize";
import { headerFields, splitBlocks, withAwaiting, type DocumentBlock } from "./document-blocks";
import { fieldSelect, icon, relTime } from "./ui";
import { buildTree, documentBytes, matchesQuery, ordered, resolveDocumentPath, type TreeNode } from "./document-tree";

interface DocumentSummary {
  path: string;
  contentHash: string;
  bytes: number;
  revision: number;
  updatedAt: number;
  updatedBy: string;
}

interface SectionView {
  ord: number;
  level: number;
  title: string;
  anchor: string;
  parentOrd: number | null;
  firstBlock: number;
  lastBlock: number;
}

interface FieldView {
  sectionOrd: number;
  name: string;
  value: string;
}

interface LinkView {
  blockOrd: number;
  ord: number;
  text: string;
  targetPath: string;
  targetAnchor: string;
}

interface BacklinkView {
  path: string;
  text: string;
  blockOrd: number;
  targetAnchor: string;
}

interface OpenDocument {
  path: string;
  kind: string;
  id: string | null;
  content: string;
  revision: number;
  updatedAt: number;
  updatedBy: string;
  sections: SectionView[];
  fields: FieldView[];
  links: LinkView[];
}

interface RevisionView {
  revision: number;
  bytes: number;
  writtenAt: number;
  writtenBy: string;
}

interface ProjectSummary {
  id: string;
  name: string;
}

interface ProjectSection {
  kind: string;
  title: string;
  count: number;
  open?: number;
  note?: string;
}

interface SectionDoc {
  path: string;
  id: string | null;
  title: string;
  sections: string[];
  updatedAt: number;
  answered?: boolean;
  accepted?: boolean;
}

type Aside = "outline" | "backlinks" | "history";

const state = {
  projects: [] as ProjectSummary[],
  sections: [] as ProjectSection[],
  entities: [] as EntitySummary[],
  entityKind: "" as string,
  entityRows: [] as EntityRow[],
  entityLoading: false,
  entityFeature: "" as string,
  entityOnly: null as Set<string> | null,
  entityOnlyLabel: "" as string,
  entityOnlyRecords: 0,
  findings: [] as EntityFinding[],
  entityIndex: new Map<string, EntityRef>(),
  kind: "" as string,
  kindPaths: null as Set<string> | null,
  sectionDocs: [] as SectionDoc[],
  sectionLoading: false,
  projectId: "" as string,
  documents: [] as DocumentSummary[],
  open: null as OpenDocument | null,
  revisions: [] as RevisionView[],
  backlinks: [] as BacklinkView[],
  editing: null as { anchor: string; title: string; body: string } | null,
  query: "" as string,
  expanded: new Set<string>(),
  aside: "outline" as Aside,
  notice: "" as string,
  loading: false,
  saving: false,
};

let host: HTMLDivElement | null = null;
/** Рабочее место живёт и своей страницей, и внутри проекта — перерисовку задаёт тот, кто его встроил. */
let redraw: () => void = () => drawStandalone();
let pendingScroll: string | null = null;

export function resetDocuments(): void {
  state.open = null;
  state.revisions = [];
  state.backlinks = [];
  state.editing = null;
  state.notice = "";
}

/** Разметка разбирается заново только при смене документа, а не на каждое нажатие в поиске. */
const rendered = { key: "", html: "" };

function bodyHtml(open: OpenDocument): string {
  const key = `${open.path}@${open.revision}`;
  if (rendered.key !== key) {
    installMarkdownSanitizer();
    rendered.key = key;
    rendered.html = String(marked.parse(open.content, { async: false }));
  }
  return rendered.html;
}

// ─── дерево ────────────────────────────────────────────────────────────────

function countLeaves(node: TreeNode): number {
  if (node.document && node.children.size === 0) return 1;
  let total = 0;
  for (const child of node.children.values()) total += countLeaves(child);
  return total;
}

/** Открытый документ должен быть виден: раскрываем все папки на пути к нему. */
function expandTo(path: string): void {
  const segments = path.split("/");
  for (let i = 1; i < segments.length; i += 1) state.expanded.add(segments.slice(0, i).join("/"));
}

function treeNodeTpl(node: TreeNode, needle: string, depth: number): TemplateResult | typeof nothing {
  if (!matchesQuery(node, needle)) return nothing;
  const children = ordered(node);

  if (node.document && children.length === 0) {
    const open = state.open?.path === node.path;
    return html`
      <button
        type="button"
        class="documents-leaf ${open ? "is-open" : ""}"
        style="--depth:${depth}"
        aria-current=${open ? "true" : "false"}
        @click=${() => void openDocument(node.path)}
      >
        <span class="documents-leaf-icon" aria-hidden="true">${icon(File, 13)}</span>
        <span class="documents-leaf-name">${node.name}</span>
        <span class="documents-leaf-meta">r${node.document.revision} · ${documentBytes(node.document.bytes)}</span>
      </button>
    `;
  }

  // Пока идёт поиск папки раскрыты, иначе — как выбрал пользователь.
  const open = Boolean(needle) || state.expanded.has(node.path);
  return html`
    <div class="documents-branch">
      <button
        type="button"
        class="documents-folder ${open ? "is-open" : ""}"
        style="--depth:${depth}"
        aria-expanded=${open ? "true" : "false"}
        @click=${() => {
          if (state.expanded.has(node.path)) state.expanded.delete(node.path);
          else state.expanded.add(node.path);
          redraw();
        }}
      >
        <span class="documents-twisty" aria-hidden="true">${icon(open ? ChevronDown : ChevronRight, 13)}</span>
        <span class="documents-folder-icon" aria-hidden="true">${icon(open ? FolderOpen : Folder, 14)}</span>
        <span class="documents-folder-name">${node.name}</span>
        <span class="documents-folder-count">${countLeaves(node)}</span>
      </button>
      ${
        open
          ? html`<div class="documents-children" style="--depth:${depth}">
              ${children.map((child) => treeNodeTpl(child, needle, depth + 1))}
            </div>`
          : nothing
      }
    </div>
  `;
}

async function selectKind(kind: string): Promise<void> {
  state.kind = kind;
  if (!kind) {
    state.kindPaths = null;
    return redraw();
  }
  state.sectionLoading = true;
  state.open = null;
  redraw();
  try {
    const r = await api<{ documents: SectionDoc[] }>(
      `/api/projects/${encodeURIComponent(state.projectId)}/section?kind=${encodeURIComponent(kind)}`,
    );
    state.sectionDocs = r.documents;
    state.kindPaths = new Set(r.documents.map((d) => d.path));
  } catch (error) {
    state.notice = errMessage(error);
    state.kindPaths = null;
    state.sectionDocs = [];
  } finally {
    state.sectionLoading = false;
    redraw();
  }
}

/** Дерево показывает выбранный раздел: разделы — это то, из чего проект состоит. */
function visibleDocuments(): DocumentSummary[] {
  return state.kindPaths ? state.documents.filter((d) => state.kindPaths!.has(d.path)) : state.documents;
}

/** Раздел в сайдбаре: выбранный раскрывается своими документами, остальные свёрнуты. */
function sectionNavTpl(section: ProjectSection, needle: string): TemplateResult {
  const active = state.kind === section.kind;
  const docs = active
    ? state.sectionDocs.filter(
        (d) => !needle || d.path.toLowerCase().includes(needle) || d.title.toLowerCase().includes(needle),
      )
    : [];
  return html`
    <div class="documents-branch">
      <button
        type="button"
        class="documents-section ${active ? "active" : ""}"
        aria-expanded=${active ? "true" : "false"}
        @click=${() => void selectKind(active ? "" : section.kind)}
      >
        <span class="documents-section-name">${section.title}</span>
        <span class="documents-section-count ${section.open ? "attention" : ""}">
          ${section.open ? `${section.open} / ${section.count}` : section.count}
        </span>
      </button>
      ${
        active
          ? html`<div class="documents-section-docs">
              ${
                docs.length
                  ? docs.map(
                      (doc) => html`
                        <button
                          type="button"
                          class="documents-leaf ${state.open?.path === doc.path ? "is-open" : ""}"
                          @click=${() => void openDocument(doc.path)}
                        >
                          <span class="documents-leaf-name">${doc.id ?? doc.title}</span>
                          ${
                            doc.answered === false || doc.accepted === false
                              ? html`<span class="documents-leaf-mark">•</span>`
                              : nothing
                          }
                        </button>
                      `,
                    )
                  : html`<p class="documents-empty">Ничего не нашлось.</p>`
              }
            </div>`
          : nothing
      }
    </div>
  `;
}

/** Сущности идут выше видов документов: по ним работают, а документ — их источник. */
/** Виды, у которых есть своя таблица сущностей, из списка документов убраны. */
function documentKinds(): ProjectSection[] {
  const replaced = new Set(state.entities.map((e) => e.replaces).filter(Boolean));
  return state.sections.filter((section) => !replaced.has(section.kind));
}

function entityNavTpl(): TemplateResult | typeof nothing {
  if (!state.entities.length) return nothing;
  const alarm = (kind: string) => state.findings.filter((f) => f.kind === kind).reduce((n, f) => n + f.count, 0);
  return html`
    <div class="documents-group">
      <h3 class="documents-group-head">Сущности</h3>
      ${state.entities.map((entity) => {
        const found = alarm(entity.kind);
        return html`
          <button
            type="button"
            class="documents-section ${state.entityKind === entity.kind ? "active" : ""}"
            @click=${() => void selectEntity(entity.kind)}
          >
            <span class="documents-section-name">${entity.title}</span>
            <span class="documents-section-count ${found ? "attention" : ""}">
              ${found ? `${found} / ${entity.count}` : entity.count}
            </span>
          </button>
        `;
      })}
    </div>
  `;
}

function treeTpl(): TemplateResult {
  const needle = state.query.trim().toLowerCase();
  const visible = ordered(buildTree(visibleDocuments())).filter((child) => matchesQuery(child, needle));
  return html`
    <nav class="documents-tree" aria-label="Документы">
      <div class="documents-search">
        <input
          type="search"
          placeholder="Поиск по пути…"
          aria-label="Поиск документа"
          .value=${state.query}
          @input=${(e: Event) => {
            state.query = (e.target as HTMLInputElement).value;
            redraw();
          }}
        />
      </div>
      <div class="documents-nav">
        <h3 class="documents-group-head">Файлы</h3>
        <button
          type="button"
          class="documents-section ${state.kind ? "" : "active"}"
          @click=${() => void selectKind("")}
        >
          <span class="documents-section-name">Всё дерево</span>
          <span class="documents-section-count">${state.documents.length}</span>
        </button>
        ${
          state.kind
            ? nothing
            : html`<div class="documents-tree-scroll">
                ${
                  visible.length
                    ? visible.map((child) => treeNodeTpl(child, needle, 0))
                    : html`<p class="documents-empty">Ничего не нашлось.</p>`
                }
              </div>`
        }
        ${entityNavTpl()}
        <div class="documents-group">
          <h3 class="documents-group-head">Документы</h3>
          ${documentKinds().map((section) => sectionNavTpl(section, needle))}
        </div>
      </div>
      <div class="documents-tree-foot">${visibleDocuments().length} документов</div>
    </nav>
  `;
}

// ─── правая колонка ────────────────────────────────────────────────────────

function outlineTpl(open: OpenDocument): TemplateResult {
  if (!open.sections.length) return html`<p class="documents-empty">У документа нет заголовков.</p>`;
  return html`
    <ul class="documents-outline">
      ${open.sections.map(
        (section) => html`
          <li style="--depth:${section.level - 1}">
            <button type="button" class="documents-outline-jump" @click=${() => jumpTo(section.anchor)}>
              ${section.title}
            </button>
            <button
              type="button"
              class="documents-outline-edit"
              title="Править раздел"
              aria-label=${`Править раздел «${section.title}»`}
              @click=${() => void beginEdit(section)}
            >
              ✎
            </button>
          </li>
        `,
      )}
    </ul>
  `;
}

function backlinksTpl(): TemplateResult {
  if (!state.backlinks.length) return html`<p class="documents-empty">На этот документ никто не ссылается.</p>`;
  const byPath = new Map<string, BacklinkView[]>();
  for (const backlink of state.backlinks) {
    const bucket = byPath.get(backlink.path) ?? [];
    bucket.push(backlink);
    byPath.set(backlink.path, bucket);
  }
  return html`
    <ul class="documents-backlinks">
      ${[...byPath.entries()].map(
        ([path, items]) => html`
          <li>
            <button type="button" class="documents-backlink-path" @click=${() => void openDocument(path)}>
              ${path}
            </button>
            ${items.map((item) => html`<span class="documents-backlink-text">«${item.text}»</span>`)}
          </li>
        `,
      )}
    </ul>
  `;
}

function historyTpl(): TemplateResult {
  if (!state.revisions.length) return html`<p class="documents-empty">История пуста.</p>`;
  return html`
    <ul class="documents-history-list">
      ${state.revisions.map(
        (revision) => html`
          <li>
            <b>r${revision.revision}</b>
            <span>${revision.writtenBy}</span>
            <span>${relTime(revision.writtenAt)}</span>
            <span>${documentBytes(revision.bytes)}</span>
          </li>
        `,
      )}
    </ul>
  `;
}

function asideBodyTpl(open: OpenDocument): TemplateResult {
  if (state.aside === "outline") return outlineTpl(open);
  if (state.aside === "backlinks") return backlinksTpl();
  return historyTpl();
}

function asideTpl(open: OpenDocument): TemplateResult {
  const tab = (id: Aside, label: string, count: number) => html`
    <button
      type="button"
      role="tab"
      aria-selected=${state.aside === id ? "true" : "false"}
      class="documents-aside-tab ${state.aside === id ? "active" : ""}"
      @click=${() => {
        state.aside = id;
        redraw();
      }}
    >
      ${label}<span class="documents-aside-count">${count}</span>
    </button>
  `;
  return html`
    <aside class="documents-aside">
      <div class="documents-aside-tabs" role="tablist" aria-label="Сведения о документе">
        ${tab("outline", "Оглавление", open.sections.length)} ${tab("backlinks", "Ссылки сюда", state.backlinks.length)}
        ${tab("history", "Ревизии", state.revisions.length)}
      </div>
      <div class="documents-aside-body">${asideBodyTpl(open)}</div>
    </aside>
  `;
}

// ─── тело документа ────────────────────────────────────────────────────────

function propertiesTpl(open: OpenDocument): TemplateResult | typeof nothing {
  const properties = headerFields(open.fields);
  if (!properties.length) return nothing;
  return html`
    <dl class="documents-properties">
      ${properties.map(
        (field) => html`
          <div class="documents-property">
            <dt>${field.name}</dt>
            <dd>${field.value}</dd>
          </div>
        `,
      )}
    </dl>
  `;
}

function editorTpl(open: OpenDocument): TemplateResult {
  const editing = state.editing!;
  return html`
    <div class="documents-editor-wrap">
      <div class="documents-editor-head">
        <span>Правлю раздел <b>${editing.title}</b></span>
        <span class="documents-hint">ревизия ${open.revision} — правка отвергается, если её обгонят</span>
      </div>
      <textarea
        class="documents-editor"
        rows="24"
        aria-label=${`Тело раздела «${editing.title}»`}
        .value=${editing.body}
        @input=${(e: Event) => {
          editing.body = (e.target as HTMLTextAreaElement).value;
        }}
      ></textarea>
      <div class="documents-editor-actions">
        <button type="button" class="btn primary" ?disabled=${state.saving} @click=${() => void saveSection()}>
          ${state.saving ? "Сохраняю…" : "Сохранить"}
        </button>
        <button
          type="button"
          class="btn"
          ?disabled=${state.saving}
          @click=${() => {
            state.editing = null;
            redraw();
          }}
        >
          Отмена
        </button>
      </div>
    </div>
  `;
}

function documentTpl(open: OpenDocument): TemplateResult {
  const segments = open.path.split("/");
  return html`
    <article class="documents-main">
      <header class="documents-head">
        <nav class="documents-breadcrumb" aria-label="Путь">
          ${segments.map(
            (segment, index) =>
              html`${index ? html`<span class="documents-crumb-sep">/</span>` : nothing}<span
                  class="documents-crumb ${index === segments.length - 1 ? "current" : ""}"
                  >${segment}</span
                >`,
          )}
        </nav>
        <div class="documents-meta">
          ревизия ${open.revision} · ${open.updatedBy} · ${relTime(open.updatedAt)} · ${open.links.length} ссылок наружу
        </div>
      </header>
      ${propertiesTpl(open)}
      ${
        state.editing
          ? editorTpl(open)
          : html`<div class="documents-body markdown" @click=${onBodyClick}>${blocksTpl(open)}</div>`
      }
    </article>
  `;
}

const ROLE_LABEL: Record<string, string> = {
  answer: "Ответ",
  awaiting: "Ответа нет",
  decision: "Решение",
  consequence: "Последствия",
  rejected: "Отвергнутые варианты",
  criteria: "Критерий приёмки",
  proof: "Чем доказывается",
  attention: "Требует внимания",
};

/** Разбор в блоки делаем один раз на документ: он стоит разбора DOM. */
const blocked = { key: "", blocks: [] as DocumentBlock[] };

function blocksOf(open: OpenDocument): DocumentBlock[] {
  const key = `${open.path}@${open.revision}`;
  if (blocked.key !== key) {
    const host = window.document.createElement("div");
    host.innerHTML = bodyHtml(open);
    blocked.key = key;
    blocked.blocks = withAwaiting(open.kind, splitBlocks(open.kind, host));
  }
  return blocked.blocks;
}

function blockTpl(block: DocumentBlock): TemplateResult {
  const label = ROLE_LABEL[block.role];
  return html`
    <section class="doc-block role-${block.role}">
      ${
        block.title
          ? html`<h3 class="doc-block-title">
              ${block.title}${label && label !== block.title ? html`<span>${label}</span>` : nothing}
            </h3>`
          : nothing
      }
      ${
        block.role === "awaiting"
          ? html`<p class="doc-block-empty">Вопрос открыт — ответа в документе нет.</p>`
          : html`<div class="doc-block-body">${unsafeHTML(block.html)}</div>`
      }
    </section>
  `;
}

function blocksTpl(open: OpenDocument): TemplateResult {
  const blocks = blocksOf(open);
  if (!blocks.length) return html`${unsafeHTML(bodyHtml(open))}`;
  return html`${blocks.map((block) => blockTpl(block))}`;
}

function onBodyClick(event: Event): void {
  const ref = (event.target as HTMLElement).closest(".entity-ref") as HTMLElement | null;
  if (ref) {
    event.preventDefault();
    const path = ref.dataset["path"] ?? "";
    const inner = ref.dataset["anchor"] ?? "";
    if (inner) pendingScroll = inner;
    return void openDocument(path);
  }
  const anchor = (event.target as HTMLElement).closest("a");
  if (!anchor) return;
  const href = anchor.getAttribute("href") ?? "";
  if (href.startsWith("#")) {
    event.preventDefault();
    return jumpTo(href.slice(1));
  }
  const target = resolveDocumentPath(state.open?.path ?? "", href);
  if (!target || !state.documents.some((document) => document.path === target)) return;
  event.preventDefault();
  void openDocument(target);
}

function jumpTo(anchor: string): void {
  pendingScroll = anchor;
  redraw();
}

/** Заголовки разметки и секции из базы — один список в одном порядке, поэтому их можно сшить по счёту. */
export function labelDocumentHeadings(): void {
  const open = state.open;
  const body = document.querySelector(".documents-body");
  if (!open || !body) return;
  // Ссылки ставим после разметки: заголовки уже на месте, а повтор безвреден.
  linkEntities(body, state.entityIndex, open.path);
  body.querySelectorAll("h1, h2, h3, h4, h5, h6").forEach((heading, index) => {
    const section = open.sections[index];
    if (section) heading.id = section.anchor;
  });
  if (!pendingScroll) return;
  const anchor = pendingScroll;
  pendingScroll = null;
  body.querySelector(`#${CSS.escape(anchor)}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
}

/** Три колонки рабочего места: дерево, документ, сведения. Годится и для страницы, и для дашборда проекта. */
export function documentsWorkspaceTpl(): TemplateResult {
  return html`
    ${state.notice ? html`<div class="documents-notice">${state.notice}</div>` : nothing}
    <div class="documents-layout">
      ${treeTpl()}
      ${state.open ? documentTpl(state.open) : html`<article class="documents-main">${mainEmptyTpl()}</article>`}
      ${state.open ? asideTpl(state.open) : nothing}
    </div>
  `;
}

/** Размечает произвольный текст тем же обеззараживателем — например описание вехи. */
export function renderMarkdownHtml(content: string): string {
  installMarkdownSanitizer();
  return String(marked.parse(content, { async: false }));
}

/** Открывает конкретный документ — например прогон задачи, выбранный на доске. */
/**
 * Открыть сущность извне — с главного экрана проекта. Находка обещает конкретику
 * («45 решено, но ответ не записан»), и вести она обязана в свой список, а не на
 * общую вкладку: иначе обещание не выполнено и человек ищет заново.
 */
export async function openEntityKind(
  projectId: string,
  kind: string,
  onRedraw: () => void,
  only?: { keys: Set<string>; label: string; records: number },
): Promise<void> {
  await mountProjectDocuments(projectId, onRedraw);
  state.entityKind = "";
  state.entityOnly = only?.keys ?? null;
  state.entityOnlyLabel = only?.label ?? "";
  state.entityOnlyRecords = only?.records ?? 0;
  await selectEntity(kind);
}

export async function openDocumentAt(projectId: string, path: string, onRedraw: () => void): Promise<void> {
  await mountProjectDocuments(projectId, onRedraw);
  await openDocument(path);
}

/** Встраивает рабочее место в чужую страницу: чей проект и чем перерисовывать. */
export async function mountProjectDocuments(projectId: string, onRedraw: () => void): Promise<void> {
  if (state.projectId === projectId && state.documents.length) {
    redraw = onRedraw;
    return;
  }
  redraw = onRedraw;
  state.projectId = projectId;
  resetDocuments();
  state.documents = [];
  await refresh();
}

/** Возвращает перерисовку своей странице — иначе она осталась бы привязанной к дашборду. */
export function unmountProjectDocuments(): void {
  redraw = () => drawStandalone();
}

/** Карточка документа в указателе: идентификатор, суть и признак, зависящий от вида. */
/** Пока документ не открыт: указатель выбранного раздела либо приглашение выбрать. */
function entityPaneTpl(): TemplateResult {
  const entity = state.entities.find((e) => e.kind === state.entityKind);
  if (state.entityLoading) return html`<p class="documents-empty">Читаю каталог…</p>`;
  if (!state.entityRows.length) return html`<p class="documents-empty">Здесь пока пусто.</p>`;
  // Пришли из находки — показываем ровно её строки, иначе обещание «31» не выполнено.
  const only = state.entityOnly;
  const rows = only ? state.entityRows.filter((r) => only.has(rowKey(r))) : state.entityRows;
  return html`
    <div class="entity-pane">
      <header class="entity-head">
        ${
          state.entityFeature
            ? html`<button type="button" class="entity-back" @click=${() => void selectEntity("features")}>
                ← Области
              </button>`
            : nothing
        }
        <h2>${state.entityFeature ? `Область ${state.entityFeature}` : (entity?.title ?? state.entityKind)}</h2>
        <p class="entity-source">
          ${rows.length}${only ? ` из ${state.entityRows.length}` : ""} ·
          ${
            state.entityFeature
              ? `сделано ${state.entityRows.filter((r) => r["state"] === "done").length}`
              : `источник: ${entity?.source ?? ""}`
          }
        </p>
        ${
          only
            ? html`<button
                type="button"
                class="entity-back"
                @click=${() => {
                  state.entityOnly = null;
                  state.entityOnlyLabel = "";
                  redraw();
                }}
              >
                находка «${state.entityOnlyLabel}»: ${state.entityOnlyRecords}
                ${plural(state.entityOnlyRecords, "запись", "записи", "записей")} в ${rows.length}
                ${plural(rows.length, "строке", "строках", "строках")} · показать все →
              </button>`
            : nothing
        }
      </header>
      ${findingsTpl(state.entityKind, state.findings)}
      ${entityTableTpl(state.entityKind, rows, (row) => {
        if (state.entityKind === "features") return void openFeature(String(row["id"] ?? ""));
        const path = typeof row["path"] === "string" ? row["path"] : "";
        if (path) void openDocument(path);
      })}
    </div>
  `;
}

function mainEmptyTpl(): TemplateResult {
  if (state.entityKind) return entityPaneTpl();
  if (state.kind) return sectionIndexTpl();
  return html`<p class="documents-empty">${state.loading ? "Читаю…" : "Выберите раздел проекта слева."}</p>`;
}

/** Признак, зависящий от вида: у вопроса — ответ, у решения — принято ли оно. */
function sectionMark(doc: SectionDoc): string {
  if (doc.answered === false) return "ждёт ответа";
  return doc.accepted === false ? "отклонено" : "";
}

function sectionCardTpl(doc: SectionDoc): TemplateResult {
  const mark = sectionMark(doc);
  return html`
    <button type="button" class="section-card ${mark ? "is-open" : ""}" @click=${() => void openDocument(doc.path)}>
      <span class="section-card-head">
        ${doc.id ? html`<span class="section-card-id">${doc.id}</span>` : nothing}
        ${mark ? html`<span class="section-card-mark">${mark}</span>` : nothing}
      </span>
      <span class="section-card-title">${doc.title}</span>
      ${
        doc.sections.length
          ? html`<span class="section-card-parts">${doc.sections.slice(0, 4).join(" · ")}</span>`
          : nothing
      }
    </button>
  `;
}

/** Указатель раздела: вопросы без ответа идут первыми — они и есть работа. */
function sectionIndexTpl(): TemplateResult {
  if (state.sectionLoading) return html`<p class="documents-empty">Читаю раздел…</p>`;
  const docs = state.sectionDocs;
  if (!docs.length) return html`<p class="documents-empty">В разделе пусто.</p>`;

  const needsWork = docs.filter((d) => d.answered === false || d.accepted === false);
  const rest = docs.filter((d) => !needsWork.includes(d));
  const group = (label: string, list: SectionDoc[]) =>
    list.length
      ? html`<h3 class="section-group">${label}<span>${list.length}</span></h3>
          <div class="section-cards">${list.map((doc) => sectionCardTpl(doc))}</div>`
      : nothing;

  return html`
    <div class="section-index">
      ${needsWork.length ? group("Требуют внимания", needsWork) : nothing}
      ${group(needsWork.length ? "Остальные" : "Документы", rest)}
    </div>
  `;
}

function drawStandalone(): void {
  if (appState.currentView !== "documents" || !appState.mainEl) return;
  if (!host || host.parentElement !== appState.mainEl) {
    host = document.createElement("div");
    host.className = "pane documents-page";
    appState.mainEl.replaceChildren(host);
  }
  render(
    listPageTpl({
      title: "Документы",
      controls: fieldSelect({
        ariaLabel: "Проект",
        value: state.projectId,
        onChange: (value) => void selectProject(value),
        options: state.projects.map((project) => html`<option value=${project.id}>${project.name}</option>`),
      }),
      onRefresh: () => void refresh(),
      empty: state.loading ? "Читаю…" : "У этого проекта пока нет документов.",
      rows: [documentsWorkspaceTpl()],
    }),
    host,
  );
  labelDocumentHeadings();
}

async function selectProject(id: string): Promise<void> {
  state.projectId = id;
  setActiveScope(scopeOfProjectId(id));
  resetDocuments();
  await refresh();
}

async function refresh(): Promise<void> {
  if (!state.projectId) return;
  state.loading = true;
  redraw();
  try {
    const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
    const [docs, sections, entities, findings, index] = await Promise.all([
      api<{ documents: DocumentSummary[] }>(`${base}/documents`),
      api<{ sections: ProjectSection[] }>(`${base}/sections`).catch(() => ({ sections: [] })),
      api<{ entities: EntitySummary[] }>(`${base}/entities`).catch(() => ({ entities: [] })),
      api<{ findings: EntityFinding[] }>(`${base}/entities/findings`).catch(() => ({ findings: [] })),
      api<{ index: EntityRef[] }>(`${base}/entities/index`).catch(() => ({ index: [] })),
    ]);
    state.documents = docs.documents;
    state.sections = sections.sections ?? [];
    state.entities = entities.entities ?? [];
    state.findings = findings.findings ?? [];
    state.entityIndex = new Map((index.index ?? []).map((ref) => [ref.id, ref]));
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.loading = false;
    redraw();
  }
}

/** Открыть каталог сущности: документ при этом закрывается — это другой взгляд на проект. */
/** Раскрыть область: те же истории, но только её, с состоянием каждой. */
async function openFeature(featureId: string): Promise<void> {
  state.entityFeature = featureId;
  state.entityKind = "stories";
  state.entityRows = [];
  state.entityLoading = true;
  state.open = null;
  redraw();
  try {
    const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
    const list = await api<{ items: EntityRow[] }>(
      `${base}/entity?kind=stories&feature=${encodeURIComponent(featureId)}`,
    );
    state.entityRows = list.items ?? [];
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.entityLoading = false;
    redraw();
  }
}

async function selectEntity(kind: string): Promise<void> {
  state.entityFeature = "";
  if (state.entityKind === kind) {
    state.entityKind = "";
    state.entityRows = [];
    redraw();
    return;
  }
  state.entityKind = kind;
  state.entityRows = [];
  state.entityLoading = true;
  state.open = null;
  state.kind = "";
  redraw();
  try {
    const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
    const list = await api<{ items: EntityRow[] }>(`${base}/entity?kind=${encodeURIComponent(kind)}`);
    state.entityRows = list.items ?? [];
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.entityLoading = false;
    redraw();
  }
}

async function openDocument(path: string): Promise<void> {
  state.loading = true;
  redraw();
  try {
    const query = `?path=${encodeURIComponent(path)}`;
    const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
    const [document, history, backlinks] = await Promise.all([
      api<{
        document: OpenDocument & { content: string; kind?: string; id?: string | null };
        sections: SectionView[];
        fields: FieldView[];
        links: LinkView[];
      }>(`${base}/document${query}`),
      api<{ revisions: RevisionView[] }>(`${base}/document/history${query}`),
      api<{ backlinks: BacklinkView[] }>(`${base}/document/backlinks${query}`),
    ]);
    state.open = {
      path,
      kind: document.document.kind ?? "other",
      id: document.document.id ?? null,
      content: document.document.content,
      revision: document.document.revision,
      updatedAt: document.document.updatedAt,
      updatedBy: document.document.updatedBy,
      sections: document.sections,
      fields: document.fields,
      links: document.links ?? [],
    };
    state.revisions = history.revisions;
    state.backlinks = backlinks.backlinks;
    expandTo(path);
    state.editing = null;
    state.notice = "";
  } catch (error) {
    state.notice = errMessage(error);
  } finally {
    state.loading = false;
    redraw();
  }
}

async function beginEdit(section: SectionView): Promise<void> {
  if (state.editing?.anchor === section.anchor) {
    state.editing = null;
    return redraw();
  }
  try {
    const base = `/api/projects/${encodeURIComponent(state.projectId)}`;
    const query = `?path=${encodeURIComponent(state.open!.path)}&anchor=${encodeURIComponent(section.anchor)}`;
    const r = await api<{ body: string }>(`${base}/document/section${query}`);
    state.editing = { anchor: section.anchor, title: section.title, body: r.body };
  } catch (error) {
    state.notice = errMessage(error);
  }
  redraw();
}

async function saveSection(): Promise<void> {
  const open = state.open;
  const editing = state.editing;
  if (!open || !editing) return;
  state.saving = true;
  redraw();
  try {
    await api(`/api/projects/${encodeURIComponent(state.projectId)}/document/section`, {
      method: "PUT",
      body: JSON.stringify({
        path: open.path,
        anchor: editing.anchor,
        body: editing.body,
        expectedRevision: open.revision,
      }),
    });
    state.editing = null;
    state.saving = false;
    await openDocument(open.path);
    await refresh();
  } catch (error) {
    state.notice = errMessage(error);
    state.saving = false;
    redraw();
  }
}

export async function renderDocumentsPage(): Promise<void> {
  if (appState.currentView !== "documents") return;
  unmountProjectDocuments();
  redraw();
  try {
    const r = await api<{ projects: ProjectSummary[] }>("/api/projects");
    state.projects = r.projects;
    const active = projectIdOfScope(appState.activeScope);
    if (active && r.projects.some((p) => p.id === active)) state.projectId = active;
    else if (!state.projectId && r.projects[0]) state.projectId = r.projects[0].id;
  } catch (error) {
    state.notice = errMessage(error);
  }
  if (appState.currentView !== "documents") return;
  await refresh();
}
