WITH сущ AS (
  SELECT id, entity_kind||coalesce('/'||nullif(entity_name,''),'') src FROM project_requirements WHERE project_id = $1
  UNION ALL SELECT id, entity_kind||coalesce('/'||nullif(entity_name,''),'') FROM project_checks WHERE project_id = $1
  UNION ALL SELECT id, entity_kind||coalesce('/'||nullif(entity_name,''),'') FROM project_screens WHERE project_id = $1
  UNION ALL SELECT id, entity_kind||coalesce('/'||nullif(entity_name,''),'') FROM project_needs WHERE project_id = $1
  UNION ALL SELECT id, entity_kind||coalesce('/'||nullif(entity_name,''),'') FROM project_stories WHERE project_id = $1)
SELECT e.id || ' — сущность есть в наборе, а документа, где она написана, нет: источник назван строкой (' || e.src || '), но самого имени этот документ не говорит' AS detail
  FROM сущ e
 WHERE NOT EXISTS (SELECT 1 FROM named_id_role n WHERE n.project_id = $1 AND n.said_id = e.id AND n.role = 'defines')
   AND true
 ORDER BY 1
