SELECT 'датчик «design-text-size» ' || fact_gap($1, 'design-text-size') || ': задан ли где-то размер текста числом — неизвестно' AS detail WHERE NOT fact_fresh($1, 'design-text-size')
UNION ALL
SELECT f.name || ' — размер текста числом мимо шкалы: «' || trim(f.detail) || '»' || coalesce(' (' || nullif(s.note, '') || ')', '') FROM code_fact f LEFT JOIN project_sensor_spec s ON s.project_id = f.project_id AND s.fact = f.kind WHERE f.project_id = $1 AND f.kind = 'design-text-size'
ORDER BY 1
