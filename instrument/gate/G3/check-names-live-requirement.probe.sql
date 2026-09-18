INSERT INTO project_checks (project_id, id, area, requirement_id, spec, entity_kind, entity_name) VALUES ($1,'TC-PROBE-01','PROBE','FR-НЕТ-99','проба','test-cases','') ON CONFLICT DO NOTHING
