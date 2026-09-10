/**
 * Проза называет сущности по идентификатору: «см. `FR-CFG-01`», «закрывает Q-10»,
 * «Article 4». Пока это просто текст, читатель обязан искать документ руками.
 *
 * Разметку переписываем по узлам текста, а не строкой: внутри `code`, `pre` и уже
 * существующих ссылок трогать нечего, и замена в HTML-строке ломала бы их атрибуты.
 */
export interface EntityRef {
  id: string;
  kind: string;
  path: string;
  anchor?: string;
}

/** Порядок важен: длинные образцы идут первыми, иначе `NFR-01` съест `FR-01`. */
const PATTERN =
  /\b(?:Article\s+\d+|ADR-\d{3,4}|(?:NFR|FR)-[A-Z0-9]+(?:-\d+[a-z]?)?|TC-[A-Z0-9]+-\d+[a-z]?|US-[A-Z0-9]+-\d+|SCR-[A-Z0-9]+-\d+|Q-\d+|[MV]\d+-T[0-9a-z]+)\b/g;

const SKIP = new Set(["A", "CODE", "PRE", "SCRIPT", "STYLE", "BUTTON", "TEXTAREA", "H1"]);

export function findEntityIds(text: string): string[] {
  return [...text.matchAll(PATTERN)].map((m) => m[0]!.replace(/\s+/g, " "));
}

/**
 * Обходит текст под `root` и оборачивает найденные идентификаторы кнопкой.
 * Возвращает, сколько ссылок поставлено — по нему видно, что указатель доехал.
 */
export function linkEntities(root: Element, known: ReadonlyMap<string, EntityRef>, self?: string): number {
  const doc = root.ownerDocument;
  const walker = doc.createTreeWalker(root, 4 /* NodeFilter.SHOW_TEXT */);
  const targets: Text[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const parent = (node as Text).parentElement;
    if (!parent || parent.closest(".entity-ref")) continue;
    let skip = false;
    for (let el: Element | null = parent; el && el !== root; el = el.parentElement) {
      if (SKIP.has(el.tagName)) {
        skip = true;
        break;
      }
    }
    if (!skip && PATTERN.test(node.nodeValue ?? "")) targets.push(node as Text);
    PATTERN.lastIndex = 0;
  }

  let linked = 0;
  for (const node of targets) {
    const text = node.nodeValue ?? "";
    const fragment = doc.createDocumentFragment();
    let at = 0;
    for (const match of text.matchAll(PATTERN)) {
      const id = match[0]!.replace(/\s+/g, " ");
      const ref = known.get(id);
      if (!ref || ref.path === self) continue;
      if (match.index! > at) fragment.append(doc.createTextNode(text.slice(at, match.index!)));
      const button = doc.createElement("button");
      button.type = "button";
      button.className = "entity-ref";
      button.dataset["path"] = ref.path;
      if (ref.anchor) button.dataset["anchor"] = ref.anchor;
      button.title = `${ref.kind} · ${ref.path}`;
      button.textContent = match[0]!;
      fragment.append(button);
      at = match.index! + match[0]!.length;
      linked += 1;
    }
    if (!at) continue;
    if (at < text.length) fragment.append(doc.createTextNode(text.slice(at)));
    node.replaceWith(fragment);
  }
  return linked;
}
