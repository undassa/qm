SELECT 'датчик «icon-button-unlabelled» ' || fact_gap($1, 'icon-button-unlabelled') || ': есть ли кнопка-иконка без подписи — неизвестно' AS detail WHERE NOT fact_fresh($1, 'icon-button-unlabelled')
UNION ALL
SELECT f.name || ' — кнопка-иконка без подписи для экранного чтения: «' || trim(f.detail) || '»' FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'icon-button-unlabelled'
ORDER BY 1
