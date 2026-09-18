SELECT id FROM project_plan_tasks WHERE project_id = $1 AND state <> 'closed'
