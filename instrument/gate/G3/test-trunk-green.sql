WITH best AS (SELECT max(at) AS at FROM test_run WHERE project_id = $1 AND NOT dirty)
SELECT 'упала на стволе: ' || r.check_name AS detail
  FROM test_run r, best
 WHERE r.project_id = $1 AND NOT r.dirty AND r.at = best.at AND r.verdict = 'failed'
   -- Проверку, которую ещё напишет НЕЗАКРЫТАЯ задача, поломкой ствола не зовут:
   -- она падает потому, что работа не сделана, и об этом говорят счётчики.
   AND NOT EXISTS (SELECT 1 FROM task_ready_item i
                     JOIN project_plan_tasks t ON t.project_id = i.project_id AND t.id = i.task_id
                    WHERE i.project_id = $1 AND i.check_id = r.check_name AND t.state <> 'closed')
   -- ЗЕРКАЛО ОБЯЗАНО ПАДАТЬ, И ЭТО НЕ ПОЛОМКА.
   --
   -- Красная фаза держится тем, что проверка пары красна до того, как пару
   -- напишут: `red-observed-failing` прямо требует увидеть её упавшей. Пока
   -- пункт этого не знал, он звал поломкой ствола ровно то, чего сам набор
   -- добивается, — и позеленеть не мог ни в один день, пока в проекте открыта
   -- хоть одна красная фаза. Набор `tot-ade` принёс это заявкой 202 с двумя
   -- именами; общее у них не имя файла, а связь: проверку назвала своей
   -- незакрытая красная задача.
   AND NOT EXISTS (SELECT 1 FROM project_task_check c
                     JOIN project_plan_tasks t ON t.project_id = c.project_id AND t.id = c.task_id
                    WHERE c.project_id = $1 AND c.check_id = r.check_name
                      AND t.kind = 'red' AND t.state <> 'closed')
 ORDER BY 1
