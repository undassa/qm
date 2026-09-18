SELECT 'датчик «protocol-item» не свеж: есть ли в коде протокол (enum Op и EventMsg) — не установлено' AS entity WHERE NOT fact_fresh($1, 'protocol-item')
UNION ALL
SELECT f.name || ' — ' || f.detail FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item' AND f.detail ~ '^pub enum (Op|EventMsg)\y'
UNION ALL
SELECT 'объявленная операция протокола ' || p.op FROM project_protocol_op p WHERE p.project_id = $1
