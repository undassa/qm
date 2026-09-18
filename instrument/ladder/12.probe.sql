INSERT INTO version_state (project_id, version, state, changed_at, changed_by)
SELECT $1, 'проба-самотеста', 'closed', 1, 'самотест'
ON CONFLICT (project_id, version) DO UPDATE SET state = 'closed'
