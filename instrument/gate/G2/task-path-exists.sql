-- ЧТО ДАТЧИК ЧИТАЕТ, А НЕ ЧТО ЛЕЖИТ В ДЕРЕВЕ. Каталоги правило берёт из
-- поданных фактов, поэтому «в дереве нет» и «датчик туда не смотрит» дают одну
-- и ту же пустоту. Правило утверждало первое, и это стоило дня: адрес
-- `.config/nextest.toml` лежал в дереве, набор ссылался на него прозой в шести
-- местах, а находка звала его несуществующим — потому что `.config/**` не было
-- в `reads` датчика `repo-file`. Исполнитель час искал ошибку в дереве, завёл
-- обход и написал заявку с неверной причиной («каталоги с точки»): четыре
-- дотовых каталога в перечне были, пятого не было.
--
-- Верх пути сравнивается с верхом глоба, и полного разбора глоба не нужно:
-- перечень чтения объявлен по верхним каталогам (`crates/**`, `.github/**`,
-- `.config/**`), а правило спрашивает про КАТАЛОГ, а не про файл.
WITH читают AS (
  SELECT DISTINCT split_part(btrim(глоб), '/', 1) AS верх
    FROM project_sensor_spec s,
         LATERAL regexp_split_to_table(s.reads, '\s+') AS g(глоб)
   WHERE s.project_id = $1 AND s.fact IN ('repo-file', 'code-file') AND btrim(глоб) <> ''),
-- ПОДАЮЩИЙ БЕЗ ОБЪЯВЛЕНИЯ ЧИТАЕТ НЕИЗВЕСТНО ОТКУДА, и сказать о нём «туда не
-- смотрит» нельзя. Тогда правило говорит по-старому: прежняя формулировка
-- лучше новой и ложной.
объявлены AS (
  SELECT NOT EXISTS (
           SELECT 1 FROM code_fact f
            WHERE f.project_id = $1 AND f.kind IN ('repo-file', 'code-file')
              AND NOT EXISTS (SELECT 1 FROM project_sensor_spec s
                               WHERE s.project_id = $1 AND s.fact = f.kind)) AS все),
лист AS (
  SELECT l.task_id, l.leaf, l.target_dir, split_part(l.target_dir, '/', 1) AS верх
    FROM project_task_tree_leaf l
   WHERE l.project_id = $1 AND (fact_fresh($1, 'repo-file') OR fact_fresh($1, 'code-file'))
     AND l.is_path AND NOT l.exempt AND NOT l.forward_declared
     AND l.target_dir <> '' AND l.op <> '+'
     AND NOT EXISTS (SELECT 1 FROM project_code_dir d
                      WHERE d.project_id = $1 AND d.dir = l.target_dir))
SELECT 'датчик файлов дерева «repo-file» ' || fact_gap($1, 'repo-file')
       || ': есть ли каталоги задач — неизвестно, и это не зелёное' AS detail
 WHERE NOT (fact_fresh($1, 'repo-file') OR fact_fresh($1, 'code-file'))
   AND EXISTS (SELECT 1 FROM project_task_tree_leaf l
                WHERE l.project_id = $1 AND l.is_path AND NOT l.exempt)
UNION ALL
SELECT п.task_id || ' — ' || п.leaf || ': ни один датчик дерева не читает «' || п.верх
       || '» — есть там что-нибудь или нет, набор не мерил. Каталог есть: '
       || 'добавьте его в `reads` датчика `repo-file` (`sensor-spec-add`). Нет: поправьте адрес'
  FROM лист п, объявлены о
 WHERE о.все AND NOT EXISTS (SELECT 1 FROM читают ч WHERE ч.верх = п.верх)
UNION ALL
SELECT п.task_id || ' — ' || п.leaf || ': нет даже каталога ' || п.target_dir
  FROM лист п, объявлены о
 WHERE NOT о.все OR EXISTS (SELECT 1 FROM читают ч WHERE ч.верх = п.верх)
 ORDER BY 1
