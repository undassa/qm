-- ДВЕ СТРОКИ, ПОТОМУ ЧТО ВЕТКИ ДВЕ, И ОДНА ПОДСАДКА ОСТАВЛЯЛА ВТОРУЮ БЕЗ
-- САМОТЕСТА. Правило различает «каталога нет» и «датчик туда не смотрит», и
-- различие это выражено ВЕРХНИМ каталогом пути: `crates` датчик читает,
-- `нет-такого-верха-пробы` — нет. Подсадка, знающая только про второй случай,
-- прошла бы и на правиле, у которого первая ветка вырезана.
INSERT INTO project_task_tree_leaf(project_id, task_id, ord, dir, leaf, is_path, exempt, target_dir, forward_declared)
SELECT $1, min(id), 9998, 'проба', 'нет-такого-верха-пробы/каталог', true, false, 'нет-такого-верха-пробы/каталог', false
  FROM project_plan_tasks WHERE project_id=$1
UNION ALL
SELECT $1, min(id), 9997, 'проба', 'crates/нет-такого-каталога-пробы/файл.rs', true, false, 'crates/нет-такого-каталога-пробы', false
  FROM project_plan_tasks WHERE project_id=$1
