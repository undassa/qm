import { sendJson } from "../http.ts";
import { isObj } from "./shared.ts";
import { projectGroupRef } from "../../projects/project-store.ts";
import { summaryOf, type ProjectDocumentStore } from "../../projects/project-document-store.ts";
import { documentId, KIND_META, KIND_ORDER, kindOfPath, type DocumentKind } from "../../projects/document-kind.ts";
import { type ApiCtx, type Route } from "./route.ts";

interface Access {
  projectId: string;
  principalId: string;
  documents: ProjectDocumentStore;
}

async function access(ctx: ApiCtx, projectId: string): Promise<Access | null> {
  const documents = ctx.deps.projectDocuments;
  const projects = ctx.deps.projects;
  if (!documents || !projects) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return null;
  }
  const requested = (ctx.url.searchParams.get("principalId") ?? bodyPrincipal(ctx)).trim();
  const principalId = ctx.capability ? ctx.capability.actorId : requested;
  if (!principalId) {
    sendJson(ctx.res, 400, { error: "bad_request", message: "principalId required" });
    return null;
  }
  if (ctx.capability && requested && requested !== ctx.capability.actorId) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return null;
  }
  const member = await projects.membership(projectGroupRef(projectId), principalId).catch(() => false);
  if (member !== true) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return null;
  }
  return { projectId, principalId, documents };
}

function bodyPrincipal(ctx: ApiCtx): string {
  const body = isObj(ctx.body) ? ctx.body : {};
  return typeof body["principalId"] === "string" ? body["principalId"] : "";
}

function requiredPath(ctx: ApiCtx): string | null {
  const path = (ctx.url.searchParams.get("path") ?? "").trim();
  if (path) return path;
  sendJson(ctx.res, 400, { error: "bad_request", message: "path required" });
  return null;
}

async function listDocuments(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const prefix = (ctx.url.searchParams.get("prefix") ?? "").trim();
  const documents = await granted.documents.list(granted.projectId, prefix || undefined);
  return sendJson(ctx.res, 200, { documents });
}

async function getDocument(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const document = await granted.documents.get(granted.projectId, path);
  if (!document) return sendJson(ctx.res, 404, { error: "not_found" });
  const [sections, fields, links] = await Promise.all([
    granted.documents.sections(granted.projectId, path),
    granted.documents.fields(granted.projectId, path),
    granted.documents.links(granted.projectId, path),
  ]);
  return sendJson(ctx.res, 200, {
    // Вид документа решает вёрстку страницы, поэтому едет вместе с содержимым.
    document: { ...summaryOf(document), content: document.content, kind: kindOfPath(path), id: documentId(path) },
    sections,
    fields,
    links,
  });
}

async function getSection(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const anchor = (ctx.url.searchParams.get("anchor") ?? "").trim();
  if (!anchor) return sendJson(ctx.res, 400, { error: "bad_request", message: "anchor required" });
  const found = await granted.documents.section(granted.projectId, path, anchor);
  return found ? sendJson(ctx.res, 200, found) : sendJson(ctx.res, 404, { error: "not_found" });
}

async function getProgress(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const progress = ctx.deps.projectProgress;
  if (!progress) return sendJson(ctx.res, 404, { error: "not_found" });
  return sendJson(ctx.res, 200, await progress.read(granted.projectId));
}

async function getBoard(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const progress = ctx.deps.projectProgress;
  if (!progress) return sendJson(ctx.res, 404, { error: "not_found" });
  return sendJson(ctx.res, 200, { tasks: await progress.board(granted.projectId) });
}

async function getTask(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const progress = ctx.deps.projectProgress;
  if (!progress) return sendJson(ctx.res, 404, { error: "not_found" });
  const taskId = (ctx.url.searchParams.get("taskId") ?? "").trim();
  if (!taskId) return sendJson(ctx.res, 400, { error: "bad_request", message: "taskId required" });
  const task = await progress.task(granted.projectId, taskId);
  return task ? sendJson(ctx.res, 200, { task }) : sendJson(ctx.res, 404, { error: "not_found" });
}

async function getBacklinks(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const backlinks = await granted.documents.backlinks(granted.projectId, path);
  return sendJson(ctx.res, 200, { backlinks });
}

async function getHistory(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const revisions = await granted.documents.history(granted.projectId, path);
  return sendJson(ctx.res, 200, { revisions });
}

async function getRevision(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const revision = Number(ctx.url.searchParams.get("revision"));
  if (!Number.isInteger(revision) || revision < 1)
    return sendJson(ctx.res, 400, { error: "bad_request", message: "revision required" });
  const document = await granted.documents.atRevision(granted.projectId, path, revision);
  return document ? sendJson(ctx.res, 200, { document }) : sendJson(ctx.res, 404, { error: "not_found" });
}

const REFUSAL: Record<string, { status: number; message: string }> = {
  invalid_path: { status: 400, message: "path is outside the document set" },
  not_found: { status: 404, message: "no such document" },
  no_such_section: { status: 404, message: "no such section" },
  conflict: { status: 409, message: "the document moved on since you read it" },
};

async function putSection(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const body = isObj(ctx.body) ? ctx.body : {};
  const path = typeof body["path"] === "string" ? body["path"] : "";
  const anchor = typeof body["anchor"] === "string" ? body["anchor"] : "";
  const text = typeof body["body"] === "string" ? body["body"] : null;
  if (!path || !anchor || text === null)
    return sendJson(ctx.res, 400, { error: "bad_request", message: "path, anchor and body required" });
  const expectedRevision = body["expectedRevision"];
  const result = await granted.documents.putSection({
    projectId: granted.projectId,
    path,
    anchor,
    body: text,
    author: granted.principalId,
    ...(typeof expectedRevision === "number" ? { expectedRevision } : {}),
  });
  if (result.status === "written" || result.status === "unchanged")
    return sendJson(ctx.res, 200, { status: result.status, document: summaryOf(result.document) });
  const refusal = REFUSAL[result.status]!;
  return sendJson(ctx.res, refusal.status, {
    error: result.status,
    message: refusal.message,
    ...(result.status === "conflict" ? { document: summaryOf(result.document) } : {}),
  });
}

/**
 * Запись целого документа. Раздельная правка (`putSection`) не покрывает того, ради чего
 * харнес и пишет: НОВЫЙ документ — требование, решение, задача, вопрос — разделов ещё не
 * имеет, и заводится он целиком или никак.
 *
 * `expectedRevision` не обязателен, и это осознанно: заводя документ, писатель не знает
 * номера, которого ещё нет. Кто читал документ раньше и правит его — обязан назвать номер,
 * иначе потеряет чужую правку молча; отказ `conflict` для того и стоит.
 */
async function putDocument(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const body = isObj(ctx.body) ? ctx.body : {};
  const path = typeof body["path"] === "string" ? body["path"] : "";
  const content = typeof body["content"] === "string" ? body["content"] : null;
  if (!path || content === null)
    return sendJson(ctx.res, 400, { error: "bad_request", message: "path and content required" });
  const expectedRevision = body["expectedRevision"];
  const result = await granted.documents.put({
    projectId: granted.projectId,
    path,
    content,
    author: granted.principalId,
    ...(typeof expectedRevision === "number" ? { expectedRevision } : {}),
  });
  if (result.status === "written" || result.status === "unchanged")
    return sendJson(ctx.res, 200, { status: result.status, document: summaryOf(result.document) });
  const refusal = REFUSAL[result.status]!;
  return sendJson(ctx.res, refusal.status, {
    error: result.status,
    message: refusal.message,
    ...(result.status === "conflict" ? { document: summaryOf(result.document) } : {}),
  });
}

/**
 * Удаление документа. Отдельный маршрут, а не запись пустого содержимого: документ с
 * пустым телом и отсутствующий документ — разные состояния набора, и проекции считают их
 * по-разному.
 */
async function deleteDocument(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const path = requiredPath(ctx);
  if (path === null) return;
  const removed = await granted.documents.remove(granted.projectId, path, granted.principalId);
  return removed ? sendJson(ctx.res, 200, { status: "removed", path }) : sendJson(ctx.res, 404, { error: "not_found" });
}

/**
 * Пересборка всех проекций набора.
 *
 * Отдельным вызовом, а не следствием каждой записи, и причина замерена: пересборка идёт
 * около минуты, а правка документа — доли секунды. Записывающий делает свои правки и зовёт
 * пересборку один раз в конце.
 *
 * Плата за это — окно, в котором документы новые, а проекции старые. Оно не молчит:
 * `stale` в ответе `/progress` считается сравнением отметки пересборки с последней правкой
 * документа, и интерфейс показывает его словами.
 */
async function reproject(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const run = ctx.deps.projectReproject;
  if (!run) return sendJson(ctx.res, 404, { error: "not_found", message: "reprojection needs a database" });
  const { report, findings } = await run(granted.projectId);
  return sendJson(ctx.res, 200, { status: "reprojected", report, findings });
}

/**
 * Разделы проекта: сколько документов каждого вида и что в них требует внимания.
 * Это не файловое дерево — это то, из чего проект состоит.
 */
async function listSections(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const all = await granted.documents.list(granted.projectId);

  const byKind = new Map<DocumentKind, number>();
  for (const document of all) {
    const kind = kindOfPath(document.path);
    byKind.set(kind, (byKind.get(kind) ?? 0) + 1);
  }

  // Состояние вопроса объявлено в документе полем «Состояние» — берём его, а не
  // догадку по наличию раздела «Ответ»: она завышала число открытых втрое.
  const open = await ctx.deps.projectQuestions?.counts(granted.projectId).then((c) => c.open);

  const sections = KIND_ORDER.filter((kind) => byKind.has(kind)).map((kind) => ({
    kind,
    title: KIND_META[kind].title,
    count: byKind.get(kind) ?? 0,
    ...(kind === "question" && open ? { open, note: `${open} открытых` } : {}),
  }));
  return sendJson(ctx.res, 200, { sections });
}

/** Документы одного раздела с их идентификаторами — для типовой страницы. */
async function listSection(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const kind = (ctx.url.searchParams.get("kind") ?? "").trim() as DocumentKind;
  if (!KIND_META[kind]) return sendJson(ctx.res, 400, { error: "bad_request", message: "unknown kind" });
  const all = await granted.documents.list(granted.projectId);
  const paths = all.filter((d) => kindOfPath(d.path) === kind);

  // Заголовок и признаки берём из разобранной структуры — раздел показывает данные, не имена файлов.
  const documents = await Promise.all(
    paths.map(async (d) => {
      const sections = await granted.documents.sections(granted.projectId, d.path);
      const titles = sections.map((s) => s.title);
      const heading = titles[0] ?? d.path;
      return {
        path: d.path,
        id: documentId(d.path),
        title: heading,
        bytes: d.bytes,
        revision: d.revision,
        updatedAt: d.updatedAt,
        sections: titles.slice(1, 8),
        // У вопроса «Ответ» — раздел: без него вопрос открыт.
        ...(kind === "question" ? { answered: titles.includes("Ответ") } : {}),
        ...(kind === "decision" ? { accepted: d.path.includes("/accepted/") } : {}),
      };
    }),
  );
  return sendJson(ctx.res, 200, { kind, title: KIND_META[kind].title, documents });
}

export const projectDocumentRoutes: Route[] = [
  { method: "GET", path: "/v1/projects/:id/sections", auth: "source", handle: listSections },
  { method: "GET", path: "/v1/projects/:id/section", auth: "source", handle: listSection },
  { method: "GET", path: "/v1/projects/:id/documents", auth: "source", handle: listDocuments },
  { method: "GET", path: "/v1/projects/:id/document", auth: "source", handle: getDocument },
  { method: "GET", path: "/v1/projects/:id/document/section", auth: "source", handle: getSection },
  { method: "GET", path: "/v1/projects/:id/progress", auth: "source", handle: getProgress },
  { method: "GET", path: "/v1/projects/:id/board", auth: "source", handle: getBoard },
  { method: "GET", path: "/v1/projects/:id/task", auth: "source", handle: getTask },
  { method: "GET", path: "/v1/projects/:id/document/backlinks", auth: "source", handle: getBacklinks },
  { method: "GET", path: "/v1/projects/:id/document/history", auth: "source", handle: getHistory },
  { method: "GET", path: "/v1/projects/:id/document/revision", auth: "source", handle: getRevision },
  { method: "PUT", path: "/v1/projects/:id/document/section", auth: "source", handle: putSection },
  { method: "PUT", path: "/v1/projects/:id/document", auth: "source", handle: putDocument },
  { method: "DELETE", path: "/v1/projects/:id/document", auth: "source", handle: deleteDocument },
  { method: "POST", path: "/v1/projects/:id/reproject", auth: "source", handle: reproject },
];
