-- Судится открытая версия (и закрытые): задачи и требования следующей и поздних
-- версий её не держат (решение владельца 2026-10-07). Без открытой версии — весь набор.
SELECT t.id || ' — ' || tr.requirement_id || ' доказывается ' || string_agg(DISTINCT p.proof_id, ', ' ORDER BY p.proof_id)
       || ', а задача этого не называет' AS detail
  FROM project_plan_tasks t
  JOIN task_requirement tr ON tr.project_id = t.project_id AND tr.task_id = t.id
  JOIN project_requirement_proof p ON p.project_id = t.project_id AND p.requirement_id = tr.requirement_id
  JOIN project_documents d ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
 WHERE t.project_id = $1 AND t.state <> 'closed' AND t.kind = 'dev'
   AND NOT EXISTS (SELECT 1 FROM task_scope sc WHERE sc.project_id = t.project_id AND sc.task_id = t.id AND sc.scope IN ('next','later'))
   AND d.content !~ ('(^|[^A-Za-z0-9_-])' || regexp_replace(p.proof_id, '([.*+?^${}()|\[\]\\])', '\\\1', 'g') || '([^A-Za-z0-9_-]|$)')
 GROUP BY t.id, tr.requirement_id
 ORDER BY 1
