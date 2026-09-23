SELECT t.id || ' — родитель не назван полем' AS detail FROM project_plan_tasks t WHERE t.project_id = $1 AND t.kind = 'red' AND t.parent_task_id = ''
UNION ALL
-- ПУСТОЙ ПЕРЕЧЕНЬ И НЕПРИЗНАННОЕ ИМЯ — РАЗНЫЕ СОСТОЯНИЯ, и раньше они звались
-- одним словом. `V5-T49` получила «перечень проверок пуст», когда перечень стоял
-- и выглядел как у шести соседних пар: имя `art_14_covered_values_are_cut_and_named_once`
-- не подошло ни под один объявленный образец `id.check`. Исполнитель видел перечень
-- своими глазами и заключил, что врёт прибор, — и пошёл искать пропажу там, где её
-- нет. Сообщение обязано отличать «в наборе ничего не написано» от «написано, но
-- ни одно имя не признано», и во втором случае назвать и то, что стояло, и то,
-- чему оно должно соответствовать: иначе находку нечем закрыть, кроме угадывания.
SELECT t.id || ' — перечень проверок пуст' FROM project_plan_tasks t
 WHERE t.project_id = $1 AND t.kind = 'red'
   AND NOT EXISTS (SELECT 1 FROM project_task_check c WHERE c.project_id = t.project_id AND c.task_id = t.id)
   AND NOT EXISTS (SELECT 1 FROM project_document_fields f
                    WHERE f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name
                      AND f.name IN (SELECT value FROM scheme($1) WHERE role = 'field.red-checks') AND trim(f.value) <> '')
UNION ALL
SELECT t.id || ' — ни одно имя в перечне не подошло под объявленные образцы `id.check`: стоит «' || left(trim(f.value), 200) || '»; объявлены: '
       || coalesce((SELECT string_agg(s.value, ', ' ORDER BY s.ord) FROM scheme($1) s WHERE s.role = 'id.check'),
                   'ни одного, поэтому действует зашитый TC-[A-Z]+-[0-9]+[a-z]?')
  FROM project_plan_tasks t
  JOIN project_document_fields f ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name
                                AND f.name IN (SELECT value FROM scheme($1) WHERE role = 'field.red-checks') AND trim(f.value) <> ''
 WHERE t.project_id = $1 AND t.kind = 'red'
   AND NOT EXISTS (SELECT 1 FROM project_task_check c WHERE c.project_id = t.project_id AND c.task_id = t.id)
UNION ALL
SELECT t.id || ' — нет раздела «Признак готовности»' FROM project_plan_tasks t WHERE t.project_id = $1 AND t.kind = 'red' AND NOT EXISTS (SELECT 1 FROM project_document_sections s WHERE s.project_id = t.project_id AND s.entity_kind = t.entity_kind AND s.entity_name = t.entity_name AND s.title = 'Признак готовности')
UNION ALL
SELECT t.id || ' — родителя ' || t.parent_task_id || ' нет среди задач' FROM project_plan_tasks t WHERE t.project_id = $1 AND t.kind = 'red' AND t.parent_task_id <> '' AND NOT EXISTS (SELECT 1 FROM project_plan_tasks p WHERE p.project_id = t.project_id AND p.id = t.parent_task_id)
ORDER BY 1
