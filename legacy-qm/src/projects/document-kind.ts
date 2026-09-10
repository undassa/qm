/**
 * Вид документа — не папка, а то, чем документ является: вопрос, решение, история,
 * задача. От вида зависит вёрстка страницы: у вопроса свои блоки, у истории свои.
 *
 * Каталоги корпуса повторяют вид документа (`00-frame/document-plan.md` §7),
 * поэтому вид выводится из пути и ничего не додумывает.
 */
export const DOCUMENT_KINDS = [
  "question",
  "decision",
  "story",
  "task",
  "milestone",
  "run",
  "screen",
  "feature",
  "frame",
  "intent",
  "design",
  "proof",
  "after",
  "reference",
  "other",
] as const;
export type DocumentKind = (typeof DOCUMENT_KINDS)[number];

export interface KindMeta {
  kind: DocumentKind;
  /** Как раздел называется человеку. */
  title: string;
  /** Единственное число — для заголовка страницы документа. */
  one: string;
}

export const KIND_META: Record<DocumentKind, KindMeta> = {
  question: { kind: "question", title: "Вопросы", one: "Вопрос" },
  decision: { kind: "decision", title: "Решения", one: "Решение" },
  story: { kind: "story", title: "Истории", one: "История" },
  task: { kind: "task", title: "Задачи", one: "Задача" },
  milestone: { kind: "milestone", title: "Этапы", one: "Этап" },
  run: { kind: "run", title: "Прогоны", one: "Прогон" },
  screen: { kind: "screen", title: "Экраны", one: "Экран" },
  feature: { kind: "feature", title: "Функциональные области", one: "Область" },
  frame: { kind: "frame", title: "Рамка", one: "Документ рамки" },
  intent: { kind: "intent", title: "Намерение", one: "Документ намерения" },
  design: { kind: "design", title: "Устройство", one: "Документ устройства" },
  proof: { kind: "proof", title: "Доказательство", one: "Документ доказательства" },
  after: { kind: "after", title: "После выпуска", one: "Документ эксплуатации" },
  reference: { kind: "reference", title: "Справка", one: "Справка" },
  other: { kind: "other", title: "Прочее", one: "Документ" },
};

/** Порядок разделов — порядок конвейера, а не алфавит. */
export const KIND_ORDER: readonly DocumentKind[] = [
  "frame",
  "question",
  "intent",
  "story",
  "feature",
  "screen",
  "design",
  "decision",
  "proof",
  "milestone",
  "task",
  "run",
  "after",
  "reference",
  "other",
];

const RULES: { kind: DocumentKind; test: RegExp }[] = [
  { kind: "question", test: /^00-frame\/questions\// },
  { kind: "decision", test: /^30-design\/decisions\// },
  { kind: "story", test: /^10-intent\/use-cases\// },
  { kind: "feature", test: /^10-intent\/functional\// },
  { kind: "screen", test: /^20-surface\// },
  { kind: "task", test: /^50-plan\/v\d+\/[MV]\d+\// },
  { kind: "milestone", test: /^50-plan\/v\d+\/[MV]\d+\.md$/ },
  { kind: "run", test: /^60-runs\// },
  { kind: "frame", test: /^00-frame\// },
  { kind: "intent", test: /^10-intent\// },
  { kind: "design", test: /^30-design\// },
  { kind: "proof", test: /^40-proof\// },
  { kind: "after", test: /^70-after\// },
  { kind: "reference", test: /^90-reference\// },
];

export function kindOfPath(path: string): DocumentKind {
  for (const rule of RULES) if (rule.test.test(path)) return rule.kind;
  return "other";
}

const ID_IN_NAME = /([A-Z]{1,4}-[A-Z0-9]+(?:-\d+[a-z]?)?|[A-Z]-\d+|[MV]\d+-T[0-9a-z]+|[MV]\d+)/;

/** Идентификатор документа, если он есть в имени файла: `Q-328`, `ADR-0082`, `M1-T10`. */
export function documentId(path: string): string | null {
  const name = path.split("/").at(-1)?.replace(/\.md$/, "") ?? "";
  const direct = ID_IN_NAME.exec(name);
  if (direct) return direct[1]!;
  // решения названы `0082-slug`: номер несёт смысл, префикс добавляем сами
  const adr = /^(\d{3,4})-/.exec(name);
  return adr ? `ADR-${adr[1]}` : null;
}
