SELECT l.decision_id || ' → статья ' || l.target AS detail FROM project_decision_links l
      LEFT JOIN project_articles a ON a.project_id=l.project_id AND a.number=NULLIF(l.target,'')::int
     WHERE l.project_id=$1 AND l.kind='amends-article' AND a.number IS NULL
