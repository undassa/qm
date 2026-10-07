-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
SELECT 'датчик проверок ' || fact_gap($1, 'requirement-test') || ', и доказательств в наборе нет: покрыто ли требование — неизвестно' AS detail
 WHERE NOT EXISTS (SELECT 1 FROM project_requirement_proof WHERE project_id = $1)
   AND NOT fact_fresh($1, 'requirement-test')
UNION ALL
SELECT r.id || CASE WHEN EXISTS (SELECT 1 FROM code_fact f WHERE f.project_id = r.project_id AND f.kind = 'requirement-test' AND f.name = r.id)
                    THEN ' — доказано только фактом датчика «requirement-test», а датчик ' || fact_gap($1, 'requirement-test') || ': такой факт доказательством не считается'
                    ELSE ' — нечем доказано: ни одного доказательства рода, объявленного годным' END FROM project_requirements r
 WHERE r.project_id = $1 AND r.kind = 'FR'
   AND (EXISTS (SELECT 1 FROM project_requirement_proof WHERE project_id = $1)
        OR fact_fresh($1, 'requirement-test'))
   AND NOT requirement_proved($1, r.id)
   AND NOT EXISTS (SELECT 1 FROM requirement_scope rs WHERE rs.project_id = r.project_id AND rs.requirement_id = r.id AND rs.scope IN ('next','later'))
 ORDER BY 1
