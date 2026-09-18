WITH образец AS (
  SELECT k.name AS вид, coalesce(
           (SELECT string_agg('(?:' || regexp_replace(s.value, '^\^|\$$', '', 'g') || ')', '|') FROM scheme($1) s WHERE s.role = 'id.' || k.name),
           regexp_replace(k.spec->>'id', '^\^|\$$', '', 'g')) AS p
    FROM kind_layout k WHERE k.spec->>'id' IS NOT NULL),
различимый AS (
  SELECT вид, p FROM образец
   WHERE NOT EXISTS (SELECT 1 FROM unnest(ARRAY['M1','v1','abc','a:b','src/main.rs','12','foo_bar','fr_01_x','m1_t2_x','abc-def','G1']) w
                      WHERE w ~ ('^(?:' || p || ')$'))),
текст AS (
  SELECT t.id AS task_id, l.line AS content
    FROM project_plan_tasks t
    JOIN project_documents d ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
   CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') AS l(line)
   WHERE t.project_id = $1 AND t.state <> 'closed' AND l.line ~ '[A-Z][^\n]*[0-9]'
     AND l.line !~* 'удал|отмен|снят|прежн|историч|устар|не существ'),
имя AS (
  SELECT DISTINCT т.task_id, р.вид, m[1] AS name
    FROM текст т CROSS JOIN различимый р
    CROSS JOIN LATERAL regexp_matches(т.content, '(?:^|[^A-Za-z0-9_-])(' || р.p || ')(?![A-Za-z0-9_-])', 'g') m
   WHERE m[1] ~ '[A-Z]' AND m[1] ~ '[0-9]')
SELECT и.task_id || ' — ' || и.вид || ' ' || и.name || ' в наборе нет' AS detail
  FROM имя и
 WHERE NOT EXISTS (SELECT 1 FROM entity_row e WHERE e.project_id = $1 AND lower(e.id) = lower(и.name))
   AND NOT EXISTS (SELECT 1 FROM project_plan_tasks x WHERE x.project_id = $1 AND lower(x.id) = lower(и.name))
   AND NOT EXISTS (SELECT 1 FROM project_risks x WHERE x.project_id = $1 AND lower(x.id) = lower(и.name))
 ORDER BY 1
