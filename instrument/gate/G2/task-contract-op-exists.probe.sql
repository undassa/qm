WITH пр AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
            VALUES ($1, 'design-view', 'probe-selftest-protocol', E'# протокол подсадки\n\n`ProbeKnownOp`', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest')
            RETURNING project_id),
     роль AS (INSERT INTO scheme_term (project_id, role, value, ord, why)
              SELECT пр.project_id, 'doc.protocol', 'design-view:probe-selftest-protocol', 99, 'подсадка самотеста' FROM пр RETURNING project_id),
     поле AS (INSERT INTO scheme_term (project_id, role, value, ord, why)
              SELECT роль.project_id, 'field.task-contract-ops', 'Операции подсадки', 99, 'подсадка самотеста' FROM роль RETURNING project_id),
     d AS (INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, updated_at, updated_by)
           SELECT поле.project_id, 'task', 'M9-T9990', E'# M9-T9990 · подсадка самотеста', 'probe-selftest', 0, 1, 1757000000000, 'probe-selftest' FROM поле
           RETURNING project_id),
     t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name, origin)
           SELECT d.project_id, 'M9-T9990', (SELECT min(milestone_id) FROM project_plan_tasks WHERE project_id = $1), 9990,
                  'подсадка самотеста', 'S', 'not_started', 'dev', 'task', 'M9-T9990', 'declared' FROM d
           RETURNING project_id)
INSERT INTO project_document_fields (project_id, section_ord, ord, name, shape, value_raw, value, entity_kind, entity_name)
SELECT t.project_id, 0, 0, 'Операции подсадки', 'row', '`ProbeUnknownOp`', 'ProbeUnknownOp', 'task', 'M9-T9990' FROM t
