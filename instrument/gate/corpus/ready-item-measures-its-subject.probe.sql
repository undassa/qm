-- ПОДСАЖИВАЕТСЯ СОСТОЯНИЕ, А НЕ ПОВЕДЕНИЕ ИСПОЛНИТЕЛЯ: пункт готовности, чей
-- грепп ищет имя задачи. Цепочка версия → веха → задача нужна целиком: у
-- `task_ready_item` внешний ключ на задачу.
WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-grep-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'GREP-PROBE', v.id, 9991, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'GREP-PROBE', m.id, 1, 'проба самотеста', '', 'open', 'dev', 'task', 'GREP-PROBE' FROM m RETURNING id)
INSERT INTO task_ready_item (project_id, task_id, ord, check_id, text, done, origin)
SELECT $1, t.id, 1, '',
       'проба самотеста: инструмент ищет имя' || E'\n' || '    rg -n ''V9-T9|M9-T9'' crates/' || E'\n' || '    — стубов не осталось',
       false, 'проба'
  FROM t
