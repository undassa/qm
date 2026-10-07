-- Задачи кода ОТКРЫТОЙ ВЕРСИИ закрыты. Будущие версии её не держат (решение
-- владельца 2026-10-07); набор без открытой версии судится целиком.
SELECT t.id || ' — ' || t.state AS detail FROM project_plan_tasks t JOIN task_scope sc ON sc.project_id = t.project_id AND sc.task_id = t.id AND (sc.scope IS NULL OR sc.scope = 'open') WHERE t.project_id = $1 AND t.kind <> 'red' AND t.state <> 'closed' ORDER BY t.id
