INSERT INTO test_run (project_id, check_name, commit_sha, dirty, verdict, at, actor)
VALUES ($1::text, 'PROBE-TRUNK-GREEN', 'probe', false, 'failed', (extract(epoch from now()) * 1000)::bigint, 'probe')
