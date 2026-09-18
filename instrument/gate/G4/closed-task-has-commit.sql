SELECT id FROM project_plan_tasks WHERE project_id=$1 AND state='closed' AND (closing_commit IS NULL OR closing_commit='')
