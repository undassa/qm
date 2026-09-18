INSERT INTO project_decisions (project_id, id, number, title, entity_kind, entity_name, status) VALUES ($1,'ADR-9998',9998,'проба','decision','ADR-9998','accepted') ON CONFLICT DO NOTHING
