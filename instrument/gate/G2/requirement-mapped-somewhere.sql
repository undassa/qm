-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
SELECT 'датчик контракта ' || fact_gap($1, 'requirement-in-contract') || ': назван ли требованием контракт — неизвестно, а не «названо»' AS detail WHERE NOT fact_fresh($1, 'requirement-in-contract')
UNION ALL
SELECT r.id FROM project_requirements r WHERE r.project_id = $1 AND r.kind = 'FR'
  AND fact_fresh($1, 'requirement-in-contract')
  AND NOT EXISTS (SELECT 1 FROM requirement_scope rs WHERE rs.project_id = r.project_id AND rs.requirement_id = r.id AND rs.scope IN ('next','later'))
  AND NOT EXISTS (SELECT 1 FROM project_named_id n WHERE n.project_id = r.project_id AND n.entity_kind IN ('design-view','data-model','decision') AND n.said_id = r.id)
  AND NOT EXISTS (SELECT 1 FROM code_fact c WHERE c.project_id = r.project_id AND c.kind = 'requirement-in-contract' AND c.name = r.id)
ORDER BY 1
