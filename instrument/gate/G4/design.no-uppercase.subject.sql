SELECT 'датчик «design-uppercase» читает ' || s.reads AS entity FROM project_sensor_spec s WHERE s.project_id = $1 AND s.fact = 'design-uppercase'
UNION ALL
SELECT f.name FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'design-uppercase'
