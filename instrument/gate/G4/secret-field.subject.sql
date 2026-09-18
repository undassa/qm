SELECT f.name AS entity FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'crate-manifest'
