SELECT m[1] AS entity
  FROM project_documents d,
       regexp_matches(d.content, '^\s*(ART-18\s+page:derived-not-stored\s+\S+)\s*$', 'gn') m
 WHERE d.project_id = $1 AND d.entity_kind = 'constitution'
