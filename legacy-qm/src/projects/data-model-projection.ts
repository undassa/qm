import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Проект базы данных описан двумя способами сразу: миграция объявляет, какие
 * таблицы заводит («№ | Файл | Таблицы»), а разделы описывают колонки
 * («Таблица | Ключевые колонки», «Таблица | Колонки»). Эти два перечня обязаны
 * сходиться, и расходятся они молча.
 */
export interface ProjectDbTable {
  name: string;
  /** Номер миграции, которой таблица заводится; пусто — миграция её не называет. */
  migration: string;
  migrationFile: string;
  columns: string;
  path: string;
}

export interface ProjectMigration {
  number: string;
  file: string;
  tables: string[];
}

const MIGRATION_HEADER = ["№", "Файл", "Таблицы"];
/** Описание колонок встречается под несколькими шапками — все они об одном. */
const COLUMN_HEADERS = [
  ["Таблица", "Ключевые колонки"],
  ["Таблица", "Колонки"],
];
const TABLE_NAME = /^\s*`?([a-z][a-z0-9_]*)`?\s*$/;
// Имена таблиц в ячейке миграции разделены точкой-разделителем или запятой.
const NAME_SEPARATOR = /[·,]/;

export interface DataModelCell {
  blockOrd: number;
  row: number;
  col: number;
  value: string;
}

export interface DataModelBlock {
  ord: number;
  kind: string;
  raw: string;
}

export interface DataModelSource {
  path: string;
  cells: readonly DataModelCell[];
  /**
   * Большинство таблиц описано не строкой, а разделом «### `signals` — …» и
   * блоком кода под ним. Считать такие таблицы неописанными значит выдумать
   * находку на почти весь проект базы.
   */
  sections?: readonly { ord: number; title: string }[];
  blocks?: readonly DataModelBlock[];
}

/**
 * Колонки описаны блоками кода, и форм у них три: под заголовком с именем таблицы
 * («`signals` — факт, а не процесс»), под заголовком с несколькими именами
 * («policies и policy_versions») и под обычным заголовком раздела («5. Корреляция»),
 * где имя стоит в начале строки. Опираться на заголовок ненадёжно, поэтому строки
 * разбираются по именам, уже известным из миграций и таблиц описания: выдумать
 * таблицу из случайного слова так нельзя.
 */
const LINE_TABLE = /^\s*`?([a-z][a-z0-9_]*)`?\s+(\S.*)$/;
const SINGLE_NAME_TITLE = /^\s*`?([a-z][a-z0-9_]*)`?\s*(?:[—–-].*)?$/;

function codeLines(raw: string): string[] {
  return raw
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("```"))
    .filter((line) => line.trim().length > 0);
}

function columnsFromCode(source: DataModelSource, known: ReadonlySet<string>): Map<string, string> {
  const out = new Map<string, string>();
  const sections = [...(source.sections ?? [])].sort((a, b) => a.ord - b.ord);
  const blocks = [...(source.blocks ?? [])].sort((a, b) => a.ord - b.ord);

  const titleAt = (ord: number): string => {
    let title = "";
    for (const s of sections) {
      if (s.ord > ord) break;
      title = s.title;
    }
    return title;
  };

  for (const block of blocks) {
    if (block.kind !== "code") continue;
    const lines = codeLines(block.raw);
    let matched = false;
    for (const line of lines) {
      const parsed = LINE_TABLE.exec(line);
      const name = parsed?.[1];
      if (!name || !known.has(name)) continue;
      matched = true;
      if (!out.has(name)) out.set(name, parsed![2]!.replace(/\s+/g, " ").trim());
    }
    if (matched) continue;
    // Строки имени не назвали — тогда блок целиком про таблицу из заголовка.
    const single = SINGLE_NAME_TITLE.exec(titleAt(block.ord))?.[1];
    if (!single || !known.has(single) || out.has(single)) continue;
    const body = lines.join(" ").replace(/\s+/g, " ").trim();
    if (body) out.set(single, body);
  }
  return out;
}

export interface DataModelProjection {
  tables: ProjectDbTable[];
  migrations: ProjectMigration[];
}

function blocksOf(cells: readonly DataModelCell[]): Map<number, Map<number, string[]>> {
  const blocks = new Map<number, Map<number, string[]>>();
  for (const cell of cells) {
    const rows = blocks.get(cell.blockOrd) ?? new Map<number, string[]>();
    const row = rows.get(cell.row) ?? [];
    row[cell.col] = cell.value;
    rows.set(cell.row, row);
    blocks.set(cell.blockOrd, rows);
  }
  return blocks;
}

function headerIs(head: string[] | undefined, wanted: string[]): boolean {
  return Boolean(head) && wanted.every((name, i) => (head![i] ?? "").trim() === name);
}

export function projectDataModel(source: DataModelSource): DataModelProjection {
  const blocks = blocksOf(source.cells);
  const migrations: ProjectMigration[] = [];
  const byTable = new Map<string, ProjectDbTable>();

  for (const [, rows] of [...blocks.entries()].sort((a, b) => a[0] - b[0])) {
    const head = rows.get(0);

    if (headerIs(head, MIGRATION_HEADER)) {
      for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
        if (ord === 0) continue;
        const number = (row[0] ?? "").trim();
        if (!/^\d+$/.test(number)) continue;
        const tables = (row[2] ?? "")
          .split(NAME_SEPARATOR)
          .map((part) => TABLE_NAME.exec(part)?.[1])
          .filter((name): name is string => Boolean(name));
        migrations.push({ number, file: (row[1] ?? "").trim(), tables });
      }
      continue;
    }

    if (!COLUMN_HEADERS.some((wanted) => headerIs(head, wanted))) continue;
    for (const [ord, row] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
      if (ord === 0) continue;
      const name = TABLE_NAME.exec(row[0] ?? "")?.[1];
      if (!name) continue;
      const columns = (row[1] ?? "").trim();
      const known = byTable.get(name);
      // Первое описание главное; более позднее лишь дополняет пустое.
      if (known) {
        if (!known.columns && columns) known.columns = columns;
        continue;
      }
      byTable.set(name, { name, migration: "", migrationFile: "", columns, path: source.path });
    }
  }

  for (const migration of migrations) {
    for (const name of migration.tables) {
      const table = byTable.get(name);
      if (table && !table.migration) {
        table.migration = migration.number;
        table.migrationFile = migration.file;
        continue;
      }
      if (!table) {
        byTable.set(name, {
          name,
          migration: migration.number,
          migrationFile: migration.file,
          columns: "",
          path: source.path,
        });
      }
    }
  }

  // Блоки кода дополняют то, чего не дали таблицы описаний.
  for (const [name, columns] of columnsFromCode(source, new Set(byTable.keys()))) {
    const table = byTable.get(name);
    if (table && !table.columns) table.columns = columns;
  }

  return {
    tables: [...byTable.values()].sort((a, b) => a.name.localeCompare(b.name)),
    migrations: migrations.sort((a, b) => a.number.localeCompare(b.number)),
  };
}

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_db_tables(
    project_id TEXT NOT NULL, name TEXT NOT NULL, migration TEXT NOT NULL DEFAULT '',
    migration_file TEXT NOT NULL DEFAULT '', columns TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, name)
  )`,
  `CREATE TABLE IF NOT EXISTS project_db_migrations(
    project_id TEXT NOT NULL, number TEXT NOT NULL, file TEXT NOT NULL DEFAULT '',
    tables INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, number)
  )`,
];

type Row = Record<string, unknown>;

function rowToTable(row: Row): ProjectDbTable {
  return {
    name: String(row["name"]),
    migration: String(row["migration"] ?? ""),
    migrationFile: String(row["migration_file"] ?? ""),
    columns: String(row["columns"] ?? ""),
    path: String(row["path"] ?? ""),
  };
}

export interface DataModelStore {
  replace(projectId: string, projection: DataModelProjection): Promise<void>;
  list(projectId: string): Promise<ProjectDbTable[]>;
  counts(projectId: string): Promise<{ tables: number; migrations: number; described: number }>;
  /** Миграция заводит таблицу, а колонок её нигде не описано. */
  withoutColumns(projectId: string): Promise<ProjectDbTable[]>;
  /** Колонки описаны, а какая миграция заводит таблицу — не сказано. */
  withoutMigration(projectId: string): Promise<ProjectDbTable[]>;
  close(): Promise<void>;
}

export function createPostgresDataModelStore(connectionString: string): DataModelStore {
  const pg = createPgPool(connectionString, SCHEMA);
  return {
    async replace(projectId, projection) {
      return withPgTransaction(await pg.pool(), async (client) => {
        await client.query(`DELETE FROM project_db_tables WHERE project_id=$1`, [projectId]);
        await client.query(`DELETE FROM project_db_migrations WHERE project_id=$1`, [projectId]);
        for (const t of projection.tables) {
          await client.query(
            `INSERT INTO project_db_tables(project_id, name, migration, migration_file, columns, path)
             VALUES ($1,$2,$3,$4,$5,$6)`,
            [projectId, t.name, t.migration, t.migrationFile, t.columns, t.path],
          );
        }
        for (const m of projection.migrations) {
          await client.query(
            `INSERT INTO project_db_migrations(project_id, number, file, tables) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING`,
            [projectId, m.number, m.file, m.tables.length],
          );
        }
      });
    },

    async list(projectId) {
      const { rows } = await pg.query(`SELECT * FROM project_db_tables WHERE project_id=$1 ORDER BY name`, [projectId]);
      return rows.map((row) => rowToTable(row as Row));
    },

    async counts(projectId) {
      const { rows } = await pg.query(
        `SELECT (SELECT count(*) FROM project_db_tables WHERE project_id=$1)                    AS tables,
                (SELECT count(*) FROM project_db_migrations WHERE project_id=$1)                AS migrations,
                (SELECT count(*) FROM project_db_tables WHERE project_id=$1 AND columns <> '')  AS described`,
        [projectId],
      );
      const r = rows[0] as Row;
      return {
        tables: Number(r["tables"]),
        migrations: Number(r["migrations"]),
        described: Number(r["described"]),
      };
    },

    async withoutColumns(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_db_tables WHERE project_id=$1 AND columns='' ORDER BY name`,
        [projectId],
      );
      return rows.map((row) => rowToTable(row as Row));
    },

    async withoutMigration(projectId) {
      const { rows } = await pg.query(
        `SELECT * FROM project_db_tables WHERE project_id=$1 AND migration='' ORDER BY name`,
        [projectId],
      );
      return rows.map((row) => rowToTable(row as Row));
    },

    close: () => pg.close(),
  };
}
