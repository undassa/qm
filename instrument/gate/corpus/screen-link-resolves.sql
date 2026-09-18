SELECT r.source || ' → ' || r.screen_id AS detail FROM project_screen_references r
            LEFT JOIN project_screens s ON s.project_id=r.project_id AND s.id=r.screen_id
           WHERE r.project_id=$1 AND s.id IS NULL
