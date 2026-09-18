SELECT id || ' — ' || left(title, 40) AS detail FROM project_decisions
     WHERE project_id=$1 AND (date = '' OR deciders = '')
