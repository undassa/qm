UPDATE project_plan_tasks SET state='not_started' WHERE project_id=$1 AND state='closed'
