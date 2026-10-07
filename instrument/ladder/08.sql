-- Этап закрытой версии — история: его задачи переехали в новые версии, и
-- пустым он может быть законно (нарезка myack на версии 2026-10-07 оставила
-- M2…M8 пустыми под закрытой 0.0.9). Судятся этапы незакрытых версий.
SELECT m.id || ' — задач нет' AS detail
  FROM project_plan_milestones m
 WHERE m.project_id = $1 AND m.closed = ''
   AND NOT EXISTS (SELECT 1 FROM version_state vs
                    WHERE vs.project_id = m.project_id AND vs.version = m.version_id AND vs.state = 'closed')
   AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t
                    WHERE t.project_id = m.project_id AND t.milestone_id = m.id)
 ORDER BY m.id
