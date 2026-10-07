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
              LEFT JOIN task_scope sc ON sc.project_id = t.project_id AND sc.task_id = t.id
              WHERE t.project_id = $1
                AND t.kind = (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1)
                -- Подсадка в открытую версию: ступень судит только её.
                AND (sc.scope IS NULL OR sc.scope = 'open')
              ORDER BY t.id LIMIT 1)
