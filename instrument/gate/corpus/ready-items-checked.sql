SELECT t.id || ' — пункт приёмки не проверен: ' || r.text
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done
   AND r.check_id <> ''
   AND NOT EXISTS (SELECT 1 FROM code_fact c
                    WHERE c.project_id = t.project_id
                      AND ((c.kind = 'test-name' AND c.name = r.check_id)
                        OR (c.kind = 'written-tc' AND position('fn ' || r.check_id || '(' in c.detail) > 0)))
UNION ALL
SELECT t.id || ' — пункт приёмки без названной проверки: ' || r.text
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done AND r.check_id = ''
