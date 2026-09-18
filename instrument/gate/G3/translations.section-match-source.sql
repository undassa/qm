WITH штампы AS (
  SELECT regexp_replace(regexp_replace(f.name, E'\n[ \t]*(//!|///)', ' ', 'g'), '[ \t]+', ' ', 'g') AS н,
         coalesce(substring(f.detail from 'названо в (.*)$'), f.detail) AS файл
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'section-stamp'),
разбор AS (
  SELECT файл, substring(н from '^`([^`]+)`') AS адрес, substring(н from '^`[^`]+` §(.+?) · `[0-9a-f]{8}`$') AS раздел,
         substring(н from '`([0-9a-f]{8})`$') AS отп
    FROM штампы),
цели AS (
  SELECT DISTINCT р.адрес, d.content
    FROM разбор р JOIN project_documents d ON d.project_id = $1 AND р.адрес ~ '^[a-z][a-z-]*:'
     AND d.entity_kind = split_part(р.адрес, ':', 1) AND d.entity_name = substring(р.адрес from '^[a-z][a-z-]*:(.*)$')),
строки AS (
  SELECT ц.адрес, s.н, s.т FROM цели ц, regexp_split_to_table(ц.content, E'\n') WITH ORDINALITY AS s(т, н)),
заголовки AS (SELECT адрес, н, trim(regexp_replace(т, '^#+', '')) AS заг FROM строки WHERE т ~ '^#'),
тексты AS (
  SELECT DISTINCT ON (з.адрес, з.заг) з.адрес, з.заг,
         left(encode(sha256(convert_to(з.заг || coalesce(E'\n' || (
           SELECT string_agg(rtrim(с.т), E'\n' ORDER BY с.н) FROM строки с
            WHERE с.адрес = з.адрес AND с.н > з.н AND trim(с.т) <> ''
              AND с.н < coalesce((SELECT min(х.н) FROM заголовки х WHERE х.адрес = з.адрес AND х.н > з.н), 2147483647)), ''),
         'UTF8')), 'hex'), 8) AS отп
    FROM заголовки з ORDER BY з.адрес, з.заг, з.н DESC),
выбор AS (
  SELECT р.файл, р.адрес, р.раздел, р.отп,
         CASE WHEN EXISTS (SELECT 1 FROM тексты т WHERE т.адрес = р.адрес AND т.заг = р.раздел) THEN 1
              ELSE (SELECT count(*) FROM тексты т WHERE т.адрес = р.адрес AND left(т.заг, length(р.раздел)) = р.раздел
                       AND substring(т.заг from length(р.раздел) + 1 for 1) !~ '\d')::int END AS кандидатов,
         coalesce((SELECT т.отп FROM тексты т WHERE т.адрес = р.адрес AND т.заг = р.раздел),
                  (SELECT min(т.отп) FROM тексты т WHERE т.адрес = р.адрес AND left(т.заг, length(р.раздел)) = р.раздел
                      AND substring(т.заг from length(р.раздел) + 1 for 1) !~ '\d')) AS текст
    FROM разбор р WHERE EXISTS (SELECT 1 FROM цели ц WHERE ц.адрес = р.адрес))
SELECT 'датчик «section-stamp» ' || fact_gap($1, 'section-stamp') || ': сверять отпечатки разделов нечем' AS detail WHERE NOT fact_fresh($1, 'section-stamp')
UNION ALL
SELECT р.файл || ' — адрес ' || р.адрес || ' файлом: набор адресуется видом и именем (вид:имя)' FROM разбор р WHERE р.адрес !~ '^[a-z][a-z-]*:'
UNION ALL
SELECT р.файл || ' — ' || р.адрес || ': такого документа в наборе нет' FROM разбор р WHERE р.адрес ~ '^[a-z][a-z-]*:' AND NOT EXISTS (SELECT 1 FROM цели ц WHERE ц.адрес = р.адрес)
UNION ALL
SELECT в.файл || ' — ' || в.адрес || ' §' || в.раздел || ': ' || CASE WHEN в.кандидатов = 0 THEN 'раздела нет' ELSE 'начало имени неоднозначно' END FROM выбор в WHERE в.кандидатов <> 1
UNION ALL
SELECT в.файл || ' — ' || в.адрес || ' §' || в.раздел || ' переписан: отпечаток ' || в.отп || ', текст даёт ' || в.текст || ' — перевод сделан с прежней редакции' FROM выбор в WHERE в.кандидатов = 1 AND в.текст <> в.отп
ORDER BY 1
