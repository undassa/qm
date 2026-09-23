SELECT 'датчик крейтов ' || fact_gap($1, 'crate-manifest') || ': знает ли набор о том, что лежит в репозитории — неизвестно' AS detail
 WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT c.name || ' — крейт есть в репозитории, но набором не объявлен'
  FROM project_crate c WHERE c.project_id = $1 AND c.in_repo AND c.does = '' AND c.does_not = ''
UNION ALL
SELECT c.name || ' — крейт объявлен набором, а среди прочитанного датчиком «crate-manifest» его нет: ' ||
       CASE WHEN EXISTS (SELECT 1 FROM project_plan_tasks t
                           JOIN project_document_fields f
                             ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                            AND f.entity_name = t.entity_name AND f.name ILIKE 'крейты'
                          WHERE t.project_id = c.project_id
                            AND split_part(f.value, ' — ', 1) ~ ('\y' || c.name || '\y'))
            THEN 'задача, объявившая себя его строителем, закрыта — а манифеста всё равно не подано'
            ELSE 'ни одна задача не берётся его завести: объявление без работы' END
  FROM project_crate c
 WHERE c.project_id = $1 AND NOT c.in_repo
   AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t
                     JOIN project_document_fields f
                       ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                      AND f.entity_name = t.entity_name AND f.name ILIKE 'крейты'
                    WHERE t.project_id = c.project_id AND t.state <> 'closed'
                      AND split_part(f.value, ' — ', 1) ~ ('\y' || c.name || '\y'))
ORDER BY 1
