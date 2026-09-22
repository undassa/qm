WITH д AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1, 'task', 'M9-TNOPAIR', E'# M9-TNOPAIR · подсадка самотеста', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
           RETURNING entity_name),
     з AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT $1, д.entity_name, (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9993,
                  'подсадка самотеста: задача кода без красной пары', 'S', 'closed', 'dev', 'task', д.entity_name, 'declared'
             FROM д
           RETURNING id)
INSERT INTO task_state (project_id, task_id, state, closing_commit, seen_at, closed_at)
SELECT $1, з.id, 'closed', '', (extract(epoch FROM now()) * 1000)::bigint, (extract(epoch FROM now()) * 1000)::bigint FROM з
