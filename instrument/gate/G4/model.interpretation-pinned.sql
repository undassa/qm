WITH RECURSIVE крейты AS (
  SELECT c.name AS крейт, m.name AS манифест
    FROM code_fact m JOIN code_fact c ON c.project_id = m.project_id AND c.kind = 'crate'
         AND c.place = m.name
   WHERE m.project_id = $1 AND m.kind = 'crate-manifest'),
рёбра AS (
  SELECT k.крейт AS от, substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') AS к
    FROM code_fact f JOIN крейты k ON k.манифест = substring(f.name from '^(.*):\d+$')
   WHERE f.project_id = $1 AND f.kind = 'direct-dep'),
достижимо AS (
  SELECT от AS читатель, к AS крейт FROM рёбра WHERE от IN ('tot-index', 'tot-page')
  UNION
  SELECT д.читатель, р.к FROM достижимо д JOIN рёбра р ON р.от = д.крейт)
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какие крейты есть — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «crate» ' || fact_gap($1, 'crate') || ': как зовутся крейты — неизвестно' WHERE NOT fact_fresh($1, 'crate')
UNION ALL
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': достаёт ли читатель модели до провайдера — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT 'датчик «judgement-provenance» ' || fact_gap($1, 'judgement-provenance') || ': несёт ли суждение провенанс — неизвестно' WHERE NOT fact_fresh($1, 'judgement-provenance')
UNION ALL
SELECT DISTINCT д.читатель || ' достаёт до tot-llm по зависимостям: чтение модели может позвать LLM' FROM достижимо д WHERE д.крейт = 'tot-llm'
UNION ALL
SELECT 'tot_index::Judgement без поля facts: Vec<FactId> — суждение не несёт провенанса' WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'judgement-provenance')
ORDER BY 1
