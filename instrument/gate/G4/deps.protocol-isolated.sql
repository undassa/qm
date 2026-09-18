WITH карта AS (
  SELECT DISTINCT c[1] AS крейт, (m[1])::int AS слой
    FROM project_article_gates g JOIN project_articles a ON a.project_id = g.project_id AND a.number = g.article,
         regexp_matches(a.body, '(?:^|\n)\|\s*(\d+)\s*·[^|]*\|([^|]*)\|', 'g') m,
         regexp_matches(m[2], '`([a-z][a-z0-9_-]*)`', 'g') c
   WHERE g.project_id = $1 AND g.gate = 'deps:layering'),
крейты AS (
  SELECT c.name AS крейт, m.name AS манифест, regexp_replace(m.name, 'Cargo\.toml$', '') AS каталог
    FROM code_fact m JOIN code_fact c ON c.project_id = m.project_id AND c.kind = 'crate' AND substring(c.detail from 'названо в (.*)$') = m.name
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
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': зависит ли фундамент от воркспейса — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT 'карта слоёв не объявлена: фундамент (слой 0) назвать нечем — объявите статью с таблицей слоёв парой с deps:layering дверью article-gate-add' WHERE NOT EXISTS (SELECT 1 FROM project_article_gates g WHERE g.project_id = $1 AND g.gate = 'deps:layering')
UNION ALL
SELECT 'ART-' || lpad(g.article::text, 2, '0') || ' — таблица слоёв не называет слоя 0: фундамент назвать нечем' FROM project_article_gates g WHERE g.project_id = $1 AND g.gate = 'deps:layering' AND NOT EXISTS (SELECT 1 FROM карта WHERE слой = 0)
UNION ALL
SELECT з.крейт || ' — фундамент (слой 0) зависит от крейта воркспейса ' || з.имя || ': ' || з.место FROM зав з JOIN карта с ON с.крейт = з.крейт AND с.слой = 0 WHERE з.имя IN (SELECT крейт FROM крейты) AND з.имя <> з.крейт
ORDER BY 1
