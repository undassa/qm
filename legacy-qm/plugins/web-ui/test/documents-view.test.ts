import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { buildTree, resolveDocumentPath } from "../src/document-tree.ts";

const doc = (path: string) => ({
  path,
  contentHash: "",
  bytes: 0,
  revision: 1,
  updatedAt: 0,
  updatedBy: "",
});

test("дерево вложено по-настоящему, а не разложено по одному уровню папок", () => {
  const root = buildTree([doc("50-plan/v1/M1/M1-T1.md"), doc("50-plan/v1.md"), doc("10-intent/srs.md")]);

  assert.deepEqual([...root.children.keys()].sort(), ["10-intent", "50-plan"]);
  const plan = root.children.get("50-plan")!;
  // v1.md и папка v1 — разные узлы, хотя имена почти совпадают
  assert.deepEqual([...plan.children.keys()].sort(), ["v1", "v1.md"]);
  const task = plan.children.get("v1")!.children.get("M1")!.children.get("M1-T1.md")!;
  assert.equal(task.path, "50-plan/v1/M1/M1-T1.md", "у листа полный путь, а не только имя");
  assert.equal(task.document?.revision, 1);
});

test("папка без собственного документа всё равно попадает в дерево", () => {
  const root = buildTree([doc("a/b/c.md")]);
  assert.equal(root.children.get("a")!.document, null);
  assert.equal(root.children.get("a")!.children.get("b")!.document, null);
});

test("относительная ссылка разрешается от папки документа", () => {
  assert.equal(resolveDocumentPath("50-plan/v1/M1.md", "M2.md"), "50-plan/v1/M2.md");
  assert.equal(resolveDocumentPath("50-plan/v1/M1.md", "../../10-intent/srs.md"), "10-intent/srs.md");
  assert.equal(resolveDocumentPath("50-plan/v1/M1.md", "./M2.md"), "50-plan/v1/M2.md");
  assert.equal(resolveDocumentPath("50-plan/v1/M1.md", "srs.md#цели"), "50-plan/v1/srs.md", "якорь отбрасывается");
});

test("внешняя ссылка не выдаётся за документ — иначе клик уводил бы не туда", () => {
  for (const href of ["https://example.com/x.md", "mailto:a@b.c", "#раздел", "/files/x.md", ""]) {
    assert.equal(resolveDocumentPath("50-plan/v1/M1.md", href), "", href);
  }
});

test("тело документа рисуется размеченным и через общий обеззараживатель", () => {
  const source = readFileSync(new URL("../src/documents.ts", import.meta.url), "utf8");
  assert.match(source, /installMarkdownSanitizer\(\)/, "разметка обязана проходить через общую очистку");
  assert.match(
    source,
    /unsafeHTML\(bodyHtml\(open\)\)/,
    "тело документа рисуется, а не показывается одними заголовками",
  );
  assert.match(source, /document\/backlinks/, "страница спрашивает обратные ссылки");
});

test("документы работают прямо в проекте, а не только отдельной страницей", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /documentsWorkspaceTpl\(\)/, "дашборд проекта показывает само рабочее место");
  assert.match(contexts, /mountProjectDocuments\(projectId, drawContexts\)/, "и перерисовывает его собой");

  const documents = readFileSync(new URL("../src/documents.ts", import.meta.url), "utf8");
  assert.match(
    documents,
    /export function documentsWorkspaceTpl/,
    "рабочее место отделено от страницы, иначе его нельзя встроить",
  );
  assert.match(documents, /unmountProjectDocuments\(\);\s+redraw\(\);/, "своя страница забирает перерисовку обратно");
});

test("дерево ведёт себя как файловый проводник, а не как всегда раскрытый список", () => {
  const source = readFileSync(new URL("../src/documents.ts", import.meta.url), "utf8");
  assert.match(
    source,
    /expanded: new Set<string>\(\)/,
    "по умолчанию папки закрыты — иначе 849 документов вываливаются разом",
  );
  assert.match(source, /function expandTo\(/, "путь к открытому документу раскрывается сам");
  assert.match(source, /expandTo\(path\);/, "и делает это при открытии документа");
  assert.doesNotMatch(source, /state\.collapsed/, "прежняя обратная логика убрана целиком");
});

test("настройки проекта живут вкладкой, а не правой колонкой", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /contextsState\.tab === "settings"/, "у проекта есть вкладка настроек");
  assert.match(contexts, /function contextSettingsTpl/, "содержимое настроек — один шаблон на оба места");
  assert.match(contexts, /tabbed\s*\?\s*nothing\s*:\s*html`<aside/s, "у проекта с вкладками правой колонки нет");
});

test("вкладка документов занимает экран целиком, а не сидит в панели", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /class="context-documents-full"/, "рабочее место без панельной обёртки");
  assert.doesNotMatch(
    contexts,
    /context-panel context-documents/,
    "иначе оно снова окажется внутри контейнера с отступами",
  );
  assert.match(contexts, /documents-fullbleed/, "область снимает свои отступы и ограничение ширины");

  const css = readFileSync(new URL("../src/shell.css", import.meta.url), "utf8");
  assert.match(
    css,
    /\.context-documents-full \.documents-layout \{[^}]*height: 100%/s,
    "колонки тянутся на всю высоту",
  );
});

test("доска устроена под разработку: гейты, фазы, задача с проверками и прогоном", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /function gateStripTpl/, "гейты видны над доской");
  assert.match(contexts, /function stepperTpl/, "вехи выбираются степпером — задачи идут по порядку");
  assert.match(contexts, /function milestonePanelTpl/, "у выбранной вехи есть панель описания");
  assert.match(contexts, /function issueViewTpl/, "задача открывается карточкой");
  assert.match(contexts, /task\.runPath/, "карточка ведёт в сессию разработки");
  assert.match(contexts, /r\.checks\.map/, "карточка показывает проверки требований");
  assert.match(contexts, /BOARD_COLUMNS/, "внутри дорожки — колонки по статусу, как в jira");
  assert.match(contexts, /function lozengeTpl/, "статус показывается лозенгом");
  assert.doesNotMatch(contexts, /Заблокированы/, "«заблокировано» — флаг на карточке, а не колонка");
});

test("идентификатор задачи не печатается дважды", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /function taskTitle/, "название уже начинается с идентификатора");
  assert.match(contexts, /taskTitle\(task\.id, task\.title\)/);
});

test("задача открывается дровером, а не постоянной колонкой", () => {
  const contexts = readFileSync(new URL("../src/contexts.ts", import.meta.url), "utf8");
  assert.match(contexts, /function issueDrawerTpl/, "задача живёт в дровере поверх доски");
  assert.match(contexts, /class="issue-drawer"/);
  assert.match(contexts, /@close=\$\{closeIssueDrawer\}/, "Esc и клик по фону закрывают через нативный dialog");
  assert.match(contexts, /drawer && !drawer\.open\) drawer\.showModal\(\)/, "дровер показывается модально");
  assert.doesNotMatch(contexts, /class="board-detail"/, "постоянной колонки задачи больше нет");
});
