INSERT INTO code_fact (project_id, kind, name, detail)
VALUES ($1::text, 'surface-blocktype-wildcard', 'crates/tot-ui/src/probe-selftest.rs', 'match b { BlockType::Hero => (), _ => () }')
