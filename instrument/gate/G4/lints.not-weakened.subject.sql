SELECT 'датчик «workspace-lint» не свеж: какие уровни линтов у воркспейса — не установлено' AS entity WHERE NOT fact_fresh($1, 'workspace-lint')
UNION ALL
SELECT f.name || ' — ' || f.detail FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'workspace-lint'
UNION ALL
SELECT 'lint.floor · ' || s.value FROM scheme($1) s WHERE s.role = 'lint.floor'
