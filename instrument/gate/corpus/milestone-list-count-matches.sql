WITH сказано AS (SELECT d.entity_name эт, m[1] что, m[2]::int n FROM project_documents d CROSS JOIN LATERAL regexp_matches(d.content,'<summary>(Проверки|Истории|Экраны) \((\d+)\)','g') m WHERE d.project_id = $1 AND d.entity_kind='milestone')
SELECT s.эт || ' · «' || s.что || '»: перечень называет ' || s.n || ', а в наборе ' || t.n AS detail
  FROM сказано s
  JOIN LATERAL (SELECT count(*) n FROM project_milestone_links l WHERE l.project_id = $1 AND l.milestone_id = s.эт AND l.kind = CASE s.что WHEN 'Проверки' THEN 'check' WHEN 'Истории' THEN 'story' ELSE 'screen' END) t ON true
 WHERE s.n <> t.n
   AND true
 ORDER BY 1
