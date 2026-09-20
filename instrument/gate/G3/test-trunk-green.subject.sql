SELECT check_name FROM test_run WHERE project_id = $1 AND NOT dirty LIMIT 1
