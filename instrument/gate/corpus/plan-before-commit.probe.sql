-- ПОДСАДКА ДВУСТОРОННЯЯ: одна задача обязана найтись, вторая — нет.
--
-- Первая закрыта и не имеет ни плана, ни разбора — находка.
-- Вторая закрыта и несёт разбор предполёта с непустым телом, сделанный РАНЬШЕ
-- закрытия, — не находка. Без неё правило, отвергающее обе формы, прошло бы
-- подсадку целиком: одна находка ожидается и одна была бы.
WITH одна AS (
  INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
  SELECT $1, 'M9-TPLAN', min(milestone_id), 9991, 'подсадка самотеста', 'S', 'closed',
         (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1), 'declared'
    FROM project_plan_tasks WHERE project_id = $1
  RETURNING id),
вторая AS (
  INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, origin)
  SELECT $1, 'M9-TPREF', min(milestone_id), 9992, 'подсадка самотеста: разбор есть', 'S', 'closed',
         (SELECT task_kind FROM phase WHERE task_kind <> '' ORDER BY ord LIMIT 1), 'declared'
    FROM project_plan_tasks WHERE project_id = $1
  RETURNING id)
INSERT INTO preflight_verdict (project_id, task_id, at, task_revision, verdict, findings, body)
SELECT $1, вторая.id, 1, 1, 'ready', 0, 'разбор подсадки: непустое тело' FROM вторая
