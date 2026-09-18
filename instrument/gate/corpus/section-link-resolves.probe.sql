UPDATE project_documents d SET content = d.content || E'\n\nсм. `' || t.имя || '.md` §994.1 — подсадка самотеста'
  FROM (SELECT p.entity_name AS имя FROM project_documents p
         WHERE p.project_id = $1 AND p.entity_name <> ''
           AND EXISTS (SELECT 1 FROM project_document_sections s WHERE s.project_id = $1 AND s.entity_kind = p.entity_kind AND s.entity_name = p.entity_name AND s.title ~ '^[0-9]')
         ORDER BY p.entity_name LIMIT 1) t
 WHERE d.project_id = $1 AND d.entity_name = t.имя
