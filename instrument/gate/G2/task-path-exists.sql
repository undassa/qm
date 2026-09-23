-- ЧТО ДАТЧИК ПРОЧЁЛ, А НЕ ЧТО ЛЕЖИТ В ДЕРЕВЕ. Каталоги правило берёт из
-- поданных фактов, поэтому путь, до которого датчик не дошёл, даёт ту же
-- пустоту, что и несуществующий, — а правило утверждало второе.
--
-- Что это стоило: адрес `.config/nextest.toml` лежал в дереве `tot-ade`, набор
-- ссылался на него прозой в шести местах, а находка звала каталог
-- несуществующим — `.config/**` не было в перечне чтения датчика `repo-file`.
-- Исполнитель час искал ошибку в дереве, завёл обход и написал заявку с
-- неверной причиной («пропускают каталоги с точки»): дотовых каталогов в
-- перечне было четыре, пятого не было.
--
-- СПРАШИВАЕТСЯ ПРОЧИТАННОЕ, А НЕ ОБЪЯВЛЕННОЕ, и это существенно. Первая
-- редакция разбирала глобы из `project_sensor_spec.reads` — и завела вторую
-- копию предиката, который на деле живёт в обходчике (`client.rs::walk`): тот
-- безусловно пропускает `.git`, `node_modules`, `target`, `dist`, `.venv`
-- независимо от объявления, а глоб вида `**/*.rs` разбирает как «спуск от
-- корня». Копия не знала ни того ни другого и на законном объявлении врала бы
-- в обе стороны. Верхи `project_code_dir` — это то, куда обход ДОШЁЛ, то есть
-- ответ по замеру, а не по декларации; исключения и грамматика глоба в него
-- входят даром.
WITH верхи AS (
  SELECT DISTINCT split_part(d.dir, '/', 1) AS верх
    FROM project_code_dir d WHERE d.project_id = $1),
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
-- Датчик не прочёл в дереве вообще ничего: сказать «туда не дошли» можно о
-- любом пути, и это не разбор, а шум. Прежняя формулировка честнее.
SELECT п.task_id || ' — ' || п.leaf || ': ни один датчик дерева не дошёл до «' || п.верх
       || '» — есть там что-нибудь или нет, набор не мерил. Каталог есть: '
       || 'переобъявите перечень чтения датчика `repo-file` ЦЕЛИКОМ — сперва `sensor-specs`, '
       || 'потом `sensor-spec-add` со всем перечнем: дверь заменяет `reads`, а не дополняет. '
       || 'Каталога нет: поправьте адрес'
  FROM лист п
 WHERE EXISTS (SELECT 1 FROM верхи)
   AND NOT EXISTS (SELECT 1 FROM верхи в WHERE в.верх = п.верх)
UNION ALL
SELECT п.task_id || ' — ' || п.leaf || ': нет даже каталога ' || п.target_dir
  FROM лист п
 WHERE NOT EXISTS (SELECT 1 FROM верхи)
    OR EXISTS (SELECT 1 FROM верхи в WHERE в.верх = п.верх)
 ORDER BY 1
