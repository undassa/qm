UPDATE project_plan_tasks SET closing_commit='' WHERE project_id=$1 AND state='closed'
