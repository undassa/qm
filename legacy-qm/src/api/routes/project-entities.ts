import { sendJson } from "../http.ts";
import { isObj } from "./shared.ts";
import { projectGroupRef } from "../../projects/project-store.ts";
import { type ApiCtx, type Route } from "./route.ts";

/** Promise.all по именам: добавить проверку не значит пересчитать позиции. */
async function allOf<T extends Record<string, Promise<unknown>>>(tasks: T): Promise<{ [K in keyof T]: Awaited<T[K]> }> {
  const names = Object.keys(tasks);
  const values = await Promise.all(Object.values(tasks));
  return Object.fromEntries(names.map((name, i) => [name, values[i]])) as { [K in keyof T]: Awaited<T[K]> };
}

/**
 * Каталог сущностей проекта: не файлы, а то, из чего проект состоит — статьи,
 * вопросы, решения, истории, экраны. У каждой сущности своя таблица, поэтому
 * счётчик здесь считается запросом, а не разбором документа на лету.
 */
export const ENTITY_KINDS = [
  "articles",
  "needs",
  "questions",
  "requirements",
  "checks",
  "features",
  "stories",
  "screens",
  "decisions",
  "runs",
  "traceability",
  "plan",
  "board",
  "risks",
  "terms",
  "dbTables",
] as const;
export type EntityKind = (typeof ENTITY_KINDS)[number];

/**
 * Вид документа, который эта сущность заменяет в сайдбаре: показывать «Вопросы»
 * дважды — раз таблицей и раз списком файлов — значит спрашивать, какой из двух счётчиков верен.
 */
export const ENTITY_REPLACES_KIND: Partial<Record<EntityKind, string>> = {
  questions: "question",
  decisions: "decision",
  stories: "story",
  screens: "screen",
  features: "feature",
  runs: "run",
};

export const ENTITY_META: Record<EntityKind, { title: string; one: string; icon: string; source: string }> = {
  articles: { title: "Статьи конституции", one: "статья", icon: "gavel", source: "00-frame/constitution.md" },
  questions: { title: "Вопросы", one: "вопрос", icon: "help", source: "00-frame/questions/" },
  decisions: { title: "Решения", one: "решение", icon: "scale", source: "30-design/decisions/" },
  stories: { title: "Истории", one: "история", icon: "story", source: "10-intent/use-cases/" },
  screens: { title: "Экраны", one: "экран", icon: "screen", source: "20-surface/" },
  needs: { title: "Потребности", one: "потребность", icon: "need", source: "10-intent/strs.md" },
  requirements: { title: "Требования", one: "требование", icon: "req", source: "10-intent/srs.md" },
  checks: { title: "Проверки", one: "проверка", icon: "check", source: "40-proof/" },
  features: { title: "Области", one: "область", icon: "area", source: "10-intent/functional/" },
  runs: { title: "Прогоны", one: "прогон", icon: "run", source: "60-runs/" },
  plan: {
    title: "План документов",
    one: "документ плана",
    icon: "plan",
    source: "00-frame/document-plan.md",
  },
  risks: { title: "Риски", one: "риск", icon: "risk", source: "10-intent/risks.md" },
  terms: { title: "Термины", one: "термин", icon: "term", source: "00-frame/glossary.md" },
  dbTables: { title: "Таблицы базы", one: "таблица", icon: "db", source: "30-design/data-model.md" },
  board: {
    title: "Доска состояний",
    one: "задача на доске",
    icon: "board",
    source: "50-plan/v1/status.md",
  },
  traceability: {
    title: "Трассируемость",
    one: "подсистема",
    icon: "matrix",
    source: "10-intent/traceability.md",
  },
};

function isEntityKind(value: string): value is EntityKind {
  return (ENTITY_KINDS as readonly string[]).includes(value);
}

async function member(ctx: ApiCtx, projectId: string): Promise<boolean> {
  const projects = ctx.deps.projects;
  if (!projects) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return false;
  }
  const body = isObj(ctx.body) ? ctx.body : {};
  const requested = (ctx.url.searchParams.get("principalId") ?? (body["principalId"] as string) ?? "").trim();
  const principalId = ctx.capability ? ctx.capability.actorId : requested;
  if (!principalId) {
    sendJson(ctx.res, 400, { error: "bad_request", message: "principalId required" });
    return false;
  }
  if (ctx.capability && requested && requested !== ctx.capability.actorId) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return false;
  }
  const ok = await projects.membership(projectGroupRef(projectId), principalId).catch(() => false);
  if (ok !== true) {
    sendJson(ctx.res, 404, { error: "not_found" });
    return false;
  }
  return true;
}

/** Счётчики каталога: пустое хранилище даёт ноль, а не отсутствие раздела. */
export interface CatalogEntity {
  kind: EntityKind;
  title: string;
  one: string;
  icon: string;
  source: string;
  count: number;
  replaces: string;
}

async function catalogOf(d: ApiCtx["deps"], projectId: string): Promise<{ entities: CatalogEntity[]; open: number }> {
  // Именованный разбор, как и у находок: список растёт, и порядок здесь не должен решать.
  const got = await allOf({
    articles: d.projectArticles?.list(projectId).catch(() => []) ?? Promise.resolve([]),
    trace: d.projectTraceability?.list(projectId).catch(() => []) ?? Promise.resolve([]),
    plan: d.projectDocumentPlan?.list(projectId).catch(() => []) ?? Promise.resolve([]),
    questions: d.projectQuestions?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    decisions: d.projectDecisions?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    surface: d.projectSurface?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    features: d.projectFeatures?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    runs: d.projectRunLog?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    proof: d.projectProof?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    needs: d.projectNeeds?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    board: d.projectPlanStatus?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    risks: d.projectRisks?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    terms: d.projectGlossary?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    dbTables: d.projectDataModel?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
  });

  const counts: Record<EntityKind, number> = {
    articles: got.articles.length,
    questions: got.questions?.total ?? 0,
    decisions: got.decisions?.total ?? 0,
    stories: got.surface?.stories ?? 0,
    screens: got.surface?.screens ?? 0,
    requirements: got.proof?.requirements ?? 0,
    checks: got.proof?.checks ?? 0,
    features: got.features?.features ?? 0,
    runs: got.runs?.total ?? 0,
    traceability: got.trace.length,
    needs: got.needs?.needs ?? 0,
    plan: got.plan.length,
    board: got.board?.total ?? 0,
    risks: got.risks?.total ?? 0,
    terms: got.terms?.terms ?? 0,
    dbTables: got.dbTables?.tables ?? 0,
  };

  return {
    entities: ENTITY_KINDS.map((kind) => ({
      kind,
      ...ENTITY_META[kind],
      count: counts[kind],
      replaces: ENTITY_REPLACES_KIND[kind] ?? "",
    })),
    open: got.questions?.open ?? 0,
  };
}

async function listCatalog(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  return sendJson(ctx.res, 200, await catalogOf(ctx.deps, projectId));
}

async function listEntities(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  const kind = (ctx.url.searchParams.get("kind") ?? "").trim();
  if (!isEntityKind(kind)) return sendJson(ctx.res, 404, { error: "not_found" });
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  switch (kind) {
    case "articles":
      return sendJson(ctx.res, 200, { items: (await d.projectArticles?.list(projectId)) ?? [] });
    case "questions":
      return sendJson(ctx.res, 200, { items: (await d.projectQuestions?.list(projectId)) ?? [] });
    case "decisions":
      return sendJson(ctx.res, 200, { items: (await d.projectDecisions?.list(projectId)) ?? [] });
    case "stories": {
      // Область раскрывается в свои истории: тот же набор колонок, но сузить нужно на сервере.
      const feature = (ctx.url.searchParams.get("feature") ?? "").trim();
      const items = feature
        ? await d.projectSurface?.storiesOfFeature(projectId, feature)
        : await d.projectSurface?.listStories(projectId);
      return sendJson(ctx.res, 200, { items: items ?? [] });
    }
    case "requirements":
      return sendJson(ctx.res, 200, { items: (await d.projectProof?.listRequirements(projectId)) ?? [] });
    case "checks":
      return sendJson(ctx.res, 200, { items: (await d.projectProof?.listChecks(projectId)) ?? [] });
    case "features":
      return sendJson(ctx.res, 200, { items: (await d.projectFeatures?.list(projectId)) ?? [] });
    case "runs":
      return sendJson(ctx.res, 200, { items: (await d.projectRunLog?.list(projectId)) ?? [] });
    case "needs":
      return sendJson(ctx.res, 200, { items: (await d.projectNeeds?.list(projectId)) ?? [] });
    case "plan":
      return sendJson(ctx.res, 200, { items: (await d.projectDocumentPlan?.list(projectId)) ?? [] });
    case "dbTables":
      return sendJson(ctx.res, 200, { items: (await d.projectDataModel?.list(projectId)) ?? [] });
    case "risks":
      return sendJson(ctx.res, 200, { items: (await d.projectRisks?.list(projectId)) ?? [] });
    case "terms":
      return sendJson(ctx.res, 200, { items: (await d.projectGlossary?.list(projectId)) ?? [] });
    case "board":
      return sendJson(ctx.res, 200, { items: (await d.projectPlanStatus?.list(projectId)) ?? [] });
    case "traceability":
      return sendJson(ctx.res, 200, { items: (await d.projectTraceability?.list(projectId)) ?? [] });
    default:
      return sendJson(ctx.res, 200, { items: (await d.projectSurface?.listScreens(projectId)) ?? [] });
  }
}

/** Что каталог нашёл сам: висячие ссылки и решения без записанного ответа. */
async function listFindings(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  // Именованный разбор, а не позиционный: список рос, и порядок дважды разъезжался.
  const ask = <T>(promise: Promise<T[]> | undefined): Promise<T[]> => promise?.catch(() => []) ?? Promise.resolve([]);
  const found = await allOf({
    articleRefs: ask(d.projectArticles?.danglingReferences(projectId)),
    decided: ask(d.projectQuestions?.decidedWithoutAnswer(projectId)),
    refinements: ask(d.projectDecisions?.danglingRefinements(projectId)),
    requirements: ask(d.projectDecisions?.danglingRequirementsInForce(projectId)),
    screens: ask(d.projectSurface?.danglingScreens(projectId)),
    featureGone: ask(d.projectFeatures?.danglingStories(projectId)),
    unclaimed: ask(d.projectFeatures?.unclaimedStories(projectId)),
    runGone: ask(d.projectRunLog?.danglingTasks(projectId)),
    closedNoRun: ask(d.projectRunLog?.closedWithoutRun(projectId)),
    uncoveredNfr: ask(d.projectProof?.uncoveredNonFunctional(projectId)),
    drift: ask(d.projectTraceability?.drift(projectId)),
    needsNoStory: ask(d.projectNeeds?.withoutStory(projectId)),
    needsDangling: ask(d.projectNeeds?.danglingNeeds(projectId)),
    needsContradiction: ask(d.projectNeeds?.citedButNotDeclared(projectId)),
    planUnresolved: ask(d.projectDocumentPlan?.unresolved(projectId)),
    planWrong: ask(d.projectDocumentPlan?.presentButDeclaredAbsent(projectId)),
    planCountDrift: ask(d.projectDocumentPlan?.countDrift(projectId)),
    boardDisagree: ask(d.projectPlanStatus?.disagreements(projectId)),
    boardUnknown: ask(d.projectPlanStatus?.unknownTasks(projectId)),
    boardMissing: ask(d.projectPlanStatus?.missingFromBoard(projectId)),
    boardNoCommit: ask(d.projectPlanStatus?.closedWithoutCommit(projectId)),
    riskNoOwner: ask(d.projectRisks?.openWithoutOwner(projectId)),
    riskBadDecision: ask(d.projectRisks?.settledByMissingDecision(projectId)),
    termsUnused: ask(d.projectGlossary?.unused(projectId)),
    dbNoColumns: ask(d.projectDataModel?.withoutColumns(projectId)),
    dbNoMigration: ask(d.projectDataModel?.withoutMigration(projectId)),
    missingAreas: ask(d.projectTraceability?.missingAreas(projectId)),
    closesGone: ask(d.projectDecisions?.danglingQuestions(projectId)),
    stillOpen: ask(d.projectDecisions?.closedButOpenQuestions(projectId)),
    articlesGone: ask(d.projectDecisions?.danglingArticles(projectId)),
  });

  return sendJson(ctx.res, 200, {
    findings: [
      {
        kind: "articles",
        item: "ссылка на несуществующую статью",
        count: found.articleRefs.length,
        detail: found.articleRefs,
      },
      { kind: "questions", item: "решено, но ответ не записан", count: found.decided.length, detail: found.decided },
      {
        kind: "decisions",
        item: "уточняет несуществующее решение",
        count: found.refinements.length,
        detail: found.refinements,
      },
      {
        kind: "decisions",
        item: "действующее решение ссылается на удалённое требование",
        count: found.requirements.length,
        detail: found.requirements,
      },
      {
        kind: "screens",
        item: "ссылка на несуществующий экран",
        count: found.screens.length,
        detail: found.screens,
      },
      {
        kind: "decisions",
        item: "закрывает несуществующий вопрос",
        count: found.closesGone.length,
        detail: found.closesGone,
      },
      {
        kind: "questions",
        item: "решение закрыло, а вопрос открыт",
        count: found.stillOpen.length,
        detail: found.stillOpen,
      },
      {
        kind: "articles",
        item: "правит несуществующую статью",
        count: found.articlesGone.length,
        detail: found.articlesGone,
      },
      {
        kind: "features",
        item: "область называет несуществующую историю",
        count: found.featureGone.length,
        detail: found.featureGone,
      },
      {
        kind: "stories",
        item: "история не заявлена ни одной областью",
        count: found.unclaimed.length,
        detail: found.unclaimed,
      },
      { kind: "runs", item: "прогон задачи, которой нет в плане", count: found.runGone.length, detail: found.runGone },
      {
        kind: "runs",
        item: "закрытая задача без записи о прогоне",
        count: found.closedNoRun.length,
        detail: found.closedNoRun,
      },
      {
        kind: "needs",
        item: "потребность не подхвачена ни одной историей",
        count: found.needsNoStory.length,
        detail: found.needsNoStory,
      },
      {
        kind: "needs",
        item: "история ссылается на потребность, которой нет в реестре",
        count: found.needsDangling.length,
        detail: found.needsDangling,
      },
      {
        kind: "stories",
        item: "потребность названа в требованиях, но не объявлена историей",
        count: found.needsContradiction.length,
        detail: found.needsContradiction,
      },
      {
        kind: "plan",
        item: "имя из плана не отзывается ни одним документом",
        count: found.planUnresolved.length,
        detail: found.planUnresolved,
      },
      {
        kind: "plan",
        item: "объявлено «нет», а документ есть",
        count: found.planWrong.length,
        detail: found.planWrong,
      },
      {
        kind: "plan",
        item: "план назвал число, которое не сходится с посчитанным",
        count: found.planCountDrift.length,
        detail: found.planCountDrift,
      },
      {
        kind: "board",
        item: "доска и сама задача говорят о состоянии разное",
        count: found.boardDisagree.length,
        detail: found.boardDisagree,
      },
      {
        kind: "board",
        item: "доска называет задачу, которой нет в плане",
        count: found.boardUnknown.length,
        detail: found.boardUnknown,
      },
      {
        kind: "board",
        item: "задача есть в плане, но не на доске",
        count: found.boardMissing.length,
        detail: found.boardMissing,
      },
      {
        kind: "board",
        item: "закрыта, а коммит не назван",
        count: found.boardNoCommit.length,
        detail: found.boardNoCommit,
      },
      {
        kind: "risks",
        item: "открытый риск без владельца или без признака",
        count: found.riskNoOwner.length,
        detail: found.riskNoOwner,
      },
      {
        kind: "risks",
        item: "риск улажен решением, которого нет",
        count: found.riskBadDecision.length,
        detail: found.riskBadDecision,
      },
      {
        kind: "terms",
        item: "термин объявлен, но нигде не употреблён",
        count: found.termsUnused.length,
        detail: found.termsUnused,
      },
      {
        kind: "dbTables",
        item: "таблица заводится миграцией, а колонок её не описано",
        count: found.dbNoColumns.length,
        detail: found.dbNoColumns,
      },
      {
        kind: "dbTables",
        item: "колонки описаны, а миграция не названа",
        count: found.dbNoMigration.length,
        detail: found.dbNoMigration,
      },
      {
        kind: "traceability",
        item: "матрица разошлась с посчитанным",
        count: found.drift.length,
        detail: found.drift,
      },
      {
        kind: "traceability",
        item: "подсистема есть в требованиях, но матрица о ней молчит",
        count: found.missingAreas.length,
        detail: found.missingAreas,
      },
      {
        kind: "requirements",
        item: "нефункциональное требование без единой проверки",
        count: found.uncoveredNfr.length,
        detail: found.uncoveredNfr,
      },
    ].filter((f) => f.count > 0),
  });
}

/**
 * Указатель «идентификатор → где он объявлен». Нужен браузеру, чтобы `FR-CFG-01`
 * в прозе стал ссылкой: без него каждая такая ссылка стоила бы запроса к серверу.
 */
async function entityIndex(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  const [articles, questions, decisions, stories, screens, features, runs, requirements, checks] = await Promise.all([
    d.projectArticles?.list(projectId).catch(() => []) ?? [],
    d.projectQuestions?.list(projectId).catch(() => []) ?? [],
    d.projectDecisions?.list(projectId).catch(() => []) ?? [],
    d.projectSurface?.listStories(projectId).catch(() => []) ?? [],
    d.projectSurface?.listScreens(projectId).catch(() => []) ?? [],
    d.projectFeatures?.list(projectId).catch(() => []) ?? [],
    d.projectRunLog?.list(projectId).catch(() => []) ?? [],
    d.projectProof?.listRequirements(projectId).catch(() => []) ?? [],
    d.projectProof?.listChecks(projectId).catch(() => []) ?? [],
  ]);

  const index: { id: string; kind: EntityKind; path: string; anchor?: string }[] = [];
  const add = (kind: EntityKind, id: string, path: string, anchor?: string) => {
    if (!id || !path) return;
    index.push(anchor ? { id, kind, path, anchor } : { id, kind, path });
  };

  for (const a of articles) add("articles", `Article ${a.number}`, a.path, a.anchor);
  for (const q of questions) add("questions", q.id, q.path);
  for (const x of decisions) add("decisions", x.id, x.path);
  for (const s of stories) add("stories", s.id, s.path);
  for (const s of screens) add("screens", s.id, s.path);
  for (const f of features) add("features", f.id, f.path);
  for (const r of runs) if (!r.isMilestone) add("runs", r.id, r.path);
  for (const r of requirements) add("requirements", r.id, r.path);
  for (const c of checks) add("checks", c.id, c.path);

  return sendJson(ctx.res, 200, { index });
}

/**
 * Хребет проекта: цепочка «потребность → история → требование → проверка →
 * задача → прогон» и разрывы на стыках. Стык, для которого в данных нет связи,
 * помечается «не проверяется» — зелёным он быть не может, иначе зелёный
 * перестанет означать проверенное.
 */
async function chain(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  const got = await allOf({
    needs: d.projectNeeds?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    surface: d.projectSurface?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    proof: d.projectProof?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    board: d.projectPlanStatus?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    runs: d.projectRunLog?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    needsNoStory: d.projectNeeds?.withoutStory(projectId).catch(() => []) ?? Promise.resolve([]),
    storyConflict: d.projectNeeds?.citedButNotDeclared(projectId).catch(() => []) ?? Promise.resolve([]),
    uncoveredNfr: d.projectProof?.uncoveredNonFunctional(projectId).catch(() => []) ?? Promise.resolve([]),
    closedNoRun: d.projectRunLog?.closedWithoutRun(projectId).catch(() => []) ?? Promise.resolve([]),
  });

  const links = [
    { key: "needs", title: "потребность", count: got.needs?.needs ?? 0, note: "заявлено сторонами" },
    { key: "stories", title: "история", count: got.surface?.stories ?? 0, note: "что человек хочет сделать" },
    {
      key: "requirements",
      title: "требование",
      count: got.proof?.requirements ?? 0,
      note: `${(got.proof?.requirements ?? 0) - (got.proof?.nfr ?? 0)} функциональных, ${got.proof?.nfr ?? 0} нефункциональных`,
    },
    { key: "checks", title: "проверка", count: got.proof?.checks ?? 0, note: "чем доказывается" },
    {
      key: "tasks",
      title: "задача",
      count: got.board?.total ?? 0,
      note: `${got.board?.closed ?? 0} закрыто, ${got.board?.claimed ?? 0} в работе`,
    },
    { key: "runs", title: "прогон", count: got.runs?.total ?? 0, note: "запись о том, как делалось" },
  ];

  const joints = [
    {
      from: "needs",
      to: "stories",
      checked: true,
      broken: got.needsNoStory.length,
      what: "потребность, которую не подхватила ни одна история",
    },
    {
      from: "stories",
      to: "requirements",
      checked: true,
      broken: got.storyConflict.length,
      what: "требования называют потребность, которой история не объявила",
    },
    {
      from: "requirements",
      to: "checks",
      checked: true,
      broken: got.uncoveredNfr.length,
      what: "требование без единой проверки",
    },
    {
      from: "checks",
      to: "tasks",
      checked: false,
      broken: 0,
      what: "связи проверки с задачей в данных нет — стык не проверяется",
    },
    {
      from: "tasks",
      to: "runs",
      checked: true,
      broken: got.closedNoRun.length,
      what: "закрытая задача без записи о прогоне",
    },
  ];

  return sendJson(ctx.res, 200, { links, joints });
}

/**
 * Процесс харнеса: Ф0 рамка ─G0─► Ф1 намерение ─G1─► Ф2 устройство ─G2─►
 * Ф3 проверки ─G3─► Ф4 генерация ─G4─► Ф5 выпуск. Этап сущности выводится из
 * пути её источника — тем же отображением, которым живут гейты, чтобы этап
 * нельзя было назначить мимо правила.
 */
export const PHASES = [
  { key: "G0", title: "Рамка", holds: "во что верим и чего не знаем", prefix: "00-frame/" },
  { key: "G1", title: "Намерение", holds: "кому это нужно и что должно быть", prefix: "10-intent/" },
  { key: "G2", title: "Устройство", holds: "как это устроено и почему так", prefix: "20-surface/|30-design/" },
  { key: "G3", title: "Проверки", holds: "чем доказывается, что работает", prefix: "40-proof/" },
  { key: "G4", title: "Генерация", holds: "что делается и как это шло", prefix: "50-plan/|60-runs/" },
  { key: "after", title: "Выпуск", holds: "что происходит после", prefix: "70-after/" },
] as const;

export function phaseOfSource(source: string): string {
  for (const phase of PHASES) {
    if (phase.prefix.split("|").some((p) => source.startsWith(p))) return phase.key;
  }
  return "";
}

async function process(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  const catalogRes = await catalogOf(d, projectId);
  const gates = d.projectGates ? await d.projectGates.list(projectId).catch(() => []) : [];

  const byPhase = new Map<string, { kind: EntityKind; title: string; count: number }[]>();
  for (const entity of catalogRes.entities) {
    const phase = phaseOfSource(entity.source);
    if (!phase) continue;
    byPhase.set(phase, [
      ...(byPhase.get(phase) ?? []),
      { kind: entity.kind, title: entity.title, count: entity.count },
    ]);
  }

  const phases = PHASES.map((phase) => {
    const own = gates.filter((g) => g.phase === phase.key);
    const failed = own.filter((g) => g.state === "failed" || g.state === "refused").length;
    return {
      key: phase.key,
      title: phase.title,
      holds: phase.holds,
      entities: byPhase.get(phase.key) ?? [],
      gate: own.length ? { total: own.length, passed: own.filter((g) => g.state === "passed").length, failed } : null,
    };
  });

  return sendJson(ctx.res, 200, { phases });
}

/**
 * Конституция как страница работы, а не как текст. У каждой статьи рядом стоит
 * то, чем она держится: кто её цитирует, какие решения её правили и какая
 * машинная проверка её стережёт. Статья без проверки держится только на памяти
 * читающего — это и есть главная находка страницы.
 */
async function constitution(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;

  const got = await allOf({
    articles: d.projectArticles?.list(projectId).catch(() => []) ?? Promise.resolve([]),
    gates: d.projectGates?.list(projectId).catch(() => []) ?? Promise.resolve([]),
    amendments: d.projectDecisions?.amendedArticles(projectId).catch(() => []) ?? Promise.resolve([]),
  });

  const amendedBy = new Map<number, string[]>();
  for (const link of got.amendments) {
    const number = Number(link.target);
    if (!Number.isFinite(number)) continue;
    amendedBy.set(number, [...(amendedBy.get(number) ?? []), link.decisionId]);
  }

  const articles = got.articles.map((a) => {
    const checks = got.gates.filter((g) => g.article === a.number);
    return {
      number: a.number,
      title: a.title,
      body: a.body,
      anchor: a.anchor,
      citedBy: a.citedBy,
      amendedBy: amendedBy.get(a.number) ?? [],
      checks: checks.map((g) => ({ phase: g.phase, item: g.item, state: g.state, violations: g.violations })),
    };
  });

  return sendJson(ctx.res, 200, {
    articles,
    unchecked: articles.filter((a) => a.checks.length === 0).length,
    uncited: articles.filter((a) => a.citedBy === 0).length,
  });
}

/**
 * Вопросы как реестр известных дыр — и мерка, которой он меряется, взята у него
 * же: `00-frame/questions/README.md` объявляет, что ответ — это ссылка на
 * документ, который вопрос теперь несёт, а слово состояния ответом не считается.
 * Страница показывает, сколько закрытий этой мерке отвечает.
 */
async function questions(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const answers = (await ctx.deps.projectQuestions?.answers(projectId).catch(() => [])) ?? [];

  const closed = answers.filter((a) => a.state !== "open");
  const by = (kind: string) => closed.filter((a) => a.kind === kind).length;
  return sendJson(ctx.res, 200, {
    questions: answers,
    total: answers.length,
    open: answers.filter((a) => a.state === "open").length,
    closed: closed.length,
    byLink: by("link"),
    notByLink: closed.length - by("link") - by("missing"),
    missing: by("missing"),
    deadLinks: by("dead-link"),
    unknownNames: answers.filter((a) => a.kind === "unknown-name").flatMap((a) => a.broken),
    holdingGate: answers.filter((a) => a.state === "open" && a.holds === "holds").map((a) => a.id),
  });
}

/**
 * Требования и то, чем они доказаны. Две находки страницы разной природы:
 * требование без единой проверки заявлено, но не доказано; имя, на которое
 * корпус ссылается при отсутствующем требовании, — след снятого требования,
 * который гниёт молча: адреса тут нет, а значит и правилу битых ссылок нечего
 * ловить.
 */
async function requirements(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const got = await allOf({
    items: d.projectProof?.listRequirements(projectId).catch(() => []) ?? Promise.resolve([]),
    absent: d.projectProof?.citedButAbsent(projectId).catch(() => []) ?? Promise.resolve([]),
    counts:
      d.projectProof?.counts(projectId).catch(() => null) ??
      Promise.resolve(null as { requirements: number; nfr: number; checks: number; covered: number } | null),
  });

  const uncovered = got.items.filter((r) => r.checks === 0);
  return sendJson(ctx.res, 200, {
    requirements: got.items,
    citedButAbsent: got.absent,
    total: got.items.length,
    checks: got.counts?.checks ?? 0,
    uncovered: uncovered.length,
    uncoveredNfr: uncovered.filter((r) => r.kind === "NFR").length,
  });
}

/**
 * Решения: то, чем проект связал себе руки. Страница держится на разнице между
 * «записано» и «в силе». Действующее решение, опирающееся на снятое имя, — это
 * обещание, которого больше нет; отмена, записанная одной стороной, — развилка,
 * о которой знает только один из двух документов.
 */
async function decisions(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);

  const got = await allOf({
    items: ask(d.projectDecisions?.list(projectId)),
    links: ask(d.projectDecisions?.listLinks(projectId)),
    danglingReqs: ask(d.projectDecisions?.danglingRequirementsInForce(projectId)),
    danglingQuestions: ask(d.projectDecisions?.danglingQuestions(projectId)),
    closedButOpen: ask(d.projectDecisions?.closedButOpenQuestions(projectId)),
    danglingRefinements: ask(d.projectDecisions?.danglingRefinements(projectId)),
  });

  const byDecision = new Map<string, string[]>();
  for (const l of got.danglingReqs) byDecision.set(l.decisionId, [...(byDecision.get(l.decisionId) ?? []), l.target]);

  const status = new Map(got.items.map((x) => [x.id, x.status]));
  // «Отменяет» здесь почти всегда частичное: заменяется слой, часть, миграция,
  // раздел. Состояние цели показывается рядом, чтобы расхождение было видно.
  const supersedes = got.links
    .filter((l) => l.kind === "supersedes")
    .map((l) => ({ decisionId: l.decisionId, target: l.target, targetStatus: status.get(l.target) ?? null }));

  // Отмена, записанная одной стороной: решение числится отменённым, а тот, кто
  // его отменил, об этом не заявляет. По одной записи развилку не восстановить.
  const claimed = new Set(supersedes.map((l) => l.target));
  const supersededWithoutClaim = got.items
    .filter((x) => x.status === "superseded" && !claimed.has(x.id))
    .map((x) => x.id);

  return sendJson(ctx.res, 200, {
    decisions: got.items.map((x) => ({
      id: x.id,
      title: x.title,
      path: x.path,
      status: x.status,
      date: x.date,
      deciders: x.deciders,
      closes: got.links.filter((l) => l.kind === "closes" && l.decisionId === x.id).length,
      danglingRequirements: byDecision.get(x.id) ?? [],
    })),
    supersedes,
    total: got.items.length,
    accepted: got.items.filter((x) => x.status === "accepted").length,
    superseded: got.items.filter((x) => x.status === "superseded").length,
    closes: got.links.filter((l) => l.kind === "closes").length,
    danglingRequirements: got.danglingReqs.length,
    decisionsWithDangling: byDecision.size,
    danglingQuestions: got.danglingQuestions.length,
    closedButOpen: got.closedButOpen.length,
    danglingRefinements: got.danglingRefinements.length,
    supersededWithoutClaim,
  });
}

/**
 * Проверки: то, чем требования доказываются. Страница показывает не счёт, а
 * глубину — сколько проверок стоит за требованием. Одна проверка на требование
 * значит один способ ошибиться и ни одной сверки.
 *
 * Чего страница не знает, она говорит вслух: доказывает ли проверка поведение
 * или проходит при любом, из корпуса не следует. Об этом же — открытые Q-323 и
 * Q-325 самого набора.
 */
async function checks(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const got = await allOf({
    checks: d.projectProof?.listChecks(projectId).catch(() => []) ?? Promise.resolve([]),
    requirements: d.projectProof?.listRequirements(projectId).catch(() => []) ?? Promise.resolve([]),
  });

  const depth = new Map<number, number>();
  for (const r of got.requirements) depth.set(r.checks, (depth.get(r.checks) ?? 0) + 1);

  const depthOf = new Map(got.requirements.map((r) => [r.id, r.checks]));
  const areas = new Map<string, { checks: number; covered: Set<string> }>();
  for (const c of got.checks) {
    const key = c.area || "—";
    const cell = areas.get(key) ?? { checks: 0, covered: new Set<string>() };
    cell.checks++;
    if (c.requirementId) cell.covered.add(c.requirementId);
    areas.set(key, cell);
  }

  return sendJson(ctx.res, 200, {
    checks: got.checks,
    total: got.checks.length,
    requirements: got.requirements.length,
    covered: got.requirements.filter((r) => r.checks > 0).length,
    shallow: got.requirements.filter((r) => r.checks === 1).length,
    deep: got.requirements.filter((r) => r.checks > 1).length,
    uncovered: got.requirements.filter((r) => r.checks === 0).length,
    depth: [...depth.entries()].sort((a, b) => a[0] - b[0]).map(([n, requirements]) => ({ n, requirements })),
    areas: [...areas.entries()]
      .map(([area, cell]) => ({
        area,
        checks: cell.checks,
        requirements: cell.covered.size,
        shallow: [...cell.covered].filter((id) => depthOf.get(id) === 1).length,
      }))
      .sort((a, b) => b.checks - a.checks || b.requirements - a.requirements),
    /** Весь набор объявлен одним документом — его и называем. */
    source: got.checks[0]?.path ?? "",
  });
}

/**
 * Намерение: потребность → история → требование. Одна цепочка, две страницы, и
 * смотрят они на неё с разных концов.
 *
 * Связь потребности и истории пишется с двух сторон: потребность объявляет свои
 * истории, история называет свою потребность. Записанная с одной стороны, она
 * держится ровно до первого читателя, который пошёл с другого конца.
 */
async function intent(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);
  const got = await allOf({
    needs: ask(d.projectNeeds?.list(projectId)),
    oneSided: ask(d.projectNeeds?.citedButNotDeclared(projectId)),
    orphans: ask(d.projectNeeds?.withoutStory(projectId)),
    stories: ask(d.projectSurface?.listStories(projectId)),
    danglingNeeds: ask(d.projectNeeds?.danglingNeeds(projectId)),
  });

  const cited = new Map<string, string[]>();
  for (const l of got.oneSided) cited.set(l.needId, [...(cited.get(l.needId) ?? []), l.storyId]);

  return sendJson(ctx.res, 200, {
    needs: got.needs,
    stories: got.stories,
    oneSided: got.oneSided,
    oneSidedNeeds: [...cited.entries()].map(([needId, stories]) => ({ needId, stories })),
    orphanNeeds: got.orphans.map((n) => n.id),
    danglingNeeds: got.danglingNeeds,
    totalNeeds: got.needs.length,
    totalStories: got.stories.length,
  });
}

/**
 * Модель данных — граница между тем, что заводит миграция, и тем, что описывает
 * документ. Расходятся они молча: миграция применяется и без описания, документ
 * читается и без миграции, а несовпадение видно только тому, кто сверит оба.
 */
async function dataModel(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);
  const got = await allOf({
    tables: ask(d.projectDataModel?.list(projectId)),
    counts: d.projectDataModel?.counts(projectId).catch(() => null) ?? Promise.resolve(null),
    withoutColumns: ask(d.projectDataModel?.withoutColumns(projectId)),
    withoutMigration: ask(d.projectDataModel?.withoutMigration(projectId)),
  });

  const byMigration = new Map<string, { tables: number; silent: number }>();
  const silent = new Set(got.withoutColumns.map((t) => t.name));
  for (const t of got.tables) {
    const key = t.migration || "—";
    const cell = byMigration.get(key) ?? { tables: 0, silent: 0 };
    cell.tables++;
    if (silent.has(t.name)) cell.silent++;
    byMigration.set(key, cell);
  }

  return sendJson(ctx.res, 200, {
    tables: got.tables.map((t) => ({ ...t, described: !silent.has(t.name) })),
    migrations: [...byMigration.entries()]
      .map(([migration, cell]) => ({ migration, ...cell }))
      .sort((a, b) => a.migration.localeCompare(b.migration)),
    total: got.tables.length,
    described: got.tables.length - got.withoutColumns.length,
    migrationsDeclared: got.counts?.migrations ?? 0,
    withoutColumns: got.withoutColumns.map((t) => t.name),
    withoutMigration: got.withoutMigration.map((t) => t.name),
  });
}

/**
 * План документов — обещание корпуса о самом себе: какие документы должны быть,
 * что каждый содержит и сколько в нём предметов. Три способа для плана разойтись
 * с деревом, и все три здесь названы: имя не отзывается документом, документ есть
 * при объявленном «нет», заявленное число не совпадает с посчитанным.
 */
async function documentPlan(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);
  const got = await allOf({
    items: ask(d.projectDocumentPlan?.list(projectId)),
    unresolved: ask(d.projectDocumentPlan?.unresolved(projectId)),
    wrong: ask(d.projectDocumentPlan?.presentButDeclaredAbsent(projectId)),
    drift: ask(d.projectDocumentPlan?.countDrift(projectId)),
  });
  const missing = new Set(got.unresolved.map((x) => x.name));
  const wrong = new Set(got.wrong.map((x) => x.name));

  return sendJson(ctx.res, 200, {
    items: got.items.map((x) => ({ ...x, missing: missing.has(x.name), wrong: wrong.has(x.name) })),
    drift: got.drift,
    total: got.items.length,
    unresolved: got.unresolved.map((x) => x.name),
    presentButDeclaredAbsent: got.wrong.map((x) => x.name),
  });
}

/**
 * Термины: один предмет — одно имя. Словарь ценен не длиной, а тем, что имена из
 * него живут в работе: термин, которого нет нигде, кроме самого словаря, ничего
 * не удерживает — он не мешает завести второе имя тому же предмету.
 */
async function glossary(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);
  const got = await allOf({
    terms: ask(d.projectGlossary?.list(projectId)),
    unused: ask(d.projectGlossary?.unused(projectId)),
    reach: ask(d.projectGlossary?.reach(projectId)),
  });
  const used = got.terms.map((t) => t.used);
  return sendJson(ctx.res, 200, {
    terms: got.terms,
    total: got.terms.length,
    unused: got.unused.map((t) => t.id),
    reach: got.reach,
    /** Медиана употребления: среднее здесь врёт — хвост из очень частых имён его тянет. */
    median: used.length ? [...used].sort((a, b) => a - b)[Math.floor(used.length / 2)]! : 0,
    widest: used.length ? Math.max(...used) : 0,
  });
}

/**
 * Остальные предметы этапов, у каждого своя сторона одного вопроса: держится ли
 * связь. Экран — нарисован ли впустую; область — подхвачена ли история; строка
 * трассируемости — совпадает ли заявленное с посчитанным; риск — назван ли
 * хозяин у открытого; прогон — что он оставил незакрытым.
 */
async function rest(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const d = ctx.deps;
  const ask = <T>(p: Promise<T[]> | undefined) => p?.catch(() => [] as T[]) ?? Promise.resolve([] as T[]);
  const got = await allOf({
    screens: ask(d.projectSurface?.listScreens(projectId)),
    orphanScreens: ask(d.projectSurface?.orphanScreens(projectId)),
    screenRefs: ask(d.projectSurface?.allScreenReferences(projectId)),
    danglingScreens: ask(d.projectSurface?.danglingScreens(projectId)),
    features: ask(d.projectFeatures?.list(projectId)),
    unclaimedStories: ask(d.projectFeatures?.unclaimedStories(projectId)),
    claims: ask(d.projectTraceability?.list(projectId)),
    drift: ask(d.projectTraceability?.drift(projectId)),
    risks: ask(d.projectRisks?.list(projectId)),
    risksNoOwner: ask(d.projectRisks?.openWithoutOwner(projectId)),
    risksBadDecision: ask(d.projectRisks?.settledByMissingDecision(projectId)),
    runs: ask(d.projectRunLog?.list(projectId)),
    runsDangling: ask(d.projectRunLog?.danglingTasks(projectId)),
    closedWithoutRun: ask(d.projectRunLog?.closedWithoutRun(projectId)),
  });

  return sendJson(ctx.res, 200, {
    screens: got.screens,
    orphanScreens: got.orphanScreens.map((s) => s.id),
    screenRefs: got.screenRefs,
    danglingScreens: got.danglingScreens.length,
    features: got.features,
    unclaimedStories: got.unclaimedStories.map((s) => s.id),
    claims: got.claims,
    drift: got.drift,
    risks: got.risks,
    risksNoOwner: got.risksNoOwner.map((r) => r.id),
    risksBadDecision: got.risksBadDecision.map((r) => r.id),
    runs: got.runs,
    runsDangling: got.runsDangling.map((r) => r.id),
    closedWithoutRun: got.closedWithoutRun.map((t) => t.id),
  });
}

/**
 * Чем можно закрыть вопрос и годится ли черновик ответа. Мерка та же, что считает
 * страницу: одна реализация на оба применения, иначе они разойдутся в том, что
 * считать закрытым.
 */
async function answerHelp(ctx: ApiCtx): Promise<void> {
  const projectId = ctx.params.id!;
  if (!(await member(ctx, projectId))) return;
  const store = ctx.deps.projectQuestions;
  if (!store) return sendJson(ctx.res, 200, { carriers: [] });

  if (ctx.req.method === "POST") {
    const body = (ctx.body ?? {}) as Record<string, unknown>;
    const path = typeof body["path"] === "string" ? body["path"] : "";
    const draft = typeof body["draft"] === "string" ? body["draft"] : "";
    if (!path) return sendJson(ctx.res, 400, { error: "bad_request", message: "path required" });
    return sendJson(ctx.res, 200, await store.checkDraft(projectId, path, draft));
  }
  return sendJson(ctx.res, 200, { carriers: await store.carriers(projectId).catch(() => []) });
}

export const projectEntityRoutes: Route[] = [
  { method: "GET", path: "/v1/projects/:id/entities", auth: "source", handle: listCatalog },
  { method: "GET", path: "/v1/projects/:id/entities/findings", auth: "source", handle: listFindings },
  { method: "GET", path: "/v1/projects/:id/entities/index", auth: "source", handle: entityIndex },
  { method: "GET", path: "/v1/projects/:id/chain", auth: "source", handle: chain },
  { method: "GET", path: "/v1/projects/:id/process", auth: "source", handle: process },
  { method: "GET", path: "/v1/projects/:id/constitution", auth: "source", handle: constitution },
  { method: "GET", path: "/v1/projects/:id/questions", auth: "source", handle: questions },
  { method: "GET", path: "/v1/projects/:id/answer", auth: "source", handle: answerHelp },
  { method: "POST", path: "/v1/projects/:id/answer", auth: "source", handle: answerHelp },
  { method: "GET", path: "/v1/projects/:id/requirements", auth: "source", handle: requirements },
  { method: "GET", path: "/v1/projects/:id/decisions", auth: "source", handle: decisions },
  { method: "GET", path: "/v1/projects/:id/checks", auth: "source", handle: checks },
  { method: "GET", path: "/v1/projects/:id/intent", auth: "source", handle: intent },
  { method: "GET", path: "/v1/projects/:id/data-model", auth: "source", handle: dataModel },
  { method: "GET", path: "/v1/projects/:id/document-plan", auth: "source", handle: documentPlan },
  { method: "GET", path: "/v1/projects/:id/glossary", auth: "source", handle: glossary },
  { method: "GET", path: "/v1/projects/:id/rest", auth: "source", handle: rest },
  { method: "GET", path: "/v1/projects/:id/entity", auth: "source", handle: listEntities },
];
