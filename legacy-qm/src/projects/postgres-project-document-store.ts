import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";
import {
  parseDocument,
  resolveLinkTarget,
  type DocumentBlock,
  type DocumentField,
  type DocumentStructure,
} from "./document-structure.ts";
import {
  bodyOfSection,
  replaceSectionBody,
  documentBytes,
  hashDocument,
  MAX_DOCUMENT_BYTES,
  normalizeDocumentPath,
  type ProjectDocument,
  type ProjectDocumentRevision,
  type ProjectDocumentStore,
  type ProjectDocumentSummary,
} from "./project-document-store.ts";

const SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_documents(
    project_id   TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    content      TEXT   NOT NULL,
    content_hash TEXT   NOT NULL,
    bytes        INTEGER NOT NULL,
    revision     BIGINT NOT NULL,
    updated_at   BIGINT NOT NULL,
    updated_by   TEXT   NOT NULL,
    PRIMARY KEY (project_id, path)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_revisions(
    id           BIGSERIAL PRIMARY KEY,
    project_id   TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    content      TEXT   NOT NULL,
    content_hash TEXT   NOT NULL,
    bytes        INTEGER NOT NULL,
    revision     BIGINT NOT NULL,
    written_at   BIGINT NOT NULL,
    written_by   TEXT   NOT NULL,
    UNIQUE (project_id, path, revision)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_blocks(
    project_id TEXT NOT NULL, path TEXT NOT NULL, ord INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('heading','prose','table','code','list','html','blank')),
    level INTEGER, raw TEXT NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_sections(
    project_id TEXT NOT NULL, path TEXT NOT NULL, ord INTEGER NOT NULL,
    level INTEGER NOT NULL, title TEXT NOT NULL, anchor TEXT NOT NULL,
    parent_ord INTEGER, first_block INTEGER NOT NULL, last_block INTEGER NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_cells(
    project_id TEXT NOT NULL, path TEXT NOT NULL, block_ord INTEGER NOT NULL,
    row_ord INTEGER NOT NULL, col INTEGER NOT NULL, raw TEXT NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY (project_id, path, block_ord, row_ord, col)
  )`,
  `CREATE TABLE IF NOT EXISTS project_document_links(
    project_id TEXT NOT NULL, path TEXT NOT NULL, block_ord INTEGER NOT NULL, ord INTEGER NOT NULL,
    label TEXT NOT NULL, target_path TEXT NOT NULL, target_anchor TEXT NOT NULL,
    PRIMARY KEY (project_id, path, block_ord, ord)
  )`,
  // Ссылка на документ определяется ПОЛЕМ, а не строкой, которую каждый читатель разрешает
  // заново. `target_path` — написание автора (`../../10-intent/srs.md`), артефакт того, что
  // набор когда-то лежал файлами; `target_document` — сам документ набора. Пусто, если цель
  // набору не принадлежит: чужая схема, абсолютный путь, якорь внутри своего же документа.
  `ALTER TABLE project_document_links ADD COLUMN IF NOT EXISTS target_document TEXT NOT NULL DEFAULT ''`,
  `CREATE TABLE IF NOT EXISTS project_document_fields(
    project_id TEXT NOT NULL, path TEXT NOT NULL, section_ord INTEGER NOT NULL, ord INTEGER NOT NULL,
    name TEXT NOT NULL, shape TEXT NOT NULL CHECK (shape IN ('row','bullet')),
    value_raw TEXT NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  )`,
  `CREATE INDEX IF NOT EXISTS project_document_links_by_target
    ON project_document_links(project_id, target_path)`,
  `CREATE INDEX IF NOT EXISTS project_document_links_by_document
    ON project_document_links(project_id, target_document)`,
  `CREATE INDEX IF NOT EXISTS project_document_fields_by_name
    ON project_document_fields(project_id, name, value)`,
];

type Row = Record<string, unknown>;

const num = (value: unknown): number => Number(value ?? 0);

function rowToDocument(row: Row): ProjectDocument {
  return {
    projectId: String(row["project_id"]),
    path: String(row["path"]),
    content: String(row["content"]),
    contentHash: String(row["content_hash"]),
    bytes: num(row["bytes"]),
    revision: num(row["revision"]),
    updatedAt: num(row["updated_at"]),
    updatedBy: String(row["updated_by"]),
  };
}

function rowToSummary(row: Row): ProjectDocumentSummary {
  return {
    path: String(row["path"]),
    contentHash: String(row["content_hash"]),
    bytes: num(row["bytes"]),
    revision: num(row["revision"]),
    updatedAt: num(row["updated_at"]),
    updatedBy: String(row["updated_by"]),
  };
}

type Client = { query: (text: string, params?: unknown[]) => Promise<{ rows: Row[]; rowCount: number }> };

async function insertRows(client: Client, sql: string, rows: unknown[][]): Promise<void> {
  const CHUNK = 500;
  for (let i = 0; i < rows.length; i += CHUNK) {
    const slice = rows.slice(i, i + CHUNK);
    const params: unknown[] = [];
    const tuples = slice.map((row) => `(${row.map((value) => `$${params.push(value)}`).join(",")})`).join(",");
    await client.query(`${sql} VALUES ${tuples}`, params);
  }
}

async function writeStructure(
  client: Client,
  projectId: string,
  path: string,
  structure: DocumentStructure,
): Promise<void> {
  for (const table of [
    "project_document_blocks",
    "project_document_sections",
    "project_document_cells",
    "project_document_links",
    "project_document_fields",
  ]) {
    await client.query(`DELETE FROM ${table} WHERE project_id=$1 AND path=$2`, [projectId, path]);
  }
  await insertRows(
    client,
    `INSERT INTO project_document_blocks(project_id, path, ord, kind, level, raw)`,
    structure.blocks.map((b) => [projectId, path, b.ord, b.kind, b.level, b.raw]),
  );
  await insertRows(
    client,
    `INSERT INTO project_document_sections(project_id, path, ord, level, title, anchor, parent_ord, first_block, last_block)`,
    structure.sections.map((s) => [
      projectId,
      path,
      s.ord,
      s.level,
      s.title,
      s.anchor,
      s.parentOrd,
      s.firstBlock,
      s.lastBlock,
    ]),
  );
  await insertRows(
    client,
    `INSERT INTO project_document_cells(project_id, path, block_ord, row_ord, col, raw, value)`,
    structure.cells.map((c) => [projectId, path, c.blockOrd, c.row, c.col, c.raw, c.value]),
  );
  await insertRows(
    client,
    `INSERT INTO project_document_links(project_id, path, block_ord, ord, label, target_path, target_anchor, target_document)`,
    structure.links.map((l) => [
      projectId,
      path,
      l.blockOrd,
      l.ord,
      l.text,
      l.targetPath,
      l.targetAnchor,
      resolveLinkTarget(path, l.targetPath),
    ]),
  );
  await insertRows(
    client,
    `INSERT INTO project_document_fields(project_id, path, section_ord, ord, name, shape, value_raw, value)`,
    structure.fields.map((f) => [projectId, path, f.sectionOrd, f.ord, f.name, f.shape, f.valueRaw, f.value]),
  );
}

export function createPostgresProjectDocumentStore(
  connectionString: string,
  opts: { now?: () => number } = {},
): ProjectDocumentStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, SCHEMA);

  const store: ProjectDocumentStore = {
    async put(input) {
      const path = normalizeDocumentPath(input.path);
      if (!path) return { status: "invalid_path" };
      const bytes = documentBytes(input.content);
      if (bytes > MAX_DOCUMENT_BYTES) return { status: "invalid_path" };
      const contentHash = hashDocument(input.content);
      const at = now();

      return withPgTransaction(await pg.pool(), async (client) => {
        const current = await client.query(
          `SELECT * FROM project_documents WHERE project_id=$1 AND path=$2 FOR UPDATE`,
          [input.projectId, path],
        );
        const existing = current.rows[0] ? rowToDocument(current.rows[0] as Row) : null;
        const seen = existing?.revision ?? 0;
        if (input.expectedRevision !== undefined && input.expectedRevision !== seen) {
          return {
            status: "conflict" as const,
            document: existing ?? {
              projectId: input.projectId,
              path,
              content: "",
              contentHash: hashDocument(""),
              bytes: 0,
              revision: 0,
              updatedAt: 0,
              updatedBy: "",
            },
          };
        }
        if (existing && existing.contentHash === contentHash) {
          return { status: "unchanged" as const, document: existing };
        }
        const revision = seen + 1;
        const written = await client.query(
          `INSERT INTO project_documents(project_id, path, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
           ON CONFLICT (project_id, path) DO UPDATE SET
             content = EXCLUDED.content, content_hash = EXCLUDED.content_hash, bytes = EXCLUDED.bytes,
             revision = EXCLUDED.revision, updated_at = EXCLUDED.updated_at, updated_by = EXCLUDED.updated_by
           RETURNING *`,
          [input.projectId, path, input.content, contentHash, bytes, revision, at, input.author],
        );
        await client.query(
          `INSERT INTO project_document_revisions(project_id, path, content, content_hash, bytes, revision, written_at, written_by)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8)`,
          [input.projectId, path, input.content, contentHash, bytes, revision, at, input.author],
        );
        await writeStructure(client as unknown as Client, input.projectId, path, parseDocument(input.content));
        return { status: "written" as const, document: rowToDocument(written.rows[0] as Row) };
      });
    },

    async get(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return null;
      const { rows } = await pg.query(`SELECT * FROM project_documents WHERE project_id=$1 AND path=$2`, [
        projectId,
        normalized,
      ]);
      return rows[0] ? rowToDocument(rows[0] as Row) : null;
    },

    async list(projectId, prefix) {
      // Префикс называет каталог, и писать его со слэшем на конце естественно.
      // Разбор пути отвергает пустой последний сегмент, и такой префикс молча
      // возвращал пустой список вместо содержимого каталога.
      const asked = prefix?.replace(/\/+$/, "") ?? "";
      const wanted = asked ? normalizeDocumentPath(asked) : "";
      if (asked && !wanted) return [];
      const { rows } = wanted
        ? await pg.query(
            `SELECT path, content_hash, bytes, revision, updated_at, updated_by FROM project_documents
             WHERE project_id=$1 AND (path=$2 OR path LIKE $3) ORDER BY path`,
            [projectId, wanted, `${wanted.replace(/[%_\\]/g, "\\$&")}/%`],
          )
        : await pg.query(
            `SELECT path, content_hash, bytes, revision, updated_at, updated_by FROM project_documents
             WHERE project_id=$1 ORDER BY path`,
            [projectId],
          );
      return rows.map((row) => rowToSummary(row as Row));
    },

    async remove(projectId, path, _author) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return false;
      const { rowCount } = await pg.query(`DELETE FROM project_documents WHERE project_id=$1 AND path=$2`, [
        projectId,
        normalized,
      ]);
      return rowCount > 0;
    },

    async history(projectId, path, limit = 50) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      const { rows } = await pg.query(
        `SELECT revision, content_hash, bytes, written_at, written_by FROM project_document_revisions
         WHERE project_id=$1 AND path=$2 ORDER BY revision DESC LIMIT $3`,
        [projectId, normalized, Math.max(1, Math.min(limit, 500))],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          revision: num(r["revision"]),
          contentHash: String(r["content_hash"]),
          bytes: num(r["bytes"]),
          writtenAt: num(r["written_at"]),
          writtenBy: String(r["written_by"]),
        } satisfies ProjectDocumentRevision;
      });
    },

    async atRevision(projectId, path, revision) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return null;
      const { rows } = await pg.query(
        `SELECT project_id, path, content, content_hash, bytes, revision, written_at AS updated_at, written_by AS updated_by
         FROM project_document_revisions WHERE project_id=$1 AND path=$2 AND revision=$3`,
        [projectId, normalized, revision],
      );
      return rows[0] ? rowToDocument(rows[0] as Row) : null;
    },

    async blocks(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      const { rows } = await pg.query(
        `SELECT ord, kind, level, raw FROM project_document_blocks
         WHERE project_id=$1 AND path=$2 ORDER BY ord`,
        [projectId, normalized],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          ord: num(r["ord"]),
          kind: String(r["kind"]) as DocumentBlock["kind"],
          level: r["level"] === null ? null : num(r["level"]),
          raw: String(r["raw"]),
        };
      });
    },

    async sections(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      const { rows } = await pg.query(
        `SELECT ord, level, title, anchor, parent_ord, first_block, last_block
         FROM project_document_sections WHERE project_id=$1 AND path=$2 ORDER BY ord`,
        [projectId, normalized],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          ord: num(r["ord"]),
          level: num(r["level"]),
          title: String(r["title"]),
          anchor: String(r["anchor"]),
          parentOrd: r["parent_ord"] === null ? null : num(r["parent_ord"]),
          firstBlock: num(r["first_block"]),
          lastBlock: num(r["last_block"]),
        };
      });
    },

    async section(projectId, path, anchor) {
      const document = await store.get(projectId, path);
      return document ? bodyOfSection(parseDocument(document.content), anchor) : null;
    },

    async fields(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      const { rows } = await pg.query(
        `SELECT section_ord, ord, name, shape, value_raw, value FROM project_document_fields
         WHERE project_id=$1 AND path=$2 ORDER BY ord`,
        [projectId, normalized],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          sectionOrd: num(r["section_ord"]),
          ord: num(r["ord"]),
          name: String(r["name"]),
          shape: String(r["shape"]) as DocumentField["shape"],
          valueRaw: String(r["value_raw"]),
          value: String(r["value"]),
        };
      });
    },

    async backlinks(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      // Цель разрешена при разборе и лежит полем, поэтому это обычный запрос по индексу.
      // Прежде здесь выбиралось по совпадению ИМЕНИ ФАЙЛА, а решало разрешение пути в JS:
      // ссылка была строкой, которую каждый читатель разрешал заново, и два документа с
      // одинаковым именем в разных слотах попадали в выборку оба.
      const { rows } = await pg.query(
        `SELECT path, label, block_ord, target_anchor FROM project_document_links
         WHERE project_id=$1 AND path <> $2 AND target_document = $2 ORDER BY path, block_ord`,
        [projectId, normalized],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          path: String(r["path"]),
          text: String(r["label"]),
          blockOrd: num(r["block_ord"]),
          targetAnchor: String(r["target_anchor"]),
        };
      });
    },

    async sectionTitleCount(projectId, prefix, title) {
      const { rows } = await pg.query(
        `SELECT count(DISTINCT path) AS n FROM project_document_sections
          WHERE project_id=$1 AND path LIKE $2 AND title = $3`,
        [projectId, `${prefix}%`, title],
      );
      return num((rows[0] as Row)["n"]);
    },

    async links(projectId, path) {
      const normalized = normalizeDocumentPath(path);
      if (!normalized) return [];
      const { rows } = await pg.query(
        `SELECT block_ord, ord, label, target_path, target_anchor FROM project_document_links
         WHERE project_id=$1 AND path=$2 ORDER BY block_ord, ord`,
        [projectId, normalized],
      );
      return rows.map((row) => {
        const r = row as Row;
        return {
          blockOrd: num(r["block_ord"]),
          ord: num(r["ord"]),
          text: String(r["label"]),
          targetPath: String(r["target_path"]),
          targetAnchor: String(r["target_anchor"]),
        };
      });
    },

    async reparse(projectId, prefix) {
      const listed = await store.list(projectId, prefix);
      let done = 0;
      for (const summary of listed) {
        const document = await store.get(projectId, summary.path);
        if (!document) continue;
        // Своя транзакция на документ, а не одна на набор: перечитывание идёт по тысяче
        // с лишним документов, и одна транзакция на всё держала бы их блокированными
        // целиком, пока интерфейс читает те же таблицы.
        await withPgTransaction(await pg.pool(), async (client) => {
          await writeStructure(client as unknown as Client, projectId, summary.path, parseDocument(document.content));
        });
        done += 1;
      }
      return done;
    },

    async putSection(input) {
      const document = await store.get(input.projectId, input.path);
      if (!document) return { status: "not_found" };
      const next = replaceSectionBody(document.content, input.anchor, input.body);
      if (next === null) return { status: "no_such_section" };
      return store.put({
        projectId: input.projectId,
        path: input.path,
        content: next,
        author: input.author,
        ...(input.expectedRevision !== undefined ? { expectedRevision: input.expectedRevision } : {}),
      });
    },

    close: () => pg.close(),
  };
  return store;
}
