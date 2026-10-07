-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
-- Подсказка «зачеркните» называет ИМЕНА и МЕСТО, а не образец: лишнее в
-- перечне пары снимается там, недостающее — в доказательстве родителя. Общая
-- фраза «~~имя~~» после каждой находки отправляла автора гадать, что и где.
SELECT r.id || ' — разошёлся с родителем ' || r.parent_task_id
       || ': не хватает ' || coalesce(н.names, '—') || ', лишние ' || coalesce(л.names, '—')
       || coalesce('; помянутое убранным зачеркните в перечне ' || r.id || ': ' || л.struck, '')
       || coalesce('; помянутое убранным зачеркните в доказательстве ' || r.parent_task_id || ': ' || н.struck, '')
       || coalesce((SELECT '; ' || string_agg(d.task_id || ': `' || d.check_id || '` не засчитана: снята зачёркиванием', '; ' ORDER BY d.task_id, d.check_id)
                      FROM project_check_dropped d
                     WHERE d.project_id = r.project_id AND d.task_id IN (r.id, r.parent_task_id)), '') AS detail
  FROM project_plan_tasks r
 CROSS JOIN LATERAL (
       SELECT string_agg(pc.check_id, ', ' ORDER BY pc.check_id) AS names,
              string_agg('~~`' || pc.check_id || '`~~', ', ' ORDER BY pc.check_id) AS struck
         FROM project_task_check pc
        WHERE pc.project_id = r.project_id AND pc.task_id = r.parent_task_id AND pc.said_as = 'доказательство'
          AND NOT EXISTS (SELECT 1 FROM project_task_check rc
                           WHERE rc.project_id = r.project_id AND rc.task_id = r.id AND rc.check_id = pc.check_id)) н
 CROSS JOIN LATERAL (
       SELECT string_agg(rc.check_id, ', ' ORDER BY rc.check_id) AS names,
              string_agg('~~`' || rc.check_id || '`~~', ', ' ORDER BY rc.check_id) AS struck
         FROM project_task_check rc
        WHERE rc.project_id = r.project_id AND rc.task_id = r.id
          AND NOT EXISTS (SELECT 1 FROM project_task_check pc
                           WHERE pc.project_id = r.project_id AND pc.task_id = r.parent_task_id
                             AND pc.said_as = 'доказательство' AND pc.check_id = rc.check_id)) л
 WHERE r.project_id = $1 AND r.kind = 'red' AND NOT EXISTS (SELECT 1 FROM task_scope sc WHERE sc.project_id = r.project_id AND sc.task_id = r.id AND sc.scope IN ('next','later')) AND (н.names IS NOT NULL OR л.names IS NOT NULL)
 ORDER BY 1
