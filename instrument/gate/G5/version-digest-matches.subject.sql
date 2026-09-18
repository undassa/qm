SELECT v.version FROM version_state v WHERE v.project_id = $1 AND v.state = 'closed'
