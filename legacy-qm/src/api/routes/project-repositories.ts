import { sendJson } from "../http.ts";
import { isObj } from "./shared.ts";
import { projectGroupRef } from "../../projects/project-store.ts";
import {
  REPOSITORY_PROVIDERS,
  type RepositoryInput,
  type RepositoryStore,
  type RepositoryWriteResult,
} from "../../projects/repository-store.ts";
import { type ApiCtx, type Route } from "./route.ts";

interface Access {
  projectId: string;
  principalId: string;
  repositories: RepositoryStore;
}

async function access(ctx: ApiCtx, projectId: string): Promise<Access | null> {
  const repositories = ctx.deps.projectRepositories;
  const projects = ctx.deps.projects;
  if (!repositories || !projects) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return null;
  }
  const body = isObj(ctx.body) ? ctx.body : {};
  const requested = (
    ctx.url.searchParams.get("principalId") ?? (typeof body["principalId"] === "string" ? body["principalId"] : "")
  ).trim();
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
  return { projectId, principalId, repositories };
}

function readInput(ctx: ApiCtx): Partial<RepositoryInput> {
  const body = isObj(ctx.body) ? ctx.body : {};
  const patch: Partial<RepositoryInput> = {};
  if (typeof body["name"] === "string") patch.name = body["name"];
  if (typeof body["url"] === "string") patch.url = body["url"];
  if (typeof body["provider"] === "string") patch.provider = body["provider"] as RepositoryInput["provider"];
  if (typeof body["baseBranch"] === "string") patch.baseBranch = body["baseBranch"];
  if (typeof body["credentialId"] === "string" || body["credentialId"] === null) {
    patch.credentialId = body["credentialId"] as string | null;
  }
  if (typeof body["isDefault"] === "boolean") patch.isDefault = body["isDefault"];
  return patch;
}

function sendWrite(ctx: ApiCtx, result: RepositoryWriteResult, okStatus: number): void {
  if (result.status === "ok") return sendJson(ctx.res, okStatus, { repository: result.repository });
  if (result.status === "invalid") return sendJson(ctx.res, 400, { error: "bad_request", message: result.message });
  if (result.status === "duplicate") {
    return sendJson(ctx.res, 409, { error: "conflict", message: "репозиторий с таким именем в проекте уже есть" });
  }
  return sendJson(ctx.res, 404, { error: "not_found" });
}

function requiredRepositoryId(ctx: ApiCtx): string | null {
  const id = (ctx.url.searchParams.get("repositoryId") ?? "").trim();
  if (id) return id;
  sendJson(ctx.res, 400, { error: "bad_request", message: "repositoryId required" });
  return null;
}

async function listRepositories(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const repositories = await granted.repositories.list(granted.projectId);
  return sendJson(ctx.res, 200, { repositories, providers: REPOSITORY_PROVIDERS });
}

async function createRepository(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const patch = readInput(ctx);
  if (!patch.name || !patch.url) {
    return sendJson(ctx.res, 400, { error: "bad_request", message: "name and url required" });
  }
  return sendWrite(ctx, await granted.repositories.create(granted.projectId, patch as RepositoryInput), 201);
}

async function updateRepository(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const repositoryId = requiredRepositoryId(ctx);
  if (repositoryId === null) return;
  return sendWrite(ctx, await granted.repositories.update(granted.projectId, repositoryId, readInput(ctx)), 200);
}

async function deleteRepository(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const repositoryId = requiredRepositoryId(ctx);
  if (repositoryId === null) return;
  const removed = await granted.repositories.remove(granted.projectId, repositoryId);
  return removed ? sendJson(ctx.res, 200, { removed: true }) : sendJson(ctx.res, 404, { error: "not_found" });
}

export const projectRepositoryRoutes: Route[] = [
  { method: "GET", path: "/v1/projects/:id/repositories", auth: "source", handle: listRepositories },
  { method: "POST", path: "/v1/projects/:id/repositories", auth: "source", handle: createRepository },
  { method: "PATCH", path: "/v1/projects/:id/repository", auth: "source", handle: updateRepository },
  { method: "DELETE", path: "/v1/projects/:id/repository", auth: "source", handle: deleteRepository },
];
