INSERT INTO code_fact (project_id, kind, name, detail) VALUES ($1, 'crate-manifest', 'crates/tot-probe/Cargo.toml', 'есть на диске') ON CONFLICT DO NOTHING
