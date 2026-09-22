-- Пара берётся из плана: там пересборка уже привела имя родителя
-- (`lower(btrim(…))`) и там же отсеяна красная задача, назвавшая несуществующую
-- веху. Прежнее соединение по `red_task` сверяло имя точным равенством, и пара,
-- объявленная в другом регистре, для этого пункта не существовала.
SELECT t.id || ' — закрыта, а её красная ' || rt.id || ' нет' AS detail
  FROM project_plan_tasks t
  JOIN project_plan_tasks rt ON rt.project_id = t.project_id AND rt.kind = 'red'
   AND rt.parent_task_id = t.id
 WHERE t.project_id = $1 AND t.kind <> 'red' AND t.state = 'closed' AND rt.state <> 'closed'
 ORDER BY t.id
