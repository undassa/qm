export type BlockKind = "heading" | "prose" | "table" | "code" | "list" | "html" | "blank";

export interface DocumentBlock {
  ord: number;
  kind: BlockKind;
  level: number | null;
  raw: string;
}

export interface DocumentSection {
  ord: number;
  level: number;
  title: string;
  anchor: string;
  parentOrd: number | null;
  firstBlock: number;
  lastBlock: number;
}

export interface DocumentCell {
  blockOrd: number;
  row: number;
  col: number;
  raw: string;
  value: string;
}

export interface DocumentLink {
  blockOrd: number;
  ord: number;
  text: string;
  targetPath: string;
  targetAnchor: string;
}

export type FieldShape = "row" | "bullet";

export interface DocumentField {
  sectionOrd: number;
  ord: number;
  name: string;
  shape: FieldShape;
  valueRaw: string;
  value: string;
}

export interface DocumentStructure {
  blocks: DocumentBlock[];
  sections: DocumentSection[];
  cells: DocumentCell[];
  links: DocumentLink[];
  fields: DocumentField[];
}

const FENCE = /^(\s*)(`{3,}|~{3,})/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const TABLE_ROW = /^\s*\|/;
const TABLE_SEPARATOR = /^\s*\|[\s:|-]+\|\s*$/;
const LIST_ITEM = /^\s*([-*+]|\d+[.)])\s+/;
const HTML_OPEN = /^\s*<[a-zA-Z!/]/;
/**
 * Метка ссылки не переходит на другую строку, и запрет тут не косметический.
 *
 * С `[^\]]*` метка перескакивала переводы строк, и всякая открывающая скобка без закрывающей
 * дотягивалась до `]` следующей настоящей ссылки, съедая её вместе с прозой между ними.
 * Замер на наборе myack: `Q-328` несёт в тексте интервал `[since, until)` — запись
 * полуоткрытого периода внутри кодовой вставки, — и ссылка на `ADR-0162` тремя строками ниже
 * записывалась с меткой из трёх строк прозы. Ссылки читают и обратные ссылки, и проекция
 * решений, так что цена ошибки не в одной строке таблицы.
 */
const LINK = /\[([^\]\n]*)\]\(([^)\s]+)\)/g;
const BULLET_FIELD = /^\s*[-*+]\s+\*\*([^*]+?):?\*\*:?\s*(.*)$/;

/**
 * Приводит ссылку документа к пути набора: цель хранится сырой, какой её написал автор
 * (`../../10-intent/srs.md`), а сравнивать её с путём документа можно только разрешённой.
 * Чужие схемы и абсолютные ссылки набору не принадлежат — для них возвращается пустая строка.
 */
export function resolveLinkTarget(from: string, target: string): string {
  if (!target || /^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith("#") || target.startsWith("/")) return "";
  const base = from.split("/").slice(0, -1);
  for (const part of target.split("#")[0]!.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") base.pop();
    else base.push(part);
  }
  return base.join("/");
}

export function anchorOf(title: string): string {
  return stripMarkup(title)
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

export function stripMarkup(raw: string): string {
  return raw
    .replace(LINK, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/\*\*([^*]*)\*\*/g, "$1")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1$2")
    .replace(/~~([^~]*)~~/g, "$1")
    .trim();
}

function bare(line: string): string {
  return line.replace(/\r?\n$/, "");
}

function splitLines(content: string): string[] {
  const lines = content.split("\n");
  const out = lines.map((line, i) => (i === lines.length - 1 ? line : `${line}\n`));
  return out[out.length - 1] === "" ? out.slice(0, -1) : out;
}

function kindOfLine(raw: string, next: string | undefined): BlockKind {
  const line = bare(raw);
  if (line.trim() === "") return "blank";
  if (HEADING.test(line)) return "heading";
  if (TABLE_ROW.test(line) && next !== undefined && TABLE_SEPARATOR.test(bare(next))) return "table";
  if (LIST_ITEM.test(line)) return "list";
  if (HTML_OPEN.test(line)) return "html";
  return "prose";
}

/**
 * Строка таблицы на ячейки.
 *
 * `split("|")` здесь неверен дважды, и оба случая замерены на наборе myack.
 *
 * ЭКРАНИРОВАННАЯ ЧЕРТА. Автор пишет `\|`, когда черта — часть значения; это единственный
 * способ внести её в ячейку, и markdown его так и читает. Деление по всем чертам подряд
 * резало такую ячейку надвое: строка таблицы на ТРИ колонки в `document-plan.md` лежала
 * ЧЕТЫРЬМЯ ячейками, а третья была разорвана посреди команды `ls … \| wc -l`. Цена не в
 * одной строке: проекции читают ячейки ПО НОМЕРУ КОЛОНКИ, и у такой строки они читают
 * сдвинутое — вторая колонка становится третьей. В наборе 116 экранированных черт.
 *
 * ЧЕРТА ВНУТРИ КОДОВОЙ ВСТАВКИ. Внутри `` ` `` черта — текст, а не граница, даже когда
 * автор её не экранировал.
 *
 * Экранирование СНИМАЕТСЯ: в значении ячейки живёт черта, а `\|` — способ её записать.
 * Сборка документа обратно идёт из блоков, а не из ячеек, поэтому написание здесь не теряется.
 */
export function splitRow(inner: string): string[] {
  const cells: string[] = [];
  let current = "";
  let inCode = false;
  for (let i = 0; i < inner.length; i += 1) {
    const ch = inner[i]!;
    if (ch === "\\" && inner[i + 1] === "|") {
      current += "|";
      i += 1;
      continue;
    }
    if (ch === "`") inCode = !inCode;
    if (ch === "|" && !inCode) {
      cells.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  cells.push(current);
  return cells;
}

function cellsOfTable(blockOrd: number, raw: string): DocumentCell[] {
  const out: DocumentCell[] = [];
  let row = 0;
  for (const line of raw.split("\n")) {
    if (!TABLE_ROW.test(line)) continue;
    if (TABLE_SEPARATOR.test(line)) continue;
    const trimmed = line.replace(/\r?\n$/, "").trim();
    const inner = trimmed.replace(/^\|/, "").replace(/\|$/, "");
    splitRow(inner).forEach((cell, col) => {
      out.push({ blockOrd, row, col, raw: cell, value: stripMarkup(cell) });
    });
    row += 1;
  }
  return out;
}

function linksOf(blockOrd: number, raw: string): DocumentLink[] {
  const out: DocumentLink[] = [];
  let ord = 0;
  for (const match of raw.matchAll(LINK)) {
    const target = match[2] ?? "";
    const hash = target.indexOf("#");
    out.push({
      blockOrd,
      ord: ord++,
      text: match[1] ?? "",
      targetPath: hash === -1 ? target : target.slice(0, hash),
      targetAnchor: hash === -1 ? "" : target.slice(hash + 1),
    });
  }
  return out;
}

function fieldsOfTable(sectionOrd: number, cells: DocumentCell[], blockOrd: number): DocumentField[] {
  const rows = new Map<number, DocumentCell[]>();
  for (const cell of cells) {
    if (cell.blockOrd !== blockOrd) continue;
    const bucket = rows.get(cell.row) ?? [];
    bucket.push(cell);
    rows.set(cell.row, bucket);
  }
  const out: DocumentField[] = [];
  for (const bucket of rows.values()) {
    if (bucket.length !== 2) continue;
    const [key, value] = bucket as [DocumentCell, DocumentCell];
    const name = /^\s*\*\*([^*]+?)\*\*\s*$/.exec(key.raw)?.[1]?.trim();
    if (!name) continue;
    out.push({ sectionOrd, ord: out.length, name, shape: "row", valueRaw: value.raw, value: value.value });
  }
  return out;
}

/**
 * Пункт списка может переноситься: продолжение идёт с отступом и без своего маркера.
 * Читая по одной строке, значение обрывалось посреди фразы и уносило с собой
 * незакрытую разметку — «Accepted — but its **package LAYOUT …».
 */
function joinWrappedBullets(raw: string): string[] {
  const joined: string[] = [];
  for (const line of raw.split("\n")) {
    const isContinuation = /^\s+\S/.test(line) && !/^\s*[-*+]\s/.test(line);
    if (isContinuation && joined.length > 0) joined[joined.length - 1] += ` ${line.trim()}`;
    else joined.push(line);
  }
  return joined;
}

function fieldsOfList(sectionOrd: number, raw: string): DocumentField[] {
  const out: DocumentField[] = [];
  for (const line of joinWrappedBullets(raw)) {
    const match = BULLET_FIELD.exec(bare(line));
    if (!match) continue;
    out.push({
      sectionOrd,
      ord: out.length,
      name: match[1]!.trim(),
      shape: "bullet",
      valueRaw: match[2] ?? "",
      value: stripMarkup(match[2] ?? ""),
    });
  }
  return out;
}

export function parseDocument(content: string): DocumentStructure {
  const lines = splitLines(content);
  const blocks: DocumentBlock[] = [];
  let buffer: string[] = [];
  let bufferKind: BlockKind | null = null;

  const flush = (): void => {
    if (!bufferKind || buffer.length === 0) return;
    blocks.push({ ord: blocks.length, kind: bufferKind, level: null, raw: buffer.join("") });
    buffer = [];
    bufferKind = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const fence = FENCE.exec(line);
    if (fence) {
      flush();
      const marker = fence[2]!;
      const fenced = [line];
      i += 1;
      for (; i < lines.length; i++) {
        fenced.push(lines[i]!);
        if (lines[i]!.trimStart().startsWith(marker)) break;
      }
      blocks.push({ ord: blocks.length, kind: "code", level: null, raw: fenced.join("") });
      continue;
    }

    const kind = kindOfLine(line, lines[i + 1]);
    if (kind === "heading") {
      flush();
      const level = HEADING.exec(bare(line))![1]!.length;
      blocks.push({ ord: blocks.length, kind: "heading", level, raw: line });
      continue;
    }
    if (kind === "table" || bufferKind === "table") {
      const stillTable = TABLE_ROW.test(line);
      if (bufferKind === "table" && !stillTable) flush();
      else if (bufferKind !== "table" && bufferKind !== null) flush();
      if (stillTable) {
        bufferKind = "table";
        buffer.push(line);
        continue;
      }
    }
    // Продолжение пункта списка идёт с отступом и без своего маркера. Считая его
    // прозой, мы обрывали значение поля на конце строки вместе с разметкой.
    const continuesList = bufferKind === "list" && kind === "prose" && /^\s+\S/.test(line);
    if (bufferKind !== null && bufferKind !== kind && !continuesList) flush();
    if (!continuesList) bufferKind = kind;
    buffer.push(line);
  }
  flush();

  const sections: DocumentSection[] = [];
  const stack: DocumentSection[] = [];
  for (const block of blocks) {
    if (block.kind !== "heading") continue;
    const title = HEADING.exec(bare(block.raw))![2]!.trimEnd();
    const level = block.level!;
    while (stack.length && stack[stack.length - 1]!.level >= level) stack.pop();
    const section: DocumentSection = {
      ord: block.ord,
      level,
      title: stripMarkup(title),
      anchor: anchorOf(title),
      parentOrd: stack.length ? stack[stack.length - 1]!.ord : null,
      firstBlock: block.ord,
      lastBlock: blocks.length - 1,
    };
    sections.push(section);
    stack.push(section);
  }
  for (let i = 0; i < sections.length; i++) {
    for (let j = i + 1; j < sections.length; j++) {
      if (sections[j]!.level <= sections[i]!.level) {
        sections[i]!.lastBlock = sections[j]!.ord - 1;
        break;
      }
    }
  }

  const cells: DocumentCell[] = [];
  const links: DocumentLink[] = [];
  const fields: DocumentField[] = [];
  const sectionAt = (ord: number): number => {
    let found = -1;
    for (const section of sections) if (section.ord <= ord) found = section.ord;
    return found;
  };
  for (const block of blocks) {
    if (block.kind === "table") cells.push(...cellsOfTable(block.ord, block.raw));
    links.push(...linksOf(block.ord, block.raw));
    const sectionOrd = sectionAt(block.ord);
    if (sectionOrd === -1) continue;
    let found: DocumentField[] = [];
    if (block.kind === "table") found = fieldsOfTable(sectionOrd, cells, block.ord);
    else if (block.kind === "list") found = fieldsOfList(sectionOrd, block.raw);
    for (const field of found) fields.push({ ...field, ord: fields.length });
  }

  return { blocks, sections, cells, links, fields };
}

export function renderDocument(blocks: readonly DocumentBlock[]): string {
  return [...blocks]
    .sort((a, b) => a.ord - b.ord)
    .map((block) => block.raw)
    .join("");
}
