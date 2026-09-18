UPDATE project_plan_tasks SET state='not_started' WHERE project_id=$1 AND kind<>'red' AND id=(SELECT min(id) FROM project_plan_tasks WHERE project_id=$1 AND kind<>'red' AND state='closed')
