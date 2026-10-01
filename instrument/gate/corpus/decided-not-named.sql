SELECT q.id || ' — вопрос решён, а его не называет ни задача, ни документ process: исполнять решение некому. Закрыть одним из двух: решение о продукте — назвать ' || q.id || ' в задаче, которая его исполняет; решение о процессе — назвать ' || q.id || ' в документе process (раздел или строка обряда)' AS detail
  FROM project_questions q
 WHERE q.project_id = $1 AND q.state = 'decided'
   AND NOT EXISTS (SELECT 1 FROM project_documents d
                    WHERE d.project_id = q.project_id AND d.entity_kind IN ('task','process')
                      AND d.content ~ ('\y' || q.id || '\y'))
 ORDER BY 1
