-- ОДНА СТРОКА, ПОТОМУ ЧТО ВЕТВЬ ОДНА. Правило не различает «файла нет» и
-- «датчик туда не дошёл», и подсаживать вторую ветвь не во что. Потолок назван
-- честно: приговор самотеста — «ответ пункта изменился», и одной строки для
-- него довольно.
-- ДОКУМЕНТ ПОДСАДКИ НЕСЁТ СУЩЕСТВУЮЩУЮ ВЕХУ, И ЭТО НЕ УКРАШЕНИЕ. Проба живёт
-- в транзакции и умирает с ней — но однажды не умерла, и документ остался в
-- живом наборе. Веха выводилась из имени (`M9`), такой вехи нет, и вся
-- пересборка проекций падала о внешний ключ: гейт считал по прежним данным и
-- показывал двадцать красных вместо четырёх, а набор встал на час.
--
-- Поэтому веха берётся у самого набора. Утечка остаётся мусором, но мусором
-- безвредным: пересборка её переживёт, и найти её можно спокойно.
WITH d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           SELECT $1, 'task', 'M9-T9990',
                  E'# M9-T9990 · подсадка самотеста\n\n| Поле | Значение |\n|---|---|\n| Веха | '
                  || (SELECT min(m.id) FROM project_plan_milestones m WHERE m.project_id = $1) || E' |\n',
                  'probe-selftest', 0, 1, 1757000000000, 'probe-selftest'
           RETURNING project_id),
     t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT d.project_id, 'M9-T9990', (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9990,
                  'подсадка самотеста', 'S', 'not_started', 'dev', 'task', 'M9-T9990', 'declared' FROM d
           RETURNING project_id)
INSERT INTO project_task_tree_leaf (project_id, task_id, ord, dir, leaf, is_path, exempt, target_dir, op, path)
SELECT t.project_id, 'M9-T9990', 0, '', 'probe-selftest/absent.rs', true, false, 'probe-selftest', '!', 'probe-selftest/absent.rs' FROM t
