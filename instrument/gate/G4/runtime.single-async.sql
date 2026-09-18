WITH зав AS (
  SELECT substring(f.name from '^(.*):\d+$') AS манифест, substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') AS имя, f.name AS место, trim(f.detail) AS строка
    FROM code_fact f JOIN code_fact m ON m.project_id = f.project_id AND m.kind = 'crate-manifest' AND m.name = substring(f.name from '^(.*):\d+$')
   WHERE f.project_id = $1 AND f.kind = 'direct-dep'
     AND f.detail ~ '^\s*[a-z][a-z0-9_-]*(\.workspace\s*=\s*true|\s*=\s*("[0-9*^~<>=]|\{[^}]*\y(version|workspace|path|git)\y))'
     AND substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') NOT IN ('authors', 'categories', 'description', 'documentation', 'edition', 'exclude', 'homepage', 'include', 'keywords', 'license', 'license-file', 'publish', 'readme', 'repository', 'rust-version', 'version', 'resolver', 'links', 'build', 'default-run')),
рантаймы AS (SELECT з.* FROM зав з WHERE з.имя IN ('tokio', 'async-std', 'smol', 'async-global-executor')),
главный AS (SELECT имя FROM рантаймы GROUP BY имя ORDER BY count(*) DESC, имя LIMIT 1)
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какие крейты есть — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': есть ли второй рантайм — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT р.место || ' — второй асинхронный рантайм ' || р.имя || ' рядом с ' || (SELECT имя FROM главный) FROM рантаймы р WHERE р.имя <> (SELECT имя FROM главный)
ORDER BY 1
