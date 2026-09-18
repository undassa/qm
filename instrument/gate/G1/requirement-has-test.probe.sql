INSERT INTO project_requirements (project_id, id, kind, area, text, entity_kind, entity_name, satisfied) VALUES ($1,'FR-PROBE-01','FR','PROBE','проба самотеста','srs','',false) ON CONFLICT DO NOTHING
