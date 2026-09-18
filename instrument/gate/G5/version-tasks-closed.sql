SELECT id || ' — ' || left(title, 50) AS detail FROM project_plan_tasks WHERE project_id = $1 AND state <> 'closed' ORDER BY id
