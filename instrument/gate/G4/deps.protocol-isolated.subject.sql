SELECT 'датчик «crate-manifest» не свеж: какие крейты есть — не установлено' AS entity WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT m.name FROM code_fact m WHERE m.project_id = $1 AND m.kind = 'crate-manifest'
