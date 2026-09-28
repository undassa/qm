WITH штампы AS (
  SELECT regexp_replace(regexp_replace(f.name, E'\n[ \t]*(//!|///)', ' ', 'g'), '[ \t]+', ' ', 'g') AS н,
         coalesce(nullif(f.place, ''), f.detail) AS файл
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
-- ЗАГОЛОВОК — ТЕКСТ БЕЗ ИМЕНИ СУЩНОСТИ, и определение здесь одно с
-- `translations:match-source`. Правила считали его по-разному: там `### TC-… · Имя`
-- давало `Имя`, здесь — строку целиком, и один перевод выходил свеж по одному счёту
-- и протух по другому. Цену заплатил человек: две плоскости ревью пересчитали
-- `TC-AGENT-02` вручную, получили два числа и полчаса не могли рассудить, кто прав.
-- Имя сущности — часть адреса, а не текста: переименование `S16-AC-3` →
-- `TC-AGENT-02` сценария не меняет, и отпечаток от него меняться не должен.
--
-- Правка замерена перед внесением: в `tot-ade` 501 объявление отпечатка, из них 116
-- разделов и 75 сценариев, и ни одного, которое судили бы оба правила; среди 64
-- различных штампов разделов нет ни одного с префиксом имени. В `myack` объявлений
-- перевода нет вовсе. То есть ни один отпечаток от этой правки не сдвинулся.
заголовки AS (
  SELECT адрес, н, length(substring(т from '^#+')) AS уровень,
         regexp_replace(trim(regexp_replace(т, '^#+', '')), '^[A-Z][A-Z0-9-]* · ', '') AS заг
    FROM строки WHERE т ~ '^#'),
тексты AS (
  SELECT DISTINCT ON (з.адрес, з.заг) з.адрес, з.заг,
         left(encode(sha256(convert_to(з.заг || coalesce(E'\n' || (
           SELECT string_agg(rtrim(с.т), E'\n' ORDER BY с.н) FROM строки с
            WHERE с.адрес = з.адрес AND с.н > з.н AND trim(с.т) <> ''
              -- ГРАНИЦА ТЕЛА — ЗАГОЛОВОК ТОГО ЖЕ УРОВНЯ ИЛИ ВЫШЕ, а не любой `#`.
              -- Вложенный подзаголовок — часть раздела, а не его конец; на любом `#`
              -- раздел с `####` внутри терял хвост молча. Сегодня таких нет (померено:
              -- 406 заголовков в 38 штампованных документах, ни один штамп не стоит
              -- перед более глубоким), поэтому правка ничего не сдвигает и вносится
              -- сейчас, пока цена нулевая.
              AND с.н < coalesce((SELECT min(х.н) FROM заголовки х WHERE х.адрес = з.адрес AND х.н > з.н AND х.уровень <= з.уровень), 2147483647)), ''),
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
