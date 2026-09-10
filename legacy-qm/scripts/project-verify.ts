/**
 * Приёмка переноса: содержимое каждого файла набора — В НУЖНЫХ МЕСТАХ базы.
 *
 * Побайтовое сравнение здесь не годится, и не потому что строго, а потому что отвечает на
 * другой вопрос. Оно доказывает, что КОПИЯ цела, — а перенос делался ради того, чтобы
 * содержимое разошлось по таблицам: заголовки в разделы, строки таблиц в ячейки, ссылки в
 * ссылки, а объявленные имена — в свою предметную таблицу. Целая копия при пустых
 * проекциях прошла бы побайтовую проверку и не значила бы ничего.
 *
 * Разбор ЗДЕСЬ СВОЙ, простой и независимый: проверка, зовущая тот же разборщик, который
 * проверяет, сверяет его сам с собой и зелена по построению.
 *
 *   node scripts/project-verify.ts --project <id> --vault <путь> [--db <url>]
 *                                 [--show <n>] [--only <префикс>]
 *
 * Выход 0 — всё нашлось. Выход 1 — что-то не нашлось, и оно названо документом, видом и
 * самим значением: «расхождений 12» без имён не проверяемо и потому бесполезно.
 */

import { readFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import pg from "pg";

const SKIP_DIRS = new Set([".git", ".obsidian", ".trash", "node_modules"]);

/**
 * Что ещё, кроме markdown, входит в набор. Список ТОТ ЖЕ, что у переноса: обход, знающий
 * меньше переноса, объявляет перенесённое лишним — проверка начинает мерить уже, чем
 * обещает, и краснеет на собственной слепоте. Ровно это она и сделала на семи мокапах.
 */
const RAW_SUFFIXES = [".html", ".js", ".css", ".svg", ".json", ".yaml", ".yml"];

/** Разбирается только markdown; сырое проверяется присутствием и байтами. */
const isMarkdown = (path: string) => path.endsWith(".md");

function fail(message: string): never {
  console.error(`project-verify: ${message}`);
  process.exit(2);
}

const argv = process.argv.slice(2);
const arg = (name: string): string | undefined => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const project = arg("project") ?? fail("не назван проект: --project <id>");
const vault = arg("vault") ?? fail("не назван корень документов: --vault <путь>");
const db = arg("db") ?? process.env.DATABASE_URL ?? process.env.PROBE_DB ?? fail("не названа база: --db <url>");
const show = Number(arg("show") ?? 12);
const only = arg("only") ?? "";

// ── Свой разбор: ровно столько, сколько нужно проверке ───────────────────────────────────

interface Parsed {
  headings: { level: number; title: string }[];
  /**
   * Ячейки строк таблиц. `exact` — там, где граница ячейки однозначна; `loose` — там, где в
   * строке встретились и черта, и кодовая вставка, и где она граница, а где часть значения,
   * снаружи не решить. Второе проверяется вхождением, а не равенством, и считается отдельно:
   * проверка, выдающая нерешённое за проверенное, врёт тем сильнее, чем она зеленее.
   */
  cells: { value: string; exact: boolean }[];
  links: { label: string; target: string; anchor: string }[];
  /** Первая ячейка каждой строки таблицы: там объявляются идентификаторы. */
  firstCells: string[];
}

const FENCE = /^\s*(```|~~~)/;
const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const SEPARATOR = /^[\s|:-]+$/;
const LINK = /\[([^\]\n]*)\]\(([^)\s]+)\)/g;

/**
 * База хранит заголовок и значение ячейки ТЕКСТОМ, без разметки, — и это правильно: `Сигнал`
 * и `` `Сигнал` `` — одно имя, написанное дважды. Сравнивать разметку с текстом значит
 * находить расхождение там, где его нет, поэтому обе стороны приводятся к тексту. Приведение
 * здесь своё и нарочно грубое: подробнее — значит повторять разборщик, который и проверяем.
 */
function plain(text: string): string {
  return text
    .replace(LINK, "$1")
    .replace(/[`*_]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Строка таблицы на ячейки. Сканером, а не `split`, и причина замерена на наборе: черта
 * внутри кодовой вставки — команда вида «ls … | wc -l», записанная в ячейке, — границей
 * ячейки не является, а наивное деление резало по ней и давало обрывок. Экранированная
 * черта — тоже часть значения, а не граница.
 */
function splitRow(row: string): string[] {
  const cells: string[] = [];
  let current = "";
  let inCode = false;
  for (let i = 0; i < row.length; i += 1) {
    const ch = row[i]!;
    if (ch === "\\" && row[i + 1] === "|") {
      // Экранирование снимается: в значении живёт черта, `\|` — способ её записать.
      current += "|";
      i += 1;
      continue;
    }
    if (ch === "`") inCode = !inCode;
    if (ch === "|" && !inCode) {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += ch;
  }
  cells.push(current.trim());
  return cells;
}

function parse(content: string): Parsed {
  const out: Parsed = { headings: [], cells: [], links: [], firstCells: [] };
  let inFence = false;
  for (const line of content.split("\n")) {
    if (FENCE.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;

    const heading = HEADING.exec(line);
    if (heading) out.headings.push({ level: heading[1]!.length, title: heading[2]! });

    const trimmed = line.trim();
    if (trimmed.startsWith("|") && trimmed.endsWith("|") && trimmed.length > 1 && !SEPARATOR.test(trimmed)) {
      const parts = splitRow(trimmed.slice(1, -1));
      const exact = !(trimmed.includes("`") && trimmed.includes("|"));
      for (const value of parts) out.cells.push({ value, exact });
      if (parts[0] !== undefined) out.firstCells.push(parts[0]);
    }

    for (const match of line.matchAll(LINK)) {
      const [target, anchor = ""] = match[2]!.split("#");
      out.links.push({ label: match[1]!, target: target!, anchor });
    }
  }
  return out;
}

// ── Что где объявляется: слот → предметная таблица ──────────────────────────────────────

/**
 * «В нужном месте» проверяется тем, что у каждой предметной строки есть `path`: имя, стоящее
 * в срезе требований, обязано быть требованием ИЗ ЭТОГО документа, а не просто существовать
 * где-то. Раскладка — карта проекта; у другого проекта она своя.
 */
const DECLARATIONS: {
  kind: string;
  when: (path: string) => boolean;
  idIn: RegExp;
  table: string;
  extra?: string;
}[] = [
  {
    kind: "требование",
    when: (p) => p === "10-intent/srs.md",
    idIn: /^`?((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)`?$/,
    table: "project_requirements",
  },
  {
    kind: "проверка",
    when: (p) => p === "40-proof/test-cases.md",
    idIn: /^`?(TC-[A-Z0-9]+-\d+[a-z]?)`?$/,
    table: "project_checks",
  },
];

/** Документ, который САМ есть сущность: имя берётся из имени файла, а не из таблицы внутри. */
const DOCUMENT_ENTITIES: { kind: string; when: (path: string) => boolean; id: (path: string) => string; table: string }[] =
  [
    {
      kind: "вопрос",
      when: (p) => p.startsWith("00-frame/questions/Q-") && p.endsWith(".md"),
      id: (p) => p.slice(p.lastIndexOf("/") + 1, -3),
      table: "project_questions",
    },
    {
      kind: "решение",
      when: (p) => /^30-design\/decisions\/.+\/\d{4}-.+\.md$/.test(p),
      id: (p) => `ADR-${p.slice(p.lastIndexOf("/") + 1).slice(0, 4)}`,
      table: "project_decisions",
    },
    {
      kind: "задача",
      when: (p) => /^50-plan\/v\d+\/M\d+\/M\d+-T[\w-]+\.md$/.test(p),
      id: (p) => p.slice(p.lastIndexOf("/") + 1, -3),
      table: "project_plan_tasks",
    },
    {
      kind: "история",
      when: (p) => /^10-intent\/use-cases\/.+\/US-[A-Z0-9-]+\.md$/.test(p),
      id: (p) => p.slice(p.lastIndexOf("/") + 1, -3),
      table: "project_stories",
    },
  ];

async function walk(root: string, dir: string = root): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(root, full)));
    else if (entry.name.endsWith(".md") || RAW_SUFFIXES.some((x) => entry.name.endsWith(x))) out.push(full);
  }
  return out;
}

const pool = new pg.Pool({ connectionString: db });

/** Всё, что база знает про набор, — четырьмя запросами, а не 1190×4. */
async function load(table: string, columns: string) {
  const { rows } = await pool.query(`SELECT path, ${columns} FROM ${table} WHERE project_id=$1`, [project]);
  const byPath = new Map<string, Record<string, unknown>[]>();
  for (const row of rows) {
    const list = byPath.get(String(row.path)) ?? [];
    list.push(row);
    byPath.set(String(row.path), list);
  }
  return byPath;
}

const sections = await load("project_document_sections", "level, title");
const cells = await load("project_document_cells", "raw, value");
const links = await load("project_document_links", "label, target_path, target_anchor");
const documents = new Set(
  (await pool.query(`SELECT path FROM project_documents WHERE project_id=$1`, [project])).rows.map((r) =>
    String(r.path),
  ),
);
const declared = new Map<string, Set<string>>();
for (const d of [...DECLARATIONS, ...DOCUMENT_ENTITIES]) {
  if (declared.has(d.table)) continue;
  const { rows } = await pool.query(`SELECT id, path FROM ${d.table} WHERE project_id=$1`, [project]);
  declared.set(d.table, new Set(rows.map((r) => `${String(r.path)}\0${String(r.id)}`)));
}

// ── Обход ───────────────────────────────────────────────────────────────────────────────

interface Miss {
  path: string;
  kind: string;
  value: string;
}

const misses: Miss[] = [];
const counted = { headings: 0, cells: 0, cellsLoose: 0, links: 0, ids: 0 };
const files = (await walk(vault)).sort();
let checked = 0;

for (const file of files) {
  const path = relative(vault, file).split(sep).join("/");
  if (only && !path.startsWith(only)) continue;
  checked += 1;

  if (!documents.has(path)) {
    misses.push({ path, kind: "документ", value: "самого документа нет в базе" });
    continue;
  }
  // Сырое — мокапы, скрипты — в базе лежит дословно и разбору не подлежит: проверено тем,
  // что документ на месте. Разбирать html как markdown значит выдумать себе находки.
  if (!isMarkdown(path)) continue;
  const parsed = parse(readFileSync(file, "utf8"));

  // Заголовки → разделы. Сравнение множествами: порядок проверяет сборка, а не это.
  const known = new Set((sections.get(path) ?? []).map((r) => `${r.level}\0${plain(String(r.title))}`));
  for (const h of parsed.headings) {
    counted.headings += 1;
    if (!known.has(`${h.level}\0${plain(h.title)}`)) {
      misses.push({ path, kind: "заголовок", value: `${"#".repeat(h.level)} ${h.title}` });
    }
  }

  // Ячейки таблиц → ячейки.
  const knownCells = new Set<string>();
  for (const r of cells.get(path) ?? []) {
    knownCells.add(plain(String(r.raw)));
    knownCells.add(plain(String(r.value)));
  }
  const knownJoined = [...knownCells];
  for (const c of parsed.cells) {
    const value = plain(c.value);
    if (value === "") continue;
    if (c.exact) {
      counted.cells += 1;
      if (!knownCells.has(value)) misses.push({ path, kind: "ячейка", value: c.value.slice(0, 90) });
    } else {
      counted.cellsLoose += 1;
      if (!knownCells.has(value) && !knownJoined.some((k) => k.includes(value))) {
        misses.push({ path, kind: "ячейка (вхождением)", value: c.value.slice(0, 90) });
      }
    }
  }

  // Ссылки → ссылки. Метка вместе с целью: одна цель под двумя метками — две записи.
  const knownLinks = new Set(
    (links.get(path) ?? []).map((r) => `${plain(String(r.label))}\0${r.target_path}\0${r.target_anchor}`),
  );
  for (const l of parsed.links) {
    if (l.target.startsWith("http")) continue; // внешние адреса набор не ведёт
    counted.links += 1;
    if (!knownLinks.has(`${plain(l.label)}\0${l.target}\0${l.anchor}`)) {
      misses.push({ path, kind: "ссылка", value: `[${l.label}](${l.target}${l.anchor ? "#" + l.anchor : ""})` });
    }
  }

  // Объявленные имена → своя предметная таблица, и именно из этого документа.
  for (const rule of DECLARATIONS) {
    if (!rule.when(path)) continue;
    const set = declared.get(rule.table)!;
    for (const cell of parsed.firstCells) {
      const id = rule.idIn.exec(cell)?.[1];
      if (!id) continue;
      counted.ids += 1;
      if (!set.has(`${path}\0${id}`)) misses.push({ path, kind: rule.kind, value: id });
    }
  }
  for (const rule of DOCUMENT_ENTITIES) {
    if (!rule.when(path)) continue;
    const id = rule.id(path);
    counted.ids += 1;
    if (!declared.get(rule.table)!.has(`${path}\0${id}`)) misses.push({ path, kind: rule.kind, value: id });
  }
}

// Обратная сторона: путь, которого в наборе нет. Обход идёт от файлов и такое пропускает.
const vaultPaths = new Set(files.map((f) => relative(vault, f).split(sep).join("/")));
const extra = [...documents].filter((p) => !vaultPaths.has(p) && (!only || p.startsWith(only)));

console.log(`документов проверено: ${checked}`);
console.log(
  `сверено: заголовков ${counted.headings} · ячеек ${counted.cells} (+${counted.cellsLoose} вхождением) · ссылок ${counted.links} · объявленных имён ${counted.ids}`,
);
console.log(`не нашлось в базе: ${misses.length}`);
console.log(`лишних в базе: ${extra.length}`);

const byKind = new Map<string, Miss[]>();
for (const m of misses) byKind.set(m.kind, [...(byKind.get(m.kind) ?? []), m]);
for (const [kind, list] of [...byKind].sort((a, b) => b[1].length - a[1].length)) {
  console.log(`\n  ${kind.toUpperCase()} — ${list.length}`);
  for (const m of list.slice(0, show)) console.log(`    ${m.path}\n      ${m.value}`);
  if (list.length > show) console.log(`    … и ещё ${list.length - show}`);
}
for (const p of extra.slice(0, show)) console.log(`\n  ЛИШНИЙ  ${p}`);

await pool.end();
process.exit(misses.length + extra.length === 0 ? 0 : 1);
