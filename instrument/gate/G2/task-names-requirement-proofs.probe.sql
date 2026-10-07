-- Подсадка — в открытую версию: пункт судит только её (решение владельца 2026-10-07).
WITH r AS (INSERT INTO project_requirements (project_id, id, kind, area, text, satisfied, origin)
           VALUES ($1, 'FR-PROBE-9990', 'FR', 'PROBE', 'подсадка самотеста', false, 'declared') RETURNING project_id),
     p AS (INSERT INTO project_requirement_proof (project_id, requirement_id, proof_kind, proof_id)
           SELECT r.project_id, 'FR-PROBE-9990', 'check', 'TC-PROBE-9990' FROM r RETURNING project_id),
     d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           SELECT p.project_id, 'task', 'M9-T9990', E'# M9-T9990 · подсадка самотеста', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest' FROM p
           RETURNING project_id),
     t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT d.project_id, 'M9-T9990', (SELECT m.id FROM project_plan_milestones m JOIN version_scope s ON s.project_id = m.project_id AND s.version = m.version_id WHERE m.project_id = $1 ORDER BY CASE coalesce(s.scope, 'open') WHEN 'open' THEN 0 WHEN 'closed' THEN 1 ELSE 2 END, m.id LIMIT 1), 9990,
                  'подсадка самотеста', 'S', 'not_started', 'dev', 'task', 'M9-T9990', 'declared' FROM d
           RETURNING project_id)
INSERT INTO task_requirement (project_id, task_id, requirement_id, origin)
SELECT t.project_id, 'M9-T9990', 'FR-PROBE-9990', 'declared' FROM t
