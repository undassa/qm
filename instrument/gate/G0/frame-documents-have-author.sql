SELECT k || CASE
         WHEN NOT EXISTS (SELECT 1 FROM project_documents d WHERE d.project_id = $1 AND d.entity_kind = k) THEN ' — документа нет'
         WHEN EXISTS (SELECT 1 FROM project_documents d WHERE d.project_id = $1 AND d.entity_kind = k AND length(d.content) < 400) THEN ' — документ пуст'
         ELSE ' — автор не объявлен' END AS detail
  FROM unnest(ARRAY['constitution','glossary']) AS k
 WHERE NOT EXISTS (SELECT 1 FROM project_documents d
                    WHERE d.project_id = $1 AND d.entity_kind = k
                      AND length(d.content) >= 400 AND d.author <> '')
 UNION ALL
SELECT 'план набора — ни process, ни document-plan не написан'
 WHERE NOT EXISTS (SELECT 1 FROM project_documents d
                    WHERE d.project_id = $1 AND d.entity_kind IN ('process','document-plan')
                      AND length(d.content) >= 400 AND d.author <> '')
 UNION ALL
SELECT 'реестр вопросов — не написан'
 WHERE NOT EXISTS (SELECT 1 FROM project_documents d
                    WHERE d.project_id = $1 AND d.author <> ''
                      AND (d.entity_kind = 'register'
                        OR (d.entity_kind = 'index' AND d.entity_name = '00-frame/questions')))
