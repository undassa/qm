-- Прогон — последний чистый, где что-то измерено (`test_run_last`). Прогон,
-- упавший на сборке, падений не несёт, и взятый отсюда он делал ствол зелёным
-- на пустом месте (заявка 103). Строки — своих партий (`test_run_trunk`):
-- падение с раннера CI — наблюдение красной фазы, а не поломка этого ствола.
WITH best AS (SELECT at FROM test_run_last WHERE project_id = $1)
-- ИСКЛЮЧЕНИЕ НИЖЕ ОПИРАЕТСЯ НА ДАТЧИК, И БЕЗ НЕГО ПУНКТ НЕ УТВЕРЖДАЕТ.
--
-- Проверку, которую ещё напишет незакрытая задача, пункт прощает по колонке
-- `task_ready_item.check_id`. Колонка выводится в том числе из датчика
-- `test-name`: свободно названная проверка (`undo_redo_frame`) опознаётся
-- только тем, что функция с таким именем написана. Датчик несвежий — прощение
-- пропадает, и красная фаза предстала бы поломкой ствола. Молчать тут тоже
-- нельзя: «нечем мерить» не значит «зелено».
SELECT 'датчик «test-name» ' || fact_gap($1, 'test-name')
       || ': отличить падение красной фазы от поломки ствола нечем — неизвестно, и это не зелёное' AS detail
 WHERE NOT fact_fresh($1, 'test-name')
   AND EXISTS (SELECT 1 FROM test_run_trunk r, best
                WHERE r.project_id = $1 AND NOT r.dirty AND r.at = best.at AND r.verdict = 'failed')
UNION ALL
SELECT 'упала на стволе: ' || r.check_name
  FROM test_run_trunk r, best
 WHERE r.project_id = $1 AND NOT r.dirty AND r.at = best.at AND r.verdict = 'failed'
   AND fact_fresh($1, 'test-name')
   -- Проверку, которую ещё напишет НЕЗАКРЫТАЯ задача, поломкой ствола не зовут:
   -- она падает потому, что работа не сделана, и об этом говорят счётчики.
   -- Имя сравнивается и голым: libtest зовёт модульный тест путём
   -- (`escalation::tests::a_role_on_…`), а пункт готовности — именем функции.
   --
   -- КРАСНАЯ ФАЗА И РЕГРЕССИЯ РАЗЛИЧАЮТСЯ ИСТОРИЕЙ, А НЕ ВЕРСИЕЙ ДЕРЖАТЕЛЯ.
   --
   -- Прощается лишь проверка, НИ РАЗУ не прошедшая на стволе под полным
   -- именем: красная фаза зелёной не бывала, а прошедшая и упавшая — регрессия
   -- выпущенной работы, и упоминание её открытой задачей («не ломать X»)
   -- её не снимает, как и одноимённая функция другого модуля (ревью #194).
   -- Держатель ни разу не прошедшей — ЛЮБАЯ незакрытая задача, пунктом
   -- готовности или `project_task_check`, какой бы версии она ни была: тест,
   -- написанный прежде кода для задачи поздней версии, — честная красная фаза
   -- (решение владельца 2026-10-07: версии), и требование «держит открытая
   -- версия» держало бы открытую версию ею вечно (MyAck: M2-T14 в 0.1.1).
   AND (EXISTS (SELECT 1 FROM test_run_trunk p
                 WHERE p.project_id = $1 AND p.check_name = r.check_name AND p.verdict = 'passed')
        OR NOT EXISTS (SELECT 1 FROM task_ready_item i
                         JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                        WHERE i.project_id = $1 AND t.state <> 'closed'
                          AND i.check_id IN (r.check_name, regexp_replace(r.check_name, '^.*::', ''))
                       UNION ALL
                       SELECT 1 FROM project_task_check c
                         JOIN project_plan_tasks t ON t.project_id = c.project_id AND t.id = c.task_id
                        WHERE c.project_id = $1 AND t.state <> 'closed'
                          AND c.check_id IN (r.check_name, regexp_replace(r.check_name, '^.*::', ''))))
   -- ЗЕРКАЛО ОБЯЗАНО ПАДАТЬ, И УЗНАЁТСЯ ОНО ПО БИНАРЮ, А НЕ ПО СВЯЗИ.
   --
   -- Связь через задачу этот класс не закрывает: замер 2026-09-21 дал 41
   -- падение, все 41 в зеркалах, и **24 из них не названы в наборе ни одной
   -- задачей** — спрашивать о них было некого. Красная фаза живёт в коде
   -- раньше, чем в документах, и требовать объявления значит требовать
   -- бухгалтерии там, где предмет виден прямо.
   --
   -- Имя бинаря приходит даром: `cargo test` печатает `Running tests/<файл>.rs`
   -- перед блоком каждого, и разбор его прежде выбрасывал. Граница `mirror_` —
   -- та же, по которой сам набор делит прогоны своими рецептами
   -- (`just red` берёт `-E 'binary(/^mirror_/)'`), то есть не наша выдумка.
   AND r.ran_in NOT LIKE 'mirror\_%'
   -- ЗЕРКАЛО ОБЯЗАНО ПАДАТЬ, ПОКА ПАРА НЕ НАПИСАНА, И ЭТО НЕ ПОЛОМКА.
   --
   -- Красная фаза держится тем, что проверка пары красна до того, как пару
   -- напишут: `red-observed-failing` прямо требует увидеть её упавшей. Пока
   -- пункт этого не знал, он звал поломкой ствола ровно то, чего сам набор
   -- добивается, — и позеленеть не мог ни в один день, пока в проекте открыта
   -- хоть одна красная фаза. Набор `tot-ade` принёс это заявкой 202.
   --
   -- СОСТОЯНИЕ СПРАШИВАЕТСЯ НЕ У ЗЕРКАЛА И НЕ У ЕГО ПАРЫ, А У САМОЙ ПРОВЕРКИ.
   --
   -- Первая правка брала `red.state <> 'closed'` — исключала зеркала, которых
   -- ещё НЕ НАПИСАЛИ, тогда как падает зеркало в обратном случае. Вторая шла
   -- к паре через `red.parent_task_id` и сняла лишь 5 из 45: связь с парой
   -- заполнена не у всех красных задач, и внутреннее соединение молча роняло
   -- остальных. Оба раза правило опиралось на сторону, которой могло не быть.
   --
   -- Предмет выразим без пары вовсе. Проверка принадлежит красной фазе до тех
   -- пор, пока её не забрала себе ЗАКРЫТАЯ ОБЫЧНАЯ задача: зеркало пишет её
   -- красной, пара пишет тело и закрывается, и с этого дня проверка обязана
   -- быть зелёной. Пока такой задачи нет — падение есть красная фаза, а не
   -- поломка ствола.
   AND NOT EXISTS (SELECT 1 FROM project_task_check c
                     JOIN project_plan_tasks t
                       ON t.project_id = c.project_id AND t.id = c.task_id
                    WHERE c.project_id = $1 AND c.check_id IN (r.check_name, regexp_replace(r.check_name, '^.*::', ''))
                      AND t.kind = 'red'
                      AND NOT EXISTS (SELECT 1 FROM project_task_check c2
                                        JOIN project_plan_tasks t2
                                          ON t2.project_id = c2.project_id AND t2.id = c2.task_id
                                       WHERE c2.project_id = $1
                                         AND c2.check_id IN (r.check_name, regexp_replace(r.check_name, '^.*::', ''))
                                         AND t2.kind <> 'red'
                                         AND t2.state = 'closed'))
   -- ЗАГЛУШКА, НАЗВАННАЯ ПРОВЕРКОЙ ОТКРЫТОЙ ЗАДАЧИ, — КРАСНАЯ ФАЗА MYACK.
   --
   -- Красная фаза MyAck устроена не зеркалом, а заглушкой: тест пишется
   -- первым, против `todo!("TC-ORG-02")`, и падает словами `not yet
   -- implemented: TC-ORG-02`. Имя функции теста при этом своё, бинарь —
   -- обычный, и два исключения выше его не узнают. Замер 2026-10-08: 491 из
   -- 495 падений ствола MyAck — такие заглушки, все за открытыми задачами, и
   -- пункт звал их поломкой, держа лестницу. Решение владельца 2026-10-07:
   -- тест прежде кода, против заглушки, названной проверкой, — это парадигма,
   -- а не порча.
   --
   -- Прощается только заглушка, чью проверку держит НЕЗАКРЫТАЯ задача (по
   -- пункту готовности или по `project_task_check`). Заглушка проверки,
   -- которую держат одни закрытые задачи или никто, — находка: задача
   -- закрылась, а тело так и не написано.
   --
   -- ДВА ОГРАНИЧЕНИЯ, БЕЗ КОТОРЫХ ПРОЩЕНИЕ ПРЯТАЛО БЫ РЕГРЕССИЮ.
   --
   -- Id в сообщении к упавшей проверке не привязан: зелёная проверка
   -- законченной работы, упёршаяся в чужую заглушку открытой задачи, упала бы
   -- той же фразой. Поэтому прощается лишь проверка, НИ РАЗУ не прошедшая на
   -- стволе в том же бинаре: красная фаза зелёной не бывала.
   --
   -- И закрытого держателя не перебивает открытый со-держатель (эпик-родитель
   -- держит те же TC): если проверку уже забрала себе ЗАКРЫТАЯ обычная задача,
   -- тело обязано быть написано — как в правиле зеркала выше.
   AND NOT EXISTS (SELECT 1
                     FROM (SELECT substring(r.reason FROM '^not yet implemented: ([A-Z]+-[A-Z]+-[0-9]+[a-z]?)') AS id) s
                    WHERE s.id IS NOT NULL
                      AND NOT EXISTS (SELECT 1 FROM test_run_trunk p
                                       WHERE p.project_id = $1 AND p.check_name = r.check_name
                                         AND p.ran_in IS NOT DISTINCT FROM r.ran_in AND p.verdict = 'passed')
                      AND NOT EXISTS (SELECT 1 FROM task_ready_item i
                                        JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                                       WHERE i.project_id = $1 AND i.check_id = s.id
                                         AND t.state = 'closed' AND t.kind <> 'red')
                      AND NOT EXISTS (SELECT 1 FROM project_task_check c
                                        JOIN project_plan_tasks t ON t.project_id = c.project_id AND t.id = c.task_id
                                       WHERE c.project_id = $1 AND c.check_id = s.id
                                         AND t.state = 'closed' AND t.kind <> 'red')
                      AND (EXISTS (SELECT 1 FROM task_ready_item i
                                     JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                                    WHERE i.project_id = $1 AND i.check_id = s.id AND t.state <> 'closed')
                        OR EXISTS (SELECT 1 FROM project_task_check c
                                     JOIN project_plan_tasks t ON t.project_id = c.project_id AND t.id = c.task_id
                                    WHERE c.project_id = $1 AND c.check_id = s.id AND t.state <> 'closed')))
 ORDER BY 1
