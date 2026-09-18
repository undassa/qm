SELECT f.path AS detail FROM project_code_file f WHERE f.project_id = $1 AND f.base ~ '^(utils?|helpers?|common|shared|misc|models?|types|stuff|base)[.](rs|ts|tsx)$' AND true ORDER BY 1
