SELECT t.id || ' — ' || t.state AS detail FROM project_plan_tasks t WHERE t.project_id = $1 AND t.kind <> 'red' AND t.state <> 'closed' ORDER BY t.id
