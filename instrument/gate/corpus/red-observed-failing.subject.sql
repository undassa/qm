SELECT t.id FROM project_plan_tasks t
 WHERE t.project_id = $1 AND t.kind = 'red' AND t.state = 'closed'
   AND EXISTS (SELECT 1 FROM project_task_check c WHERE c.project_id = t.project_id AND c.task_id = t.id)
 LIMIT 1
