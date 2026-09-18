SELECT 'пересборка проекций не удалась: решения набора не прочитаны' AS entity WHERE NOT EXISTS (SELECT 1 FROM reproject_state r WHERE r.project_id = $1 AND r.ok)
UNION ALL
SELECT р.entity_name FROM project_decisions р WHERE р.project_id = $1
