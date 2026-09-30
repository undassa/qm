SELECT 'датчик «dead-stack» ' || fact_gap($1, 'dead-stack') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'dead-stack') UNION ALL SELECT f.name || '  ' || f.detail AS detail
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'dead-stack'
   AND true
 ORDER BY 1
