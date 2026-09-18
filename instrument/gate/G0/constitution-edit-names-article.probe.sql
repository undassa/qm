INSERT INTO project_decision_links (project_id, decision_id, kind, target) VALUES ($1,'ADR-0001','amends-article','999') ON CONFLICT DO NOTHING
