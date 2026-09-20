WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-ready-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'READY-PROBE', v.id, 9990, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'READY-PROBE', m.id, 1, 'проба самотеста', '', 'closed', 'dev', 'task', 'READY-PROBE' FROM m RETURNING id)
INSERT INTO task_ready_item (project_id, task_id, ord, check_id, text, done, origin)
SELECT $1, t.id, 1, 'probe_check', 'проба самотеста', false, 'проба' FROM t
