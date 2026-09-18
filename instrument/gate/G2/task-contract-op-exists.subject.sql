SELECT t.id FROM project_plan_tasks t
  JOIN project_document_fields f
    ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name
   AND f.name IN (SELECT s.value FROM scheme($1) s WHERE s.role = 'field.task-contract-ops')
 WHERE t.project_id = $1 AND t.state <> 'closed'
   AND EXISTS (SELECT 1 FROM scheme($1) s WHERE s.role = 'doc.protocol')
