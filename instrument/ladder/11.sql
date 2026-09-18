SELECT g.phase || ' · ' || g.item AS detail FROM project_gates g WHERE g.project_id = $1 AND g.state = 'failed' ORDER BY g.phase, g.item
