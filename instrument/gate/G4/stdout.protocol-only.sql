WITH исходники AS (SELECT regexp_replace(m.name, 'Cargo\.toml$', '') || 'src/' AS src FROM code_fact m WHERE m.project_id = $1 AND m.kind = 'crate-manifest')
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': где исходники крейтов — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «stdout-unlocked» ' || fact_gap($1, 'stdout-unlocked') || ': снят ли где-то запрет print_stdout — неизвестно' WHERE NOT fact_fresh($1, 'stdout-unlocked')
UNION ALL
SELECT f.name || ' — снят запрет print_stdout' FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'stdout-unlocked' AND EXISTS (SELECT 1 FROM исходники и WHERE left(f.name, length(и.src)) = и.src)
ORDER BY 1
