import { sendJson } from "../http.ts";
import { isObj } from "./shared.ts";
import { projectGroupRef } from "../../projects/project-store.ts";
import {
  AGENT_HARNESSES,
  AGENT_SANDBOXES,
  type AgentInput,
  type AgentStore,
  type AgentWriteResult,
} from "../../projects/agent-store.ts";
import { type ApiCtx, type Route } from "./route.ts";

interface Access {
  projectId: string;
  principalId: string;
  agents: AgentStore;
}

async function access(ctx: ApiCtx, projectId: string): Promise<Access | null> {
  const agents = ctx.deps.projectAgents;
  const projects = ctx.deps.projects;
  if (!agents || !projects) {
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
  return { projectId, principalId, agents };
}

/** Тело формы приходит из браузера, поэтому каждое поле берём по одному и с проверкой вида. */
function readInput(ctx: ApiCtx): Partial<AgentInput> {
  const body = isObj(ctx.body) ? ctx.body : {};
  const patch: Partial<AgentInput> = {};
  if (typeof body["name"] === "string") patch.name = body["name"];
  if (typeof body["role"] === "string") patch.role = body["role"];
  if (typeof body["harness"] === "string") patch.harness = body["harness"] as AgentInput["harness"];
  if (typeof body["model"] === "string" || body["model"] === null) patch.model = body["model"] as string | null;
  if (typeof body["sandbox"] === "string" || body["sandbox"] === null) {
    patch.sandbox = body["sandbox"] as AgentInput["sandbox"];
  }
  if (typeof body["concurrency"] === "number") patch.concurrency = body["concurrency"];
  if (typeof body["enabled"] === "boolean") patch.enabled = body["enabled"];
  return patch;
}

/** Один перевод исхода записи в ответ — иначе создание и правка разойдутся в кодах. */
function sendWrite(ctx: ApiCtx, result: AgentWriteResult, okStatus: number): void {
  if (result.status === "ok") return sendJson(ctx.res, okStatus, { agent: result.agent });
  if (result.status === "invalid") return sendJson(ctx.res, 400, { error: "bad_request", message: result.message });
  if (result.status === "duplicate") {
    return sendJson(ctx.res, 409, { error: "conflict", message: "агент с таким именем в проекте уже есть" });
  }
  return sendJson(ctx.res, 404, { error: "not_found" });
}

function requiredAgentId(ctx: ApiCtx): string | null {
  const id = (ctx.url.searchParams.get("agentId") ?? "").trim();
  if (id) return id;
  sendJson(ctx.res, 400, { error: "bad_request", message: "agentId required" });
  return null;
}

async function listAgents(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const agents = await granted.agents.list(granted.projectId);
  return sendJson(ctx.res, 200, { agents, harnesses: AGENT_HARNESSES, sandboxes: AGENT_SANDBOXES });
}

async function createAgent(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const patch = readInput(ctx);
  if (!patch.name || !patch.harness) {
    return sendJson(ctx.res, 400, { error: "bad_request", message: "name and harness required" });
  }
  return sendWrite(ctx, await granted.agents.create(granted.projectId, patch as AgentInput), 201);
}

async function updateAgent(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const agentId = requiredAgentId(ctx);
  if (agentId === null) return;
  return sendWrite(ctx, await granted.agents.update(granted.projectId, agentId, readInput(ctx)), 200);
}

async function deleteAgent(ctx: ApiCtx): Promise<void> {
  const granted = await access(ctx, ctx.params.id!);
  if (!granted) return;
  const agentId = requiredAgentId(ctx);
  if (agentId === null) return;
  const removed = await granted.agents.remove(granted.projectId, agentId);
  return removed ? sendJson(ctx.res, 200, { removed: true }) : sendJson(ctx.res, 404, { error: "not_found" });
}

export const projectAgentRoutes: Route[] = [
  { method: "GET", path: "/v1/projects/:id/agents", auth: "source", handle: listAgents },
  { method: "POST", path: "/v1/projects/:id/agents", auth: "source", handle: createAgent },
  { method: "PATCH", path: "/v1/projects/:id/agent", auth: "source", handle: updateAgent },
  { method: "DELETE", path: "/v1/projects/:id/agent", auth: "source", handle: deleteAgent },
];
