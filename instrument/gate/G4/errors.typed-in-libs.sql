WITH карта AS (
  SELECT DISTINCT c[1] AS крейт, (m[1])::int AS слой
    FROM project_article_gates g JOIN project_articles a ON a.project_id = g.project_id AND a.number = g.article,
         regexp_matches(a.body, '(?:^|\n)\|\s*(\d+)\s*·[^|]*\|([^|]*)\|', 'g') m,
         regexp_matches(m[2], '`([a-z][a-z0-9_-]*)`', 'g') c
   WHERE g.project_id = $1 AND g.gate = 'deps:layering'),
крейты AS (
  SELECT c.name AS крейт, m.name AS манифест, regexp_replace(m.name, 'Cargo\.toml$', '') AS каталог
    FROM code_fact m JOIN code_fact c ON c.project_id = m.project_id AND c.kind = 'crate' AND c.place = m.name
   WHERE m.project_id = $1 AND m.kind = 'crate-manifest'),
зав AS (
  SELECT k.крейт, substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') AS имя, f.name AS место, trim(f.detail) AS строка
    FROM code_fact f JOIN крейты k ON k.манифест = substring(f.name from '^(.*):\d+$')
   WHERE f.project_id = $1 AND f.kind = 'direct-dep'
     AND f.detail ~ '^\s*[a-z][a-z0-9_-]*(\.workspace\s*=\s*true|\s*=\s*("[0-9*^~<>=]|\{[^}]*\y(version|workspace|path|git)\y))'
     AND substring(f.detail from '^\s*([a-z][a-z0-9_-]*)') NOT IN ('authors', 'categories', 'description', 'documentation', 'edition', 'exclude', 'homepage', 'include', 'keywords', 'license', 'license-file', 'publish', 'readme', 'repository', 'rust-version', 'version', 'resolver', 'links', 'build', 'default-run'))
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какие крейты есть — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'датчик «crate» ' || fact_gap($1, 'crate') || ': как зовутся крейты — неизвестно' WHERE NOT fact_fresh($1, 'crate')
UNION ALL
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': тянет ли библиотека anyhow — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT 'карта слоёв не объявлена или не разобрана: anyhow тянут крейты (' || string_agg(DISTINCT з.крейт, ', ') || '), а какой слой — поверхности, неизвестно; объявите статью с таблицей слоёв парой с deps:layering дверью article-gate-add' FROM зав з WHERE з.имя = 'anyhow' AND NOT EXISTS (SELECT 1 FROM карта) HAVING count(*) > 0
UNION ALL
SELECT з.крейт || ' — anyhow ниже слоя поверхностей (слой ' || coalesce(с.слой::text, 'вне карты') || '): ' || з.место FROM зав з LEFT JOIN карта с ON с.крейт = з.крейт WHERE з.имя = 'anyhow' AND EXISTS (SELECT 1 FROM карта) AND coalesce(с.слой, -1) < (SELECT max(слой) FROM карта)
ORDER BY 1
