SELECT 'датчик «protocol-form» ' || fact_gap($1, 'protocol-form') || ': обязателен ли актор — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-form')
UNION ALL
SELECT 'датчик «actor-type» ' || fact_gap($1, 'actor-type') || ': объявлен ли тип актора — неизвестно' AS detail WHERE NOT fact_fresh($1, 'actor-type')
UNION ALL
SELECT 'Envelope.actor — поля нет либо оно необязательно: операция может уйти без актора' WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'protocol-form')
UNION ALL
SELECT 'Actor — типа нет: поле есть, различать некого' WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'actor-type')
ORDER BY 1
