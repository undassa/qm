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
SELECT 'датчик «direct-dep» ' || fact_gap($1, 'direct-dep') || ': идут ли зависимости вниз — неизвестно' WHERE NOT fact_fresh($1, 'direct-dep')
UNION ALL
SELECT 'карта слоёв не объявлена: крейты есть, а ни одна статья не объявлена парой с deps:layering — объявите статью с таблицей «| N · слой | `крейт`, … |» дверью article-gate-add' WHERE NOT EXISTS (SELECT 1 FROM project_article_gates g WHERE g.project_id = $1 AND g.gate = 'deps:layering')
UNION ALL
SELECT 'ART-' || lpad(g.article::text, 2, '0') || ' — таблица слоёв не разобрана: слоёв ' || (SELECT count(DISTINCT слой) FROM карта) || ', ожидались номера подряд с нуля' FROM project_article_gates g WHERE g.project_id = $1 AND g.gate = 'deps:layering' AND (NOT EXISTS (SELECT 1 FROM карта) OR (SELECT count(DISTINCT слой) FROM карта) <> (SELECT max(слой) + 1 FROM карта))
UNION ALL
SELECT k.крейт || ' — крейт вне карты слоёв' FROM крейты k WHERE EXISTS (SELECT 1 FROM карта) AND k.крейт NOT IN (SELECT крейт FROM карта)
UNION ALL
SELECT з.крейт || ' (слой ' || сн.слой || ') зависит от ' || з.имя || ' (слой ' || св.слой || ') — зависимость вверх: ' || з.место FROM зав з JOIN карта сн ON сн.крейт = з.крейт JOIN карта св ON св.крейт = з.имя WHERE з.имя IN (SELECT крейт FROM крейты) AND св.слой > сн.слой
ORDER BY 1
