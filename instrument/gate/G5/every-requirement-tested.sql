SELECT 'таблица доказательств пуста: покрыто ли требование — неизвестно, а не «нет»' AS detail
 WHERE NOT EXISTS (SELECT 1 FROM project_requirement_proof WHERE project_id = $1)
UNION ALL
SELECT r.id || ' — ' || left(r.text, 50)
  FROM project_requirements r
 WHERE r.project_id = $1
   AND EXISTS (SELECT 1 FROM project_requirement_proof WHERE project_id = $1)
   AND NOT EXISTS (SELECT 1 FROM project_requirement_proof p
                    WHERE p.project_id = r.project_id AND p.requirement_id = r.id)
   AND true
 ORDER BY 1
