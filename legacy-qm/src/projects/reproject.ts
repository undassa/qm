/**
 * Пересборка всех проекций набора из документов, лежащих в базе.
 *
 * Живёт в продукте, а не в скрипте, потому что её зовут двое: команда синхронизации
 * (`scripts/project-sync.ts`) и маршрут записи. Пока она была скриптом, всякая правка
 * документа оставляла проекции старыми, и интерфейс показывал вчерашнее число, ничего об
 * этом не говоря.
 *
 * Порядок не произволен: каждая следующая опирается на предыдущую — решение ссылается на
 * вопрос, матрица сверяется с требованиями, перечень документов сверяет свои числа со всеми
 * остальными. Он исполняется сверху вниз и менять его без причины нельзя.
 */

import type pg from "pg";

import type { ProjectDocumentStore } from "./project-document-store.ts";
import { createPostgresArticleStore, projectArticles } from "./article-projection.ts";
import { createPostgresProofStore, projectProof, type Cell as ProofCell } from "./proof-projection.ts";
import {
  createPostgresNeedStore,
  projectNeeds,
  STORY_NEEDS_SECTION,
  STORY_REQUIREMENTS_SECTION,
} from "./need-projection.ts";
import { createPostgresSurfaceStore, projectSurface } from "./surface-projection.ts";
import { createPostgresFeatureStore, projectFeatures } from "./feature-projection.ts";
import { createPostgresRunLogStore, projectRuns } from "./run-projection.ts";
import { createPostgresQuestionStore, projectQuestions } from "./question-projection.ts";
import { createPostgresDecisionStore, projectDecisions } from "./decision-projection.ts";
import { createPostgresRiskStore, projectRisks } from "./risk-projection.ts";
import { createPostgresGlossaryStore, projectGlossary } from "./glossary-projection.ts";
import { createPostgresDataModelStore, projectDataModel } from "./data-model-projection.ts";
import { createPostgresTraceabilityStore, projectTraceability } from "./traceability-projection.ts";
import { createPostgresDocumentPlanStore, projectDocumentPlan } from "./document-plan-projection.ts";
import { createPostgresPlanStore, projectPlan } from "./plan-projection.ts";
import { createPostgresPlanStatusStore, projectPlanStatus } from "./plan-status-projection.ts";

/**
 * Раскладка набора — это КАРТА ПРОЕКТА, а не свойство платформы: у другого проекта слоты
 * называются иначе. Её место — в записи проекта рядом с репозиториями; пока она стоит здесь
 * значением по умолчанию, и всякий второй проект обязан передать своё.
 */
export interface DocumentLayout {
  constitution: string;
  glossary: string;
  documentPlan: string;
  questions: string;
  needRegister: string;
  risks: string;
  traceability: string;
  stories: string;
  features: string;
  screens: string;
  dataModel: string;
  decisions: string;
  plan: string;
  planStatus: string;
  runs: string;
  /** Требования и проверки ОБЪЯВЛЯЮТСЯ здесь; в остальных слотах они лишь упоминаются. */
  proof: readonly string[];
}

export const MYACK_LAYOUT: DocumentLayout = {
  constitution: "00-frame/constitution.md",
  glossary: "00-frame/glossary.md",
  documentPlan: "00-frame/document-plan.md",
  questions: "00-frame/questions/",
  needRegister: "10-intent/strs.md",
  risks: "10-intent/risks.md",
  traceability: "10-intent/traceability.md",
  stories: "10-intent/use-cases/",
  features: "10-intent/functional/",
  screens: "20-surface/",
  dataModel: "30-design/data-model.md",
  decisions: "30-design/decisions/",
  plan: "50-plan/",
  planStatus: "50-plan/v1/status.md",
  runs: "60-runs/",
  proof: ["10-intent/", "40-proof/"],
};

export interface ReprojectResult {
  /** По строке на проекцию — то же, что печатает команда синхронизации. */
  report: string[];
  /** Находки, которые проекция увидела, а гейты на файлах не видят. */
  findings: string[];
}

export async function reprojectAll(
  pool: pg.Pool,
  docs: ProjectDocumentStore,
  projectId: string,
  connectionString: string,
  layout: DocumentLayout = MYACK_LAYOUT,
): Promise<ReprojectResult> {
  const P = projectId;
  const report: string[] = [];
  const findings: string[] = [];

  async function cellsOf(path?: string) {
    const { rows } = path
      ? await pool.query(
          `SELECT path, block_ord, row_ord, col, raw, value FROM project_document_cells
            WHERE project_id=$1 AND path=$2 ORDER BY block_ord, row_ord, col`,
          [P, path],
        )
      : await pool.query(
          `SELECT path, block_ord, row_ord, col, raw, value FROM project_document_cells
            WHERE project_id=$1 ORDER BY path, block_ord, row_ord, col`,
          [P],
        );
    return rows.map((r: Record<string, unknown>) => ({
      path: String(r["path"]),
      blockOrd: Number(r["block_ord"]),
      row: Number(r["row_ord"]),
      col: Number(r["col"]),
      raw: String(r["raw"] ?? ""),
      value: String(r["value"] ?? ""),
    }));
  }

  async function sectionsOf(path: string) {
    const { rows } = await pool.query(
      `SELECT ord, title FROM project_document_sections WHERE project_id=$1 AND path=$2 ORDER BY ord`,
      [P, path],
    );
    return rows.map((r: Record<string, unknown>) => ({ ord: Number(r["ord"]), title: String(r["title"]) }));
  }

  /** Заголовок раздела, под которым стоит блок: ближайший заголовок выше него. */
  function titleOfBlock(sections: readonly { ord: number; title: string }[]) {
    return (blockOrd: number): string => {
      let title = "";
      for (const s of sections) {
        if (s.ord > blockOrd) break;
        title = s.title;
      }
      return title;
    };
  }

  async function structureOf(paths: readonly string[], opts: { bodies?: boolean } = {}) {
    const titles = new Map<string, string>();
    const sectionTitles = new Map<string, string[]>();
    const fields = new Map<string, Map<string, string>>();
    const fieldsRaw = new Map<string, Map<string, string>>();
    const bodies = new Map<string, Map<string, string>>();
    for (const path of paths) {
      const list = await docs.sections(P, path);
      titles.set(path, list[0]?.title ?? path);
      sectionTitles.set(
        path,
        list.map((s) => s.title),
      );
      const fs = await docs.fields(P, path);
      fields.set(path, new Map(fs.map((f) => [f.name, f.value])));
      fieldsRaw.set(path, new Map(fs.map((f) => [f.name, f.valueRaw])));
      if (opts.bodies) {
        const map = new Map<string, string>();
        for (const s of list) {
          const got = await docs.section(P, path, s.anchor);
          if (got) map.set(s.title, got.body);
        }
        bodies.set(path, map);
      }
    }
    return { titles, sectionTitles, fields, fieldsRaw, bodies };
  }

  const allPaths = (await docs.list(P)).map((d) => d.path);
  const under = (...prefixes: readonly string[]) => allPaths.filter((p) => prefixes.some((x) => p.startsWith(x)));

  // 1. Статьи конституции — считают ссылки по всему набору, поэтому идут первыми.
  {
    const contents = await Promise.all(
      allPaths.map(async (path) => ({ path, content: (await docs.get(P, path))?.content ?? "" })),
    );
    const constitution = contents.find((d) => d.path === layout.constitution) ?? null;
    if (!constitution) findings.push(`конституция не найдена по адресу ${layout.constitution} — статьи не проецируются`);
    const store = createPostgresArticleStore(connectionString);
    const projection = projectArticles({ constitution, documents: contents });
    await store.replace(P, projection);
    const dangling = await store.danglingReferences(P);
    report.push(`статьи: ${projection.articles.length} · ссылок ${projection.references.length} · в никуда ${dangling.length}`);
    await store.close();
  }

  // 2. Требования и проверки — на них опираются почти все остальные.
  {
    const cells = (await cellsOf()).filter((c) => layout.proof.some((x) => c.path.startsWith(x))) as ProofCell[];
    const store = createPostgresProofStore(connectionString);
    await store.replace(P, projectProof(cells));
    const counts = await store.counts(P);
    const uncovered = await store.uncoveredRequirements(P);
    // `counts.requirements` — ВСЕ требования, вместе с нефункциональными; FR получается
    // вычитанием. Сверять с гейтом проекта надо именно FR: `matrix.mjs` печатает его.
    report.push(
      `доказательство: FR ${counts.requirements - counts.nfr} · NFR ${counts.nfr} · проверок ${counts.checks} · покрыто ${counts.covered} · без проверки ${uncovered.length}`,
    );
    await store.close();
  }

  // 3. Потребности: реестр таблицей плюс раздел внутри каждой истории.
  {
    const cells = await cellsOf(layout.needRegister);
    const themeOfBlock = titleOfBlock(await sectionsOf(layout.needRegister));
    const storyPaths = under(layout.stories);
    const needBodies = new Map<string, string>();
    const reqBodies = new Map<string, string>();
    for (const path of storyPaths) {
      const list = await docs.sections(P, path);
      for (const [title, into] of [
        [STORY_NEEDS_SECTION, needBodies],
        [STORY_REQUIREMENTS_SECTION, reqBodies],
      ] as const) {
        const found = list.find((s) => s.title === title);
        if (found) into.set(path, (await docs.section(P, path, found.anchor))?.body ?? "");
      }
    }
    const store = createPostgresNeedStore(connectionString);
    await store.replace(
      P,
      projectNeeds({
        registerPath: layout.needRegister,
        cells,
        themeOfBlock,
        storyPaths,
        needsSectionOf: (p) => needBodies.get(p) ?? "",
        requirementsSectionOf: (p) => reqBodies.get(p) ?? "",
      }),
    );
    const counts = await store.counts(P);
    const orphan = await store.withoutStory(P);
    report.push(`потребности: ${counts.needs} · связей ${counts.links} · без истории ${orphan.length}`);
    await store.close();
  }

  // 4. Поверхность: экраны и истории с их требованиями.
  {
    const paths = under(layout.stories, layout.screens, layout.plan);
    const s = await structureOf(paths, { bodies: true });
    const store = createPostgresSurfaceStore(connectionString);
    await store.replace(
      P,
      projectSurface({
        paths,
        titleOf: (p) => s.titles.get(p) ?? p,
        fieldsOf: (p) => s.fields.get(p) ?? new Map(),
        sectionTitlesOf: (p) => s.sectionTitles.get(p) ?? [],
        sectionBodyOf: (p, title) => s.bodies.get(p)?.get(title) ?? "",
      }),
    );
    const counts = await store.counts(P);
    const dangling = await store.danglingScreens(P);
    const orphans = await store.orphanScreens(P);
    report.push(
      `поверхность: экранов ${counts.screens} · историй ${counts.stories} · ссылок в никуда ${dangling.length} · экранов-сирот ${orphans.length}`,
    );
    await store.close();
  }

  // 5. Области и выжимки прогонов — один разбор на два хранилища.
  {
    const paths = under(layout.features, layout.runs);
    const s = await structureOf(paths);
    const src = {
      paths,
      titleOf: (p: string) => s.titles.get(p) ?? p,
      sectionTitlesOf: (p: string) => s.sectionTitles.get(p) ?? [],
    };
    const features = createPostgresFeatureStore(connectionString);
    await features.replace(P, projectFeatures(src));
    const fc = await features.counts(P);
    const unclaimed = await features.unclaimedStories(P);
    report.push(`области: ${fc.features} · связей ${fc.links} · историй без области ${unclaimed.length}`);
    await features.close();

    const runs = createPostgresRunLogStore(connectionString);
    await runs.replace(P, projectRuns(src));
    const rc = await runs.counts(P);
    const noRun = await runs.closedWithoutRun(P);
    report.push(`выжимки: ${rc.total} · задач ${rc.tasks} · закрыто без выжимки ${noRun.length}`);
    await runs.close();
  }

  // 6. Вопросы — до решений: решение закрывает вопрос и должно найти его в базе.
  {
    const paths = under(layout.questions);
    const s = await structureOf(paths);
    const store = createPostgresQuestionStore(connectionString);
    await store.replace(
      P,
      projectQuestions({
        paths,
        titleOf: (p) => s.titles.get(p) ?? p,
        fieldsOf: (p) => s.fields.get(p) ?? new Map(),
        sectionTitlesOf: (p) => s.sectionTitles.get(p) ?? [],
      }),
    );
    const counts = await store.counts(P);
    const gap = await store.decidedWithoutAnswer(P);
    report.push(
      `вопросы: ${counts.total} · открытых ${counts.open} · решено ${counts.decided} · закрыто ${counts.closed} · без раздела «Ответ» ${gap.length}`,
    );
    await store.close();
  }

  // 7. Решения.
  {
    const paths = under(layout.decisions);
    // Тела разделов нужны: контекст, решение, последствия и отвергнутые варианты живут
    // разделами, а не полями шапки.
    const s = await structureOf(paths, { bodies: true });
    const store = createPostgresDecisionStore(connectionString);
    const projection = projectDecisions({
      paths,
      titleOf: (p) => s.titles.get(p) ?? p,
      fieldsOf: (p) => s.fields.get(p) ?? new Map(),
      sectionTitlesOf: (p) => s.sectionTitles.get(p) ?? [],
      sectionBodyOf: (p, title) => s.bodies.get(p)?.get(title) ?? "",
    });
    await store.replace(P, projection);
    const counts = await store.counts(P);
    const openButClosed = await store.closedButOpenQuestions(P);
    const noAlt = projection.decisions.filter(
      (d) => !projection.alternatives.some((a) => a.decisionId === d.id),
    ).length;
    const noConseq = projection.decisions.filter((d) => d.consequences === "").length;
    report.push(
      `решения: ${counts.total} · принятых ${counts.accepted} · заменённых ${counts.superseded} · связей ${projection.links.length} · «закрыто решением, а вопрос открыт» ${openButClosed.length}`,
    );
    report.push(
      `   отвергнутых вариантов ${projection.alternatives.length} · решений без единого варианта ${noAlt} · без записанных последствий ${noConseq}`,
    );
    await store.close();
  }

  // 8. Риски и глоссарий — обе проекции читают таблицу своего документа.
  {
    const riskStore = createPostgresRiskStore(connectionString);
    await riskStore.replace(
      P,
      projectRisks({
        path: layout.risks,
        cells: await cellsOf(layout.risks),
        sectionOfBlock: titleOfBlock(await sectionsOf(layout.risks)),
      }),
    );
    const rc = await riskStore.counts(P);
    const noOwner = await riskStore.openWithoutOwner(P);
    report.push(`риски: ${rc.total} · открытых ${rc.open} · открыт без владельца ${noOwner.length}`);
    await riskStore.close();

    const glossaryStore = createPostgresGlossaryStore(connectionString);
    await glossaryStore.replace(
      P,
      projectGlossary({
        path: layout.glossary,
        cells: await cellsOf(layout.glossary),
        areaOfBlock: titleOfBlock(await sectionsOf(layout.glossary)),
      }),
    );
    const gc = await glossaryStore.counts(P);
    const unused = await glossaryStore.unused(P);
    report.push(`глоссарий: терминов ${gc.terms} · областей ${gc.areas} · нигде не употреблён ${unused.length}`);
    await glossaryStore.close();
  }

  // 9. Модель данных.
  {
    const { rows } = await pool.query(
      `SELECT ord, kind, raw FROM project_document_blocks WHERE project_id=$1 AND path=$2 ORDER BY ord`,
      [P, layout.dataModel],
    );
    const blocks = rows.map((r: Record<string, unknown>) => ({
      ord: Number(r["ord"]),
      kind: String(r["kind"]),
      raw: String(r["raw"] ?? ""),
    }));
    const store = createPostgresDataModelStore(connectionString);
    await store.replace(
      P,
      projectDataModel({
        path: layout.dataModel,
        cells: await cellsOf(layout.dataModel),
        sections: await sectionsOf(layout.dataModel),
        blocks,
      }),
    );
    const counts = await store.counts(P);
    const noColumns = await store.withoutColumns(P);
    report.push(
      `модель данных: таблиц ${counts.tables} · миграций ${counts.migrations} · описано ${counts.described} · без колонок ${noColumns.length}`,
    );
    await store.close();
  }

  // 10. Матрица трассируемости — сверяется с требованиями, поэтому после шага 2.
  {
    const store = createPostgresTraceabilityStore(connectionString);
    await store.replace(P, projectTraceability(layout.traceability, await cellsOf(layout.traceability)));
    const drift = await store.drift(P);
    const missing = await store.missingAreas(P);
    report.push(`матрица: расхождений заявленного с фактом ${drift.length} · подсистем мимо матрицы ${missing.length}`);
    for (const d of drift) {
      findings.push(
        `матрица · ${d.area}: заявлено FR ${d.claimedRequirements}/факт ${d.actualRequirements}, описано ${d.claimedDescribed}/факт ${d.actualDescribed}`,
      );
    }
    await store.close();
  }

  // 11. План: этапы и задачи.
  {
    const paths = under(layout.plan);
    const s = await structureOf(paths);
    const store = createPostgresPlanStore(connectionString);
    await store.replace(
      P,
      projectPlan({
        paths,
        titleOf: (p) => s.titles.get(p) ?? "",
        fieldsOf: (p) => s.fieldsRaw.get(p) ?? new Map(),
      }),
    );
    const counts = await store.counts(P);
    const ready = await store.readyTasks(P);
    report.push(`план: ${JSON.stringify(counts)} · готовых к взятию ${ready.length}`);
    await store.close();
  }

  // 12. Доска состояния — после плана: она о задачах плана.
  {
    const store = createPostgresPlanStatusStore(connectionString);
    await store.replace(
      P,
      projectPlanStatus({
        path: layout.planStatus,
        cells: await cellsOf(layout.planStatus),
        milestoneOfBlock: titleOfBlock(await sectionsOf(layout.planStatus)),
      }),
    );
    const counts = await store.counts(P);
    const disagree = await store.disagreements(P);
    const missing = await store.missingFromBoard(P);
    report.push(
      `доска: ${counts.total} · закрыто ${counts.closed} · доска и задача расходятся ${disagree.length} · нет на доске ${missing.length}`,
    );
    for (const d of disagree) findings.push(`доска · ${d.taskId}: доска «${d.boardState}», задача «${d.taskState}»`);
    await store.close();
  }

  // 13. Перечень документов — последним: его числа сверяются со всеми проекциями выше.
  {
    const store = createPostgresDocumentPlanStore(connectionString);
    await store.replace(
      P,
      projectDocumentPlan({
        path: layout.documentPlan,
        cells: await cellsOf(layout.documentPlan),
        levelOfBlock: titleOfBlock(await sectionsOf(layout.documentPlan)),
      }),
    );
    const unresolved = await store.unresolved(P);
    const wrong = await store.presentButDeclaredAbsent(P);
    report.push(
      `перечень документов: имя без документа ${unresolved.length} · объявлено «нет», а документ есть ${wrong.length}`,
    );
    for (const u of unresolved) findings.push(`перечень · «${u.name}» не отзывается ни одним документом`);

    // Утверждение о числе против того, что посчитано проекциями. Это и есть то, ради чего
    // набор переехал в базу: счёт, живущий прозой рядом со своим предметом, протухает молча.
    const actual: Record<string, string> = {
      articles: `SELECT count(*) FROM project_articles WHERE project_id=$1`,
      needs: `SELECT count(*) FROM project_needs WHERE project_id=$1`,
      requirements: `SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='FR'`,
      areas: `SELECT count(DISTINCT area) FROM project_requirements WHERE project_id=$1 AND kind='FR'`,
      nfr: `SELECT count(*) FROM project_requirements WHERE project_id=$1 AND kind='NFR'`,
      screens: `SELECT count(*) FROM project_screens WHERE project_id=$1`,
      checks: `SELECT count(*) FROM project_checks WHERE project_id=$1`,
      decisionFiles: `SELECT count(*) FROM project_documents WHERE project_id=$1 AND path LIKE '30-design/decisions/%'`,
    };
    let drift = 0;
    for (const c of await store.counts(P)) {
      const sql = actual[c.subject];
      if (!sql) continue;
      const got = Number((await pool.query(sql, [P])).rows[0].count);
      if (got !== c.claimed) {
        drift += 1;
        findings.push(`счёт · ${c.name} · ${c.subject}: заявлено ${c.claimed}, факт ${got}`);
      }
    }
    report.push(`   утверждений о числе, разошедшихся с фактом: ${drift}`);
    await store.close();
  }

  return { report, findings };
}
