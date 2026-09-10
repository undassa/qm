import { createPgPool, withPgTransaction } from "../persistence/pg-pool.ts";

/**
 * Что читают под гейтом: фаза конвейера — это набор каталогов документов
 * (`00-frame/document-plan.md` §7, каталоги повторяют вид документа).
 */
export const PHASE_DOCUMENTS: Record<string, readonly string[]> = {
  G0: ["00-frame/"],
  G1: ["10-intent/"],
  G2: ["20-surface/", "30-design/"],
  G3: ["40-proof/"],
  G4: ["50-plan/", "60-runs/"],
};

export type GateKind = "query" | "command" | "signed";
export type GateState = "unknown" | "passed" | "failed" | "refused";

export interface Gate {
  phase: string;
  /**
   * Статья конституции, которую этот гейт стережёт. Решение — не механизм
   * принуждения, а исходник для него: пока статья не названа проверкой, она
   * держится только на памяти читающего.
   */
  article?: number | null;
  item: string;
  kind: GateKind;
  query: string | null;
  owner: string | null;
  state: GateState;
  violations: number;
  detail: string;
  checkedAt: number | null;
  /** Кто подписал подписной гейт и когда — машина за человека этого не делает. */
  signedBy: string | null;
  signedAt: number | null;
  /**
   * Документы, которые изменились после подписи. Пока список пуст — подпись держит;
   * первая же правка переоткрывает гейт, как и написано в модели харнеса.
   */
  staleDocuments: string[];
}

export interface GateOutcome {
  phase: string;
  item: string;
  state: GateState;
  violations: number;
  detail: string;
}

const SCHEMA = [
  `ALTER TABLE IF EXISTS project_gates ADD COLUMN IF NOT EXISTS article INTEGER`,
  `CREATE TABLE IF NOT EXISTS project_gates(
    project_id TEXT NOT NULL, phase TEXT NOT NULL, item TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('query','command','signed')),
    query TEXT, owner TEXT,
    state TEXT NOT NULL DEFAULT 'unknown' CHECK (state IN ('unknown','passed','failed','refused')),
    violations INTEGER NOT NULL DEFAULT 0,
    detail TEXT NOT NULL DEFAULT '',
    checked_at BIGINT,
    signed_by TEXT,
    signed_at BIGINT,
    article INTEGER,
    PRIMARY KEY (project_id, phase, item),
    CHECK ((kind = 'query') = (query IS NOT NULL)),
    CHECK ((kind = 'signed') = (owner IS NOT NULL))
  )`,
];

const SIGNATURE_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS project_gate_signatures(
    project_id TEXT NOT NULL, phase TEXT NOT NULL, item TEXT NOT NULL,
    path TEXT NOT NULL, content_hash TEXT NOT NULL,
    PRIMARY KEY (project_id, phase, item, path)
  )`,
];

const MIGRATIONS = [
  `ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS signed_by TEXT`,
  `ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS signed_at BIGINT`,
];

const FORBIDDEN = /\b(insert|update|delete|drop|alter|create|truncate|grant|revoke|copy|call|do)\b/i;

export function refuseQuery(query: string): string | null {
  const trimmed = query.trim();
  if (!/^select\b/i.test(trimmed)) return "запрос гейта обязан начинаться с SELECT";
  if (trimmed.includes(";")) return "запрос гейта не может нести второй оператор";
  if (FORBIDDEN.test(trimmed)) return "запрос гейта не может менять данные";
  return null;
}

/** Объявление гейта — то, что задаёт человек; состояние и подпись живут отдельно. */
export type GateDeclaration = Omit<
  Gate,
  "state" | "violations" | "detail" | "checkedAt" | "signedBy" | "signedAt" | "staleDocuments"
>;

export interface GateStore {
  declare(projectId: string, gate: GateDeclaration): Promise<void>;
  list(projectId: string, phase?: string): Promise<Gate[]>;
  run(projectId: string, phase?: string): Promise<GateOutcome[]>;
  /** Подписать пункт: подписные гейты закрывает человек, и запись помнит кто. */
  sign(projectId: string, phase: string, item: string, actor: string): Promise<Gate | null>;
  /** Снять подпись — например когда предмет пересмотрен. */
  unsign(projectId: string, phase: string, item: string): Promise<Gate | null>;
  close(): Promise<void>;
}

type Row = Record<string, unknown>;

function rowToGate(row: Row): Gate {
  return {
    phase: String(row["phase"]),
    article: row["article"] === null || row["article"] === undefined ? null : Number(row["article"]),
    item: String(row["item"]),
    kind: String(row["kind"]) as GateKind,
    query: row["query"] === null ? null : String(row["query"]),
    owner: row["owner"] === null ? null : String(row["owner"]),
    state: String(row["state"]) as GateState,
    violations: Number(row["violations"]),
    detail: String(row["detail"]),
    checkedAt: row["checked_at"] === null ? null : Number(row["checked_at"]),
    signedBy: row["signed_by"] === null ? null : String(row["signed_by"]),
    signedAt: row["signed_at"] === null ? null : Number(row["signed_at"]),
    staleDocuments: (row["stale"] as string[]) ?? [],
  };
}

export function createPostgresGateStore(connectionString: string, opts: { now?: () => number } = {}): GateStore {
  const now = opts.now ?? (() => Date.now());
  const pg = createPgPool(connectionString, [...SCHEMA, ...MIGRATIONS, ...SIGNATURE_SCHEMA]);

  return {
    async declare(projectId, gate) {
      await pg.query(
        `INSERT INTO project_gates(project_id, phase, item, kind, query, owner, article)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (project_id, phase, item) DO UPDATE SET
           kind = EXCLUDED.kind, query = EXCLUDED.query, owner = EXCLUDED.owner,
           article = EXCLUDED.article,
           -- подпись переживает переобъявление: её ставил человек, а не прогон
           state = CASE WHEN project_gates.signed_by IS NOT NULL THEN project_gates.state ELSE 'unknown' END,
           violations = 0, detail = '', checked_at = NULL`,
        [projectId, gate.phase, gate.item, gate.kind, gate.query, gate.owner, gate.article ?? null],
      );
    },

    async list(projectId, phase) {
      // Расхождение считаем тем же запросом: подпись держит, только пока прочитанное не менялось.
      const stale = `COALESCE((SELECT json_agg(s.path ORDER BY s.path)
          FROM project_gate_signatures s
          LEFT JOIN project_documents d
            ON d.project_id = s.project_id AND d.path = s.path
         WHERE s.project_id = g.project_id AND s.phase = g.phase AND s.item = g.item
           AND (d.path IS NULL OR d.content_hash <> s.content_hash)), '[]') AS stale`;
      const { rows } = phase
        ? await pg.query(
            `SELECT g.*, ${stale} FROM project_gates g WHERE g.project_id=$1 AND g.phase=$2 ORDER BY g.phase, g.item`,
            [projectId, phase],
          )
        : await pg.query(`SELECT g.*, ${stale} FROM project_gates g WHERE g.project_id=$1 ORDER BY g.phase, g.item`, [
            projectId,
          ]);
      return rows.map((row) => rowToGate(row as Row));
    },

    async run(projectId, phase) {
      const gates = await this.list(projectId, phase);
      const outcomes: GateOutcome[] = [];
      for (const gate of gates) {
        if (gate.kind !== "query") {
          outcomes.push({ phase: gate.phase, item: gate.item, state: gate.state, violations: 0, detail: gate.detail });
          continue;
        }
        const refusal = refuseQuery(gate.query!);
        const outcome: GateOutcome = refusal
          ? { phase: gate.phase, item: gate.item, state: "refused", violations: 0, detail: refusal }
          : await runQuery(await pg.pool(), projectId, gate);
        await pg.query(
          `UPDATE project_gates SET state=$4, violations=$5, detail=$6, checked_at=$7
            WHERE project_id=$1 AND phase=$2 AND item=$3`,
          [projectId, gate.phase, gate.item, outcome.state, outcome.violations, outcome.detail, now()],
        );
        outcomes.push(outcome);
      }
      return outcomes;
    },

    async sign(projectId, phase, item, actor) {
      const prefixes = PHASE_DOCUMENTS[phase] ?? [];
      return withPgTransaction(await pg.pool(), async (client) => {
        const { rows } = await client.query(
          `UPDATE project_gates SET state='passed', signed_by=$4, signed_at=$5, detail='', violations=0
            WHERE project_id=$1 AND phase=$2 AND item=$3 AND kind='signed' RETURNING *`,
          [projectId, phase, item, actor, now()],
        );
        if (!rows[0]) return null;

        // Подпись — снимок прочитанного: запоминаем каждый документ фазы и его содержимое.
        await client.query(`DELETE FROM project_gate_signatures WHERE project_id=$1 AND phase=$2 AND item=$3`, [
          projectId,
          phase,
          item,
        ]);
        if (prefixes.length) {
          await client.query(
            `INSERT INTO project_gate_signatures(project_id, phase, item, path, content_hash)
             SELECT project_id, $2, $3, path, content_hash FROM project_documents
              WHERE project_id=$1 AND path LIKE ANY($4::text[])`,
            [projectId, phase, item, prefixes.map((p) => `${p}%`)],
          );
        }
        return { ...rowToGate(rows[0] as Row), staleDocuments: [] };
      });
    },

    async unsign(projectId, phase, item) {
      return withPgTransaction(await pg.pool(), async (client) => {
        const { rows } = await client.query(
          `UPDATE project_gates SET state='unknown', signed_by=NULL, signed_at=NULL
            WHERE project_id=$1 AND phase=$2 AND item=$3 AND kind='signed' RETURNING *`,
          [projectId, phase, item],
        );
        if (!rows[0]) return null;
        await client.query(`DELETE FROM project_gate_signatures WHERE project_id=$1 AND phase=$2 AND item=$3`, [
          projectId,
          phase,
          item,
        ]);
        return { ...rowToGate(rows[0] as Row), staleDocuments: [] };
      });
    },

    close: () => pg.close(),
  };
}

async function runQuery(
  pool: Awaited<ReturnType<ReturnType<typeof createPgPool>["pool"]>>,
  projectId: string,
  gate: Gate,
): Promise<GateOutcome> {
  try {
    const rows = await withPgTransaction(pool, async (client) => {
      await client.query("SET TRANSACTION READ ONLY");
      const result = await client.query(gate.query!, [projectId]);
      return result.rows as Row[];
    });
    const first = rows[0];
    const detail = first ? Object.values(first).map(String).join(" · ").slice(0, 300) : "";
    return {
      phase: gate.phase,
      item: gate.item,
      state: rows.length === 0 ? "passed" : "failed",
      violations: rows.length,
      detail,
    };
  } catch (error) {
    return {
      phase: gate.phase,
      item: gate.item,
      state: "refused",
      violations: 0,
      detail: error instanceof Error ? error.message.slice(0, 300) : String(error),
    };
  }
}
