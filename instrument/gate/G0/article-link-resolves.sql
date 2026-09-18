SELECT r.entity_kind || ' ' || r.entity_name || ' → Article ' || r.number AS detail
            FROM project_article_references r
            LEFT JOIN project_articles a ON a.project_id=r.project_id AND a.number=r.number
           WHERE r.project_id=$1 AND a.number IS NULL
