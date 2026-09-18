SELECT id || ' — ' || left(title, 60) AS detail FROM project_questions WHERE project_id = $1 AND state = 'open' AND answer_state <> 'owner' ORDER BY id
