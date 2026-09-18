SELECT s.fact || ' — ' || CASE WHEN f.at IS NULL THEN 'ни разу не подавал'
                               ELSE 'протух: последняя подача старше объявленного срока' END AS detail
  FROM sensor s LEFT JOIN fact_push f ON f.project_id = s.project_id AND f.fact = s.fact
 WHERE s.project_id = $1 AND NOT fact_fresh(s.project_id, s.fact)
 ORDER BY 1
