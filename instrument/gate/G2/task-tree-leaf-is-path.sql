-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
SELECT l.task_id || ' — ' || coalesce(nullif(l.dir,''),'?') || ' → ' || l.leaf || ': не путь, а имя предмета' AS detail FROM project_task_tree_leaf l WHERE l.project_id = $1 AND NOT l.is_path AND NOT l.exempt AND NOT EXISTS (SELECT 1 FROM task_scope sc WHERE sc.project_id = l.project_id AND sc.task_id = l.task_id AND sc.scope IN ('next','later')) ORDER BY 1
