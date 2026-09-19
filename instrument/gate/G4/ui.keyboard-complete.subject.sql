SELECT 'датчик «grammar-action» не свеж: есть ли у поверхности перечень действий — не установлено' AS entity
 WHERE NOT fact_fresh($1, 'grammar-action')
UNION ALL
SELECT 'строка реестра constitution: ' || a.article || '  ' || a.gate
  FROM project_article_gates a
 WHERE a.project_id = $1 AND a.gate = 'ui:keyboard-complete'
