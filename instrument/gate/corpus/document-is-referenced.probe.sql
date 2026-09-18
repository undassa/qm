DELETE FROM project_document_links WHERE project_id=$1 AND target_kind='decision' AND target_name=(SELECT min(target_name) FROM project_document_links WHERE project_id=$1 AND target_kind='decision')
