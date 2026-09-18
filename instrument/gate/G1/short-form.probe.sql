INSERT INTO project_named_id(project_id, entity_kind, entity_name, said_id, caveated, from_range, heads_row)
SELECT $1, 'story', f.story_id, r.id, false, false, true
  FROM project_feature_stories f, project_requirements r
 WHERE f.project_id = $1 AND r.project_id = $1 AND r.id LIKE 'FR-%'
   AND NOT EXISTS (SELECT 1 FROM project_named_id m WHERE m.project_id = $1 AND m.entity_kind = 'feature' AND m.entity_name = f.feature_id AND m.said_id = r.id)
   AND NOT EXISTS (SELECT 1 FROM project_named_id n WHERE n.project_id = $1 AND n.entity_kind = 'story' AND n.entity_name = f.story_id AND n.said_id = r.id)
 LIMIT 1
