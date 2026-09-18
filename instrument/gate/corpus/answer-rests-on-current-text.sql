SELECT id || ' — ' || why AS detail FROM question_live WHERE project_id = $1 AND live_state = 'reopened' AND true
UNION ALL
SELECT 'судить нечем у ' || count(*) || ' вопросов: закрыты, а когда стоял ответ — не сказано' FROM question_live WHERE project_id = $1 AND state <> 'open' AND updated_at IS NULL AND coalesce(nullif(why,''),'') <> '' HAVING count(*) > 0
ORDER BY 1
