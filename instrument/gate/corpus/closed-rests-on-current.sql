SELECT l.kind || ' ' || l.id || ' — ' || l.why AS detail
  FROM entity_live l
 WHERE l.project_id = $1 AND l.live_state = 'reopened'
   AND (   (l.kind IN ('task','red-task') AND EXISTS (SELECT 1 FROM project_plan_tasks t WHERE t.project_id = $1 AND t.id = l.id AND t.state = 'closed'))
        OR (l.kind = 'milestone'          AND EXISTS (SELECT 1 FROM project_plan_milestones m WHERE m.project_id = $1 AND m.id = l.id AND m.closed <> ''))
        OR (l.kind = 'requirement'        AND EXISTS (SELECT 1 FROM project_requirements r WHERE r.project_id = $1 AND r.id = l.id AND r.satisfied)))
   AND true
 ORDER BY 1
