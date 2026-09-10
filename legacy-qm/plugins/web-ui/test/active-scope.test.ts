import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { projectIdOfScope, scopeOfProjectId } from "../src/shell-state.ts";

const PROJECT = "308ed7a2-d18f-4a76-a9c4-c792ce7de0d3";

test("область проекта и его идентификатор переводятся друг в друга", () => {
  assert.equal(scopeOfProjectId(PROJECT), `group:web-project-${PROJECT}`);
  assert.equal(projectIdOfScope(scopeOfProjectId(PROJECT)), PROJECT);
});

test("непроектная область не выдаёт себя за проект", () => {
  for (const scope of ["personal:ann", "group:engineering", null]) {
    assert.equal(projectIdOfScope(scope), null, String(scope));
  }
});

test("чаты, кроны, файлы и приложения следуют выбранному месту", () => {
  for (const file of ["sessions.ts", "crons.ts", "deploys.ts", "files.ts"]) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
    assert.match(source, /= appState\.activeScope/, `${file} не подхватывает общий выбор`);
  }
});

test("место ставит сайдбар и вход в проект — списки его только читают", () => {
  // Место ставит тот, кто ведёт: сайдбар, вход в проект, глубокая ссылка, переход из панели.
  const writers = ["shell.ts", "contexts.ts", "documents.ts", "chat.ts", "split.ts"];
  const readers = ["sessions.ts", "crons.ts", "deploys.ts", "webhooks.ts"];
  for (const file of writers) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
    assert.match(source, /setActiveScope\(/, `${file}: место должно ставиться отсюда`);
  }
  for (const file of readers) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
    assert.ok(
      !/setActiveScope\(/.test(source),
      `${file}: список не должен переставлять место — иначе сайдбар и страница разойдутся`,
    );
  }
});

test("документы выбирают проект через ту же общую область", () => {
  const source = readFileSync(new URL("../src/documents.ts", import.meta.url), "utf8");
  assert.match(source, /setActiveScope\(scopeOfProjectId\(id\)\)/);
  assert.match(source, /projectIdOfScope\(appState\.activeScope\)/);
});

test("вход в проект делает его текущим местом, а не оставляет фильтром", () => {
  const source = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  const body = source.slice(source.indexOf("function selectContext"));
  const scoped = body.slice(0, body.indexOf("\n}"));
  assert.match(scoped, /setActiveScope\(scopeId\)/, "открытый проект обязан стать выбранным местом");
  assert.match(scoped, /renderSidebarTop\(\)/, "иначе место в сайдбаре обновится только при следующей перерисовке");
});

test("навигация разделена на работу и устройство машины", () => {
  const source = readFileSync(new URL("../src/shell.ts", import.meta.url), "utf8");
  assert.match(source, /"nav-workspace",\s*\n?\s*"Work"/, "раздел работы назван");
  assert.match(source, /"nav-system",\s*\n?\s*"System"/, "раздел машины назван");
  const work = source.slice(source.indexOf('"nav-workspace"'), source.indexOf('"nav-system"'));
  assert.match(work, /navRow\("documents"/, "документы — первое в работе, там она и идёт");
  assert.ok(!/navRow\("keychain"/.test(work), "ключи — устройство машины, а не работа");
});

test("страницы не повторяют выбор места — его держит сайдбар", () => {
  for (const file of ["crons.ts", "deploys.ts", "sessions.ts", "webhooks.ts"]) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
    assert.ok(!/onScope:/.test(source), `${file}: собственный выбор области дублирует сайдбар и расходится с ним`);
  }
});

test("вебхуки следуют выбранному месту, а не своему фильтру", () => {
  const source = readFileSync(new URL("../src/webhooks.ts", import.meta.url), "utf8");
  assert.match(source, /webhooksScope = appState\.activeScope/, "иначе страница фильтрует мимо сайдбара");
});
