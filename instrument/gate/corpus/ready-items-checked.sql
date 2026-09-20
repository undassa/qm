WITH прогон AS (
  SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty),
ответ AS (
  SELECT r.check_name, bool_or(r.verdict = 'passed') AS зелена
    FROM test_run r, прогон п
   WHERE r.project_id = $1 AND NOT r.dirty AND r.at = п.at
   GROUP BY r.check_name)
SELECT t.id || ' — пункт приёмки без названной проверки: ' || r.text AS detail
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done AND r.check_id = ''
UNION ALL
SELECT t.id || ' — проверка пункта приёмки не написана: ' || r.check_id
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done
   AND r.check_id <> ''
   AND NOT EXISTS (SELECT 1 FROM code_fact c
                    WHERE c.project_id = t.project_id
                      AND ((c.kind = 'test-name' AND c.name = r.check_id)
                        OR (c.kind = 'written-tc' AND position('fn ' || r.check_id || '(' in c.detail) > 0)))
UNION ALL
-- ПРОВЕРКА НАПИСАНА — ЕЩЁ НЕ ЗНАЧИТ «ПРОШЛА».
--
-- Прежде пункт приёмки считался закрытым, если проверка с таким именем
-- нашлась в коде. Написанная и никогда не запущенная проверка закрывала пункт
-- ровно так же, как зелёная, и «готово» держалось на существовании функции.
-- Прогон у харнеса теперь свой (`test_run`, заявка 20), и вердикт берётся у
-- него: у последнего прогона с ЧИСТОГО дерева.
SELECT t.id || ' — проверка пункта приёмки упала на стволе: ' || r.check_id
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
  JOIN ответ о ON о.check_name = r.check_id
 WHERE t.project_id = $1 AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done
   AND r.check_id <> '' AND NOT о.зелена
UNION ALL
-- Прогона нет вовсе — это «неизвестно», а не «сошлось»: пункт говорит об этом
-- одной строкой, а не молчит по каждому пункту приёмки набора.
SELECT 'прогонов с чистого дерева не было: прошли ли проверки пунктов приёмки — неизвестно'
 WHERE NOT EXISTS (SELECT 1 FROM test_run WHERE project_id = $1 AND NOT dirty)
   AND EXISTS (SELECT 1 FROM task_ready_item WHERE project_id = $1 AND check_id <> '')
 ORDER BY 1
