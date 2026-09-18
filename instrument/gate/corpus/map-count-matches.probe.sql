INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
VALUES ($1, 'map', '', E'| [`map:`](map:) | подсадка самотеста | 9999 |', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
ON CONFLICT (project_id, entity_kind, entity_name)
DO UPDATE SET content = project_documents.content || E'\n| [`map:`](map:) | подсадка самотеста | 9999 |'
