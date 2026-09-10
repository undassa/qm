/**
 * ОДНОРАЗОВЫЙ перенос набора документов проекта в Postgres: документы, затем все проекции.
 *
 * Это не механизм синхронизации, и заводить его как механизм не надо: после переноса
 * истиной становится база, а корень документов замирает — синхронизировать будет нечего.
 * Команда остаётся ради второго проекта и ради повторения переноса, если первый пришлось
 * отменить.
 *
 *   node scripts/project-import.ts --project <id> --vault <путь> [--db <url>]
 *                                 [--prune] [--documents-only] [--projections-only]
 *
 * Пересборка проекций живёт не здесь, а в `src/projects/reproject.ts`, и она НЕ одноразовая:
 * её зовёт всякая запись документа через API, иначе интерфейс показывает вчерашнее число.
 */

import { readFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import pg from "pg";

import { createPostgresProjectDocumentStore } from "../src/projects/postgres-project-document-store.ts";
import { reprojectAll } from "../src/projects/reproject.ts";

const SKIP_DIRS = new Set([".git", ".obsidian", ".trash", "node_modules"]);

/**
 * Что ещё, кроме markdown, есть в наборе и обязано в него попасть.
 *
 * Мокапы — источник истины по продукту, и лежат они не текстом: три холста `.dc.html`, три
 * собранных standalone-страницы и `support.js` к ним. Перенос только `.md` оставил бы
 * половину замысла снаружи, и «набор целиком в базе» было бы неправдой. Содержимое кладётся
 * СЫРЫМ, как написано, чтобы страницу можно было отдать в отображение как есть.
 */
const RAW_SUFFIXES = [".html", ".js", ".css", ".svg", ".json", ".yaml", ".yml"];

interface Args {
  project: string;
  vault: string;
  db: string;
  prune: boolean;
  documentsOnly: boolean;
  projectionsOnly: boolean;
}

function fail(message: string): never {
  console.error(`project-sync: ${message}`);
  process.exit(2);
}

function parseArgs(argv: readonly string[]): Args {
  const get = (name: string): string | undefined => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const project = get("project");
  const vault = get("vault");
  const db = get("db") ?? process.env.DATABASE_URL ?? process.env.PROBE_DB;
  const projectionsOnly = argv.includes("--projections-only");
  if (!project) fail("не назван проект: --project <id>");
  if (!vault && !projectionsOnly) fail("не назван корень документов: --vault <путь>");
  if (!db) fail("не названа база: --db <url>, или DATABASE_URL в окружении");
  return {
    project: project!,
    vault: vault ?? "",
    db: db!,
    prune: argv.includes("--prune"),
    documentsOnly: argv.includes("--documents-only"),
    projectionsOnly,
  };
}

/** Пути в базе всегда через `/`: набор может лежать на любой файловой системе. */
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

const args = parseArgs(process.argv.slice(2));
const docs = createPostgresProjectDocumentStore(args.db);
const pool = new pg.Pool({ connectionString: args.db });

// ── 1. Документы ────────────────────────────────────────────────────────────────────────

if (!args.projectionsOnly) {
  const files = (await walk(args.vault)).sort();
  const rel = (f: string) => relative(args.vault, f).split(sep).join("/");
  let written = 0;
  let unchanged = 0;
  const refusals: string[] = [];

  for (const file of files) {
    const result = await docs.put({
      projectId: args.project,
      path: rel(file),
      content: readFileSync(file, "utf8"),
      author: "vault-sync",
    });
    if (result.status === "written") written += 1;
    else if (result.status === "unchanged") unchanged += 1;
    else refusals.push(`${rel(file)}: ${result.status}`);
  }

  const vaultPaths = new Set(files.map(rel));
  const stale = (await docs.list(args.project)).filter((d) => !vaultPaths.has(d.path)).map((d) => d.path);

  console.log(
    `документы: в наборе ${files.length} · записано ${written} · без изменений ${unchanged} · отвергнуто ${refusals.length}`,
  );
  for (const r of refusals.slice(0, 20)) console.log("   отвергнут:", r);

  // Лишнее в базе — это документ, удалённый из набора. Он не исчезает сам: `put` о нём
  // ничего не узнаёт, а проекции продолжают его считать. Молча удалять тоже нельзя —
  // ошибка в пути корня выглядит точно так же, как удаление всего набора.
  if (stale.length > 0) {
    console.log(
      `лишних в базе: ${stale.length}${args.prune ? " — удаляются (--prune)" : " — оставлены, для удаления нужен --prune"}`,
    );
    for (const s of stale.slice(0, 10)) console.log("   лишний:", s);
    if (args.prune) {
      let removed = 0;
      for (const path of stale) if (await docs.remove(args.project, path, "vault-sync")) removed += 1;
      console.log(`   удалено: ${removed}`);
    }
  }
}

// ── 2. Проекции ─────────────────────────────────────────────────────────────────────────

if (!args.documentsOnly) {
  const { report, findings } = await reprojectAll(pool, docs, args.project, args.db);
  console.log();
  for (const line of report) console.log(line);
  if (findings.length > 0) {
    console.log(`\nнаходки (${findings.length}) — их не видит ни один гейт на файлах:`);
    for (const f of findings) console.log("  ", f);
  }
}

await docs.close?.();
await pool.end();
