WITH зав AS (
  SELECT substring(f.name from '^(.*):\d+$') AS манифест, substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') AS имя, f.name AS место, trim(f.detail) AS строка
    FROM code_fact f JOIN code_fact m ON m.project_id = f.project_id AND m.kind = 'crate-manifest' AND m.name = substring(f.name from '^(.*):\d+$')
   WHERE f.project_id = $1 AND f.kind = 'direct-dep'
     AND f.detail ~ '^\s*[a-z][a-z0-9_-]*(\.workspace\s*=\s*true|\s*=\s*("[0-9*^~<>=]|\{[^}]*\y(version|workspace|path|git)\y))'
     AND substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') NOT IN ('authors', 'categories', 'description', 'documentation', 'edition', 'exclude', 'homepage', 'include', 'keywords', 'license', 'license-file', 'publish', 'readme', 'repository', 'rust-version', 'version', 'resolver', 'links', 'build', 'default-run'))
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какие крейты есть — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': объявлены ли версии в воркспейсе — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT з.место || ' — версия ' || з.имя || ' мимо воркспейса: «' || з.строка || '»' FROM зав з WHERE з.строка !~ 'workspace\s*=\s*true'
ORDER BY 1
