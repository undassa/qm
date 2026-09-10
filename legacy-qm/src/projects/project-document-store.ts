import { createHash } from "node:crypto";
import {
  parseDocument,
  renderDocument,
  resolveLinkTarget,
  type DocumentBlock,
  type DocumentField,
  type DocumentLink,
  type DocumentSection,
  type DocumentStructure,
} from "./document-structure.ts";

export interface ProjectDocument {
  projectId: string;
  path: string;
  content: string;
  contentHash: string;
  bytes: number;
  revision: number;
  updatedAt: number;
  updatedBy: string;
}

export type ProjectDocumentSummary = Omit<ProjectDocument, "content" | "projectId">;

export interface ProjectDocumentRevision {
  revision: number;
  contentHash: string;
  bytes: number;
  writtenAt: number;
  writtenBy: string;
}

export interface PutDocumentInput {
  projectId: string;
  path: string;
  content: string;
  author: string;
  expectedRevision?: number;
}

export type PutDocumentResult =
  | { status: "written"; document: ProjectDocument }
  | { status: "unchanged"; document: ProjectDocument }
  | { status: "conflict"; document: ProjectDocument }
  | { status: "invalid_path" }
  | { status: "not_found" }
  | { status: "no_such_section" };

export interface DocumentBacklink {
  path: string;
  text: string;
  blockOrd: number;
  targetAnchor: string;
}

export interface SectionBody {
  section: DocumentSection;
  body: string;
}

export interface ProjectDocumentStore {
  put(input: PutDocumentInput): Promise<PutDocumentResult>;
  get(projectId: string, path: string): Promise<ProjectDocument | null>;
  list(projectId: string, prefix?: string): Promise<ProjectDocumentSummary[]>;
  remove(projectId: string, path: string, author: string): Promise<boolean>;
  history(projectId: string, path: string, limit?: number): Promise<ProjectDocumentRevision[]>;
  atRevision(projectId: string, path: string, revision: number): Promise<ProjectDocument | null>;
  blocks(projectId: string, path: string): Promise<DocumentBlock[]>;
  sections(projectId: string, path: string): Promise<DocumentSection[]>;
  section(projectId: string, path: string, anchor: string): Promise<SectionBody | null>;
  fields(projectId: string, path: string): Promise<DocumentField[]>;
  links(projectId: string, path: string): Promise<DocumentLink[]>;
  backlinks(projectId: string, path: string): Promise<DocumentBacklink[]>;
  /** Сколько документов с этим префиксом имеют раздел с таким заголовком. */
  sectionTitleCount?(projectId: string, prefix: string, title: string): Promise<number>;
  putSection(input: PutSectionInput): Promise<PutDocumentResult>;
  /**
   * Перечитать документы из их же содержимого и переписать разбор — блоки, разделы, ячейки,
   * ссылки, поля. Содержимое и ревизии не трогаются: документ не менялся, менялся РАЗБОРЩИК.
   *
   * Без этого починка разборщика до набора не доходит: `put` при неизменившемся содержимом
   * возвращается рано и структуру не переписывает, а менять содержимое ради перечитывания
   * нечем. Возвращает число перечитанных документов.
   */
  reparse?(projectId: string, prefix?: string): Promise<number>;
  close?(): Promise<void>;
}

export interface PutSectionInput {
  projectId: string;
  path: string;
  anchor: string;
  body: string;
  author: string;
  expectedRevision?: number;
}

export const MAX_DOCUMENT_BYTES = 4 * 1024 * 1024;
const MAX_PATH_LENGTH = 512;
const SEGMENT = /^(?!\.\.?$)[A-Za-z0-9._][A-Za-z0-9 ._-]*$/u;

export function hashDocument(content: string): string {
  return createHash("sha256").update(content, "utf8").digest("hex");
}

export function normalizeDocumentPath(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > MAX_PATH_LENGTH) return null;
  if (trimmed.includes("\\") || trimmed.includes("\0")) return null;
  const segments = trimmed.split("/");
  if (segments.some((segment) => !SEGMENT.test(segment) || segment.endsWith(" "))) return null;
  return segments.join("/");
}

export function documentBytes(content: string): number {
  return Buffer.byteLength(content, "utf8");
}

interface MemoryEntry {
  document: ProjectDocument;
  revisions: ProjectDocumentRevision[];
}

const keyFor = (projectId: string, path: string): string => `${projectId}\0${path}`;

export function createMemoryProjectDocumentStore(now: () => number = Date.now): ProjectDocumentStore {
  const entries = new Map<string, MemoryEntry>();
  const contentOf = (projectId: string, path: string): string | null => {
    const normalized = normalizeDocumentPath(path);
    return normalized ? (entries.get(keyFor(projectId, normalized))?.document.content ?? null) : null;
  };
  const structureOf = (projectId: string, path: string): DocumentStructure | null => {
    const content = contentOf(projectId, path);
    return content === null ? null : parseDocument(content);
  };

  const store: ProjectDocumentStore = {
    async put(input) {
      const path = normalizeDocumentPath(input.path);
      if (!path) return { status: "invalid_path" };
      const bytes = documentBytes(input.content);
      if (bytes > MAX_DOCUMENT_BYTES) return { status: "invalid_path" };
      const key = keyFor(input.projectId, path);
      const existing = entries.get(key);
      if (existing && input.expectedRevision !== undefined && input.expectedRevision !== existing.document.revision) {
        return { status: "conflict", document: existing.document };
      }
      if (!existing && input.expectedRevision !== undefined && input.expectedRevision !== 0) {
        return { status: "conflict", document: emptyDocument(input.projectId, path) };
      }
      const contentHash = hashDocument(input.content);
      if (existing && existing.document.contentHash === contentHash) {
        return { status: "unchanged", document: existing.document };
      }
      const at = now();
      const document: ProjectDocument = {
        projectId: input.projectId,
        path,
        content: input.content,
        contentHash,
        bytes,
        revision: (existing?.document.revision ?? 0) + 1,
        updatedAt: at,
        updatedBy: input.author,
      };
      const revisions = existing?.revisions ?? [];
      revisions.push({
        revision: document.revision,
        contentHash,
        bytes,
        writtenAt: at,
        writtenBy: input.author,
      });
      entries.set(key, { document, revisions });
      return { status: "written", document };
    },

    async get(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      return normalized ? (entries.get(keyFor(projectId, normalized))?.document ?? null) : null;
    },

    async list(projectId, prefix) {
      // Префикс называет каталог: слэш на конце — обычная запись, а не ошибка.
      const asked = prefix?.replace(/\/+$/, "") ?? "";
      const wanted = asked ? normalizeDocumentPath(asked) : "";
      if (asked && !wanted) return [];
      const out: ProjectDocumentSummary[] = [];
      for (const entry of entries.values()) {
        const d = entry.document;
        if (d.projectId !== projectId) continue;
        if (wanted && d.path !== wanted && !d.path.startsWith(`${wanted}/`)) continue;
        out.push(summaryOf(d));
      }
      return out.sort((a, b) => a.path.localeCompare(b.path));
    },

    async remove(projectId, path, _author) {
      const normalized = normalizeDocumentPath(path);
      return normalized ? entries.delete(keyFor(projectId, normalized)) : false;
    },

    async history(projectId, path, limit = 50) {
      const normalized = normalizeDocumentPath(path);
      const entry = normalized ? entries.get(keyFor(projectId, normalized)) : undefined;
      return entry ? [...entry.revisions].reverse().slice(0, limit) : [];
    },

    async atRevision(projectId, path, revision) {
      const normalized = normalizeDocumentPath(path);
      const entry = normalized ? entries.get(keyFor(projectId, normalized)) : undefined;
      if (!entry) return null;
      return entry.document.revision === revision ? entry.document : null;
    },

    async blocks(projectId, path) {
      return structureOf(projectId, path)?.blocks ?? [];
    },

    async sections(projectId, path) {
      return structureOf(projectId, path)?.sections ?? [];
    },

    async section(projectId, path, anchor) {
      const structure = structureOf(projectId, path);
      return structure ? bodyOfSection(structure, anchor) : null;
    },

    async fields(projectId, path) {
      return structureOf(projectId, path)?.fields ?? [];
    },

    async links(projectId, path) {
      return structureOf(projectId, path)?.links ?? [];
    },

    async backlinks(projectId, path) {
      const target = normalizeDocumentPath(path);
      if (!target) return [];
      const found: DocumentBacklink[] = [];
      for (const entry of entries.values()) {
        const source = entry.document;
        if (source.projectId !== projectId || source.path === target) continue;
        for (const link of parseDocument(source.content).links) {
          if (resolveLinkTarget(source.path, link.targetPath) !== target) continue;
          found.push({
            path: source.path,
            text: link.text,
            blockOrd: link.blockOrd,
            targetAnchor: link.targetAnchor,
          });
        }
      }
      return found.sort((a, b) => a.path.localeCompare(b.path) || a.blockOrd - b.blockOrd);
    },

    async putSection(input) {
      const content = contentOf(input.projectId, input.path);
      if (content === null) return { status: "not_found" };
      const next = replaceSectionBody(content, input.anchor, input.body);
      if (next === null) return { status: "no_such_section" };
      return store.put({
        projectId: input.projectId,
        path: input.path,
        content: next,
        author: input.author,
        ...(input.expectedRevision !== undefined ? { expectedRevision: input.expectedRevision } : {}),
      });
    },
  };
  return store;
}

export function bodyOfSection(structure: DocumentStructure, anchor: string): SectionBody | null {
  const section = structure.sections.find((candidate) => candidate.anchor === anchor);
  if (!section) return null;
  const body = structure.blocks.filter((b) => b.ord > section.firstBlock && b.ord <= section.lastBlock);
  return { section, body: renderDocument(body) };
}

export function replaceSectionBody(content: string, anchor: string, body: string): string | null {
  const structure = parseDocument(content);
  const section = structure.sections.find((candidate) => candidate.anchor === anchor);
  if (!section) return null;
  const head = renderDocument(structure.blocks.filter((b) => b.ord <= section.firstBlock));
  const tail = renderDocument(structure.blocks.filter((b) => b.ord > section.lastBlock));
  const normalized = body === "" || body.endsWith("\n") ? body : `${body}\n`;
  return `${head}${normalized}${tail}`;
}

export function summaryOf(document: ProjectDocument): ProjectDocumentSummary {
  const { content: _content, projectId: _projectId, ...summary } = document;
  return summary;
}

function emptyDocument(projectId: string, path: string): ProjectDocument {
  return {
    projectId,
    path,
    content: "",
    contentHash: hashDocument(""),
    bytes: 0,
    revision: 0,
    updatedAt: 0,
    updatedBy: "",
  };
}
