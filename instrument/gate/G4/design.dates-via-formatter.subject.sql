SELECT 'датчик «design-ad-hoc-date» читает ' || s.reads AS entity FROM project_sensor_spec s WHERE s.project_id = $1 AND s.fact = 'design-ad-hoc-date'
UNION ALL
SELECT f.name FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'design-ad-hoc-date'
