WITH d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1, 'process', 'probe-selftest-9998', E'# подсадка самотеста\n\nОбряд исполняет Q-9998.', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
           RETURNING project_id)
INSERT INTO project_questions (project_id, id, number, title, state, origin)
SELECT d.project_id, x.id, x.n, 'подсадка самотеста', 'decided', 'declared'
  FROM d, (VALUES ('Q-9998', 9998), ('Q-9999', 9999)) AS x(id, n)
