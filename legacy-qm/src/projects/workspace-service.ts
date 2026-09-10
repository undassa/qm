import type { Sandbox } from "../sandbox/sandbox.ts";
import { branchNameFor, type TaskWorkspace, type WorkspaceStore } from "./workspace-store.ts";

export interface PrepareWorkspaceInput {
  projectId: string;
  /** Область проекта — в ней живёт песочница, поэтому копии задач переживают отдельный прогон. */
  scopeId: string;
  taskRunId: string;
  taskId: string;
  attempt: number;
  repository: { id: string; url: string; baseBranch: string };
  /** Токен доступа к приватному репозиторию; нужен только для https-адресов. */
  token?: string | null;
}

export type PrepareWorkspaceResult =
  { status: "ready"; workspace: TaskWorkspace } | { status: "failed"; workspace: TaskWorkspace | null; detail: string };

const ASKPASS = "task-askpass.sh";

/**
 * Отдаём git токен через GIT_ASKPASS, а не через адрес: иначе секрет осел бы
 * в `.git/config` клона и уехал бы вместе с ним. Файл живёт только на время подготовки.
 */
function askpassScript(token: string): string {
  return [
    "#!/bin/sh",
    'case "$1" in',
    "  Username*) echo x-access-token ;;",
    `  *) echo ${shellQuote(token)} ;;`,
    "esac",
    "",
  ].join("\n");
}

export interface WorkspaceServiceDeps {
  sandbox: Sandbox;
  workspaces: WorkspaceStore;
  timeoutMs?: number;
}

/** Одна команда git — со стандартным таймаутом и понятным текстом отказа. */
async function git(
  deps: WorkspaceServiceDeps,
  handle: Awaited<ReturnType<Sandbox["provision"]>>,
  command: string,
): Promise<{ ok: boolean; out: string; detail: string }> {
  const result = await deps.sandbox.run(handle, command, { timeoutMs: deps.timeoutMs ?? 300_000 });
  if (result.timedOut) return { ok: false, out: "", detail: `команда не уложилась в срок: ${command}` };
  if (result.code !== 0) {
    const said = (result.stderr || result.stdout || "").trim().slice(0, 600);
    return { ok: false, out: "", detail: `${command} → код ${result.code}${said ? `: ${said}` : ""}` };
  }
  return { ok: true, out: result.stdout.trim(), detail: "" };
}

/**
 * Готовит рабочую копию задачи: поднимает песочницу области проекта, приносит репозиторий
 * и отводит ветку от базовой. Коммит основания запоминается — по нему потом считается диф.
 *
 * Копия живёт в области проекта, а не прогона: повторная попытка не выкачивает репозиторий заново.
 */
export async function prepareWorkspace(
  deps: WorkspaceServiceDeps,
  input: PrepareWorkspaceInput,
): Promise<PrepareWorkspaceResult> {
  const path = `tasks/${input.taskRunId}`;
  const branch = branchNameFor(input.taskId, input.attempt);
  const workspace = await deps.workspaces.begin({
    projectId: input.projectId,
    taskRunId: input.taskRunId,
    repositoryId: input.repository.id,
    path,
    baseBranch: input.repository.baseBranch,
    branch,
  });

  const fail = async (detail: string): Promise<PrepareWorkspaceResult> => ({
    status: "failed",
    workspace: await deps.workspaces.fail(workspace.id, detail),
    detail,
  });

  const https = input.repository.url.startsWith("https://");
  if (input.token && !https) {
    return await fail("токен подходит только к https-адресу — у репозитория указан ssh");
  }

  try {
    const handle = await deps.sandbox.provision([{ scopeId: input.scopeId, mountPath: "", mode: "rw" }]);
    const url = shellQuote(input.repository.url);
    const dir = shellQuote(path);
    const base = shellQuote(input.repository.baseBranch);
    const head = shellQuote(branch);

    // Токен кладём в скрипт askpass и убираем сразу после подготовки.
    let prefix = "GIT_TERMINAL_PROMPT=0 ";
    if (input.token) {
      await deps.sandbox.writeFile(handle, ASKPASS, askpassScript(input.token));
      await deps.sandbox.run(handle, `chmod 700 ${shellQuote(ASKPASS)}`, { timeoutMs: 30_000 });
      prefix = `GIT_TERMINAL_PROMPT=0 GIT_ASKPASS="$PWD/${ASKPASS}" `;
    }
    const done = async <T>(value: T): Promise<T> => {
      if (input.token) await deps.sandbox.run(handle, `rm -f ${shellQuote(ASKPASS)}`, { timeoutMs: 30_000 });
      return value;
    };

    // Клонируем только если каталога ещё нет: повторная подготовка не должна выкачивать всё заново.
    const clone = await git(
      deps,
      handle,
      `${prefix}sh -c 'if [ -d ${dir}/.git ]; then git -C ${dir} remote set-url origin ${url}; else mkdir -p ${dir} && git clone ${url} ${dir}; fi'`,
    );
    if (!clone.ok) return await done(await fail(clone.detail));

    const fetch = await git(deps, handle, `${prefix}git -C ${dir} fetch --prune origin ${base}`);
    if (!fetch.ok) return await done(await fail(fetch.detail));

    const checkout = await git(deps, handle, `git -C ${dir} checkout -B ${head} origin/${base}`);
    if (!checkout.ok) return await done(await fail(checkout.detail));

    const commit = await git(deps, handle, `git -C ${dir} rev-parse HEAD`);
    if (!commit.ok) return await done(await fail(commit.detail));

    const ready = await deps.workspaces.ready(workspace.id, commit.out);
    await done(null);
    return ready
      ? { status: "ready", workspace: ready }
      : { status: "failed", workspace: null, detail: "запись рабочей копии пропала" };
  } catch (error) {
    return await fail(error instanceof Error ? error.message : String(error));
  }
}

/** Адрес и ветка приходят из формы, поэтому в командную строку они попадают только в кавычках. */
export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}
