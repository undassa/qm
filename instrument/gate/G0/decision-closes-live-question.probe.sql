INSERT INTO project_decision_links (project_id, decision_id, kind, target) VALUES ($1, 'ADR-0001', 'closes', 'Q-99999') ON CONFLICT DO NOTHING
