WITH d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1, 'task', 'M9-T9990', E'# M9-T9990 · подсадка самотеста\n\nотменена и удалена, называет TC-PROBE-9990 и ~~TC-PROBE-9991~~', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
           RETURNING project_id)
INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
SELECT d.project_id, 'M9-T9990', (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9990,
       'подсадка самотеста', 'S', 'not_started', 'dev', 'task', 'M9-T9990', 'declared'
  FROM d
