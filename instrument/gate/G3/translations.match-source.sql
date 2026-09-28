WITH строки AS (
  SELECT d.entity_kind AS док, s.н, s.т
    FROM project_documents d, regexp_split_to_table(d.content, E'\n') WITH ORDINALITY AS s(т, н)
   WHERE d.project_id = $1 AND d.entity_kind IN ('srs', 'ui-spec')),
шапки AS (
  SELECT док, н, substring(т from '^\*\*((?:N?FR|FR-UI)-\d+) · ') AS ид,
         trim(rtrim(substring(т from '^\*\*(?:N?FR|FR-UI)-\d+ · (.*)$'), '*')) AS заг
    FROM строки WHERE т ~ '^\*\*(?:N?FR|FR-UI)-\d+ · '),
стопы AS (SELECT док, н FROM строки WHERE т ~ '^(#|---)' UNION ALL SELECT док, н FROM шапки),
треб AS (
  SELECT ш.ид, left(encode(sha256(convert_to(ш.заг || E'\n' || coalesce((
           SELECT string_agg(rtrim(с.т), E'\n' ORDER BY с.н) FROM строки с
            WHERE с.док = ш.док AND с.н > ш.н AND trim(с.т) <> ''
              AND с.н < coalesce((SELECT min(х.н) FROM стопы х WHERE х.док = ш.док AND х.н > ш.н), 2147483647)), ''),
         'UTF8')), 'hex'), 8) AS отп
    FROM шапки ш),
пстроки AS (
  SELECT s.н, s.т FROM project_documents d, regexp_split_to_table(d.content, E'\n') WITH ORDINALITY AS s(т, н)
   WHERE d.project_id = $1 AND d.entity_kind = 'acceptance'),
пшапки AS (
  SELECT н, substring(т from '^### ([A-Z][A-Z0-9-]*) · ') AS ид, trim(substring(т from '^### [A-Z][A-Z0-9-]* · (.+)$')) AS заг
    FROM пстроки WHERE т ~ '^### [A-Z][A-Z0-9-]* · .'),
сцен AS (
  SELECT п.ид, left(encode(sha256(convert_to(п.заг || coalesce(E'\n' || (
           SELECT string_agg(rtrim(с.т), E'\n' ORDER BY с.н) FROM пстроки с
            WHERE с.н > п.н AND trim(с.т) <> ''
              -- ГРАНИЦА ТЕЛА — ЗАГОЛОВОК ТОГО ЖЕ УРОВНЯ ИЛИ ВЫШЕ, одна с
              -- `translations:section-match-source`. Вложенный подзаголовок — часть
              -- сценария, а не его конец; на любом `#` сценарий с `####` внутри терял
              -- хвост молча, и отпечаток считался бы от половины текста. Сегодня таких
              -- нет (померено: 69 заголовков документа приёмки, ни одного глубже
              -- третьего уровня внутри сценария), поэтому правка ничего не сдвигает.
              --
              -- Требований это не касается: они объявлены жирной строкой, а не
              -- заголовком, уровня у них нет, и любой `#` их тело действительно
              -- заканчивает.
              AND с.н < coalesce((SELECT min(х.н) FROM пстроки х WHERE х.н > п.н AND х.т ~ '^#{1,3}[^#]'), 2147483647)), ''),
         'UTF8')), 'hex'), 8) AS отп
    FROM пшапки п),
ключи AS (
  -- Ключ сценария — И СОБСТВЕННОЕ ИМЯ ТОЖЕ, а не только собранное из истории.
  --
  -- Правило знало один вид ключа, `S<n>-AC-<m>`, и резолвило штамп только по
  -- нему. Набор `tot-ade` переименовал сценарии в `TC-<ОБЛАСТЬ>-<n>`, и десять
  -- объявлений отпечатка от восьми таких имён перестали проверяться ничем:
  -- правило искало их среди требований, не находило, а до ветки приёмки они не
  -- доходили. Отпечаток при этом выглядит измеренным — ровно та ложь, ради
  -- которой пункт и заведён. Цена всплыла на PR: два числа пересчитали неверно,
  -- и поймал это человек, а не прибор.
  SELECT 'S' || substring(a.story_id from '\d+') || '-AC-' || a.number AS ключ, a.id AS ид
    FROM project_acceptance a WHERE a.project_id = $1
  UNION ALL
  SELECT a.id, a.id FROM project_acceptance a WHERE a.project_id = $1),
штампы AS (
  SELECT substring(f.name from '^([A-Z]+(?:-[A-Z]+)?-\d+|S\d+-AC-\d+) · ') AS ид,
         substring(f.name from '`([0-9a-f]{8})`') AS отп,
         coalesce(nullif(f.place, ''), f.detail) AS файл
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'translation-stamp')
SELECT 'датчик «translation-stamp» ' || fact_gap($1, 'translation-stamp') || ': сверять отпечатки переводов нечем' AS detail WHERE NOT fact_fresh($1, 'translation-stamp')
UNION ALL
SELECT ш.файл || ' — ' || ш.ид || ': отпечаток снят с имени, которого в наборе нет' || CASE WHEN EXISTS (SELECT 1 FROM треб т WHERE т.ид = 'FR-' || ш.ид) THEN '; требование зовётся FR-' || ш.ид ELSE '' END FROM штампы ш WHERE NOT EXISTS (SELECT 1 FROM треб т WHERE т.ид = ш.ид)
   AND NOT EXISTS (SELECT 1 FROM ключи к WHERE к.ключ = ш.ид)
   -- ИМЯ ИЩЕТСЯ И В САМОМ ДОКУМЕНТЕ ПРИЁМКИ, А НЕ ТОЛЬКО В ОБЪЯВЛЕННОМ.
   --
   -- `ключи` собраны из `project_acceptance` — таблицы, которую наполняет
   -- ДВЕРЬ. Сценарий, названный двумя историями, получает в ней две строки под
   -- позиционными именами, и собственного имени у него там нет ни одного;
   -- штамп `TC-HARN-07 · …` ключа себе не находит, и пункт говорит «имени
   -- нет» — про имя, которое стоит заголовком в документе двумя строками ниже
   -- по этому же запросу.
   --
   -- Замер 21.09: две находки из двух были такими, и обе ложные. Сценариев,
   -- названных больше чем одной историей, в наборе ровно два — `TC-HARN-07` и
   -- `TC-SANDBOX-05`, — и упали ровно они. Отпечатки у обоих верны.
   --
   -- `сцен` разбирает заголовки `### TC-… · …` прямо из документа приёмки и
   -- ни от какой двери не зависит. Это и есть источник: объявление может
   -- отстать, документ — нет.
   AND NOT EXISTS (SELECT 1 FROM сцен с WHERE с.ид = ш.ид)
UNION ALL
SELECT ш.файл || ' — ' || ш.ид || ' переписан: отпечаток ' || ш.отп || ', текст даёт ' || т.отп || ' — перевод сделан с прежней редакции' FROM штампы ш JOIN треб т ON т.ид = ш.ид WHERE т.отп <> ш.отп
UNION ALL
SELECT ш.файл || ' — ' || ш.ид || ': пункта приёмки с этим ключом в наборе нет' FROM штампы ш WHERE ш.ид ~ '^S\d+-AC-' AND NOT EXISTS (SELECT 1 FROM ключи к JOIN сцен с ON с.ид = к.ид WHERE к.ключ = ш.ид)
UNION ALL
SELECT ш.файл || ' — ' || ш.ид || ' (' || к.ид || ') переписан: отпечаток ' || ш.отп || ', текст даёт ' || с.отп || ' — перевод сделан с прежней редакции' FROM штампы ш JOIN ключи к ON к.ключ = ш.ид JOIN сцен с ON с.ид = к.ид WHERE с.отп <> ш.отп
UNION ALL
-- Штамп, названный именем сценария, сверяется с документом напрямую: ключа в
-- объявленном у него может не быть вовсе (см. довод выше), а сверить его текст
-- есть с чем.
SELECT ш.файл || ' — ' || ш.ид || ' переписан: отпечаток ' || ш.отп || ', текст даёт ' || с.отп || ' — перевод сделан с прежней редакции'
  FROM штампы ш JOIN сцен с ON с.ид = ш.ид
 WHERE с.отп <> ш.отп
   AND NOT EXISTS (SELECT 1 FROM ключи к WHERE к.ключ = ш.ид)
ORDER BY 1
