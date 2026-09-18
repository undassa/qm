SELECT 'датчик «crate-manifest» не свеж: какие крейты есть — не установлено' AS entity WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «direct-dep» не свеж: какие рантаймы тянут крейты — не установлено' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT f.name FROM code_fact f JOIN code_fact m ON m.project_id = f.project_id AND m.kind = 'crate-manifest' AND m.name = substring(f.name from '^(.*):\d+$')
 WHERE f.project_id = $1 AND f.kind = 'direct-dep' AND f.detail ~ '^\s*(tokio|async-std|smol|async-global-executor)(\.workspace\s*=\s*true|\s*=\s*("[0-9*^~<>=]|\{))'
