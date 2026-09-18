SELECT 'набор пуст' AS detail WHERE (SELECT count(*) FROM project_documents WHERE project_id = $1) = 0
