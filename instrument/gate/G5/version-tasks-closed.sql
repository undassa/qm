-- Все задачи ОТКРЫТОЙ ВЕРСИИ закрыты — тесты и код. Закрыть версию значит
-- закрыть её задачи, а не весь план (решение владельца 2026-10-07).
SELECT t.id || ' — ' || left(t.title, 50) AS detail FROM project_plan_tasks t JOIN task_scope sc ON sc.project_id = t.project_id AND sc.task_id = t.id AND (sc.scope IS NULL OR sc.scope = 'open') WHERE t.project_id = $1 AND t.state <> 'closed' ORDER BY t.id
