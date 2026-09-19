WITH границы AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         substring(f.detail from '^pub (?:enum|struct) (\w+)') AS тип
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item'),
варианты AS (
  SELECT substring(v.detail from '^([A-Z][A-Za-z]*)') AS вариант,
         (SELECT б.тип FROM границы б
           WHERE б.файл = substring(v.name from '^(.*):\d+$') AND б.н < substring(v.name from ':(\d+)$')::int
           ORDER BY б.н DESC LIMIT 1) AS тип
    FROM code_fact v WHERE v.project_id = $1 AND v.kind = 'protocol-variant'),
опы AS (SELECT DISTINCT вариант FROM варианты WHERE тип = 'Op' AND вариант IS NOT NULL)
SELECT 'датчик «protocol-item» ' || fact_gap($1, 'protocol-item') || ': где в протоколе enum Op — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-item')
UNION ALL
SELECT 'датчик «protocol-variant» ' || fact_gap($1, 'protocol-variant') || ': какие операции есть в протоколе — неизвестно' WHERE NOT fact_fresh($1, 'protocol-variant')
UNION ALL
SELECT 'протокол — enum Op не разобран: протокол не прочитан' WHERE NOT EXISTS (SELECT 1 FROM опы)
UNION ALL
SELECT 'Op::' || вариант || ' — операция записывает или хранит страницу' FROM опы WHERE вариант ~ '^Page(Store|Save|Cache|Write|Put|Persist|Edit|Update|Set)'
ORDER BY 1
