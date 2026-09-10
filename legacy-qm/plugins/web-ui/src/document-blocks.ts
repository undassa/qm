/**
 * Роль раздела в документе. Виды документов в корпусе устроены очень регулярно:
 * у решения всегда «Контекст · Решение · Последствия · Отвергнутые варианты»,
 * у истории — «Зачем человеку · Критерий приёмки · Чем доказывается» и так далее.
 * Роль задаёт вёрстку блока, поэтому страница читается как документ своего вида,
 * а не как разметка.
 */
export type BlockRole =
  | "lead"
  | "answer"
  | "awaiting"
  | "decision"
  | "consequence"
  | "rejected"
  | "criteria"
  | "proof"
  | "links"
  | "attention"
  | "history"
  | "plain";

const BY_KIND: Record<string, Record<string, BlockRole>> = {
  question: {
    вопрос: "lead",
    "вопрос и контекст": "lead",
    "что решить": "lead",
    ответ: "answer",
    решение: "decision",
    цена: "consequence",
    связано: "links",
    "история изменений": "history",
  },
  decision: {
    контекст: "lead",
    context: "lead",
    решение: "decision",
    decision: "decision",
    последствия: "consequence",
    consequences: "consequence",
    "отвергнутые варианты": "rejected",
    "rejected alternatives": "rejected",
  },
  story: {
    "зачем это человеку": "lead",
    "потребности, из которых это выросло": "plain",
    "требования, на которые опирается": "links",
    "критерий приёмки": "criteria",
    "чем доказывается": "proof",
    "куда смотреть дальше": "links",
  },
  screen: {
    компоновка: "lead",
    компоненты: "plain",
    состояния: "plain",
    данные: "plain",
    связи: "links",
    "открытые вопросы": "attention",
  },
  run: {
    "что появилось": "lead",
    "тронутая часть": "plain",
    "что создано": "plain",
    "оставлено открытым": "attention",
    "найдено и решено": "consequence",
  },
};

/** Роль раздела: сперва правило вида, потом общие имена, иначе обычный блок. */
export function roleOf(kind: string, title: string): BlockRole {
  const key = title.trim().toLowerCase();
  const byKind = BY_KIND[kind]?.[key];
  if (byKind) return byKind;
  if (key.startsWith("история")) return "history";
  if (key.startsWith("открытые вопросы")) return "attention";
  return "plain";
}

export interface DocumentBlock {
  title: string;
  level: number;
  role: BlockRole;
  /** Разметка раздела — уже очищенная, её остаётся показать. */
  html: string;
}

/**
 * Режет отрисованную разметку на разделы по заголовкам. Делим готовый DOM, а не текст:
 * так деление совпадает с тем, что видит человек, и `#` внутри кода не считается заголовком.
 */
export function splitBlocks(kind: string, root: ParentNode): DocumentBlock[] {
  const blocks: DocumentBlock[] = [];
  let current: { title: string; level: number; nodes: string[] } | null = null;

  const flush = () => {
    if (!current) return;
    const html = current.nodes.join("").trim();
    if (current.title || html) {
      blocks.push({ title: current.title, level: current.level, role: roleOf(kind, current.title), html });
    }
    current = null;
  };

  for (const node of Array.from(root.childNodes)) {
    const element = node.nodeType === 1 ? (node as Element) : null;
    const heading = element && /^H[1-6]$/.test(element.tagName) ? Number(element.tagName[1]) : 0;
    if (heading) {
      flush();
      current = { title: element!.textContent?.trim() ?? "", level: heading, nodes: [] };
      continue;
    }
    if (!current) current = { title: "", level: 0, nodes: [] };
    current.nodes.push(element ? element.outerHTML : (node.textContent ?? ""));
  }
  flush();
  return blocks;
}

/** У вопроса без раздела «Ответ» ответа нет — это и есть открытый вопрос. */
export function withAwaiting(kind: string, blocks: DocumentBlock[]): DocumentBlock[] {
  if (kind !== "question" || blocks.some((b) => b.role === "answer")) return blocks;
  const history = blocks.findIndex((b) => b.role === "history");
  const stub: DocumentBlock = { title: "Ответ", level: 2, role: "awaiting", html: "" };
  if (history === -1) return [...blocks, stub];
  return [...blocks.slice(0, history), stub, ...blocks.slice(history)];
}

/**
 * Свойства документа — только поля из шапки, до первого раздела. Ниже по тексту
 * образец полей ловит обычные пункты со смысловым выделением («**Агрегат хранит
 * инварианты** …»), и шапка превращалась в обрывки фраз.
 */
export function headerFields<T extends { sectionOrd: number }>(fields: readonly T[]): T[] {
  return fields.filter((field) => field.sectionOrd === 0);
}
