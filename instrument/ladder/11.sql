SELECT g.phase || ' · ' || i.item AS detail
  FROM project_gates g
  JOIN gate_item i ON i.phase = g.phase AND i.id = g.id
 WHERE g.project_id = $1 AND g.state = 'failed'
 ORDER BY g.phase, i.item
