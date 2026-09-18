SELECT р.entity_name || ' — статус называет замену или отмену: «' || left(р.status_text, 160) || '»' AS detail FROM project_decisions р WHERE р.project_id = $1 AND р.status_text ~* '(заменён|заменен|отменён|отменен|superseded|replaced by)'
ORDER BY 1
