SELECT split_part(f.name, ' ', 1) || ' → ' || f.detail AS entity FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'contract-schema' AND f.detail LIKE 'помечено x-source: task:%'
