-- «В ДЕРЕВЕ НЕТ» ПРАВИЛО СКАЗАТЬ НЕ МОЖЕТ. Имена тестов берутся у датчика
-- `test-name`, а он читает по объявленному перечню: у `tot-ade` это
-- `crates/**/*.rs`, и тест вне `crates` неотличим здесь от ненаписанного.
-- Досягаемость обходчика в SQL не выражается, поэтому обе возможности
-- называются читателю, а правило утверждает только измеренное.
WITH строка AS (
  SELECT t.id AS task_id, l.line
    FROM project_plan_tasks t
    JOIN project_documents d ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
   CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') AS l(line)
   WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state <> 'closed' AND l.line LIKE '|%'),
проверка AS (
  SELECT DISTINCT c.task_id, c.check_id, r.id AS зеркало
    FROM project_task_check c
    JOIN строка s ON s.task_id = c.task_id
     AND s.line ~ ('^\|\s*`?' || c.check_id || '`?\s*\|') AND s.line !~* 'закрывает\s+`?[MmVv][0-9]+-[Tt][0-9A-Za-z]+'
    JOIN red_task r ON r.project_id = c.project_id AND r.parent_task = c.task_id
    JOIN project_plan_tasks rt ON rt.project_id = r.project_id AND rt.id = r.id AND rt.state = 'closed'
   WHERE c.project_id = $1 AND c.said_as = 'доказательство'
     AND c.check_id ~ '^[a-z][a-z0-9]*_[a-z0-9_]+$')
SELECT 'датчик «test-name» ' || fact_gap($1, 'test-name')
       || ': написаны ли тесты задач с закрытым зеркалом — неизвестно, и это не зелёное' AS detail
 WHERE NOT fact_fresh($1, 'test-name') AND EXISTS (SELECT 1 FROM проверка)
UNION ALL
SELECT п.task_id || ' — зеркало ' || п.зеркало || ' закрыто, а теста ' || п.check_id
       || ' среди прочитанного датчиком нет. Нет его в дереве или датчик туда не дошёл'
       || ' — покажет `sensor-specs`'
  FROM проверка п
 WHERE fact_fresh($1, 'test-name')
   AND NOT EXISTS (SELECT 1 FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'test-name' AND f.name = п.check_id)
 ORDER BY 1
