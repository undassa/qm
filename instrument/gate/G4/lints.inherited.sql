SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какие манифесты есть — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «lints-inherited» ' || fact_gap($1, 'lints-inherited') || ': наследуют ли крейты линты — неизвестно' WHERE NOT fact_fresh($1, 'lints-inherited')
UNION ALL
SELECT m.name || ' — манифест крейта без [lints] workspace = true' FROM code_fact m WHERE m.project_id = $1 AND m.kind = 'crate-manifest' AND NOT EXISTS (SELECT 1 FROM code_fact l WHERE l.project_id = $1 AND l.kind = 'lints-inherited' AND l.name = m.name)
ORDER BY 1
