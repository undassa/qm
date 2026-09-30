SELECT 'датчик «retired-term» ' || fact_gap($1, 'retired-term') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'retired-term') UNION ALL SELECT f.name || '  ' || f.detail AS detail
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'retired-term'
   AND true
 ORDER BY 1
