UPDATE version_state SET state = 'closed' WHERE project_id = $1 AND state = 'open'
