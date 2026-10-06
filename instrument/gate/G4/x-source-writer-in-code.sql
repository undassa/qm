-- ЗАДАЧА, НАЗВАННАЯ ПИСАТЕЛЕМ, ЗАКРЫТА — ЗНАЧИТ, КОД КОЛОНКУ ПИШЕТ.
--
-- Пометка `x-source: task:<id>` в G3 обещает: колонку запишет эта задача. Когда
-- задача закрыта, обещание проверяемо: датчик харнеса со ствола (`column-write`)
-- находит `INSERT INTO т (…к…)`, `UPDATE т SET к` или `ON CONFLICT … DO UPDATE
-- SET к`. Не нашёл — находка на закрытой задаче, то есть переделка. Запись,
-- которой датчик не видит (ORM, собранный SQL), — повод научить датчик в
-- репозитории харнеса, а не новая пометка. Решение владельца по заявке #21.
--
-- Имя факта датчика контракта — `т.к`, `т.к · обязательна` или `т · без записи`:
-- до пробела — адрес. У таблицы годится запись любой её колонки.
WITH помеченные AS (
  SELECT DISTINCT split_part(f.name, ' ', 1) AS at,
         substring(f.detail FROM '^помечено x-source: task:\s*([A-Za-z0-9][A-Za-z0-9._-]*)') AS task
    FROM code_fact f
   WHERE f.project_id = $1 AND f.kind = 'contract-schema' AND f.detail LIKE 'помечено x-source: task:%'),
судимые AS (
  SELECT п.at, п.task FROM помеченные п
    JOIN project_plan_tasks t ON t.project_id = $1 AND t.id = п.task AND t.kind = 'dev' AND t.state = 'closed')
SELECT 'датчик записи колонок «column-write» ' || fact_gap($1, 'column-write')
       || ': пишет ли код колонки закрытых задач — неизвестно' AS detail
 WHERE EXISTS (SELECT 1 FROM судимые) AND NOT fact_fresh($1, 'column-write')
UNION ALL
SELECT с.task || ' закрыта и названа писателем ' || с.at
       || ', а код его не пишет: ни INSERT, ни UPDATE SET, ни ON CONFLICT DO UPDATE'
  FROM судимые с
 WHERE fact_fresh($1, 'column-write')
   AND NOT EXISTS (SELECT 1 FROM code_fact w
                    WHERE w.project_id = $1 AND w.kind = 'column-write'
                      AND (w.name = с.at OR (strpos(с.at, '.') = 0 AND left(w.name, length(с.at) + 1) = с.at || '.')))
 ORDER BY 1
