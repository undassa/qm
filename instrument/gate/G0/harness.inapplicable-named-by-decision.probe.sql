-- Подсадка: пункт, которого не называет ни одно решение, объявлен неприменимым.
-- Правило обязано назвать его поимённо.
UPDATE project_gates SET applicable = false
 WHERE project_id = $1 AND id = (
   SELECT g.id FROM project_gates g
    WHERE g.project_id = $1
      AND NOT EXISTS (SELECT 1 FROM project_documents d
                       WHERE d.project_id = $1 AND d.entity_kind = 'decision'
                         AND position(g.id IN d.content) > 0)
    ORDER BY g.phase, g.id LIMIT 1)
