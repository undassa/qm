WITH открыть AS (
  UPDATE project_gates
     SET state = 'passed',
         result = jsonb_set(coalesce(result, '{}'::jsonb), '{computed}', to_jsonb('passed'::text))
   WHERE project_id = $1
     AND phase IN (SELECT p.gate FROM phase p
                    WHERE p.ord < (SELECT min(ord) FROM phase WHERE task_kind <> ''))
  RETURNING 1)
UPDATE project_plan_tasks SET state = 'not_started'
 WHERE project_id = $1
   AND id = (SELECT t.id FROM project_plan_tasks t
              WHERE t.project_id = $1
                AND t.kind = (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1)
              ORDER BY t.id LIMIT 1)
