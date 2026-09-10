export interface DocumentNode {
  path: string;
  revision: number;
  bytes: number;
}

export interface TreeNode {
  name: string;
  path: string;
  children: Map<string, TreeNode>;
  document: DocumentNode | null;
}

/** Собирает настоящее вложенное дерево из путей: папка и одноимённый файл остаются разными узлами. */
export function buildTree(documents: readonly DocumentNode[]): TreeNode {
  const root: TreeNode = { name: "", path: "", children: new Map(), document: null };
  for (const document of documents) {
    let node = root;
    const segments = document.path.split("/");
    segments.forEach((segment, index) => {
      const path = segments.slice(0, index + 1).join("/");
      let child = node.children.get(segment);
      if (!child) {
        child = { name: segment, path, children: new Map(), document: null };
        node.children.set(segment, child);
      }
      node = child;
    });
    node.document = document;
  }
  return root;
}

export function matchesQuery(node: TreeNode, needle: string): boolean {
  if (!needle) return true;
  if (node.path.toLowerCase().includes(needle)) return true;
  for (const child of node.children.values()) if (matchesQuery(child, needle)) return true;
  return false;
}

/** Папки идут прежде файлов, дальше — по имени. */
export function ordered(node: TreeNode): TreeNode[] {
  return [...node.children.values()].sort((a, b) => {
    const folderFirst = Number(b.children.size > 0) - Number(a.children.size > 0);
    return folderFirst || a.name.localeCompare(b.name);
  });
}

/** Разрешает относительную ссылку в путь набора; чужие схемы и абсолютные ссылки остаются обычными. */
export function resolveDocumentPath(from: string, href: string): string {
  if (!href || /^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("#") || href.startsWith("/")) return "";
  const base = from.split("/").slice(0, -1);
  for (const part of href.split("#")[0]!.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") base.pop();
    else base.push(part);
  }
  return base.join("/");
}

export function documentBytes(n: number): string {
  return n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`;
}
