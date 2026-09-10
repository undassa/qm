import { randomUUID } from "node:crypto";
import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

export const REPOSITORY_PROVIDERS = ["github", "gitlab", "other"] as const;
export type RepositoryProvider = (typeof REPOSITORY_PROVIDERS)[number];

export interface ProjectRepository {
  id: string;
  projectId: string;
  /** Короткая метка: ею репозиторий называют в задаче и в прогоне. */
  name: string;
  url: string;
  provider: RepositoryProvider;
  baseBranch: string;
  /** Ссылка на учётку в keychain; секреты здесь не хранятся. */
  credentialId: string | null;
  isDefault: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface RepositoryInput {
  name: string;
  url: string;
  provider?: RepositoryProvider;
  baseBranch?: string;
  credentialId?: string | null;
  isDefault?: boolean;
}

export type RepositoryWriteResult =
  | { status: "ok"; repository: ProjectRepository }
  | { status: "invalid"; message: string }
  | { status: "duplicate" }
  | { status: "not_found" };

const HTTPS = /^https:\/\/([^/\s@]+)\/(\S+)$/;
const SCP = /^[A-Za-z0-9._-]+@([^:\s]+):(\S+)$/;
const SSH = /^ssh:\/\/[^@\s]+@([^/\s]+)\/(\S+)$/;
const MAX_NAME = 60;
const MAX_URL = 400;

/** Хост из адреса — по нему выводится провайдер и по нему же адрес признаётся адресом. */
export function repositoryHost(url: string): string | null {
  const trimmed = url.trim();
  const match = HTTPS.exec(trimmed) ?? SSH.exec(trimmed) ?? SCP.exec(trimmed);
  return match ? (match[1] ?? null) : null;
}

/**
 * Провайдера выводим из хоста: от него зависит, как потом спрашивать состояние PR.
 * Незнакомый хост — не ошибка, это самостоятельный git.
 */
export function providerOf(url: string): RepositoryProvider {
  const host = (repositoryHost(url) ?? "").toLowerCase();
  if (host === "github.com" || host.endsWith(".github.com")) return "github";
  if (host === "gitlab.com" || host.startsWith("gitlab.")) return "gitlab";
  return "other";
}

export function validateRepository(input: Partial<RepositoryInput>): string | null {
  const name = (input.name ?? "").trim();
  if (!name) return "имя репозитория обязательно";
  if (name.length > MAX_NAME) return `имя длиннее ${MAX_NAME} символов`;

  const url = (input.url ?? "").trim();
  if (!url) return "адрес репозитория обязателен";
  if (url.length > MAX_URL) return `адрес длиннее ${MAX_URL} символов`;
  if (!repositoryHost(url)) return "адрес должен быть https://, ssh:// или git@host:path";

  const branch = (input.baseBranch ?? "main").trim();
  if (!branch) return "базовая ветка обязательна";
  if (/\s/.test(branch)) return "в имени ветки не бывает пробелов";

  if (input.provider !== undefined && !REPOSITORY_PROVIDERS.includes(input.provider)) {
    return `провайдер ${String(input.provider)} не поддерживается`;
  }
  return null;
}

function normalize(input: RepositoryInput): Omit<ProjectRepository, "id" | "projectId" | "createdAt" | "updatedAt"> {
  const url = input.url.trim();
  return {
    name: input.name.trim(),
    url,
    provider: input.provider ?? providerOf(url),
    baseBranch: (input.baseBranch ?? "main").trim(),
    credentialId: input.credentialId?.trim() || null,
    isDefault: input.isDefault ?? false,
  };
}

export interface RepositoryStore {
  list(projectId: string): Promise<ProjectRepository[]>;
  get(projectId: string, id: string): Promise<ProjectRepository | null>;
  /** Репозиторий по умолчанию — тот, куда пойдёт прогон, если задача не назвала другой. */
  primary(projectId: string): Promise<ProjectRepository | null>;
  create(projectId: string, input: RepositoryInput): Promise<RepositoryWriteResult>;
  update(projectId: string, id: string, patch: Partial<RepositoryInput>): Promise<RepositoryWriteResult>;
  remove(projectId: string, id: string): Promise<boolean>;
  close?(): Promise<void>;
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_repositories(
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    provider TEXT NOT NULL,
    base_branch TEXT NOT NULL DEFAULT 'main',
    credential_id TEXT,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
  )`,
  `CREATE UNIQUE INDEX IF NOT EXISTS project_repositories_name
    ON project_repositories(project_id, lower(name))`,
  // По умолчанию может быть только один: иначе «куда писать» перестаёт иметь однозначный ответ.
  `CREATE UNIQUE INDEX IF NOT EXISTS project_repositories_one_default
    ON project_repositories(project_id) WHERE is_default`,
];

type Row = Record<string, unknown>;

function rowToRepository(row: Row): ProjectRepository {
  return {
    id: String(row["id"]),
    projectId: String(row["project_id"]),
    name: String(row["name"]),
    url: String(row["url"]),
    provider: String(row["provider"]) as RepositoryProvider,
    baseBranch: String(row["base_branch"]),
    credentialId: row["credential_id"] === null ? null : String(row["credential_id"]),
    isDefault: row["is_default"] === true,
    createdAt: Number(row["created_at"]),
    updatedAt: Number(row["updated_at"]),
  };
}

// ─── память ────────────────────────────────────────────────────────────────

export function createMemoryRepositoryStore(now: () => number = Date.now): RepositoryStore {
  const items = new Map<string, ProjectRepository>();
  const of = (projectId: string) => [...items.values()].filter((r) => r.projectId === projectId);
  const taken = (projectId: string, name: string, exceptId?: string) =>
    of(projectId).some((r) => r.name.toLowerCase() === name.toLowerCase() && r.id !== exceptId);
  const clearDefault = (projectId: string, exceptId: string) => {
    for (const other of of(projectId)) {
      if (other.id !== exceptId && other.isDefault) items.set(other.id, { ...other, isDefault: false });
    }
  };

  return {
    async list(projectId) {
      return of(projectId).sort((a, b) => Number(b.isDefault) - Number(a.isDefault) || a.name.localeCompare(b.name));
    },

    async get(projectId, id) {
      const found = items.get(id);
      return found && found.projectId === projectId ? found : null;
    },

    async primary(projectId) {
      const all = of(projectId);
      return all.find((r) => r.isDefault) ?? all[0] ?? null;
    },

    async create(projectId, input) {
      const invalid = validateRepository(input);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(input);
      if (taken(projectId, fields.name)) return { status: "duplicate" };
      const at = now();
      // Первый репозиторий проекта становится основным сам — иначе «по умолчанию» не будет ни одного.
      const isDefault = fields.isDefault || of(projectId).length === 0;
      const repository: ProjectRepository = {
        id: randomUUID(),
        projectId,
        ...fields,
        isDefault,
        createdAt: at,
        updatedAt: at,
      };
      items.set(repository.id, repository);
      if (isDefault) clearDefault(projectId, repository.id);
      return { status: "ok", repository };
    },

    async update(projectId, id, patch) {
      const current = items.get(id);
      if (!current || current.projectId !== projectId) return { status: "not_found" };
      const merged = { ...current, ...patch } as RepositoryInput;
      const invalid = validateRepository(merged);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(merged);
      if (taken(projectId, fields.name, id)) return { status: "duplicate" };
      const repository: ProjectRepository = { ...current, ...fields, updatedAt: now() };
      items.set(id, repository);
      if (repository.isDefault) clearDefault(projectId, id);
      return { status: "ok", repository };
    },

    async remove(projectId, id) {
      const found = items.get(id);
      if (!found || found.projectId !== projectId) return false;
      items.delete(id);
      // Проект не должен остаться без основного, пока есть хоть один репозиторий.
      if (found.isDefault) {
        const next = of(projectId)[0];
        if (next) items.set(next.id, { ...next, isDefault: true });
      }
      return true;
    },
  };
}

// ─── postgres ──────────────────────────────────────────────────────────────

function isDuplicate(error: unknown): boolean {
  return typeof error === "object" && error !== null && (error as { code?: string }).code === "23505";
}

export function createPostgresRepositoryStore(
  connectionString: string,
  opts: { now?: () => number } = {},
): RepositoryStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, SCHEMA);

  return {
    async list(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_repositories WHERE project_id=$1 ORDER BY is_default DESC, lower(name)`,
        [projectId],
      );
      return rows.map((row) => rowToRepository(row as Row));
    },

    async get(projectId, id) {
      const { rows } = await pg.query(`SELECT * FROM project_repositories WHERE project_id=$1 AND id=$2`, [
        projectId,
        id,
      ]);
      return rows[0] ? rowToRepository(rows[0] as Row) : null;
    },

    async primary(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_repositories WHERE project_id=$1 ORDER BY is_default DESC, created_at LIMIT 1`,
        [projectId],
      );
      return rows[0] ? rowToRepository(rows[0] as Row) : null;
    },

    async create(projectId, input) {
      const invalid = validateRepository(input);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(input);
      try {
        return await withPgTransaction(await pg.pool(), async (client) => {
          const count = await client.query(`SELECT count(*) AS n FROM project_repositories WHERE project_id=$1`, [
            projectId,
          ]);
          const isDefault = fields.isDefault || Number((count.rows[0] as Row)["n"]) === 0;
          if (isDefault) {
            await client.query(`UPDATE project_repositories SET is_default=FALSE WHERE project_id=$1 AND is_default`, [
              projectId,
            ]);
          }
          const at = now();
          const { rows } = await client.query(
            `INSERT INTO project_repositories(id, project_id, name, url, provider, base_branch, credential_id, is_default, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9) RETURNING *`,
            [
              randomUUID(),
              projectId,
              fields.name,
              fields.url,
              fields.provider,
              fields.baseBranch,
              fields.credentialId,
              isDefault,
              at,
            ],
          );
          return { status: "ok" as const, repository: rowToRepository(rows[0] as Row) };
        });
      } catch (error) {
        if (isDuplicate(error)) return { status: "duplicate" };
        throw error;
      }
    },

    async update(projectId, id, patch) {
      const current = await this.get(projectId, id);
      if (!current) return { status: "not_found" };
      const merged = { ...current, ...patch } as RepositoryInput;
      const invalid = validateRepository(merged);
      if (invalid) return { status: "invalid", message: invalid };
      const fields = normalize(merged);
      try {
        return await withPgTransaction(await pg.pool(), async (client) => {
          if (fields.isDefault) {
            await client.query(
              `UPDATE project_repositories SET is_default=FALSE WHERE project_id=$1 AND id<>$2 AND is_default`,
              [projectId, id],
            );
          }
          const { rows } = await client.query(
            `UPDATE project_repositories SET name=$3, url=$4, provider=$5, base_branch=$6,
               credential_id=$7, is_default=$8, updated_at=$9
              WHERE project_id=$1 AND id=$2 RETURNING *`,
            [
              projectId,
              id,
              fields.name,
              fields.url,
              fields.provider,
              fields.baseBranch,
              fields.credentialId,
              fields.isDefault,
              now(),
            ],
          );
          return rows[0]
            ? { status: "ok" as const, repository: rowToRepository(rows[0] as Row) }
            : { status: "not_found" as const };
        });
      } catch (error) {
        if (isDuplicate(error)) return { status: "duplicate" };
        throw error;
      }
    },

    async remove(projectId, id) {
      return withPgTransaction(await pg.pool(), async (client) => {
        const { rows } = await client.query(
          `DELETE FROM project_repositories WHERE project_id=$1 AND id=$2 RETURNING is_default`,
          [projectId, id],
        );
        if (!rows[0]) return false;
        if ((rows[0] as Row)["is_default"] === true) {
          await client.query(
            `UPDATE project_repositories SET is_default=TRUE
              WHERE id = (SELECT id FROM project_repositories WHERE project_id=$1 ORDER BY created_at LIMIT 1)`,
            [projectId],
          );
        }
        return true;
      });
    },

    close: () => pg.close(),
  };
}
