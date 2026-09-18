SELECT t.id || ' — ' || CASE
         WHEN v.seen IS NULL THEN 'предполёта не было'
         WHEN v.seen = 0 THEN 'предполёт был, но правка на тот момент неизвестна'
         WHEN v.blocked THEN 'вердикт: не браться — ' || v.findings || ' находок'
         WHEN v.open_findings > 0 THEN 'открытых находок ' || v.open_findings
              || ': починить документ и повторить предполёт на новой правке; ответ нужен владельцу — question-add и вердикт blocked'
         ELSE 'предполёт был до правки задачи: правка ' || d.revision || ', вердикт на ' || v.seen
       END AS detail
  FROM project_plan_tasks t
  JOIN project_documents d
    ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
  JOIN task_phase tp
    ON tp.project_id = t.project_id AND tp.task_id = t.id AND tp.open
  LEFT JOIN LATERAL (
       SELECT max(p.task_revision) AS seen,
              bool_or(p.verdict = 'blocked' AND p.task_revision = d.revision) AS blocked,
              max(p.findings) AS findings,
              coalesce((SELECT q.findings FROM preflight_verdict q
                         WHERE q.project_id = t.project_id AND q.task_id = t.id
                         ORDER BY q.at DESC LIMIT 1), 0) AS open_findings
         FROM preflight_verdict p
        WHERE p.project_id = t.project_id AND p.task_id = t.id) v ON true
 WHERE t.project_id = $1 AND t.state <> 'closed'
   AND (v.seen IS DISTINCT FROM d.revision OR v.blocked OR v.open_findings > 0)
 ORDER BY t.milestone_id, t.ord
