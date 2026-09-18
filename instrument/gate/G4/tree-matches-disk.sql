SELECT 'датчик «tree-file» ' || fact_gap($1, 'tree-file') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'tree-file')
UNION ALL
SELECT f.name || '  —  ' || f.detail FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'tree-file'
   AND (f.detail ~ '^объявлено есть( · .*)?, на диске нет$' OR f.detail ~ '^объявлено нет( · .*)?, на диске есть$')
UNION ALL
SELECT f.name || '  —  состояние пути не узнано: в дереве проекта пишется «есть» или «нет», пояснение — после « · »'
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'tree-file'
   AND f.detail !~ '^объявлено (есть|нет)( · .*)?, на диске (есть|нет)$'
 ORDER BY 1
