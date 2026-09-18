WITH границы AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         substring(f.detail from '^pub (?:enum|struct) (\w+)') AS тип
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item'),
варианты AS (
  SELECT substring(v.name from '^(.*):\d+$') AS файл, substring(v.detail from '^([A-Z][A-Za-z]*)') AS вариант,
         (SELECT б.тип FROM границы б
           WHERE б.файл = substring(v.name from '^(.*):\d+$') AND б.н < substring(v.name from ':(\d+)$')::int
           ORDER BY б.н DESC LIMIT 1) AS тип
    FROM code_fact v WHERE v.project_id = $1 AND v.kind = 'protocol-variant'),
опы AS (SELECT DISTINCT вариант FROM варианты WHERE тип = 'Op' AND вариант IS NOT NULL),
события AS (SELECT DISTINCT вариант FROM варианты WHERE тип = 'EventMsg' AND вариант IS NOT NULL),
тело AS (
  SELECT б.файл, б.н AS от, (SELECT min(з.н) FROM границы з WHERE з.файл = б.файл AND з.н > б.н AND з.тип IS NULL) AS до
    FROM границы б WHERE б.тип = 'Op'),
слова AS (
  SELECT w.name, w.detail FROM code_fact w JOIN тело ON тело.файл = substring(w.name from '^(.*):\d+$')
   WHERE w.project_id = $1 AND w.kind = 'op-body-word'
     AND substring(w.name from ':(\d+)$')::int BETWEEN тело.от AND coalesce(тело.до, 2147483647))
SELECT 'датчик «protocol-variant» ' || fact_gap($1, 'protocol-variant') || ': что умеет протокол — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-variant')
UNION ALL
SELECT 'датчик «resume-carries-diff» ' || fact_gap($1, 'resume-carries-diff') || ': несёт ли возврат дифф — неизвестно' AS detail WHERE NOT fact_fresh($1, 'resume-carries-diff')
UNION ALL
SELECT 'протокол — enum Op или EventMsg не разобран: протокол не прочитан' WHERE NOT EXISTS (SELECT 1 FROM опы) OR NOT EXISTS (SELECT 1 FROM события)
UNION ALL
SELECT 'SessionResumed — события нет: возврат в сессию ничем не описан' WHERE EXISTS (SELECT 1 FROM события) AND NOT EXISTS (SELECT 1 FROM события WHERE вариант = 'SessionResumed')
UNION ALL
SELECT 'SessionResumed — не несёт PageDiff: форма возврата не диффом' WHERE EXISTS (SELECT 1 FROM события WHERE вариант = 'SessionResumed') AND NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'resume-carries-diff')
UNION ALL
SELECT 'Op::' || вариант || ' — отдаёт пересказ вместо диффа' FROM опы WHERE вариант ~ '(Summary|Recap|Digest|Retell)'
UNION ALL
SELECT 'EventMsg::' || вариант || ' — отдаёт пересказ вместо диффа' FROM события WHERE вариант ~ '(Summary|Recap|Digest|Retell)'
ORDER BY 1
