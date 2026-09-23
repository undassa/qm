-- «В ДЕРЕВЕ» ЗДЕСЬ ЗНАЧИТ «СРЕДИ ПОДАННОГО ДАТЧИКОМ». Число берётся у
-- `project_crate.test_functions`, собранного из фактов рода `test-fn`, а тот
-- читает по объявленному перечню. Расхождение таблицы с числом датчика —
-- находка настоящая, но называть источник деревом значило бы утверждать
-- то, чего пункт не мерил.
SELECT s.subject || ' — в таблице ' || s.said || ' тестовых функций, среди прочитанного датчиком ' || c.test_functions AS detail FROM project_traceability_said s JOIN project_crate c ON c.project_id = s.project_id AND c.name = s.subject WHERE s.project_id = $1 AND s.column_name = 'тестовых функций' AND s.said <> c.test_functions UNION ALL SELECT c.name || ' — среди прочитанного датчиком ' || c.test_functions || ' тестовых функций, а таблица о крейте молчит' FROM project_crate c WHERE c.project_id = $1 AND c.in_repo AND c.test_functions > 0 AND NOT EXISTS (SELECT 1 FROM project_traceability_said s WHERE s.project_id = c.project_id AND s.column_name = 'тестовых функций' AND s.subject = c.name) ORDER BY 1
