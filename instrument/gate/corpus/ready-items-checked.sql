SELECT t.id || ' — пункт приёмки не проверен: ' || r.text
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done
