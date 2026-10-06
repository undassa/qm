-- ПОМЕТКА x-source ЗАСЧИТЫВАЕТСЯ, ТОЛЬКО КОГДА НАЗЫВАЕТ ЗАДАЧУ ИЗ ПЛАНА.
--
-- Датчик уже не снимает находку пометкой без `task:<id>` — её ловят
-- `column-input`, `mandatory-input`, `column-foreign`, `table-without-insert`.
-- Здесь вторая половина: названная задача существует, и это задача кода.
-- Снятая задача из плана уходит, и пометка на неё — та же находка, что без
-- пометки. Кода G3 не требует: писатель ещё не написан, и требовать его здесь
-- значило бы запереть фазу её же продолжением. Его ищет `x-source-writer-in-code`
-- в G4, когда задача закрыта. Решение владельца по заявке #21, первая часть.
SELECT 'датчик «contract-schema» ' || fact_gap($1, 'contract-schema') || ': пометки x-source не прочитаны' AS detail
 WHERE NOT fact_fresh($1, 'contract-schema')
UNION ALL
SELECT f.name || '  —  пометка x-source называет задачу ' || coalesce(m.task, '?')
       || ', а задачи кода с таким именем в плане нет: писатель колонки не назван'
  FROM code_fact f
 CROSS JOIN LATERAL (SELECT substring(f.detail FROM '^помечено x-source: task:\s*([A-Za-z0-9][A-Za-z0-9._-]*)') AS task) m
 WHERE f.project_id = $1 AND f.kind = 'contract-schema' AND f.detail LIKE 'помечено x-source: task:%'
   AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t
                    WHERE t.project_id = $1 AND t.id = m.task AND t.kind <> 'red')
 ORDER BY 1
