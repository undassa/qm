SELECT l.task_id FROM project_task_tree_leaf l
  JOIN project_plan_tasks t ON t.project_id = l.project_id AND t.id = l.task_id AND t.state <> 'closed'
 WHERE l.project_id = $1 AND l.op <> ''
