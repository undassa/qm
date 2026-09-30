SELECT 'датчик «docs-path» ' || fact_gap($1, 'docs-path') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'docs-path') UNION ALL SELECT f.name || '  ' || f.detail AS detail
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'docs-path'
   AND true
 ORDER BY 1
