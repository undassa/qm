SELECT t.id FROM project_plan_tasks t
  JOIN task_requirement tr ON tr.project_id = t.project_id AND tr.task_id = t.id
 WHERE t.project_id = $1 AND t.state <> 'closed' AND t.kind = 'dev'
