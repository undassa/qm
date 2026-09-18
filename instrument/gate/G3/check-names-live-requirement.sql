SELECT c.id FROM project_checks c
             LEFT JOIN project_requirements r ON r.project_id=c.project_id AND r.id=c.requirement_id
            WHERE c.project_id=$1 AND c.requirement_id IS NOT NULL AND r.id IS NULL
