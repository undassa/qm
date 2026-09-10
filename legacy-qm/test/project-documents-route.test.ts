import "./support/auto-fake-sprites.ts";

import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import type { Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createInsecureTestServer } from "../src/api/server.ts";
import { projectDocumentRoutes } from "../src/api/routes/project-documents.ts";
import { buildApp, serverDeps } from "../src/wiring.ts";
import { testConfig } from "./support/test-config.ts";

const DOC = "# M0-T1 · Задача\n\n## Что делать\n\nПервое.\n\n## Чем доказывается\n\nВторое.\n";

function listen(server: Server): Promise<string> {
  return new Promise((resolve) =>
    server.listen(0, "127.0.0.1", () => resolve(`http://127.0.0.1:${(server.address() as AddressInfo).port}`)),
  );
}

async function stand(t: { after: (fn: () => Promise<void> | void) => void }): Promise<{
  base: string;
  projectId: string;
  owner: string;
}> {
  const config = testConfig({ dataDir: mkdtempSync(join(tmpdir(), "project-documents-")) });
  const built = buildApp(config);
  const server = createInsecureTestServer(built.app, serverDeps(config, built));
  const base = await listen(server);
  t.after(() => new Promise<void>((resolve) => server.close(() => resolve())));

  const owner = "ann";
  const created = await fetch(`${base}/v1/projects`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ principalId: owner, name: "cortexy" }),
  });
  assert.equal(created.status, 201, "проект заведён");
  const { project } = (await created.json()) as { project: { id: string } };
  await built.projectDocuments.put({ projectId: project.id, path: "task.md", content: DOC, author: owner });
  return { base, projectId: project.id, owner };
}

test("документы закрыты от агента: только подпись источника", () => {
  assert.ok(
    projectDocumentRoutes.every((route) => route.auth === "source"),
    "агент получит документы инструментом MCP, а не этой дверью — до тех пор она для поверхности",
  );
});

test("участник видит перечень, документ и его строение", async (t) => {
  const { base, projectId, owner } = await stand(t);
  const p = `principalId=${encodeURIComponent(owner)}`;

  const listed = await fetch(`${base}/v1/projects/${projectId}/documents?${p}`);
  assert.equal(listed.status, 200);
  const { documents } = (await listed.json()) as { documents: { path: string }[] };
  assert.deepEqual(
    documents.map((d) => d.path),
    ["task.md"],
  );

  const one = await fetch(`${base}/v1/projects/${projectId}/document?${p}&path=task.md`);
  assert.equal(one.status, 200);
  const body = (await one.json()) as {
    document: { content: string; revision: number };
    sections: { title: string; anchor: string }[];
  };
  assert.equal(body.document.content, DOC);
  assert.deepEqual(
    body.sections.map((s) => s.title),
    ["M0-T1 · Задача", "Что делать", "Чем доказывается"],
  );
});

test("посторонний не отличает чужой проект от несуществующего", async (t) => {
  const { base, projectId } = await stand(t);
  const r = await fetch(`${base}/v1/projects/${projectId}/documents?principalId=mallory`);
  assert.equal(r.status, 404, "отказ и отсутствие выглядят одинаково");
});

test("правка секции меняет её одну и поднимает ревизию", async (t) => {
  const { base, projectId, owner } = await stand(t);
  const r = await fetch(`${base}/v1/projects/${projectId}/document/section`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ principalId: owner, path: "task.md", anchor: "что-делать", body: "\nТретье.\n\n" }),
  });
  assert.equal(r.status, 200);
  assert.equal(((await r.json()) as { document: { revision: number } }).document.revision, 2);

  const after = await fetch(
    `${base}/v1/projects/${projectId}/document?principalId=${encodeURIComponent(owner)}&path=task.md`,
  );
  const { document } = (await after.json()) as { document: { content: string } };
  assert.equal(document.content, DOC.replace("\nПервое.\n\n", "\nТретье.\n\n"));
});

test("устаревшая правка отвергается с указанием нынешней ревизии", async (t) => {
  const { base, projectId, owner } = await stand(t);
  const stale = await fetch(`${base}/v1/projects/${projectId}/document/section`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      principalId: owner,
      path: "task.md",
      anchor: "что-делать",
      body: "\nЧетвёртое.\n\n",
      expectedRevision: 7,
    }),
  });
  assert.equal(stale.status, 409);
  const refused = (await stale.json()) as { error: string; document: { revision: number } };
  assert.equal(refused.error, "conflict");
  assert.equal(refused.document.revision, 1);
});

test("несуществующая секция отличима от несуществующего документа", async (t) => {
  const { base, projectId, owner } = await stand(t);
  const put = (path: string, anchor: string): Promise<Response> =>
    fetch(`${base}/v1/projects/${projectId}/document/section`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ principalId: owner, path, anchor, body: "" }),
    });

  const noSection = await put("task.md", "нет-такой");
  assert.equal(noSection.status, 404);
  assert.equal(((await noSection.json()) as { error: string }).error, "no_such_section");

  const noDocument = await put("нет.md", "что-делать");
  assert.equal(((await noDocument.json()) as { error: string }).error, "not_found");
});

test("история отдаёт ревизии, новейшую первой", async (t) => {
  const { base, projectId, owner } = await stand(t);
  await fetch(`${base}/v1/projects/${projectId}/document/section`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ principalId: owner, path: "task.md", anchor: "что-делать", body: "\nПятое.\n\n" }),
  });
  const r = await fetch(
    `${base}/v1/projects/${projectId}/document/history?principalId=${encodeURIComponent(owner)}&path=task.md`,
  );
  const { revisions } = (await r.json()) as { revisions: { revision: number; writtenBy: string }[] };
  assert.deepEqual(
    revisions.map((x) => x.revision),
    [2, 1],
  );
  assert.equal(revisions[0]!.writtenBy, owner);
});
