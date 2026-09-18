WITH holder AS (
  SELECT g.phase AS gate, g.id AS rule, p.ord AS gate_ord,
         btrim(replace(replace(f.value, '`', ''), chr(160), ' ')) AS holder
    FROM project_gates g
    JOIN phase p ON p.gate = g.phase
    JOIN project_documents d
      ON d.project_id = g.project_id AND d.entity_kind = 'question'
     AND d.content ~ ('\y' || g.id || '\y')
    JOIN project_document_fields f
      ON f.project_id = d.project_id AND f.entity_kind = d.entity_kind
     AND f.entity_name = d.entity_name AND f.name = 'Держатель'
     AND btrim(f.value) <> ''
   WHERE g.project_id = $1 AND g.state = 'failed'),
петля AS (
  SELECT h.gate, h.rule,
         string_agg(DISTINCT h.holder || ' — ' || coalesce(tp.phase, 'фаза не объявлена'), ' · ') AS named,
         bool_and(coalesce(tp.phase_ord, -1) > h.gate_ord) AS позже
    FROM holder h
    LEFT JOIN task_phase tp ON tp.project_id = $1 AND tp.task_id = h.holder
   GROUP BY 1, 2)
SELECT gate || ' · ' || rule || ' — красен, и каждый названный держатель стоит позже его фазы ('
       || named || '): петля, а не ожидание' AS detail
  FROM петля
 WHERE позже
   AND true
 ORDER BY 1
