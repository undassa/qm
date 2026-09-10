import test from "node:test";
import assert from "node:assert/strict";
import { cleanRepositoryInput, createProjectStore } from "../src/projects/project-store.ts";

const store = (): ReturnType<typeof createProjectStore> =>
  createProjectStore(undefined, { isActiveMember: () => true });

async function withProject(): Promise<{
  projects: ReturnType<typeof createProjectStore>;
  id: string;
  owner: string;
}> {
  const projects = store();
  const project = await projects.create({ name: "cortexy", ownerId: "ann" });
  return { projects, id: project.id, owner: "ann" };
}

test("a remote that is not a git address is refused before it reaches the project", () => {
  for (const remote of [
    "",
    "not a url",
    "http://insecure.example.com/repo",
    "javascript:alert(1)",
    "file:///etc/passwd",
    "git@host",
    "https://host",
  ]) {
    assert.equal(cleanRepositoryInput({ name: "repo", remote }), null, JSON.stringify(remote));
  }
  assert.deepEqual(cleanRepositoryInput({ name: "myack", remote: "git@github.com:myack-dev/myack.git" }), {
    name: "myack",
    remote: "git@github.com:myack-dev/myack.git",
    defaultBranch: "main",
  });
  assert.deepEqual(
    cleanRepositoryInput({ name: "mh", remote: "https://github.com/undassa/mh", defaultBranch: "harness" }),
    {
      name: "mh",
      remote: "https://github.com/undassa/mh",
      defaultBranch: "harness",
    },
  );
});

test("a member attaches a repository, and it comes back on the project", async () => {
  const { projects, id, owner } = await withProject();
  const added = await projects.addRepository(id, owner, {
    name: "myack",
    remote: "git@github.com:myack-dev/myack.git",
  });
  assert.equal(added.status, "ok");
  const project = await projects.get(id);
  assert.equal(project?.repositories?.length, 1);
  const repository = project!.repositories![0]!;
  assert.equal(repository.name, "myack");
  assert.equal(repository.defaultBranch, "main");
  assert.equal(repository.addedBy, owner);
  assert.ok(repository.id);
});

test("a project refuses two repositories under one name", async () => {
  const { projects, id, owner } = await withProject();
  const input = { name: "myack", remote: "git@github.com:myack-dev/myack.git" };
  assert.equal((await projects.addRepository(id, owner, input)).status, "ok");
  const twice = await projects.addRepository(id, owner, { ...input, remote: "https://github.com/other/other" });
  assert.equal(twice.status, "invalid_repository");
  assert.equal((await projects.get(id))?.repositories?.length, 1);
});

test("a stranger may not attach a repository to someone else's project", async () => {
  const { projects, id } = await withProject();
  const denied = await projects.addRepository(id, "mallory", {
    name: "theirs",
    remote: "git@github.com:mallory/theirs.git",
  });
  assert.equal(denied.status, "forbidden");
  assert.equal((await projects.get(id))?.repositories, undefined);
});

test("detaching removes only the named repository, and says when there was nothing to detach", async () => {
  const { projects, id, owner } = await withProject();
  await projects.addRepository(id, owner, { name: "one", remote: "git@github.com:o/one.git" });
  await projects.addRepository(id, owner, { name: "two", remote: "git@github.com:o/two.git" });
  const project = await projects.get(id);
  const first = project!.repositories!.find((r) => r.name === "one")!;

  const removed = await projects.removeRepository(id, owner, first.id);
  assert.equal(removed.status, "ok");
  assert.equal(removed.status === "ok" && removed.changed, true);
  assert.deepEqual(
    (await projects.get(id))?.repositories?.map((r) => r.name),
    ["two"],
  );

  const again = await projects.removeRepository(id, owner, first.id);
  assert.equal(again.status === "ok" && again.changed, false);
});

test("an unknown project is not found rather than silently created", async () => {
  const projects = store();
  const missing = await projects.addRepository("no-such-project", "ann", {
    name: "x",
    remote: "git@github.com:o/x.git",
  });
  assert.equal(missing.status, "not_found");
});
