-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
WITH поле AS (
  SELECT t.id AS task_id, f.value_raw AS v
    FROM project_plan_tasks t
    JOIN project_document_fields f
      ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name
     AND f.name IN (SELECT s.value FROM scheme($1) s WHERE s.role = 'field.task-contract-ops')
   WHERE t.project_id = $1 AND t.state <> 'closed' AND NOT EXISTS (SELECT 1 FROM task_scope sc WHERE sc.project_id = t.project_id AND sc.task_id = t.id AND sc.scope IN ('next','later'))),
протокол AS (
  SELECT s.value AS где, d.content
    FROM scheme($1) s
    JOIN project_documents d ON d.project_id = $1 AND d.entity_kind = split_part(s.value, ':', 1)
                            AND d.entity_name = split_part(s.value, ':', 2)
   WHERE s.role = 'doc.protocol')
SELECT DISTINCT п.task_id || ' — операции ' || m[1] || ' нет ни в одном документе протокола набора' AS detail
  FROM поле п
 CROSS JOIN LATERAL regexp_matches(п.v, '`([A-Z][A-Za-z0-9]+)`', 'g') m
 WHERE EXISTS (SELECT 1 FROM протокол)
   AND NOT EXISTS (SELECT 1 FROM протокол пр WHERE пр.content ~ ('(^|[^A-Za-z0-9_])' || m[1] || '([^A-Za-z0-9_]|$)'))
 ORDER BY 1
