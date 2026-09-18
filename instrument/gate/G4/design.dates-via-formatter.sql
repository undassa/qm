SELECT 'датчик «design-ad-hoc-date» ' || fact_gap($1, 'design-ad-hoc-date') || ': форматируется ли где-то дата по месту — неизвестно' AS detail WHERE NOT fact_fresh($1, 'design-ad-hoc-date')
UNION ALL
SELECT f.name || ' — дата форматируется по месту, мимо общего форматтера: «' || trim(f.detail) || '»' || coalesce(' (' || nullif(s.note, '') || ')', '') FROM code_fact f LEFT JOIN project_sensor_spec s ON s.project_id = f.project_id AND s.fact = f.kind WHERE f.project_id = $1 AND f.kind = 'design-ad-hoc-date'
ORDER BY 1
