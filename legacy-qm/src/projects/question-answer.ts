/**
 * Чем закрыт вопрос — по мерке, которую реестр вопросов написал себе сам
 * (`00-frame/questions/README.md`): «вопрос закрывается тем, что на него
 * ответили, и ответ — это ссылка на документ, который теперь его несёт. Пока
 * ссылки нет, вопрос не закрыт, а пересказан».
 *
 * Здесь эта мерка становится проверкой. Она различает не «хорошо/плохо», а то,
 * что читателю с ответом делать: по ссылке он попадёт в место, по имени — будет
 * искать, по слову — не найдёт ничего.
 */
export type AnswerKind =
  /** Ссылка на документ корпуса, и документ на месте. */
  | "link"
  /** Ссылка есть, документа нет: адрес обещает проверяемость и не даёт её. */
  | "dead-link"
  /** Назван идентификатор, и он существует: место найдётся, но искать придётся. */
  | "name"
  /** Назван идентификатор, которого в наборе нет. */
  | "unknown-name"
  /** Ни ссылки, ни имени — слово состояния, которое не стареет и не проверяется. */
  | "words"
  /** Раздела «Ответ» нет вовсе. */
  | "missing";

export interface AnswerVerdict {
  kind: AnswerKind;
  /** Относительные адреса из ответа, как они написаны. */
  links: string[];
  /** Идентификаторы, названные в ответе. */
  names: string[];
  /** Из них — те, которых в наборе нет, либо адреса, которые не разрешились. */
  broken: string[];
}

/** Раздел документа по заголовку любого уровня. */
export function sectionOf(content: string, heading: string): string | null {
  const re = new RegExp(`\\n#{1,6}\\s*${heading}\\s*\\n([\\s\\S]*?)(?=\\n#{1,6}\\s|$)`, "i");
  const m = re.exec(`\n${content}`);
  return m ? m[1]!.trim() : null;
}

const LINK = /\[[^\]]*\]\(([^)\s#]+)(?:#[^)\s]*)?\)/g;
const BACKTICKED = /`([^`\s]+)`/g;

/**
 * Идентификатор набора кончается числом: `FR-RT-09`, `ADR-0087`, `TC-ORG-28`.
 * Правило заодно отсекает то, что похоже на имя, но именем набора не является:
 * заголовок HTTP (`Idempotency-Key`, `X-Yabeda-User-Id`) и образец (`TC-nn`).
 */
const IDENTIFIER = /^[A-Z][A-Za-z]*(?:-[A-Za-z]+)*-\d+$/;

export function isIdentifier(value: string): boolean {
  return IDENTIFIER.test(value);
}

/** Разрешение относительного адреса без обращения к файловой системе. */
export function resolvePath(dir: string, href: string): string {
  const parts = `${dir}/${href}`.split("/");
  const out: string[] = [];
  for (const part of parts) {
    if (part === "" || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return out.join("/");
}

export function classifyAnswer(input: {
  /** Текст раздела «Ответ»; `null` — раздела в документе нет. */
  body: string | null;
  /** Каталог самого вопроса: относительные адреса считаются от него. */
  dir: string;
  hasDocument: (path: string) => boolean;
  hasName: (id: string) => boolean;
}): AnswerVerdict {
  const body = input.body?.trim() ?? "";
  if (!input.body || !body) return { kind: "missing", links: [], names: [], broken: [] };

  // Внешний адрес местом в корпусе не является: он никем не ведётся и корпусом
  // не проверяется, поэтому ответом по мерке реестра не считается.
  const links = [...body.matchAll(LINK)].map((m) => m[1]!).filter((l) => !/^[a-z]+:/i.test(l));
  if (links.length) {
    // Сломанным называем разрешённый путь, а не то, как он написан: искать
    // читателю всё равно по нему.
    const broken = links
      .map((l) => resolvePath(input.dir, l))
      .filter((p) => !input.hasDocument(p));
    return { kind: broken.length ? "dead-link" : "link", links, names: [], broken };
  }

  const names = [...new Set([...body.matchAll(BACKTICKED)].map((m) => m[1]!).filter(isIdentifier))];
  if (names.length) {
    const broken = names.filter((n) => !input.hasName(n));
    return { kind: broken.length ? "unknown-name" : "name", links: [], names, broken };
  }

  return { kind: "words", links: [], names: [], broken: [] };
}
