-- ПРЕДМЕТ — выжимки этапов. Их нет — правилу нечего судить, и это говорится
-- словами `subjectWhy`, а не зелёным вердиктом и не находкой.
SELECT 1 FROM project_runs_log WHERE project_id = $1 AND is_milestone LIMIT 1
