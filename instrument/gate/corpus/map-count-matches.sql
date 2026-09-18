WITH строка AS (
  SELECT l.line
    FROM project_documents d
    CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') AS l(line)
   WHERE d.project_id = $1 AND d.entity_kind = 'map' AND l.line ~ '^\|.*\|\s*[0-9]+\s*\|\s*$'),
часть AS (
  SELECT s.line, regexp_match(p.part, '^\s*\[`([a-z][a-z0-9-]*)(:([^`]*))?`\]\([^)]*\)\s*$') AS m
    FROM строка s CROSS JOIN LATERAL regexp_split_to_table(split_part(s.line, '|', 2), '·') AS p(part)),
счёт AS (
  SELECT ч.line,
         bool_and(ч.m IS NOT NULL AND EXISTS (SELECT 1 FROM kind_layout k WHERE k.name = ч.m[1])) AS виды,
         string_agg(coalesce(ч.m[1] || CASE WHEN coalesce(ч.m[3], '') = '' THEN '' ELSE ':' || ч.m[3] END, '?'), ' · ') AS что,
         sum(CASE WHEN ч.m IS NULL THEN 0
                  WHEN coalesce(ч.m[3], '') = ''
                    THEN (SELECT count(*) FROM project_documents x WHERE x.project_id = $1 AND x.entity_kind = ч.m[1])
                  ELSE (SELECT count(*) FROM project_documents x
                         WHERE x.project_id = $1 AND x.entity_kind = ч.m[1] AND x.entity_name = ч.m[3]) END) AS документов
    FROM часть ч GROUP BY ч.line)
SELECT 'карта: ' || с.что || ' — сказано документов ' || (regexp_match(с.line, '\|\s*([0-9]+)\s*\|\s*$'))[1]
       || ', а в наборе ' || с.документов AS detail
  FROM счёт с
 WHERE с.виды AND (regexp_match(с.line, '\|\s*([0-9]+)\s*\|\s*$'))[1]::bigint <> с.документов
 ORDER BY 1
