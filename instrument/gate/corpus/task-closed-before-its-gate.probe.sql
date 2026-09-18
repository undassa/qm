WITH t AS (
  INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
  SELECT $1, 'M9-TPHASE', min(milestone_id), 9992, 'подсадка самотеста', 'S', 'closed',
         (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord DESC LIMIT 1), 'declared'
    FROM project_plan_tasks WHERE project_id = $1
  RETURNING 1)
UPDATE project_gates
   SET state = 'failed',
       result = jsonb_set(coalesce(result, '{}'::jsonb), '{computed}', to_jsonb('failed'::text))
 WHERE project_id = $1
   AND phase = (SELECT p.gate FROM phase p
                 WHERE p.ord < (SELECT max(ord) FROM phase WHERE task_kind <> '')
                 ORDER BY p.ord LIMIT 1)
