WITH best AS (SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty)
SELECT 'упала на стволе: ' || r.check_name AS detail
  FROM test_run r, best
 WHERE r.project_id = $1 AND NOT r.dirty AND r.at = best.at AND r.verdict = 'failed'
   -- Проверку, которую ещё напишет НЕЗАКРЫТАЯ задача, поломкой ствола не зовут:
   -- она падает потому, что работа не сделана, и об этом говорят счётчики.
   AND NOT EXISTS (SELECT 1 FROM task_ready_item i
                     JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                    WHERE i.project_id = $1 AND i.check_id = r.check_name AND t.state <> 'closed')
   -- ЗЕРКАЛО ОБЯЗАНО ПАДАТЬ, ПОКА ПАРА НЕ НАПИСАНА, И ЭТО НЕ ПОЛОМКА.
   --
   -- Красная фаза держится тем, что проверка пары красна до того, как пару
   -- напишут: `red-observed-failing` прямо требует увидеть её упавшей. Пока
   -- пункт этого не знал, он звал поломкой ствола ровно то, чего сам набор
   -- добивается, — и позеленеть не мог ни в один день, пока в проекте открыта
   -- хоть одна красная фаза. Набор `tot-ade` принёс это заявкой 202.
   --
   -- СОСТОЯНИЕ СПРАШИВАЕТСЯ У ПАРЫ, А НЕ У ЗЕРКАЛА, И ПРЕЖДЕ БЫЛО НАОБОРОТ.
   -- Первая правка брала `t.state <> 'closed'` у самой красной задачи — то
   -- есть исключала зеркала, которых ещё НЕ НАПИСАЛИ. А падает зеркало ровно
   -- в обратном случае: красная задача закрыта (проверка написана), пара ещё
   -- нет (тела нет). Соседний пункт `red-observed-failing` держит ту же связь
   -- с `t.state = 'closed'` и находит по ней двести строк — то есть закрытая
   -- красная задача и есть обычное состояние, а исключение покрывало редкое.
   -- На `tot-ade` это давало 45 ложных из 73: пункт звал поломкой ствола
   -- каждое живое зеркало проекта и держал фазу Ф4 закрытой.
   --
   -- Направление предиката — тот же класс, что «сколько КАЛЛЕРОВ» против «в
   -- какую сторону спрашивает каждый»: связь была верная, сторона — нет.
   AND NOT EXISTS (SELECT 1 FROM project_task_check c
                     JOIN project_plan_tasks red
                       ON red.project_id = c.project_id AND red.id = c.task_id
                     JOIN project_plan_tasks pair
                       ON pair.project_id = red.project_id AND pair.id = red.parent_task_id
                    WHERE c.project_id = $1 AND c.check_id = r.check_name
                      AND red.kind = 'red' AND pair.state <> 'closed')
 ORDER BY 1
