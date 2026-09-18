SELECT m.id || ' — задач нет' AS detail
  FROM project_plan_milestones m
 WHERE m.project_id = $1 AND m.closed = ''
   AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t
                    WHERE t.project_id = m.project_id AND t.milestone_id = m.id)
 ORDER BY m.id
