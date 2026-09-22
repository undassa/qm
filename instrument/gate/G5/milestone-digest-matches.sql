-- ПУНКТ СУДИТ СОГЛАСИЕ ВЫЖИМОК, А НЕ ТРЕБУЕТ ИХ ЗАВЕДЕНИЯ.
--
-- Прежняя редакция начиналась ветвью «выжимок этапов нет: сверять диапазон не
-- с чем» — и тем превращала правило о согласии в требование держать хронику.
-- Набор, который хроники не держит, получал находку за то, чего не обещал.
--
-- ЗАМЕР 22.09: tot-ade отказался от разборов случившегося письменно, в своём
-- же нормативном `document-plan` §`chronicle:` — «Разборов случившегося набор
-- не держит: он описывает требуемое состояние, а урок случившегося становится
-- правилом или гейтом», — и вида `run` нет среди двадцати пяти, объявленных
-- его картой. Пункт требовал около полусотни документов истории, написанных
-- задним числом по коммитам: не свидетельство, а пересказ. Свойство «выжимки
-- нет» стало предметом `.subject.sql`: нечего мерить — это не «чисто» и не
-- находка.
--
-- Что пункт судит и сегодня: выжимка этапа называет число задач и обещает по
-- выжимке на каждую. Разошлись — и выжимка описывает диапазон, которого не
-- было; читают её после закрытия этапа, когда проверить уже нечем.
WITH сказ AS (
  SELECT r.milestone, (x[1])::int AS сказано
    FROM project_runs_log r
    JOIN project_documents d
      ON d.project_id = r.project_id AND d.entity_kind = 'run' AND d.entity_name = r.id
    JOIN scheme($1) t ON t.role = 'digest.task-count'
   CROSS JOIN LATERAL regexp_matches(d.content, '\|\s*' || t.value || '\s*\|\s*([0-9]+)\s+из', 'g') AS x
   WHERE r.project_id = $1 AND r.is_milestone)
SELECT с.milestone
       || ' — выжимка говорит '
       || с.сказано
       || ' задач, а задачных выжимок '
       || (SELECT count(*) FROM project_runs_log t2
            WHERE t2.project_id = $1 AND NOT t2.is_milestone AND t2.milestone = с.milestone) AS detail
  FROM сказ с
 WHERE с.сказано <> (SELECT count(*) FROM project_runs_log t2
                      WHERE t2.project_id = $1 AND NOT t2.is_milestone AND t2.milestone = с.milestone)
 ORDER BY 1
