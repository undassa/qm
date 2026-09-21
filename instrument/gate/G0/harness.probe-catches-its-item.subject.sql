-- Предмет — замеренные пункты набора: нет замеров, нечего и судить.
SELECT g.id FROM project_gates g WHERE g.project_id = $1 LIMIT 1
