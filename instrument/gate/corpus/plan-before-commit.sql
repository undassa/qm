SELECT t.id || ' — закрыта, а плана не записано: чем её делали, не объяснить' AS detail
  FROM project_plan_tasks t
  LEFT JOIN task_state ts
    ON ts.project_id = t.project_id AND ts.task_id = t.id AND ts.state = 'closed'
 WHERE t.project_id = $1 AND t.state = 'closed'
   AND coalesce(nullif(ts.closed_at, 0), ts.seen_at, 9223372036854775807) >= $2
   AND NOT EXISTS (SELECT 1 FROM task_plan x
                    WHERE x.project_id = t.project_id AND x.task_id = t.id)
   AND true
 ORDER BY 1
