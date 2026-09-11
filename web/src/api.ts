/**
 * Разговор с поверхностью. Новая версия живёт на том же входе, поэтому личность
 * приходит сама и заголовков подмешивать не нужно.
 */
export interface Project {
  scopeId: string;
  name: string;
  projectId: string;
  /** Сколько документов в наборе; ноль — законное состояние. */
  documents: number;
  /** Когда последний раз правили. `null` — набор пуст, и правды нет вовсе. */
  updatedAt: number | null;
}

export interface PhaseEntity {
  kind: string;
  title: string;
  count: number;
}

export interface PhaseGate {
  total: number;
  passed: number;
  failed: number;
}

export interface Phase {
  key: string;
  title: string;
  holds: string;
  entities: PhaseEntity[];
  gate: PhaseGate | null;
}

export interface Finding {
  kind: string;
  item: string;
  count: number;
  detail?: unknown[];
}

export interface ChainLink {
  key: string;
  title: string;
  count: number;
  note: string;
}

export interface ChainJoint {
  from: string;
  to: string;
  checked: boolean;
  broken: number;
  what: string;
}

export type Row = Record<string, unknown>;

async function get<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${res.status} ${path}`);
  return (await res.json()) as T;
}

/** Область проекта закодирована в идентификаторе: `group:web-project-<uuid>`. */
export function projectIdOfScope(scopeId: string): string | null {
  const m = /^group:web-project-(.+)$/.exec(scopeId);
  return m?.[1] ?? null;
}

export async function loadProjects(): Promise<Project[]> {
  const data = await get<{
    contexts?: { scopeId: string; name: string; documents: number; updatedAt: number | null }[];
  }>("/api/contexts");
  return (data.contexts ?? []).flatMap((c) => {
    const projectId = projectIdOfScope(c.scopeId);
    return projectId
      ? [{ scopeId: c.scopeId, name: c.name, projectId, documents: c.documents, updatedAt: c.updatedAt }]
      : [];
  });
}

export const loadPhases = (projectId: string) => get<{ phases: Phase[] }>(`/api/projects/${projectId}/process`);
export const loadFindings = (projectId: string) => get<{ findings: Finding[] }>(`/api/projects/${projectId}/entities/findings`);
export const loadChain = (projectId: string) =>
  get<{ links: ChainLink[]; joints: ChainJoint[] }>(`/api/projects/${projectId}/chain`);
export const loadEntity = (projectId: string, kind: string) =>
  get<{ items: Row[] }>(`/api/projects/${projectId}/entity?kind=${encodeURIComponent(kind)}`);

export interface ArticleCheck {
  phase: string;
  item: string;
  state: string;
  violations: number;
}

export interface Article {
  number: number;
  title: string;
  body: string;
  anchor: string;
  citedBy: number;
  amendedBy: string[];
  checks: ArticleCheck[];
}

export interface Constitution {
  articles: Article[];
  unchecked: number;
  uncited: number;
}

export const loadConstitution = (projectId: string) => get<Constitution>(`/api/projects/${projectId}/constitution`);

/** Задача доски: та же, что видит старая поверхность, плюс счёт доказательств. */
export interface BoardTask {
  id: string;
  title: string;
  milestoneId: string;
  state: string;
  size: string;
  blockedBy: number;
  dependsOn: string[];
  kind: string;
  artifacts: string[];
  checks: number;
  /** Требований на задаче и сколько из них несёт хоть одну проверку. */
  requirements: number;
  covered: number;
}

export const loadBoard = (projectId: string) =>
  get<{ tasks: BoardTask[] }>(`/api/projects/${projectId}/board`).then((d) => d.tasks ?? []);

export type AnswerKind = "link" | "dead-link" | "name" | "unknown-name" | "words" | "missing";

export interface Question {
  id: string;
  number: number;
  title: string;
  path: string;
  state: "open" | "decided" | "closed";
  /** Поле «Гейт» из шапки: что вопрос держит и почему сторож на него не краснеет. */
  gate: string;
  /** Когда заведён и когда закрыт, как их пишет сам документ. */
  openedAt: string;
  closedAt: string;
  /** Держит ли вопрос гейт: «no» — реестр сказал это прямо, «unsaid» — не сказал. */
  holds: "holds" | "no" | "unsaid";
  level: string;
  kind: AnswerKind;
  broken: string[];
}

export interface QuestionBoard {
  questions: Question[];
  total: number;
  open: number;
  closed: number;
  byLink: number;
  notByLink: number;
  missing: number;
  deadLinks: number;
  unknownNames: string[];
  /** Открытые вопросы, которые что-то держат: с них и начинают. */
  holdingGate: string[];
}

export const loadQuestions = (projectId: string) => get<QuestionBoard>(`/api/projects/${projectId}/questions`);

export const loadDocument = (projectId: string, path: string) =>
  get<{ document?: { content?: string } }>(
    `/api/projects/${projectId}/document?path=${encodeURIComponent(path)}`,
  ).then((d) => d.document?.content ?? "");

export interface Requirement {
  id: string;
  kind: string;
  area: string;
  text: string;
  path: string;
  satisfied: boolean;
  /** Сколько проверок объявлено на это требование. */
  checks: number;
}

export interface RequirementBoard {
  requirements: Requirement[];
  /** Имена, на которые корпус ссылается, а требования с таким именем нет. */
  /** `inSrs` — назван ли снятый номер в самом документе требований. */
  citedButAbsent: { id: string; documents: number; where: string[]; inSrs: boolean }[];
  total: number;
  checks: number;
  uncovered: number;
  uncoveredNfr: number;
}

export const loadRequirements = (projectId: string) =>
  get<RequirementBoard>(`/api/projects/${projectId}/requirements`);

export interface Decision {
  id: string;
  title: string;
  path: string;
  status: string;
  date: string;
  deciders: string;
  /** Сколько вопросов решение закрыло. */
  closes: number;
  /** Имена требований, на которые оно ссылается, а требования нет. */
  danglingRequirements: string[];
}

export interface DecisionBoard {
  decisions: Decision[];
  /** Объявления отмены и состояние того, кого отменяют. */
  supersedes: { decisionId: string; target: string; targetStatus: string | null }[];
  /** Числится отменённым, а отменивший этого не заявляет. */
  supersededWithoutClaim: string[];
  total: number;
  accepted: number;
  superseded: number;
  closes: number;
  danglingRequirements: number;
  decisionsWithDangling: number;
  danglingQuestions: number;
  closedButOpen: number;
  danglingRefinements: number;
}

export const loadDecisions = (projectId: string) => get<DecisionBoard>(`/api/projects/${projectId}/decisions`);

export interface Check {
  id: string;
  area: string;
  requirementId: string;
  spec: string;
  path: string;
}

export interface CheckBoard {
  checks: Check[];
  total: number;
  requirements: number;
  covered: number;
  /** Требований, за которыми стоит ровно одна проверка. */
  shallow: number;
  deep: number;
  uncovered: number;
  depth: { n: number; requirements: number }[];
  /** Срез считается по проверкам и требованиям, которые они покрывают. */
  areas: { area: string; checks: number; requirements: number; shallow: number }[];
  source: string;
}

export const loadChecks = (projectId: string) => get<CheckBoard>(`/api/projects/${projectId}/checks`);

export interface Need {
  id: string;
  number: number;
  text: string;
  sides: string;
  sources: string;
  theme: string;
  priority: string;
  path: string;
  /** Сколько историй объявила сама потребность. */
  stories: number;
}

export interface Story {
  id: string;
  title: string;
  path: string;
  area: string;
  persona: string;
  phase: string;
  feature: string;
  state: string;
  tasks: number;
  done: number;
}

export interface IntentBoard {
  needs: Need[];
  stories: Story[];
  /** История называет потребность, а потребность о ней молчит. */
  oneSided: { needId: string; storyId: string; kind: string }[];
  oneSidedNeeds: { needId: string; stories: string[] }[];
  orphanNeeds: string[];
  danglingNeeds: { needId: string; storyId: string; kind: string }[];
  totalNeeds: number;
  totalStories: number;
}

export const loadIntent = (projectId: string) => get<IntentBoard>(`/api/projects/${projectId}/intent`);

export interface DbTable {
  name: string;
  migration: string;
  migrationFile: string;
  /** Колонки одной строкой, как их пишет модель данных: разделитель — «·». */
  columns: string;
  path: string;
  described: boolean;
}

export interface DataModelBoard {
  tables: DbTable[];
  migrations: { migration: string; tables: number; silent: number }[];
  /** Миграций объявлено набором; часть может не дать ни одной описанной таблицы. */
  migrationsDeclared: number;
  total: number;
  described: number;
  withoutColumns: string[];
  withoutMigration: string[];
}

export const loadDataModel = (projectId: string) => get<DataModelBoard>(`/api/projects/${projectId}/data-model`);

export interface PlanEntry {
  name: string;
  level: string;
  contains: string;
  stateText: string;
  /** Что план утверждает: документ есть или его нет. */
  claim: string;
  path: string;
  /** Сколько документов отозвалось на это имя. */
  matches: number;
  missing: boolean;
  wrong: boolean;
}

export interface PlanBoard {
  items: PlanEntry[];
  drift: { name: string; claimed: number; actual: number }[];
  total: number;
  unresolved: string[];
  presentButDeclaredAbsent: string[];
}

export const loadDocumentPlan = (projectId: string) => get<PlanBoard>(`/api/projects/${projectId}/document-plan`);

export interface Term {
  id: string;
  term: string;
  meaning: string;
  area: string;
  path: string;
  /** В скольких документах помимо словаря встречается машинное имя. */
  used: number;
}

export interface GlossaryBoard {
  terms: Term[];
  /** Докуда термин дотянулся: этап корпуса и сколько в нём документов. */
  reach: { id: string; stage: string; documents: number }[];
  total: number;
  unused: string[];
  median: number;
  widest: number;
}

export const loadGlossary = (projectId: string) => get<GlossaryBoard>(`/api/projects/${projectId}/glossary`);

export interface RestBoard {
  screens: { id: string; title: string; path: string; area: string; citedBy: number }[];
  orphanScreens: string[];
  screenRefs: { source: string; sourceKind: string; screenId: string }[];
  danglingScreens: number;
  features: { id: string; title: string; path: string; stories: number; done: number; inProgress: number }[];
  unclaimedStories: string[];
  claims: {
    area: string;
    title: string;
    requirements: number;
    described: number;
    inContract: number;
    inCode: string;
    path: string;
  }[];
  drift: unknown[];
  risks: {
    id: string;
    title: string;
    state: string;
    impact: string;
    probability: string;
    owner: string;
    settledBy: string;
    path: string;
  }[];
  risksNoOwner: string[];
  risksBadDecision: string[];
  runs: {
    id: string;
    title: string;
    milestone: string;
    version: string;
    isMilestone: boolean;
    leftOpen: boolean;
    sections: number;
    path: string;
  }[];
  runsDangling: string[];
  closedWithoutRun: string[];
}

export const loadRest = (projectId: string) => get<RestBoard>(`/api/projects/${projectId}/rest`);

export interface Carrier {
  id: string;
  kind: string;
  title: string;
  path: string;
}

export const loadCarriers = (projectId: string) =>
  get<{ carriers: Carrier[] }>(`/api/projects/${projectId}/answer`).then((d) => d.carriers ?? []);

/** Черновик проверяется на сервере той же меркой, что считает страницу. */
export async function checkDraft(
  projectId: string,
  path: string,
  draft: string,
): Promise<{ kind: AnswerKind; broken: string[] }> {
  const res = await fetch(`/api/projects/${projectId}/answer`, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify({ path, draft }),
  });
  if (!res.ok) return { kind: "missing", broken: [] };
  const got = (await res.json()) as { kind?: AnswerKind; broken?: string[] };
  return { kind: got.kind ?? "missing", broken: got.broken ?? [] };
}

/** Вид сущностей: сколько их и спроецирован ли вид вовсе. */
export interface KindRow {
  kind: string;
  shape: "document" | "inner";
  single: boolean;
  in?: string | null;
  /** `null` — вида в базе нет; это не ноль, и путать нельзя. */
  count: number | null;
  projected: boolean;
}

export const loadKinds = (projectId: string) =>
  get<{ kinds: KindRow[] }>(`/api/projects/${projectId}/kinds`).then((d) => d.kinds ?? []);

export const loadIds = (projectId: string, kind: string) =>
  get<{ ids: string[] }>(`/api/projects/${projectId}/ids?kind=${encodeURIComponent(kind)}`).then((d) => d.ids ?? []);

export interface Entity {
  kind: string;
  id: string;
  content?: string;
  revision?: number;
  updatedBy?: string;
  entity?: Record<string, unknown>;
}

/** `brief` — без текста сущности: читалка берёт его блоками, разобранными сервером. */
export const loadEntityByName = (projectId: string, kind: string, id?: string, brief = false) =>
  get<Entity>(
    `/api/projects/${projectId}/entity?kind=${encodeURIComponent(kind)}` +
      (id ? `&id=${encodeURIComponent(id)}` : "") +
      (brief ? "&brief=true" : ""),
  );

/** Документ корпуса, каким его держит `project_documents`. */
export interface DocumentSummary {
  path: string;
  bytes: number;
  revision: number;
  updatedAt: number;
  updatedBy: string;
}

export const loadDocuments = (projectId: string) =>
  get<{ documents: DocumentSummary[] }>(`/api/projects/${projectId}/documents`).then((d) => d.documents ?? []);

/**
 * Вызов инструмента. Имя и ответ те же, что у MCP: словарь один на обе стороны,
 * иначе интерфейс и харнес спросят разное и разойдутся молча.
 */
/**
 * Запись дверью. Веб до сих пор только читал: на сорок пять дверей заведения
 * приходился один `POST`, и тот для проверки черновика. Правка шла мимо
 * интерфейса — через `mh call`.
 *
 * Отказ двери — это ОТВЕТ, а не сбой: дверь объясняет, почему не приняла, и
 * объяснение надо показать человеку, а не проглотить.
 */
export async function write(
  projectId: string,
  name: string,
  args: Record<string, unknown>,
): Promise<{ ok: boolean; why: string; got: unknown }> {
  const res = await fetch(`/api/projects/${projectId}/tool/${name}`, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(args),
  });
  let got: unknown = null;
  try {
    got = await res.json();
  } catch {
    return { ok: false, why: "сервер ответил не разбираемым телом", got: null };
  }
  const d = got as { why?: string; status?: string; error?: string };
  if (!res.ok) return { ok: false, why: d?.why ?? d?.error ?? `сервер отказал (${res.status})`, got };
  // Дверь может принять запрос и отказать по существу — это тоже «не записано».
  const плохо = d?.status && !["declared", "written", "renamed", "ok", "dropped"].includes(d.status);
  return { ok: !плохо, why: плохо ? (d.why ?? d.status ?? "") : "", got };
}

export function tool<T>(projectId: string, name: string, args: Record<string, string | number> = {}): Promise<T> {
  const query = Object.entries(args)
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
    .join("&");
  return get<T>(`/api/projects/${projectId}/tool/${name}${query ? `?${query}` : ""}`);
}

/** Плитка прогресса: три числа, никогда одно. */
export interface Tile {
  tile: string;
  done: number;
  open: number;
  unknown: number;
  percent: number | null;
  says: string;
}

export interface Step {
  ord: number;
  question: string;
  state: string;
  ownerKind: string;
  owner: string;
  touches: string;
  why?: string;
  /** Сколько нашлось и что именно. Пусто у пройденной: искать было нечего. */
  violations?: number;
  detail?: string[];
}

export interface NextStep {
  process: string;
  at: Step | null;
  passed: number[];
  skipped: { ord: number; why: string }[];
  unanswerable: { ord: number; why: string; question: string }[];
  corpusPhaseOpen: boolean;
  /** Когда положение измерено и не устарело ли: набор мог измениться только что. */
  checkedAt?: number | null;
  stale?: boolean;
}

export interface Section {
  ord: number;
  level: number;
  title: string;
  anchor: string;
  /** Вес раздела **без** вложенных подразделов: сколько блоков, знаков, таблиц. */
  blocks: number;
  chars: number;
  tables: number;
  code: number;
}

export const loadSections = (projectId: string, kind: string, id?: string) =>
  tool<{ sections: Section[] }>(projectId, "sections", id ? { kind, id } : { kind }).then((d) => d.sections ?? []);

export const loadSection = (projectId: string, kind: string, anchor: string, id?: string) =>
  tool<{ body: string; revision: number }>(projectId, "section", id ? { kind, id, anchor } : { kind, anchor });

export interface Backlink {
  from: string;
  label: string | null;
}

export const loadBacklinks = (projectId: string, kind: string, id?: string) =>
  tool<{ backlinks: Backlink[]; count: number }>(projectId, "backlinks", id ? { kind, id } : { kind });

export const loadBlocks = (
  projectId: string,
  kind: string,
  id?: string,
  anchor?: string,
  own = false,
) =>
  tool<{ blocks: import("./Blocks").DocBlock[]; count: number }>(projectId, "blocks", {
    kind,
    ...(id ? { id } : {}),
    ...(anchor ? { anchor, ...(own ? { own: "true" } : {}) } : {}),
  });

/** Сводка по виду: форма одна на все разделы — имя, заголовок, словесные колонки, числовые. */
export interface SummaryRow {
  id: string;
  title: string;
  [column: string]: string | number;
}
export interface Summary {
  kind: string;
  count: number;
  words: string[];
  numbers: string[];
  columns: string[];
  rows: SummaryRow[];
}
export const loadSummary = (projectId: string, kind: string) =>
  tool<Summary>(projectId, "summary", { kind });

/** Связи сущности — то, что показывает панель раздела. */
export interface LinkItem { id: string; title: string; kind: string }
export const loadLinks = (projectId: string, kind: string, id: string) =>
  tool<{ sets: Record<string, LinkItem[]> }>(projectId, "links-of", { kind, id });

/** Снятые слова: их в словаре нет — они оттуда убраны, и это разные факты. */
export const loadRetired = (projectId: string) =>
  tool<{ rows: { term: string }[] }>(projectId, "retired-terms", {}).then((d) => d.rows.map((r) => r.term));
