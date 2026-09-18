SELECT 'датчик «icon-button-unlabelled» читает ' || s.reads AS entity FROM project_sensor_spec s WHERE s.project_id = $1 AND s.fact = 'icon-button-unlabelled'
UNION ALL
SELECT f.name FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'icon-button-unlabelled'
