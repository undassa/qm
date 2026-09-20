WITH границы AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         substring(f.detail from '^pub (?:enum|struct) (\w+)') AS тип
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'grammar-type'),
действия AS (
  SELECT DISTINCT substring(v.detail from '^([A-Z][A-Za-z]*)') AS действие
    FROM code_fact v
   WHERE v.project_id = $1 AND v.kind = 'grammar-action'
     AND substring(v.detail from '^([A-Z][A-Za-z]*)') IS NOT NULL
     AND (SELECT г.тип FROM границы г
           WHERE г.файл = substring(v.name from '^(.*):\d+$')
             AND г.н < substring(v.name from ':(\d+)$')::int AND г.тип IS NOT NULL
           ORDER BY г.н DESC LIMIT 1) = 'Action'),
строки AS (
  -- Имя действия берётся ПЕРЕД стрелкой, а не с начала строки. Рукав таблицы
  -- пишут `Self::Copy => "y",`, и образец «с начала» вынимал из него `Self` —
  -- один и тот же для всех одиннадцати. Ни одно действие не находило своей
  -- строки, и пункт сообщал, что до всей поверхности не дойти с клавиатуры.
  -- Набор `tot-ade` принёс это заявкой 200 и правильно не стал подгонять
  -- датчик под запрос: датчик снимал ровно то, что просили.
  SELECT DISTINCT substring(b.detail from '([A-Za-z0-9_]+)\s*=>') AS действие,
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
SELECT 'датчик «grammar-type» ' || fact_gap($1, 'grammar-type')
       || ': чей это вариант — неизвестно, а без границы типа в счёт идут чужие' AS detail
 WHERE NOT fact_fresh($1, 'grammar-type')
UNION ALL
SELECT 'таблица грамматики не прочитана: ни одного действия в enum Action'
 WHERE fact_fresh($1, 'grammar-action') AND fact_fresh($1, 'grammar-type')
   AND NOT EXISTS (SELECT 1 FROM действия)
UNION ALL
SELECT 'Action::' || д.действие || ' — до действия не дойти с клавиатуры: строки таблицы грамматики нет'
  FROM действия д
 WHERE NOT EXISTS (SELECT 1 FROM строки с WHERE с.действие = д.действие AND с.клавиша <> '')
UNION ALL
SELECT 'клавиша «' || с.клавиша || '» занята двумя действиями: ' || string_agg(с.действие, ', ' ORDER BY с.действие)
  FROM строки с WHERE с.клавиша IS NOT NULL AND с.клавиша <> ''
   AND с.действие IN (SELECT действие FROM действия)
 GROUP BY с.клавиша HAVING count(*) > 1
ORDER BY 1
