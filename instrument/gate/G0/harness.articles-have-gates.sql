WITH пары AS (
  SELECT 'ART-' || lpad(a.article::text, 2, '0') AS статья, a.article AS номер, a.gate AS гейт, a.state AS статус
    FROM project_article_gates a WHERE a.project_id = $1),
реестр AS (
  SELECT m[1] AS статья, m[2] AS гейт, m[3] AS статус
    FROM project_documents d, regexp_matches(d.content, '^\s*(ART-\d+)\s+(\S+)\s+(enforced|advisory|planned)\s*$', 'gn') m
   WHERE d.project_id = $1 AND d.entity_kind = 'constitution')
SELECT 'статьи — ' || count(*) || ' в конституции, а ни одной пары статья⇄гейт не объявлено: объявите пары дверью article-gate-add (state enforced либо planned)' AS detail
  FROM project_articles a WHERE a.project_id = $1 AND NOT EXISTS (SELECT 1 FROM пары) HAVING count(*) > 0
UNION ALL
SELECT 'статья ' || a.number || ' «' || a.title || '» — ни одной пары с гейтом: статья без гейта — пожелание' FROM project_articles a WHERE a.project_id = $1 AND EXISTS (SELECT 1 FROM пары) AND NOT EXISTS (SELECT 1 FROM пары п WHERE п.номер = a.number)
UNION ALL
SELECT п.статья || ' — пара с ' || п.гейт || ' объявлена, а статьи с этим номером нет' FROM пары п WHERE NOT EXISTS (SELECT 1 FROM project_articles a WHERE a.project_id = $1 AND a.number = п.номер)
UNION ALL
SELECT п.статья || ' — гейт ' || п.гейт || ' объявлен enforced, а пункта с этим именем нет' FROM пары п WHERE п.статус = 'enforced' AND NOT EXISTS (SELECT 1 FROM gate_item g WHERE g.id = п.гейт)
UNION ALL
SELECT р.статья || ' — пара с ' || р.гейт || ' (' || р.статус || ') есть в реестре конституции, а объявлением не заведена' FROM реестр р WHERE NOT EXISTS (SELECT 1 FROM пары п WHERE п.статья = р.статья AND п.гейт = р.гейт AND п.статус = р.статус)
UNION ALL
SELECT п.статья || ' — пара с ' || п.гейт || ' объявлена, а в реестре конституции её нет' FROM пары п WHERE EXISTS (SELECT 1 FROM реестр) AND NOT EXISTS (SELECT 1 FROM реестр р WHERE р.статья = п.статья AND р.гейт = п.гейт)
ORDER BY 1
