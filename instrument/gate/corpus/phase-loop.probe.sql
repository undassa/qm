WITH pick AS (
  SELECT g.phase AS gate, g.id AS rule,
         (SELECT tp.task_id FROM task_phase tp
           WHERE tp.project_id = $1 AND tp.phase_ord > p.ord
           ORDER BY tp.task_id LIMIT 1) AS holder
    FROM project_gates g
    JOIN phase p ON p.gate = g.phase
   WHERE g.project_id = $1
     AND EXISTS (SELECT 1 FROM task_phase tp
                  WHERE tp.project_id = $1 AND tp.phase_ord > p.ord)
     AND NOT EXISTS (SELECT 1 FROM project_documents d
                       JOIN project_document_fields f
                         ON f.project_id = d.project_id AND f.entity_kind = 'question'
                        AND f.entity_name = d.entity_name AND f.name = 'Держатель'
                      WHERE d.project_id = $1 AND d.entity_kind = 'question'
                        AND d.content ~ ('\y' || g.id || '\y'))
   ORDER BY p.ord, g.id LIMIT 1),
doc AS (
  INSERT INTO project_documents (project_id, entity_kind, entity_name, content,
                                 content_hash, bytes, revision, updated_at, updated_by)
  SELECT $1, 'question', 'Q-ПРОБА', 'подсадка самотеста: пункт ' || rule,
         '', 0, 0, 0, 'самотест'
    FROM pick
  RETURNING entity_name),
field AS (
  INSERT INTO project_document_fields (project_id, section_ord, ord, name, shape,
                                       value_raw, value, entity_kind, entity_name)
  SELECT $1, 0, 0, 'Держатель', 'row', pick.holder, pick.holder, 'question', doc.entity_name
    FROM pick, doc
  RETURNING 1)
UPDATE project_gates
   SET state = 'failed',
       result = jsonb_set(coalesce(result, '{}'::jsonb), '{computed}', to_jsonb('failed'::text))
 WHERE project_id = $1 AND (phase, id) IN (SELECT gate, rule FROM pick)
