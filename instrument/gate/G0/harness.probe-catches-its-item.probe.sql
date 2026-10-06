-- Подсадка: пункт, чья проба объявлена не сработавшей. Правило обязано назвать
-- его поимённо — иначе оно не читает приговор пробы вовсе.
UPDATE project_gates SET probe_ok = false
 WHERE project_id = $1 AND id = (SELECT id FROM project_gates WHERE project_id = $1 AND applicable IS NOT FALSE
             ORDER BY phase, id LIMIT 1)
