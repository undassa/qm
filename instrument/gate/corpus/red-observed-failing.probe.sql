WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-red-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'RED-PROBE', v.id, 9991, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'R-RED-PROBE', m.id, 1, 'проба самотеста', '', 'closed', 'red', 'red-task', 'R-RED-PROBE' FROM m RETURNING id),
c AS (INSERT INTO project_task_check (project_id, task_id, check_id, said_as) SELECT $1, t.id, 'probe_red_check', 'красная фаза' FROM t RETURNING 1)
INSERT INTO test_run (project_id, check_name, commit_sha, dirty, verdict, at, actor)
VALUES ($1, 'probe_other_check', 'probe', false, 'failed', (extract(epoch from now()) * 1000)::bigint, 'probe'),
       ($1, 'probe_red_check', 'probe', true, 'failed', (extract(epoch from now()) * 1000)::bigint, 'probe')
