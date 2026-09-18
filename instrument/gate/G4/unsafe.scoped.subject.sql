SELECT 'датчик «unsafe-site» не свеж: есть ли unsafe в исходниках крейтов — не установлено' AS entity WHERE NOT fact_fresh($1, 'unsafe-site')
UNION ALL
SELECT 'датчик «crate-manifest» не свеж: где исходники крейтов — не установлено' WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT f.name FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'unsafe-site'
   AND EXISTS (SELECT 1 FROM code_fact m WHERE m.project_id = $1 AND m.kind = 'crate-manifest'
                  AND left(f.name, length(regexp_replace(m.name, 'Cargo\.toml$', '') || 'src/')) = regexp_replace(m.name, 'Cargo\.toml$', '') || 'src/')
