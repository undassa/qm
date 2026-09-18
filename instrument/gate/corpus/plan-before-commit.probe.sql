INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
SELECT $1, 'M9-TPLAN', min(milestone_id), 9991, 'подсадка самотеста', 'S', 'closed',
       (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1), 'declared'
  FROM project_plan_tasks WHERE project_id = $1
