SELECT 'srs — автор не объявлен' AS detail
 WHERE NOT EXISTS (SELECT 1 FROM project_documents d
                    WHERE d.project_id = $1 AND d.entity_kind = 'srs' AND d.author <> '')
