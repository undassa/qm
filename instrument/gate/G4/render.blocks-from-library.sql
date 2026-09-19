WITH типы_блока AS (
  SELECT DISTINCT substring(v.detail from '^\s*([A-Z][A-Za-z]*)') AS вариант
    FROM code_fact v, code_fact b
   WHERE v.project_id = $1 AND v.kind = 'protocol-variant'
     AND b.project_id = $1 AND b.kind = 'protocol-item'
     AND b.detail ~ '^pub enum BlockType\y'
     AND substring(v.name from '^(.*):\d+$') = substring(b.name from '^(.*):\d+$')
     AND substring(v.name from ':(\d+)$')::int > substring(b.name from ':(\d+)$')::int),
свои AS (
  SELECT substring(v.detail from '^\s*([A-Z][A-Za-z]*)') AS вариант, v.name AS где
    FROM code_fact v WHERE v.project_id = $1 AND v.kind = 'surface-variant'
     AND (v.name LIKE 'crates/tot-ui/src/%' OR v.name LIKE 'crates/tot-tui/src/%'))
SELECT 'датчик «surface-variant» ' || fact_gap($1, 'surface-variant') || ': какие варианты у поверхности — неизвестно' AS detail WHERE NOT fact_fresh($1, 'surface-variant')
UNION ALL
SELECT 'датчик «surface-blocktype-wildcard» ' || fact_gap($1, 'surface-blocktype-wildcard') || ': глотает ли поверхность тип блока — неизвестно' WHERE NOT fact_fresh($1, 'surface-blocktype-wildcard')
UNION ALL
SELECT 'BlockType не разобран: библиотека типов блока не прочитана' WHERE NOT EXISTS (SELECT 1 FROM типы_блока)
UNION ALL
SELECT с.где || ': `' || с.вариант || '` — имя варианта закрытой библиотеки объявлено поверхностью'
  FROM свои с WHERE с.вариант IN (SELECT вариант FROM типы_блока)
UNION ALL
SELECT f.name || ' — матчит `BlockType` рукавом `_ =>`: новый тип блока пройдёт молча'
  FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'surface-blocktype-wildcard'
   AND (f.name LIKE 'crates/tot-ui/src/%' OR f.name LIKE 'crates/tot-tui/src/%')
ORDER BY 1
