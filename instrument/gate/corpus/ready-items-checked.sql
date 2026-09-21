WITH прогон AS (
  SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty),
-- ПЕРЕАДРЕСОВАННЫЙ ПУНКТ НЕ ОБЕЩАЕТ, А ОТКАЗЫВАЕТ.
--
-- «Сценарий `TC-SANDBOX-02` здесь не закрывается: совершённое необратимое
-- перечисляет в `receipt` `M3-T13`» — это не обещание доказать сценарий, а
-- отказ с адресом. Правило видело в строке имя сценария и требовало
-- доказательства; предмет строки — ровно противоположное. Пять находок из
-- одиннадцати были такими.
--
-- Слово отказа объявлено набором (`word.elsewhere`), а не угадано здесь:
-- «здесь не закрыва», «пройденными не отмеча». Глагол «закрывает» в роль не
-- положен намеренно — он даёт двести попаданий по набору и стоит в
-- положительном случае тоже; адресата вернее брать образцом `id.task`.
переадресован AS (
  SELECT r.task_id, r.ord, r.check_id, r.text
    FROM task_ready_item r
   WHERE r.project_id = $1
     AND EXISTS (SELECT 1 FROM scheme($1) w
                  WHERE w.role = 'word.elsewhere'
                    AND lower(r.text) LIKE '%' || lower(w.value) || '%')),
-- Адресат — имя задачи В ТОМ ЖЕ ПУНКТЕ. Пункт собирается из перенесённых
-- строк проекцией: построчный счёт объявил бы безадресными десять пунктов из
-- тридцати девяти, у которых адресат стоит во второй половине.
адресат AS (
  SELECT DISTINCT п.task_id, п.ord, п.check_id, п.text, m[1] AS кому
    FROM переадресован п
    CROSS JOIN LATERAL regexp_matches(п.text,
         '(?:^|[^A-Za-z0-9_-])('
         || (SELECT string_agg(regexp_replace(value, '^\^|\$$', '', 'g'), '|')
               FROM scheme($1) WHERE role = 'id.task')
         || ')(?![A-Za-z0-9_-])', 'g') m),
ответ AS (
  SELECT r.check_name, bool_or(r.verdict = 'passed') AS зелена
    FROM test_run r, прогон п
   WHERE r.project_id = $1 AND NOT r.dirty AND r.at = п.at
     -- ЗЕРКАЛО ОБЯЗАНО ПАДАТЬ, И ЭТО НЕ ПОЛОМКА. Граница та же, по которой
     -- отсеивает `G3 · test-trunk-green`, — имя бинаря, а не связь через
     -- задачу. Замер 21.09: единственная находка ветки «упала на стволе»,
     -- `M2-T3 · s10_ac_1_finding_reaches_its_delta_in_three_transitions`,
     -- жила в `mirror_finding_to_delta.rs` и падала по построению.
     AND r.ran_in NOT LIKE 'mirror\_%'
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
-- ОБЪЯВЛЕННЫЙ СПОСОБ ЗДЕСЬ НЕ СЧИТАЕТСЯ, И ЭТО РЕШЕНИЕ, А НЕ ПРОПУСК.
--
-- Пункт приёмки и правда не обязан называть тест: «пуст вывод `rg …`»,
-- «`just gate` зелён» проверяются иначе, и для этого в приборе есть
-- `readiness_method` и дверь `method-set`. Но послабление «способ объявлен —
-- пункт закрыт» было написано и отозвано в тот же день: объявление не есть
-- прохождение.
--
-- Три дыры, измеренные на живом приборе. Способ вида `command` сервер
-- выполнить не может вовсе и отвечает «неизвестно» всегда — один вызов двери
-- снимал бы пункт навсегда. Способ вида `query`, чей запрос СЕЙЧАС красен,
-- снимал бы его так же: правило спрашивало о наличии записи, а не о её
-- вердикте. И пустой `method` при `methodKind=query` проходил бы тоже — при
-- том, что сам прибор на такой пункт отвечает «у способа вида «запрос»
-- запроса нет».
--
-- Это ровно та форма, которую три ветки ниже пункт и осуждает: «написанная и
-- никогда не запущенная проверка закрывала пункт так же, как зелёная».
-- Считать способ можно будет, когда у него появится вердикт: у `query` —
-- посчитанный сервером, у `command` — поданный, как подаются прогоны тестов.
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
   -- Переадресованный пункт судится своей веткой ниже, а не этой.
   AND NOT EXISTS (SELECT 1 FROM переадресован п
                    WHERE п.task_id = r.task_id AND п.ord = r.ord)
UNION ALL
-- ПЕРЕАДРЕСАЦИЯ ПРОВЕРЯЕМА, И ПРОВЕРЯЕТСЯ СТРОЖЕ ПРЕЖНЕГО.
--
-- Пункт сказал «закрывает `Y`». Значит `Y` обязана существовать, а если `Y`
-- уже закрыта — предмет обязан быть доказан: закрытая задача, которая
-- названного не взяла, и есть долг, который прежде прятался за требованием
-- невозможного.
--
-- ПОТОЛОК НАЗВАН: пункт, переадресованный БЕЗ адресата, здесь не судится.
-- Таких сегодня три, и у всех трёх довод в самой строке — «на вехе нет»,
-- «ждёт ответа владельца», «предмет — треть сценария». Отличить довод от
-- молчания прибору нечем, и требовать адресата от всех значило бы завести
-- три находки на исправном наборе.
SELECT а.task_id || ' — пункт переадресован к ' || а.кому || ', а такой задачи в наборе нет: '
       || left(а.text, 160)
  FROM адресат а
 WHERE NOT EXISTS (SELECT 1 FROM project_plan_tasks x
                    WHERE x.project_id = $1 AND lower(x.id) = lower(а.кому))
UNION ALL
SELECT а.task_id || ' — переадресован к ' || а.кому || ', та закрыта, а ' || а.check_id
       || ' так и не доказан'
  FROM адресат а
  JOIN project_plan_tasks y ON y.project_id = $1 AND lower(y.id) = lower(а.кому)
 WHERE fact_fresh($1, 'written-tc')
   AND y.state = 'closed' AND а.check_id <> ''
   AND NOT EXISTS (SELECT 1 FROM project_written_check w
                    WHERE w.project_id = $1 AND w.check_id = а.check_id)
   AND NOT EXISTS (SELECT 1 FROM code_fact c
                    WHERE c.project_id = $1 AND c.kind = 'test-name' AND c.name = а.check_id)
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
   -- Зеркала в `ответ` не входят вовсе — см. довод там.
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
