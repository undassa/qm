INSERT INTO project_requirements (project_id, id, kind, area, text, entity_kind, entity_name, satisfied) VALUES ($1,'NFR-PROBE-02','NFR','','проба самотеста','srs','',false) ON CONFLICT DO NOTHING
