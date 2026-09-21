SELECT t.id || ' — не наблюдалась красной: ' || c.check_id
  FROM project_plan_tasks t
  JOIN project_task_check c ON c.project_id = t.project_id AND c.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'red' AND t.state = 'closed'
   -- ПАДЕНИЕ СЧИТАЕТСЯ ТОЛЬКО С ЧИСТОГО ДЕРЕВА. Прогон на дереве с чужой
   -- правкой роняет что угодно, и такое падение доказывало бы красную фазу
   -- чужой поломкой, а не тем, что проверка была написана до кода.
   AND NOT EXISTS (SELECT 1 FROM test_run r
                    WHERE r.project_id = $1 AND NOT r.dirty
                      AND r.verdict = 'failed' AND r.check_name = c.check_id)
