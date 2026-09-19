SELECT m[1] AS entity
  FROM project_documents d,
       regexp_matches(d.content, '^\s*(ART-22\s+render:blocks-from-library\s+\S+)\s*$', 'gn') m
 WHERE d.project_id = $1 AND d.entity_kind = 'constitution'
