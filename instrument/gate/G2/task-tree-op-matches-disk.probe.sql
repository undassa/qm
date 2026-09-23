-- ДВЕ СТРОКИ, И ВЕРХ ДЛЯ ПЕРВОЙ ПОДСАЖЕН ЗДЕСЬ ЖЕ. Правило различает «файла
-- нет» и «обход туда не дошёл» по верхнему каталогу пути, а верхи берёт из
-- имён прочитанных фактов. Прежняя проба сажала только `probe-selftest/absent.rs`,
-- и после того различения молча уехала бы целиком в новую ветвь: верха
-- `probe-selftest` у набора нет и взяться ему неоткуда. Прежняя формулировка
-- осталась бы без самотеста вовсе.
--
-- Потолок тот же, что у `task-path-exists`: приговор самотеста — «ответ пункта
-- изменился», и одной строки для него довольно. Две строки гарантируют, что обе
-- ветви ИСПОЛНЯЮТСЯ, а не что каждая проверена порознь.
WITH d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1, 'task', 'M9-T9990', E'# M9-T9990 · подсадка самотеста', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
           RETURNING project_id),
     k AS (INSERT INTO code_fact (project_id, kind, name, detail)
           SELECT d.project_id, 'repo-file', 'probe-selftest/иной.rs', '' FROM d RETURNING project_id),
     t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT k.project_id, 'M9-T9990', (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9990,
                  'подсадка самотеста', 'S', 'not_started', 'dev', 'task', 'M9-T9990', 'declared' FROM k
           RETURNING project_id)
INSERT INTO project_task_tree_leaf (project_id, task_id, ord, dir, leaf, is_path, exempt, target_dir, op, path)
SELECT t.project_id, 'M9-T9990', 0, '', 'probe-selftest/absent.rs', true, false, 'probe-selftest', '!', 'probe-selftest/absent.rs' FROM t
UNION ALL
SELECT t.project_id, 'M9-T9990', 1, '', 'нет-такого-верха-пробы/absent.rs', true, false, 'нет-такого-верха-пробы', '!', 'нет-такого-верха-пробы/absent.rs' FROM t
