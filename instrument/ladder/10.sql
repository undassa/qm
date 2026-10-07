-- Незакрытые задачи ОТКРЫТОЙ ВЕРСИИ: будущие версии её не держат (решение
-- владельца 2026-10-07, версия — единица работы). Набор без открытой версии
-- судится целиком, как прежде.
SELECT tp.task_id || ' — ' || tp.state AS detail
  FROM task_phase tp
  JOIN task_scope sc ON sc.project_id = tp.project_id AND sc.task_id = tp.task_id
                    AND (sc.scope IS NULL OR sc.scope = 'open')
 WHERE tp.project_id = $1 AND tp.state <> 'closed' AND tp.open
 ORDER BY tp.task_id
