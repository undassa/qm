WITH крейты AS (
  SELECT c.name AS крейт, m.name AS манифест, regexp_replace(m.name, 'Cargo\.toml$', '') AS каталог
    FROM code_fact m JOIN code_fact c ON c.project_id = m.project_id AND c.kind = 'crate' AND substring(c.detail from 'названо в (.*)$') = m.name
   WHERE m.project_id = $1 AND m.kind = 'crate-manifest'),
разрешено AS (
  SELECT split_part(s.value, ':', 1) AS крейт, nullif(split_part(s.value, ':', 2), '') AS модуль
    FROM scheme($1) s WHERE s.role = 'unsafe.allowed'),
места AS (
  SELECT f.name, k.крейт, substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         EXISTS (SELECT 1 FROM разрешено р WHERE р.крейт = k.крейт) AS крейт_разрешён,
         EXISTS (SELECT 1 FROM разрешено р WHERE р.крейт = k.крейт
                   AND (р.модуль IS NULL OR position(р.модуль in substring(f.name from length(k.каталог) + 5)) > 0)) AS можно
    FROM code_fact f JOIN крейты k ON left(f.name, length(k.каталог || 'src/')) = k.каталог || 'src/'
   WHERE f.project_id = $1 AND f.kind = 'unsafe-site')
SELECT 'датчик «unsafe-site» ' || fact_gap($1, 'unsafe-site') || ': где живёт unsafe — неизвестно' AS detail WHERE NOT fact_fresh($1, 'unsafe-site')
UNION ALL
SELECT 'датчик «safety-note» ' || fact_gap($1, 'safety-note') || ': объяснён ли unsafe — неизвестно' WHERE NOT fact_fresh($1, 'safety-note')
UNION ALL
SELECT 'датчик «crate» ' || fact_gap($1, 'crate') || ': какому крейту принадлежит место — неизвестно' WHERE NOT fact_fresh($1, 'crate')
UNION ALL
SELECT 'где разрешён unsafe, не объявлено: мест unsafe ' || count(*) || ' — объявите дверью scheme-term-set role=unsafe.allowed value=«крейт» либо «крейт:модуль»' FROM места WHERE NOT EXISTS (SELECT 1 FROM разрешено) HAVING count(*) > 0
UNION ALL
SELECT м.name || ' — unsafe вне объявленных крейтов' FROM места м WHERE EXISTS (SELECT 1 FROM разрешено) AND NOT м.крейт_разрешён
UNION ALL
SELECT м.name || ' — unsafe в ' || м.крейт || ' вне объявленной границы' FROM места м WHERE м.крейт_разрешён AND NOT м.можно
UNION ALL
SELECT м.name || ' — unsafe без // SAFETY: в трёх строках выше' FROM места м WHERE м.можно AND NOT EXISTS (SELECT 1 FROM code_fact s WHERE s.project_id = $1 AND s.kind = 'safety-note' AND substring(s.name from '^(.*):\d+$') = м.файл AND substring(s.name from ':(\d+)$')::int BETWEEN м.н - 3 AND м.н)
ORDER BY 1
