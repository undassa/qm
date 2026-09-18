WITH t AS (
  INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
  SELECT $1, 'M9-TRISK', min(milestone_id), 9990, 'подсадка самотеста', 'S', 'not_started',
         (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1), 'declared'
    FROM project_plan_tasks WHERE project_id = $1
  RETURNING id)
INSERT INTO preflight_verdict (project_id, task_id, at, task_revision, verdict, findings, body)
SELECT $1, t.id, 1, 0, 'blocked', 0, 'подсадка самотеста' FROM t
