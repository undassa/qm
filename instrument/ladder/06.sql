SELECT g.phase || ' · ' || i.item AS detail FROM project_gates g
  JOIN gate_item i ON i.phase = g.phase AND i.id = g.id
 WHERE g.project_id = $1 AND g.state = 'failed' AND (g.phase = 'corpus' OR g.phase = (SELECT ph.gate FROM phase ph WHERE EXISTS (SELECT 1 FROM project_gates x WHERE x.project_id = $1 AND x.phase = ph.gate AND x.state <> 'passed') ORDER BY ph.ord LIMIT 1)) ORDER BY g.phase, i.item
