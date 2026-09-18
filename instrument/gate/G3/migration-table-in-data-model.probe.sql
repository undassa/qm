INSERT INTO code_fact (project_id, kind, name, detail) VALUES ($1,'migration-table','проба_таблица','0000_проба.sql') ON CONFLICT DO NOTHING
