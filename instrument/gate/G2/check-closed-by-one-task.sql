WITH строка AS (
  SELECT t.id AS task_id, t.state, l.line
    FROM project_plan_tasks t
    JOIN project_documents d ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
   CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') AS l(line)
   WHERE t.project_id = $1 AND t.kind = 'dev' AND l.line LIKE '|%'),
закрывает AS (
  SELECT DISTINCT c.check_id, c.task_id, s.state
    FROM project_task_check c
    JOIN строка s ON s.task_id = c.task_id
     AND s.line ~ ('^\|\s*`?' || c.check_id || '`?\s*\|') AND s.line !~* 'закрывает\s+`?[MmVv][0-9]+-[Tt][0-9A-Za-z]+'
   WHERE c.project_id = $1)
SELECT k.id || ' — закрывают сразу ' || string_agg(z.task_id, ', ' ORDER BY z.task_id)
       || ': у сценария одна задача, остальные пишут «закрывает <задача>»' AS detail
  FROM project_checks k JOIN закрывает z ON z.check_id = k.id
 WHERE k.project_id = $1
 GROUP BY k.id HAVING count(*) > 1 AND bool_or(z.state <> 'closed')
UNION ALL
SELECT k.id || ' — не закрывает ни одна задача' FROM project_checks k
 WHERE k.project_id = $1 AND NOT EXISTS (SELECT 1 FROM закрывает z WHERE z.check_id = k.id)
 ORDER BY 1
