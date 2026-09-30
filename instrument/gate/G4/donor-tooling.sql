SELECT 'датчик «donor-tooling» ' || fact_gap($1, 'donor-tooling') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'donor-tooling') UNION ALL SELECT f.name || '  ' || f.detail AS detail
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'donor-tooling'
   AND true
 ORDER BY 1
