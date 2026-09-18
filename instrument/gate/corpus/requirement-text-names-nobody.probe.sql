INSERT INTO project_requirement_sources (project_id, requirement_id, kind, target, origin) VALUES ($1,'FR-EXT-01','requirement','FR-EXT-02','projected') ON CONFLICT DO NOTHING
