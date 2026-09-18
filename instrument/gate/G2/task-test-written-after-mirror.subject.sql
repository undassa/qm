SELECT c.task_id
  FROM project_task_check c
  JOIN project_plan_tasks t ON t.project_id = c.project_id AND t.id = c.task_id AND t.kind = 'dev' AND t.state <> 'closed'
  JOIN red_task r ON r.project_id = c.project_id AND r.parent_task = c.task_id
  JOIN project_plan_tasks rt ON rt.project_id = r.project_id AND rt.id = r.id AND rt.state = 'closed'
 WHERE c.project_id = $1 AND c.said_as = 'доказательство' AND c.check_id ~ '^[a-z][a-z0-9]*_[a-z0-9_]+$'
