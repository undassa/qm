SELECT l.label || ' → ' || l.target_path AS detail FROM project_document_links l WHERE l.project_id = $1 AND l.entity_kind = 'project-map' AND l.target_kind = '' ORDER BY 1
