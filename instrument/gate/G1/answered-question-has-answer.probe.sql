UPDATE project_questions SET answer_state='unsaid', answer='' WHERE project_id=$1 AND id=(SELECT min(id) FROM project_questions WHERE project_id=$1 AND state<>'open' AND answer_state='answered')
