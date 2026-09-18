SELECT t.id || ' — вердикт «blocked», а открытых находок ноль: не браться не из-за чего — находки, из-за которых задача заблокирована, называются числом' AS detail
  FROM project_plan_tasks t
  JOIN LATERAL (SELECT p.verdict, p.findings FROM preflight_verdict p
                 WHERE p.project_id = t.project_id AND p.task_id = t.id
                 ORDER BY p.at DESC LIMIT 1) v ON true
 WHERE t.project_id = $1 AND t.state <> 'closed'
   AND v.verdict = 'blocked' AND v.findings = 0
   AND true
 ORDER BY 1
