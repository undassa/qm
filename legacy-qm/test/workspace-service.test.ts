import test from "node:test";
import assert from "node:assert/strict";
import { branchNameFor } from "../src/projects/workspace-store.ts";
import { prepareWorkspace, shellQuote, type WorkspaceServiceDeps } from "../src/projects/workspace-service.ts";
import type { TaskWorkspace, WorkspaceStore } from "../src/projects/workspace-store.ts";

test("имя ветки безопасно для git и различает попытки", () => {
  assert.equal(branchNameFor("M1-T10", 1), "task/m1-t10");
  assert.equal(branchNameFor("M1-T10", 2), "task/m1-t10-2", "вторая попытка не наступает на ветку первой");
  assert.equal(branchNameFor("M0-T2a", 1), "task/m0-t2a");
  assert.equal(branchNameFor("задача №1", 1), "task/1", "кириллица и знаки вычищаются, ссылка остаётся годной");
  assert.equal(branchNameFor("", 1), "task/task", "пустой идентификатор не даёт пустую ссылку");
});

test("адрес из формы попадает в команду только в кавычках", () => {
  assert.equal(shellQuote("https://github.com/a/b.git"), "'https://github.com/a/b.git'");
  assert.equal(shellQuote("a'; rm -rf /; echo '"), `'a'\\''; rm -rf /; echo '\\'''`);
});

interface Recorded {
  commands: string[];
}

function fakeDeps(
  run: (command: string) => { stdout?: string; stderr?: string; code?: number; timedOut?: boolean },
  recorded: Recorded,
): { deps: WorkspaceServiceDeps; saved: TaskWorkspace[] } {
  const saved: TaskWorkspace[] = [];
  const base: TaskWorkspace = {
    id: "w1",
    projectId: "p",
    taskRunId: "r1",
    repositoryId: "repo",
    state: "preparing",
    path: "tasks/r1",
    baseBranch: "main",
    baseCommit: null,
    branch: "task/m1-t1",
    detail: "",
    createdAt: 1,
    updatedAt: 1,
  };
  const workspaces: WorkspaceStore = {
    async begin() {
      saved.push(base);
      return base;
    },
    async ready(_id, baseCommit) {
      const next = { ...base, state: "ready" as const, baseCommit };
      saved.push(next);
      return next;
    },
    async fail(_id, detail) {
      const next = { ...base, state: "failed" as const, detail };
      saved.push(next);
      return next;
    },
    async forRun() {
      return null;
    },
  };
  const sandbox = {
    async provision() {
      return { id: "h1", rootDir: "/w" };
    },
    async run(_handle: unknown, command: string) {
      recorded.commands.push(command);
      const r = run(command);
      return { stdout: r.stdout ?? "", stderr: r.stderr ?? "", code: r.code ?? 0, timedOut: r.timedOut ?? false };
    },
  } as unknown as WorkspaceServiceDeps["sandbox"];
  return { deps: { sandbox, workspaces }, saved };
}

const input = {
  projectId: "p",
  scopeId: "group:web-project-p",
  taskRunId: "r1",
  taskId: "M1-T1",
  attempt: 1,
  repository: { id: "repo", url: "https://github.com/a/b.git", baseBranch: "main" },
};

test("готовая копия помнит коммит основания — по нему потом считается диф", async () => {
  const recorded: Recorded = { commands: [] };
  const { deps } = fakeDeps((c) => (c.includes("rev-parse") ? { stdout: "abc1234\n" } : {}), recorded);
  const result = await prepareWorkspace(deps, input);
  assert.equal(result.status, "ready");
  if (result.status !== "ready") return;
  assert.equal(result.workspace.baseCommit, "abc1234");
  assert.equal(result.workspace.branch, "task/m1-t1");
  assert.match(recorded.commands.join(" | "), /checkout -B 'task\/m1-t1' origin\/'main'/);
});

test("повторная подготовка не выкачивает репозиторий заново", async () => {
  const recorded: Recorded = { commands: [] };
  const { deps } = fakeDeps((c) => (c.includes("rev-parse") ? { stdout: "abc" } : {}), recorded);
  await prepareWorkspace(deps, input);
  const clone = recorded.commands.find((c) => c.includes("git clone"))!;
  assert.match(clone, /if \[ -d .*\/\.git \]/, "клон только когда каталога ещё нет");
});

test("отказ git сохраняется словами, а не молча роняет подготовку", async () => {
  const recorded: Recorded = { commands: [] };
  const { deps } = fakeDeps(
    (c) => (c.includes("fetch") ? { code: 128, stderr: "fatal: couldn't find remote ref main" } : {}),
    recorded,
  );
  const result = await prepareWorkspace(deps, input);
  assert.equal(result.status, "failed");
  if (result.status !== "failed") return;
  assert.match(result.detail, /couldn't find remote ref main/);
  assert.equal(result.workspace?.state, "failed");
  assert.equal(
    recorded.commands.some((c) => c.includes("checkout")),
    false,
    "после отказа дальше не идём",
  );
});

test("зависшая команда отличается от упавшей", async () => {
  const recorded: Recorded = { commands: [] };
  const { deps } = fakeDeps((c) => (c.includes("clone") ? { timedOut: true } : {}), recorded);
  const result = await prepareWorkspace(deps, input);
  assert.equal(result.status, "failed");
  if (result.status === "failed") assert.match(result.detail, /не уложилась в срок/);
});

test("токен уходит в askpass, а не в адрес — иначе он осел бы в .git/config клона", async () => {
  const recorded: Recorded = { commands: [] };
  const written: string[] = [];
  const { deps } = fakeDeps((c) => (c.includes("rev-parse") ? { stdout: "abc" } : {}), recorded);
  (deps.sandbox as unknown as { writeFile: (h: unknown, p: string, d: string) => Promise<void> }).writeFile = async (
    _h,
    _p,
    data,
  ) => {
    written.push(data);
  };
  const result = await prepareWorkspace(deps, { ...input, token: "ghp_секрет" });
  assert.equal(result.status, "ready");
  assert.match(written.join("\n"), /ghp_секрет/, "секрет лежит в скрипте askpass");
  const commands = recorded.commands.join(" | ");
  assert.doesNotMatch(commands, /ghp_секрет/, "и не появляется в командной строке");
  assert.match(commands, /GIT_ASKPASS=/);
  assert.match(commands, /rm -f 'task-askpass\.sh'/, "после подготовки скрипт убирается");
});

test("токен к ssh-адресу не подходит и это говорится словами", async () => {
  const recorded: Recorded = { commands: [] };
  const { deps } = fakeDeps(() => ({}), recorded);
  const result = await prepareWorkspace(deps, {
    ...input,
    repository: { id: "repo", url: "git@github.com:a/b.git", baseBranch: "main" },
    token: "ghp_x",
  });
  assert.equal(result.status, "failed");
  if (result.status === "failed") assert.match(result.detail, /только к https/);
  assert.equal(recorded.commands.length, 0, "до git дело не доходит");
});
