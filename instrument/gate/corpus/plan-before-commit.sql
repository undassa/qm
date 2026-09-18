SELECT t.id || ' — закрыта, а плана до закрытия не записано: ' || CASE
         WHEN coalesce(ts.closed_at, 0) = 0
           THEN 'когда закрыта — не подано, и раньше ли самого правила, судить нечем. Время закрытия знает коммит: подайте `at` в `task-state-push` либо объявите датчик `task-trailers`'
         WHEN NOT EXISTS (SELECT 1 FROM task_plan x
                           WHERE x.project_id = t.project_id AND x.task_id = t.id)
           THEN 'плана нет вовсе'
         ELSE 'план записан позже закрытия — это пересказ сделанного' END AS detail
  FROM project_plan_tasks t
  LEFT JOIN task_state ts
    ON ts.project_id = t.project_id AND ts.task_id = t.id AND ts.state = 'closed'
 WHERE t.project_id = $1 AND t.state = 'closed'
   AND coalesce(nullif(ts.closed_at, 0), ts.seen_at, 9223372036854775807) >= $2
   AND NOT EXISTS (SELECT 1 FROM task_plan x
                    WHERE x.project_id = t.project_id AND x.task_id = t.id
                      AND x.at <= coalesce(nullif(ts.closed_at, 0), ts.seen_at, x.at))
   AND true
 ORDER BY 1
