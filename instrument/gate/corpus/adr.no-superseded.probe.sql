UPDATE project_decisions SET status_text = status_text || ' · заменено ADR-9999' WHERE project_id = $1 AND entity_name = (SELECT min(entity_name) FROM project_decisions WHERE project_id = $1)
