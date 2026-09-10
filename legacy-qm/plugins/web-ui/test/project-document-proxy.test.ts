import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { projectDocumentRoutes } from "../../../src/api/routes/project-documents.ts";
import { projectAgentRoutes } from "../../../src/api/routes/project-agents.ts";
import { projectRunRoutes } from "../../../src/api/routes/project-runs.ts";
import { projectRepositoryRoutes } from "../../../src/api/routes/project-repositories.ts";
import { projectEntityRoutes } from "../../../src/api/routes/project-entities.ts";

/**
 * Поверхность проксирует каждый путь ядра отдельным маршрутом, а не одним правилом.
 * Забытый проводник виден только в браузере: страница молча получает 404 и рисует пустоту.
 */
test("у каждого маршрута проекта есть проводник в поверхности", () => {
  const server = readFileSync(new URL("../server/index.ts", import.meta.url), "utf8");
  const missing: string[] = [];
  const declaredRoutes = [
    ...projectDocumentRoutes,
    ...projectAgentRoutes,
    ...projectRunRoutes,
    ...projectRepositoryRoutes,
    ...projectEntityRoutes,
  ].filter((route): route is typeof route & { method: string; path: string } => "path" in route);
  assert.ok(declaredRoutes.length > 0, "маршруты документов объявлены путями");
  for (const route of declaredRoutes) {
    const surfacePath = route.path.replace("/v1/projects/:id", "/api/projects/:id");
    const declared = new RegExp(
      `method:\\s*"${route.method}"[^}]*?path:\\s*"${surfacePath.replace(/[/:]/g, (c) => `\\${c}`)}"`,
      "s",
    );
    if (!declared.test(server)) missing.push(`${route.method} ${surfacePath}`);
  }
  assert.deepEqual(missing, [], "эти маршруты ядра недостижимы из браузера");
});
