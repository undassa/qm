UPDATE project_gates SET state = 'failed'
 WHERE project_id = $1 AND state = 'passed'
