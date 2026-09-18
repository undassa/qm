SELECT 'датчик «workspace-lint» не свеж: запрещён ли print_stdout воркспейсом — не установлено' AS entity WHERE NOT fact_fresh($1, 'workspace-lint')
UNION ALL
SELECT f.name || ' — ' || f.detail FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'workspace-lint' AND trim(f.detail) ~ '^print_stdout\s*=\s*("(deny|forbid)"|\{[^}]*level\s*=\s*"(deny|forbid)")'
UNION ALL
SELECT 'lint.floor · ' || k.value FROM scheme($1) k WHERE k.role = 'lint.floor' AND k.value ~ 'print_stdout\s+(deny|forbid)'
