SELECT 'датчик «design-uppercase» ' || fact_gap($1, 'design-uppercase') || ': набрано ли что-то прописными — неизвестно' AS detail WHERE NOT fact_fresh($1, 'design-uppercase')
UNION ALL
SELECT f.name || ' — подпись прописными: «' || trim(f.detail) || '»' || coalesce(' (' || nullif(s.note, '') || ')', '') FROM code_fact f LEFT JOIN project_sensor_spec s ON s.project_id = f.project_id AND s.fact = f.kind WHERE f.project_id = $1 AND f.kind = 'design-uppercase'
ORDER BY 1
