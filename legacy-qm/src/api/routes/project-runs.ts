import { sendJson } from "../http.ts";
import { isObj } from "./shared.ts";
import { projectGroupRef, projectScopeId } from "../../projects/project-store.ts";
import { nextStates, TASK_RUN_STATES, type TaskRunState, type TaskRunStore } from "../../projects/task-run-store.ts";
import { prepareWorkspace } from "../../projects/workspace-service.ts";
import { PHASE_DOCUMENTS } from "../../projects/gate-store.ts";
import { type ApiCtx, type Route } from "./route.ts";

interface Access {
  projectId: string;
  principalId: string;
  runs: TaskRunStore;
}

async function access(ctx: ApiCtx, projectId: string): Promise<Access | null> {
  const runs = ctx.deps.projectTaskRuns;
  const projects = ctx.deps.projects;
  if (!runs || !projects) {
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
  return { projectId, principalId, runs };
}

async function listActive(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  return sendJson(ctx.res, 200, { runs: await granted.runs.active(granted.projectId), states: TASK_RUN_STATES });
}

/** История задачи плюс лента последней попытки — интерфейсу нужно и то и другое разом. */
async function taskRuns(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const taskId = (ctx.url.searchParams.get("taskId") ?? "").trim();
  if (!taskId) return sendJson(ctx.res, 400, { error: "bad_request", message: "taskId required" });
  const runs = await granted.runs.forTask(granted.projectId, taskId);
  const latest = runs[0];
  const events = latest ? await granted.runs.events(granted.projectId, latest.id) : [];
  return sendJson(ctx.res, 200, { runs, events, next: latest ? nextStates(latest.state) : [] });
}

async function assign(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const body = isObj(ctx.body) ? ctx.body : {};
  const taskId = typeof body["taskId"] === "string" ? body["taskId"].trim() : "";
  const agentId = typeof body["agentId"] === "string" ? body["agentId"].trim() : "";
  if (!taskId || !agentId) {
    return sendJson(ctx.res, 400, { error: "bad_request", message: "taskId and agentId required" });
  }
  const result = await granted.runs.assign(granted.projectId, taskId, agentId, granted.principalId);
  if (result.status === "ok") return sendJson(ctx.res, 201, { run: result.run, next: nextStates(result.run.state) });
  if (result.status === "busy_task") {
    return sendJson(ctx.res, 409, { error: "conflict", message: "у задачи уже есть живой прогон", run: result.run });
  }
  if (result.status === "busy_agent") {
    return sendJson(ctx.res, 409, {
      error: "conflict",
      message: `агент уже ведёт ${result.limit} задач — это его предел`,
    });
  }
  return sendJson(ctx.res, 404, { error: "not_found" });
}

async function move(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const runId = (ctx.url.searchParams.get("runId") ?? "").trim();
  if (!runId) return sendJson(ctx.res, 400, { error: "bad_request", message: "runId required" });
  const body = isObj(ctx.body) ? ctx.body : {};
  const to = typeof body["to"] === "string" ? (body["to"] as TaskRunState) : null;
  if (!to || !TASK_RUN_STATES.includes(to)) {
    return sendJson(ctx.res, 400, { error: "bad_request", message: "to must be a known state" });
  }
  const reason = typeof body["reason"] === "string" ? body["reason"] : "";
  const result = await granted.runs.move(granted.projectId, runId, to, granted.principalId, reason);
  if (result.status === "ok") return sendJson(ctx.res, 200, { run: result.run, next: nextStates(result.run.state) });
  if (result.status === "illegal") {
    return sendJson(ctx.res, 409, {
      error: "conflict",
      message: `из состояния «${result.from}» так перейти нельзя`,
    });
  }
  return sendJson(ctx.res, 404, { error: "not_found" });
}

/**
 * Готовит рабочую копию: поднимает песочницу, приносит репозиторий и отводит ветку.
 * Агента это ещё не запускает — только место, где он потом будет работать.
 */
async function prepare(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const runId = (ctx.url.searchParams.get("runId") ?? "").trim();
  if (!runId) return sendJson(ctx.res, 400, { error: "bad_request", message: "runId required" });

  const workspaces = ctx.deps.projectWorkspaces;
  const repositories = ctx.deps.projectRepositories;
  const sandbox = ctx.deps.sandbox;
  if (!workspaces || !repositories || !sandbox) {
    return sendJson(ctx.res, 503, { error: "unavailable", message: "песочница или хранилища не подключены" });
  }

  const runs = await granted.runs.active(granted.projectId);
  const run = runs.find((r) => r.id === runId);
  if (!run) return sendJson(ctx.res, 404, { error: "not_found" });

  const repository = await repositories.primary(granted.projectId);
  if (!repository) {
    return sendJson(ctx.res, 409, {
      error: "conflict",
      message: "у проекта нет репозитория — задаче некуда писать код",
    });
  }

  // Подготовка идёт в состоянии «готовим копию»; если прогон уже там, второй переход не нужен.
  if (run.state === "queued") {
    const moved = await granted.runs.move(granted.projectId, runId, "provisioning", granted.principalId, "копия");
    if (moved.status !== "ok") {
      return sendJson(ctx.res, 409, { error: "conflict", message: "прогон не в том состоянии" });
    }
  }

  // Секрет читаем только у владельца и только на время подготовки — в базу проекта он не попадает.
  let token: string | null = null;
  if (repository.credentialId && ctx.deps.keychain) {
    token = await ctx.deps.keychain.readOwnSecret(granted.principalId, repository.credentialId).catch(() => null);
    if (!token) {
      return sendJson(ctx.res, 409, {
        error: "conflict",
        message: "учётка репозитория недоступна — проверьте её в разделе Keychain",
      });
    }
  }

  const result = await prepareWorkspace(
    { sandbox, workspaces },
    {
      projectId: granted.projectId,
      scopeId: projectScopeId(granted.projectId),
      taskRunId: run.id,
      taskId: run.taskId,
      attempt: run.attempt,
      repository: { id: repository.id, url: repository.url, baseBranch: repository.baseBranch },
      token,
    },
  );
  if (result.status === "ready") return sendJson(ctx.res, 200, { workspace: result.workspace });
  return sendJson(ctx.res, 502, { error: "prepare_failed", message: result.detail, workspace: result.workspace });
}

async function workspaceOf(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const runId = (ctx.url.searchParams.get("runId") ?? "").trim();
  if (!runId) return sendJson(ctx.res, 400, { error: "bad_request", message: "runId required" });
  const workspaces = ctx.deps.projectWorkspaces;
  if (!workspaces) return sendJson(ctx.res, 200, { workspace: null });
  return sendJson(ctx.res, 200, { workspace: await workspaces.forRun(granted.projectId, runId) });
}

/** Список гейтов проекта с их состоянием и подписями. */
async function listGates(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const gates = ctx.deps.projectGates;
  if (!gates) return sendJson(ctx.res, 200, { gates: [] });
  return sendJson(ctx.res, 200, { gates: await gates.list(granted.projectId) });
}

/** Подпись под пунктом гейта: её ставит человек, и запись помнит кто и когда. */
async function signGate(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const gates = ctx.deps.projectGates;
  if (!gates) return sendJson(ctx.res, 404, { error: "not_found" });
  const body = isObj(ctx.body) ? ctx.body : {};
  const phase = typeof body["phase"] === "string" ? body["phase"].trim() : "";
  const item = typeof body["item"] === "string" ? body["item"].trim() : "";
  if (!phase || !item) return sendJson(ctx.res, 400, { error: "bad_request", message: "phase and item required" });
  const revoke = body["revoke"] === true;
  const gate = revoke
    ? await gates.unsign(granted.projectId, phase, item)
    : await gates.sign(granted.projectId, phase, item, granted.principalId);
  return gate
    ? sendJson(ctx.res, 200, { gate })
    : sendJson(ctx.res, 409, { error: "conflict", message: "подписью закрывается только подписной пункт" });
}

/** Что читают под гейтом: документы фазы в порядке дерева. Подпись ставится после чтения. */
async function gateReading(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const phase = (ctx.url.searchParams.get("phase") ?? "").trim();
  const prefixes = PHASE_DOCUMENTS[phase];
  if (!prefixes) return sendJson(ctx.res, 200, { phase, documents: [] });
  const documents = ctx.deps.projectDocuments;
  if (!documents) return sendJson(ctx.res, 200, { phase, documents: [] });
  const all = await documents.list(granted.projectId);
  const inPhase = all
    .filter((d) => prefixes.some((prefix) => d.path.startsWith(prefix)))
    .map((d) => ({ path: d.path, bytes: d.bytes, revision: d.revision }));
  return sendJson(ctx.res, 200, { phase, prefixes, documents: inPhase });
}

export const projectRunRoutes: Route[] = [
  { method: "GET", path: "/v1/projects/:id/gate/reading", auth: "source", handle: gateReading },
  { method: "GET", path: "/v1/projects/:id/gates", auth: "source", handle: listGates },
  { method: "POST", path: "/v1/projects/:id/gate/sign", auth: "source", handle: signGate },
  { method: "POST", path: "/v1/projects/:id/task-run/workspace", auth: "source", handle: prepare },
  { method: "GET", path: "/v1/projects/:id/task-run/workspace", auth: "source", handle: workspaceOf },
  { method: "GET", path: "/v1/projects/:id/task-runs", auth: "source", handle: listActive },
  { method: "GET", path: "/v1/projects/:id/task-run", auth: "source", handle: taskRuns },
  { method: "POST", path: "/v1/projects/:id/task-run", auth: "source", handle: assign },
  { method: "PATCH", path: "/v1/projects/:id/task-run", auth: "source", handle: move },
];
