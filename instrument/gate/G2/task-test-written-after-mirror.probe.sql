WITH d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           VALUES ($1, 'task', 'M9-T9990', E'# M9-T9990 · подсадка самотеста\n\n| Проверка | Источник |\n|---|---|\n| `probe_selftest_absent_test` | подсадка |', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
           RETURNING project_id),
     t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT d.project_id, x.id, (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9990,
                  'подсадка самотеста', 'S', x.state, x.kind, x.ek, x.id, 'declared'
             FROM d, (VALUES ('M9-T9990', 'not_started', 'dev', 'task'), ('V9-T9990', 'closed', 'red', 'red-task')) AS x(id, state, kind, ek)
           RETURNING project_id),
     r AS (INSERT INTO red_task (project_id, id, parent_task, milestone, checks)
           SELECT DISTINCT t.project_id, 'V9-T9990', 'M9-T9990', (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 1 FROM t
           RETURNING project_id),
     c AS (INSERT INTO project_task_check (project_id, task_id, check_id, said_as)
           SELECT r.project_id, 'M9-T9990', 'probe_selftest_absent_test', 'доказательство' FROM r
           RETURNING project_id)
INSERT INTO fact_push (project_id, fact, at, actor, rows)
SELECT c.project_id, 'test-name', (extract(epoch from now()) * 1000)::bigint, 'probe-selftest', 1 FROM c
ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at
