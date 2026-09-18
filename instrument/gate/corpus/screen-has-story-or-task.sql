SELECT s.id || ' — ' || left(s.title, 50) AS detail
  FROM project_screens s
 WHERE s.project_id = $1 AND s.out_of_version = ''
   AND NOT EXISTS (SELECT 1 FROM project_screen_references r
                    WHERE r.project_id = s.project_id AND r.screen_id = s.id)
   AND true
