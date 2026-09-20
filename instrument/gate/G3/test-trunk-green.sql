WITH best AS (SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty)
SELECT 'упала на стволе: ' || r.check_name
  FROM test_run r, best
 WHERE r.project_id = $1 AND NOT r.dirty AND r.at = best.at AND r.verdict = 'failed'
   AND NOT EXISTS (SELECT 1 FROM task_ready_item i
                     JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                    WHERE i.project_id = $1 AND i.check_id = r.check_name AND t.state <> 'closed')
