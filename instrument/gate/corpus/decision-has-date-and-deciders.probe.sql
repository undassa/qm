UPDATE project_decisions SET date='' WHERE project_id=$1 AND id=(SELECT id FROM project_decisions WHERE project_id=$1 ORDER BY number LIMIT 1)
