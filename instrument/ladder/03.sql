-- Ровно одна открытая версия, и она ЕСТЬ В ПЛАНЕ. Строка о снятой или
-- переименованной версии остаётся в `version_state`; считать её открытой значит
-- судить пустоту (ревью #187) — она называется здесь.
SELECT CASE WHEN count(*) = 0 THEN 'открытый выпуск не объявлен: какой из ' || (SELECT count(*) FROM project_plan_versions WHERE project_id = $1) || ' мы делаем — не сказано' ELSE 'открытыми объявлены сразу ' || count(*) || ': ' || string_agg(version, ', ') END AS detail FROM version_state WHERE project_id = $1 AND state = 'open' AND version IN (SELECT id FROM project_plan_versions WHERE project_id = $1) HAVING count(*) <> 1
UNION ALL
SELECT 'открытой объявлена ' || vs.version || ', а такой версии в плане нет: строку снять (`version-state drop`) или вернуть версию в план' FROM version_state vs WHERE vs.project_id = $1 AND vs.state = 'open' AND vs.version NOT IN (SELECT id FROM project_plan_versions WHERE project_id = $1)
