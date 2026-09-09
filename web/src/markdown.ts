/**
 * Разметка статей конституции. Корпус пользуется узким набором: жирный, курсив,
 * встроенный код, ссылки и изредка цитата — списков в статьях нет.
 *
 * Безопасность держится порядком, а не проверкой после: текст сначала
 * экранируется целиком, и только потом вставляются теги, которые породили мы
 * сами. Поэтому разметка из документа не может стать разметкой страницы.
 */
export type Block =
  | { kind: "para"; html: string }
  | { kind: "quote"; html: string }
  | { kind: "code"; text: string }
  | { kind: "heading"; level: number; html: string }
  | { kind: "rule" }
  | { kind: "table"; head: string[]; rows: string[][] };

const ESCAPE: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };

/**
 * Экранирование заодно выбрасывает NUL — на нём держится метка ниже. Текст,
 * прошедший здесь, метку подделать уже не может.
 */
export function escapeHtml(text: string): string {
  return text.replace(/\u0000/g, "").replace(/[&<>"]/g, (c) => ESCAPE[c] ?? c);
}

/**
 * Ссылка ведёт наружу или никуда. Корпус — не веб: путь вида
 * `../30-design/decisions/0138-....md` приложение ниоткуда не отдаёт, и ссылка
 * на него была бы враньём — с виду переход, на деле 404. Такой адрес не ссылка,
 * а имя, и рисуется именем.
 */
export function safeHref(raw: string): string | null {
  const href = raw.trim();
  return /^https?:\/\//i.test(href) ? href : null;
}

/**
 * Метка вынутого кода. Внутри неё ничто не разметка — потому код и вынимается
 * первым. Метка стоит на NUL, которого в тексте после экранирования нет: пробел
 * вокруг числа меткой быть не может, иначе число из прозы («статья 12»)
 * подменялось бы куском кода.
 */
const MARK = "\u0000";
const MARKED = /\u0000(\d+)\u0000/g;

/** Встроенная разметка. Код обрабатывается первым: внутри него ничто не разметка. */
export function inline(text: string): string {
  const codes: string[] = [];
  let out = escapeHtml(text).replace(/`([^`]+)`/g, (_, body: string) => {
    codes.push(body);
    return `${MARK}${codes.length - 1}${MARK}`;
  });

  out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label: string, href: string) => {
    const safe = safeHref(href);
    return safe ? `<a href="${safe}" target="_blank" rel="noreferrer noopener">${label}</a>` : label;
  });
  out = out.replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>");
  out = out.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<i>$2</i>");

  return out.replace(MARKED, (_, i: string) => `<code>${codes[Number(i)] ?? ""}</code>`);
}

/**
 * Текст делится на блоки пустой строкой. Заголовок — блок сам по себе, даже
 * если пустой строки вокруг нет: иначе он прилипает к соседнему абзацу и
 * остаётся видимой решёткой.
 */
function chunksOf(body: string): string[] {
  const out: string[] = [];
  for (const chunk of body.split(/\n{2,}/)) {
    let buffer: string[] = [];
    for (const line of chunk.split("\n")) {
      if (HEADING.test(line)) {
        if (buffer.length) out.push(buffer.join("\n"));
        out.push(line);
        buffer = [];
      } else buffer.push(line);
    }
    if (buffer.length) out.push(buffer.join("\n"));
  }
  return out;
}

const HEADING = /^(#{1,6})\s+(.*)$/;
/** Строка таблицы-разделителя: `|---|:--:|`. Ею шапка отделяется от тела. */
const RULE = /^\|?[\s:|-]+\|?$/;

const cellsOf = (line: string): string[] =>
  line.replace(/^\s*\|/, "").replace(/\|\s*$/, "").split("|").map((c) => inline(c.trim()));

/**
 * Перенос внутри абзаца остаётся переносом строки, потому что в корпусе абзацы
 * свёрстаны вручную.
 */
export function toBlocks(body: string): Block[] {
  const blocks: Block[] = [];
  for (const chunk of chunksOf(body)) {
    const text = chunk.replace(/\s+$/, "");
    if (!text.trim()) continue;
    const lines = text.split("\n");

    // Черта — это черта, а не абзац из трёх дефисов. В корпусе ею отбивают
    // шапку документа от текста, и абзацем «---» отбивка читается как опечатка.
    if (lines.length === 1 && /^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(lines[0]!)) {
      blocks.push({ kind: "rule" });
      continue;
    }

    const heading = HEADING.exec(lines[0]!);
    if (heading && lines.length === 1) {
      blocks.push({ kind: "heading", level: heading[1]!.length, html: inline(heading[2]!) });
      continue;
    }
    if (lines.every((l) => l.trimStart().startsWith(">"))) {
      blocks.push({ kind: "quote", html: inline(lines.map((l) => l.replace(/^\s*>\s?/, "")).join(" ")) });
      continue;
    }
    if (lines.every((l) => l.startsWith("    ") || l.startsWith("\t"))) {
      blocks.push({ kind: "code", text: lines.map((l) => l.replace(/^(\s{4}|\t)/, "")).join("\n") });
      continue;
    }
    // Таблица опознаётся разделителем во второй строке: одни лишь палки бывают
    // и в прозе, а разделитель — нет.
    if (lines.length >= 2 && lines[0]!.trimStart().startsWith("|") && RULE.test(lines[1]!.trim())) {
      blocks.push({
        kind: "table",
        head: cellsOf(lines[0]!),
        rows: lines.slice(2).filter((l) => l.trim()).map(cellsOf),
      });
      continue;
    }
    blocks.push({ kind: "para", html: inline(lines.join(" ")) });
  }
  return blocks;
}
