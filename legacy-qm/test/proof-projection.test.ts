import test, { before } from "node:test";
import assert from "node:assert/strict";
import { createPostgresProofStore, projectProof, type Cell } from "../src/projects/proof-projection.ts";

const URL = process.env.DATABASE_URL;
const pgSkip = URL ? false : "set DATABASE_URL (a Postgres) to run the Postgres proof-projection tests";
const PROJECT = "proof-projection-test";

before(async () => {
  if (!URL) return;
  const pg = (await import("pg")).default;
  const p = new pg.Pool({ connectionString: URL });
  for (const table of ["project_checks", "project_requirements"]) {
    await p.query(`DELETE FROM ${table} WHERE project_id=$1`, [PROJECT]).catch(() => undefined);
  }
  await p.end();
});

let ord = 0;
function row(path: string, block: number, cells: string[]): Cell[] {
  const at = ord++;
  return cells.map((raw, col) => ({
    path,
    blockOrd: block,
    row: at,
    col,
    raw,
    value: raw.replace(/`/g, "").trim(),
  }));
}

const SRS = "10-intent/srs.md";
const TC = "40-proof/test-cases.md";

const CELLS: Cell[] = [
  ...row(SRS, 1, [" FR-EXT-01 ✓уд ", " контракт описан в openapi "]),
  ...row(SRS, 1, [" FR-EXT-02 ", " каждая операция доступна программно "]),
  ...row(SRS, 1, [" NFR-18 ", " .sqlx пересобирается в CI "]),
  ...row(TC, 5, [" `TC-EXT-01` ", " `FR-EXT-01` ", " дано → когда → тогда "]),
  ...row(TC, 5, [" `TC-EXT-01b` ", " `FR-EXT-01` ", " суффиксная проверка того же требования "]),
  ...row(TC, 5, [" `TC-GHOST-01` ", " `FR-NOPE-99` ", " ссылается на необъявленное "]),
];

test("суффиксный идентификатор опознаётся — иначе связь теряется молча", () => {
  const p = projectProof(CELLS);
  assert.deepEqual(
    p.checks.map((c) => c.id),
    ["TC-EXT-01", "TC-EXT-01b", "TC-GHOST-01"],
    "TC-EXT-01b кончается буквой, и образец обязан её принять",
  );
});

test("требование берётся из первой ячейки строки, признак удовлетворения — из пометки", () => {
  const p = projectProof(CELLS);
  assert.deepEqual(
    p.requirements.map((r) => [r.id, r.kind, r.area, r.satisfied]),
    [
      ["FR-EXT-01", "FR", "EXT", true],
      ["FR-EXT-02", "FR", "EXT", false],
      ["NFR-18", "NFR", "", false],
    ],
  );
  assert.equal(p.requirements[0]!.text, "контракт описан в openapi");
});

test("упоминание в прозе не объявляет сущность", () => {
  const prose: Cell[] = [...row(SRS, 9, ["**Номер `FR-STP-15`, а не `11`:** `FR-STP-11…13` удалены", " пояснение "])];
  const p = projectProof(prose);
  assert.deepEqual(p.requirements, [], "строка, начинающаяся не с идентификатора, не объявление");
});

test("требование без единой проверки называется, а призрачная ссылка — отдельно", { skip: pgSkip }, async (t) => {
  const store = createPostgresProofStore(URL!);
  t.after(() => store.close());
  await store.replace(PROJECT, projectProof(CELLS));

  assert.deepEqual(await store.counts(PROJECT), { requirements: 3, nfr: 1, checks: 3, covered: 2 });
  assert.deepEqual(
    await store.uncoveredRequirements(PROJECT),
    ["FR-EXT-02"],
    "FR-EXT-01 закрыт двумя проверками, FR-EXT-02 — ни одной",
  );
  assert.deepEqual(await store.undeclaredReferences(PROJECT), [
    { checkId: "TC-GHOST-01", requirementId: "FR-NOPE-99" },
  ]);
});

test("повторная проекция замещает прежнюю, а не накапливает", { skip: pgSkip }, async (t) => {
  const store = createPostgresProofStore(URL!);
  t.after(() => store.close());
  await store.replace(PROJECT, projectProof(CELLS));
  await store.replace(PROJECT, projectProof(CELLS));
  assert.equal((await store.counts(PROJECT)).checks, 3, "проекция сносится и строится заново");
});

test("у нефункционального требования области нет: NFR-15 — номер, а не «область 15»", () => {
  const { requirements } = projectProof([
    { path: "10-intent/srs.md", blockOrd: 1, row: 1, col: 0, raw: "`NFR-15`", value: "NFR-15" },
    { path: "10-intent/srs.md", blockOrd: 1, row: 1, col: 1, raw: "текст", value: "Ответ ≤200 мс" },
    { path: "10-intent/srs.md", blockOrd: 1, row: 2, col: 0, raw: "`FR-CFG-01`", value: "FR-CFG-01" },
    { path: "10-intent/srs.md", blockOrd: 1, row: 2, col: 1, raw: "текст", value: "Экран имеет YAML" },
  ]);
  const nfr = requirements.find((r) => r.id === "NFR-15");
  const fr = requirements.find((r) => r.id === "FR-CFG-01");
  assert.equal(nfr?.area, "", "иначе в таблице стоит бессмысленная «область 15»");
  assert.equal(fr?.area, "CFG", "у функционального область остаётся");
});

test("объявление сильнее цитаты, в каком бы порядке ни пришли ячейки", () => {
  const { checks } = projectProof([
    { path: "10-intent/use-cases/US-PAG-19.md", blockOrd: 9, row: 1, col: 0, raw: " `TC-BUS-01` ", value: "TC-BUS-01" },
    { path: "10-intent/use-cases/US-PAG-19.md", blockOrd: 9, row: 1, col: 1, raw: " упомянута ", value: "упомянута" },
    { path: "40-proof/test-cases.md", blockOrd: 1, row: 1, col: 0, raw: " `TC-BUS-01` ", value: "TC-BUS-01" },
    { path: "40-proof/test-cases.md", blockOrd: 1, row: 1, col: 1, raw: " `NFR-06` ", value: "NFR-06" },
    { path: "40-proof/test-cases.md", blockOrd: 1, row: 1, col: 2, raw: " что проверяет ", value: "что проверяет" },
    { path: "50-plan/v1/M0/M0-T11a.md", blockOrd: 46, row: 2, col: 0, raw: " `TC-BUS-01` ", value: "TC-BUS-01" },
    { path: "50-plan/v1/M0/M0-T11a.md", blockOrd: 46, row: 2, col: 1, raw: " сделано ", value: "сделано" },
  ]);
  assert.equal(checks.length, 1, "одна проверка, а не две записи об одной");
  assert.equal(
    checks[0]!.requirementId,
    "NFR-06",
    "строка задачи плана не должна была обнулить требование, объявленное в наборе проверок",
  );
  assert.equal(checks[0]!.path, "40-proof/test-cases.md");
});
