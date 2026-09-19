WITH действия AS (
  SELECT DISTINCT substring(v.detail from '^([A-Z][A-Za-z]*)') AS действие
    FROM code_fact v
   WHERE v.project_id = $1 AND v.kind = 'grammar-action'
     AND substring(v.detail from '^([A-Z][A-Za-z]*)') IS NOT NULL),
строки AS (
  SELECT DISTINCT substring(b.detail from '^([A-Z][A-Za-z]*)') AS действие,
         substring(b.detail from '=> *"([^"]+)"') AS клавиша
    FROM code_fact b
   WHERE b.project_id = $1 AND b.kind = 'grammar-binding')
SELECT 'датчик «grammar-action» ' || fact_gap($1, 'grammar-action')
       || ': какие действия у поверхности есть — неизвестно' AS detail
 WHERE NOT fact_fresh($1, 'grammar-action')
UNION ALL
SELECT 'датчик «grammar-binding» ' || fact_gap($1, 'grammar-binding')
       || ': какие действия достижимы с клавиатуры — неизвестно' AS detail
 WHERE NOT fact_fresh($1, 'grammar-binding')
UNION ALL
SELECT 'таблица грамматики не прочитана: ни одного действия в enum Action'
 WHERE NOT EXISTS (SELECT 1 FROM действия)
UNION ALL
SELECT 'Action::' || д.действие || ' — до действия не дойти с клавиатуры: строки таблицы грамматики нет'
  FROM действия д
 WHERE NOT EXISTS (SELECT 1 FROM строки с WHERE с.действие = д.действие AND с.клавиша <> '')
UNION ALL
SELECT 'клавиша «' || с.клавиша || '» занята двумя действиями: ' || string_agg(с.действие, ', ' ORDER BY с.действие)
  FROM строки с WHERE с.клавиша IS NOT NULL AND с.клавиша <> '' GROUP BY с.клавиша HAVING count(*) > 1
ORDER BY 1
