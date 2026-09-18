SELECT tp.task_id || ' — закрыта, а фаза ' || tp.phase
       || ' не открыта: её открывает ' || h.gate AS detail
  FROM task_phase tp
  JOIN LATERAL (SELECT p2.gate FROM phase p2
                 WHERE p2.ord < tp.phase_ord AND p2.gate <> ''
                   AND coalesce((SELECT s.computed FROM gate_state s
                                  WHERE s.project_id = tp.project_id
                                    AND s.gate = p2.gate), 'open') <> 'passed'
                 ORDER BY p2.ord LIMIT 1) h ON true
 WHERE tp.project_id = $1 AND tp.state = 'closed' AND tp.open IS FALSE
   AND true
UNION ALL
SELECT tp.task_id || ' — закрыта, а вид «' || tp.kind
       || '» не отображён ни на одну фазу: судить о порядке нечем'
  FROM task_phase tp
 WHERE tp.project_id = $1 AND tp.state = 'closed' AND tp.phase IS NULL
   AND true
 ORDER BY 1
