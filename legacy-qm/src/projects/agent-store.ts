import { randomUUID } from "node:crypto";
import { createPgPool } from "../persistence/pg-pool.ts";

/** Харнесы, которые ядро действительно умеет запускать — список сверен с config.ts. */
export const AGENT_HARNESSES = ["claude", "codex", "opencode", "pi", "mock"] as const;
export type AgentHarness = (typeof AGENT_HARNESSES)[number];

/** Бэкенды песочницы, объявленные реализациями в src/sandbox. */
export const AGENT_SANDBOXES = ["local-docker", "aws-microvm", "smolmachines", "sprites"] as const;
export type AgentSandbox = (typeof AGENT_SANDBOXES)[number];

export const MAX_AGENT_CONCURRENCY = 20;

export interface ProjectAgent {
  id: string;
  projectId: string;
  name: string;
  role: string;
  harness: AgentHarness;
  /** null — наследовать модель проекта; агент не обязан её переопределять. */
  model: string | null;
  /** null — наследовать песочницу по умолчанию. */
  sandbox: AgentSandbox | null;
  /** Сколько задач агент ведёт одновременно. */
  concurrency: number;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface AgentInput {
  name: string;
  role?: string;
  harness: AgentHarness;
  model?: string | null;
  sandbox?: AgentSandbox | null;
  concurrency?: number;
  enabled?: boolean;
}

export type AgentWriteResult =
  | { status: "ok"; agent: ProjectAgent }
  | { status: "invalid"; message: string }
  | { status: "duplicate" }
  | { status: "not_found" };

const MAX_NAME = 80;
const MAX_ROLE = 500;

/**
 * Проверяем до записи, а не полагаемся на ограничения схемы: сообщение об отказе
 * должно называть поле, иначе человек в форме не поймёт, что исправлять.
 */
export function validateAgent(input: Partial<AgentInput>): string | null {
  const name = (input.name ?? "").trim();
  if (!name) return "имя агента обязательно";
  if (name.length > MAX_NAME) return `имя длиннее ${MAX_NAME} символов`;
  if ((input.role ?? "").length > MAX_ROLE) return `роль длиннее ${MAX_ROLE} символов`;
  if (input.harness !== undefined && !AGENT_HARNESSES.includes(input.harness)) {
    return `харнес ${String(input.harness)} не поддерживается`;
  }
  if (input.sandbox !== undefined && input.sandbox !== null && !AGENT_SANDBOXES.includes(input.sandbox)) {
    return `песочница ${String(input.sandbox)} не поддерживается`;
  }
  const concurrency = input.concurrency;
  if (concurrency !== undefined) {
    if (!Number.isInteger(concurrency) || concurrency < 1) return "предел параллелизма — целое от 1";
    if (concurrency > MAX_AGENT_CONCURRENCY) return `предел параллелизма больше ${MAX_AGENT_CONCURRENCY}`;
  }
  return null;
}

function normalize(input: AgentInput): Omit<ProjectAgent, "id" | "projectId" | "createdAt" | "updatedAt"> {
  return {
    name: input.name.trim(),
    role: (input.role ?? "").trim(),
    harness: input.harness,
    model: input.model?.trim() || null,
    sandbox: input.sandbox ?? null,
    concurrency: input.concurrency ?? 1,
    enabled: input.enabled ?? true,
  };
}

export interface AgentStore {
  list(projectId: string): Promise<ProjectAgent[]>;
  get(projectId: string, id: string): Promise<ProjectAgent | null>;
  create(projectId: string, input: AgentInput): Promise<AgentWriteResult>;
  update(projectId: string, id: string, patch: Partial<AgentInput>): Promise<AgentWriteResult>;
  remove(projectId: string, id: string): Promise<boolean>;
  close?(): Promise<void>;
}

// ─── память ────────────────────────────────────────────────────────────────

export function createMemoryAgentStore(now: () => number = Date.now): AgentStore {
  const agents = new Map<string, ProjectAgent>();
  const of = (projectId: string) => [...agents.values()].filter((a) => a.projectId === projectId);
  const taken = (projectId: string, name: string, exceptId?: string) =>
    of(projectId).some((a) => a.name.toLowerCase() === name.toLowerCase() && a.id !== exceptId);

  return {
    async list(projectId) {
      return of(projectId).sort((a, b) => a.name.localeCompare(b.name));
    },

    async get(projectId, id) {
      const found = agents.get(id);
      return found && found.projectId === projectId ? found : null;
    },

    async create(projectId, input) {
      const invalid = validateAgent(input);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(input);
      if (taken(projectId, fields.name)) return { status: "duplicate" };
      const at = now();
      const agent: ProjectAgent = { id: randomUUID(), projectId, ...fields, createdAt: at, updatedAt: at };
      agents.set(agent.id, agent);
      return { status: "ok", agent };
    },

    async update(projectId, id, patch) {
      const current = agents.get(id);
      if (!current || current.projectId !== projectId) return { status: "not_found" };
      const merged = { ...current, ...patch } as AgentInput;
      const invalid = validateAgent(merged);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(merged);
      if (taken(projectId, fields.name, id)) return { status: "duplicate" };
      const agent: ProjectAgent = { ...current, ...fields, updatedAt: now() };
      agents.set(id, agent);
      return { status: "ok", agent };
    },

    async remove(projectId, id) {
      const found = agents.get(id);
      if (!found || found.projectId !== projectId) return false;
      agents.delete(id);
      return true;
    },
  };
}

// ─── postgres ──────────────────────────────────────────────────────────────

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_agents(
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT '',
    harness TEXT NOT NULL,
    model TEXT,
    sandbox TEXT,
    concurrency INTEGER NOT NULL DEFAULT 1 CHECK (concurrency >= 1 AND concurrency <= 20),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
  )`,
  // Имя — то, чем агента зовут в назначении, поэтому оно обязано быть однозначным внутри проекта.
  `CREATE UNIQUE INDEX IF NOT EXISTS project_agents_name
    ON project_agents(project_id, lower(name))`,
];

type Row = Record<string, unknown>;

function rowToAgent(row: Row): ProjectAgent {
  return {
    id: String(row["id"]),
    projectId: String(row["project_id"]),
    name: String(row["name"]),
    role: String(row["role"] ?? ""),
    harness: String(row["harness"]) as AgentHarness,
    model: row["model"] === null ? null : String(row["model"]),
    sandbox: row["sandbox"] === null ? null : (String(row["sandbox"]) as AgentSandbox),
    concurrency: Number(row["concurrency"]),
    enabled: row["enabled"] === true,
    createdAt: Number(row["created_at"]),
    updatedAt: Number(row["updated_at"]),
  };
}

/** Нарушение уникального имени приходит из базы кодом 23505 — переводим в отказ, а не в падение. */
function isDuplicate(error: unknown): boolean {
  return typeof error === "object" && error !== null && (error as { code?: string }).code === "23505";
}

export function createPostgresAgentStore(connectionString: string, opts: { now?: () => number } = {}): AgentStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, SCHEMA);

  return {
    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_agents WHERE project_id=$1 ORDER BY lower(name)`, [
        projectId,
      ]);
      return rows.map((row) => rowToAgent(row as Row));
    },

    async get(projectId, id) {
      const { rows } = await pg.query(`SELECT * FROM project_agents WHERE project_id=$1 AND id=$2`, [projectId, id]);
      return rows[0] ? rowToAgent(rows[0] as Row) : null;
    },

    async create(projectId, input) {
      const invalid = validateAgent(input);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(input);
      const at = now();
      const id = randomUUID();
      try {
        const { rows } = await pg.query(
          `INSERT INTO project_agents(id, project_id, name, role, harness, model, sandbox, concurrency, enabled, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10) RETURNING *`,
          [
            id,
            projectId,
            fields.name,
            fields.role,
            fields.harness,
            fields.model,
            fields.sandbox,
            fields.concurrency,
            fields.enabled,
            at,
          ],
        );
        return { status: "ok", agent: rowToAgent(rows[0] as Row) };
      } catch (error) {
        if (isDuplicate(error)) return { status: "duplicate" };
        throw error;
      }
    },

    async update(projectId, id, patch) {
      const current = await this.get(projectId, id);
      if (!current) return { status: "not_found" };
      const merged = { ...current, ...patch } as AgentInput;
      const invalid = validateAgent(merged);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(merged);
      try {
        const { rows } = await pg.query(
          `UPDATE project_agents SET name=$3, role=$4, harness=$5, model=$6, sandbox=$7,
             concurrency=$8, enabled=$9, updated_at=$10
            WHERE project_id=$1 AND id=$2 RETURNING *`,
          [
            projectId,
            id,
            fields.name,
            fields.role,
            fields.harness,
            fields.model,
            fields.sandbox,
            fields.concurrency,
            fields.enabled,
            now(),
          ],
        );
        return rows[0] ? { status: "ok", agent: rowToAgent(rows[0] as Row) } : { status: "not_found" };
      } catch (error) {
        if (isDuplicate(error)) return { status: "duplicate" };
        throw error;
      }
    },

    async remove(projectId, id) {
      const { rows } = await pg.query(`DELETE FROM project_agents WHERE project_id=$1 AND id=$2 RETURNING id`, [
        projectId,
        id,
      ]);
      return rows.length > 0;
    },

    close: () => pg.close(),
  };
}
