WITH t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
           SELECT $1, 'M9-TPARENT', min(milestone_id), 9995, 'подсадка самотеста', 'S', 'closed', 'dev', 'declared'
             FROM project_plan_tasks WHERE project_id = $1
           RETURNING id, milestone_id)
INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin, parent_task_id)
SELECT $1, 'M9-TRED', t.milestone_id, 9994, 'красная проба', 'S', 'not_started', 'red', 'declared', t.id FROM t
