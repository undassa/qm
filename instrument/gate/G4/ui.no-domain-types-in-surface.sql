WITH протокол AS (
  SELECT substring(f.detail from '^pub (?:enum|struct) (\w+)') AS имя
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item'
     AND f.detail ~ '^pub (?:enum|struct) '),
поверхность AS (
  SELECT substring(f.detail from '^pub (?:enum|struct) (\w+)') AS имя, f.name AS где
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'surface-type'
     AND f.detail ~ '^pub (?:enum|struct) '
     AND (f.name LIKE 'crates/tot-ui/src/%' OR f.name LIKE 'crates/tot-tui/src/%'))
SELECT 'датчик «surface-type» ' || fact_gap($1, 'surface-type') || ': что объявляет поверхность — неизвестно' AS detail WHERE NOT fact_fresh($1, 'surface-type')
UNION ALL
SELECT 'датчик «protocol-item» ' || fact_gap($1, 'protocol-item') || ': что объявляет протокол — неизвестно' WHERE NOT fact_fresh($1, 'protocol-item')
UNION ALL
SELECT 'поверхность не прочитана: ни одного объявления в tot-ui и tot-tui' WHERE NOT EXISTS (SELECT 1 FROM поверхность)
UNION ALL
SELECT п.где || ': `' || п.имя || '` — доменный тип объявлен второй раз в поверхности'
  FROM поверхность п WHERE п.имя IN (SELECT имя FROM протокол)
ORDER BY 1
