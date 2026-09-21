WITH прогон AS (
  SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty),
ответ AS (
  SELECT r.check_name, bool_or(r.verdict = 'passed') AS зелена
    FROM test_run r, прогон п
   WHERE r.project_id = $1 AND NOT r.dirty AND r.at = п.at
   GROUP BY r.check_name)
-- ИМЯ ПРОВЕРКИ ВЫВОДИТСЯ ИЗ ДАТЧИКА, И БЕЗ НЕГО ЭТО «НЕИЗВЕСТНО».
--
-- Пункт приёмки называет проверку либо объявленным образцом `id.check`, либо
-- тем, что функция с таким именем уже написана; второе знает датчик
-- `test-name`. Датчик несвежий — значит свободно названные проверки
-- (`undo_redo_frame`) не опознаются, и «пункт не называет проверки» было бы не
-- замером, а отсутствием замера, поданным как утверждение.
SELECT 'датчик «test-name» ' || fact_gap($1, 'test-name')
       || ': чем названы проверки пунктов приёмки — неизвестно, и это не зелёное' AS detail
 WHERE NOT fact_fresh($1, 'test-name')
   AND EXISTS (SELECT 1 FROM task_ready_item WHERE project_id = $1)
UNION ALL
SELECT t.id || ' — пункт приёмки без названной проверки: ' || r.text
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND fact_fresh($1, 'test-name')
   AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done AND r.check_id = ''
UNION ALL
SELECT t.id || ' — проверка пункта приёмки не написана: ' || r.check_id
  FROM project_plan_tasks t
  JOIN task_ready_item r ON r.project_id = t.project_id AND r.task_id = t.id
 WHERE t.project_id = $1 AND fact_fresh($1, 'test-name') AND fact_fresh($1, 'written-tc')
   AND t.kind = 'dev' AND t.state = 'closed' AND NOT r.done
   AND r.check_id <> ''
   -- «НАПИСАНА» СПРАШИВАЕТСЯ ТАМ ЖЕ, ГДЕ СПРАШИВАЕТ `G4 · closed-unwritten`.
   --
   -- Имён здесь два рода, и проверялись они одним способом. Функция находится
   -- по имени среди фактов `test-name`; СЦЕНАРИЙ `TC-…` функцией не бывает
   -- вовсе — его переводят проверки, а связь «сценарий переведён» уже сведена
   -- в `project_written_check`. Прежнее условие искало в тексте комментария
   -- подстроку `fn TC-INDEX-01(`, которой не бывает ни в одном дереве: все
   -- 24 находки про `TC-…` были о несуществующем предмете.
   AND NOT EXISTS (SELECT 1 FROM code_fact c
                    WHERE c.project_id = t.project_id
                      AND c.kind = 'test-name' AND c.name = r.check_id)
   AND NOT EXISTS (SELECT 1 FROM project_written_check w
                    WHERE w.project_id = t.project_id AND w.check_id = r.check_id)
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
-- Условие — «пункты приёмки есть», а не «есть пункт с именем проверки».
-- Второе связывало единственную ветку, которая говорит «неизвестно», с
-- разбором имени: сломайся разбор — и ветка замолчала бы ровно тогда, когда
-- сказать было особенно нужно.
SELECT 'прогонов с чистого дерева не было: прошли ли проверки пунктов приёмки — неизвестно'
 WHERE NOT EXISTS (SELECT 1 FROM test_run WHERE project_id = $1 AND NOT dirty)
   AND EXISTS (SELECT 1 FROM task_ready_item WHERE project_id = $1)
 ORDER BY 1
