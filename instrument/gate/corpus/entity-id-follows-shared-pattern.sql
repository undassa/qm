WITH сущ(kind,id) AS (
  SELECT 'requirement', id FROM project_requirements WHERE project_id = $1
  UNION ALL SELECT 'check', id FROM project_checks WHERE project_id = $1
  UNION ALL SELECT 'question', id FROM project_questions WHERE project_id = $1
  UNION ALL SELECT CASE WHEN kind='red' THEN 'red-task' ELSE 'task' END, id FROM project_plan_tasks WHERE project_id = $1
  UNION ALL SELECT 'screen', id FROM project_screens WHERE project_id = $1
  UNION ALL SELECT 'story', id FROM project_stories WHERE project_id = $1)
SELECT с.kind || ' ' || с.id || ' — имя не следует общему образцу вида (' || (k.spec->>'id') || '): раскрыватель имён его не видит, и связи этой сущности набор не находит' AS detail
  FROM сущ с JOIN kind_layout k ON k.name = с.kind AND k.spec->>'id' IS NOT NULL
 WHERE с.id !~ (k.spec->>'id')
   AND true
 ORDER BY 1
