INSERT INTO project_screens (project_id, id, title, entity_kind, entity_name, area) VALUES ($1,'SCR-PROBE-01','проба самотеста','screen','SCR-PROBE-01','probe') ON CONFLICT DO NOTHING
