SELECT tp.task_id || ' — ' || tp.state AS detail
  FROM task_phase tp
 WHERE tp.project_id = $1 AND tp.state <> 'closed' AND tp.open
 ORDER BY tp.task_id
