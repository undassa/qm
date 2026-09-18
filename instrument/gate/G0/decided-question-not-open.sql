SELECT l.decision_id || ' → ' || l.target AS detail FROM project_decision_links l
       JOIN project_questions q ON q.project_id=l.project_id AND q.id=l.target
      WHERE l.project_id=$1 AND l.kind='closes' AND q.state='open'
