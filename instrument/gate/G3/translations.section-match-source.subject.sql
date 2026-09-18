SELECT 'датчик «section-stamp» не свеж: есть ли переводы разделов с отпечатком — не установлено' AS entity WHERE NOT fact_fresh($1, 'section-stamp')
UNION ALL
SELECT f.name || ' — ' || f.detail FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'section-stamp'
