WITH границы AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         substring(f.detail from '^pub (?:enum|struct) (\w+)') AS тип
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item'),
варианты AS (
  SELECT substring(v.name from '^(.*):\d+$') AS файл, substring(v.detail from '^([A-Z][A-Za-z]*)') AS вариант,
         (SELECT б.тип FROM границы б
           WHERE б.файл = substring(v.name from '^(.*):\d+$') AND б.н < substring(v.name from ':(\d+)$')::int
           ORDER BY б.н DESC LIMIT 1) AS тип
    FROM code_fact v WHERE v.project_id = $1 AND v.kind = 'protocol-variant'),
опы AS (SELECT DISTINCT вариант FROM варианты WHERE тип = 'Op' AND вариант IS NOT NULL),
события AS (SELECT DISTINCT вариант FROM варианты WHERE тип = 'EventMsg' AND вариант IS NOT NULL),
тело AS (
  SELECT б.файл, б.н AS от, (SELECT min(з.н) FROM границы з WHERE з.файл = б.файл AND з.н > б.н AND з.тип IS NULL) AS до
    FROM границы б WHERE б.тип = 'Op'),
слова AS (
  SELECT w.name, w.detail FROM code_fact w JOIN тело ON тело.файл = substring(w.name from '^(.*):\d+$')
   WHERE w.project_id = $1 AND w.kind = 'op-body-word'
     AND substring(w.name from ':(\d+)$')::int BETWEEN тело.от AND coalesce(тело.до, 2147483647)),
док AS (SELECT d.content AS т FROM project_documents d JOIN scheme($1) s ON s.role = 'doc.protocol' AND s.value = d.entity_kind || ':' || d.entity_name
         WHERE d.project_id = $1),
раздел AS (
  SELECT substring(т from position('## 4. Операции' in т) for position('## 5.' in т) - position('## 4. Операции' in т)) AS с,
         substring(т for position('## 5.' in т) - 1) AS норм
    FROM док WHERE position('## 4. Операции' in т) > 0 AND position('## 5.' in т) > position('## 4. Операции' in т)),
строки AS (
  SELECT m[2] AS оп, m[3] AS хвост, m[1] AS строка
    FROM раздел, regexp_matches(replace(раздел.с, '\|', '∥'), '^(\| `([A-Z][A-Za-z]*)[^|]*\|(.*)\|\s*)$', 'gn') m
   WHERE m[2] <> 'Op'),
пары AS (SELECT DISTINCT с.оп, e[1] AS событие FROM строки с, regexp_matches(с.хвост, '`([A-Z][A-Za-z]*)', 'g') e),
длит AS (
  SELECT DISTINCT x[1] AS оп FROM док,
         regexp_matches(coalesce(substring(док.т from '\*\*Длительные операции\*\*.*?\n\n(.*?)\n\n'), ''), '`([A-Z][A-Za-z]*)`', 'g') x),
исходники AS (SELECT regexp_replace(m.name, 'Cargo\.toml$', '') || 'src/' AS src FROM code_fact m WHERE m.project_id = $1 AND m.kind = 'crate-manifest'),
протокол AS (
  SELECT DISTINCT каталог FROM (
    SELECT DISTINCT ON (p.name) regexp_replace(m.name, 'Cargo\.toml$', '') AS каталог
      FROM code_fact p JOIN code_fact m ON m.project_id = p.project_id AND m.kind = 'crate-manifest'
       AND left(p.name, length(regexp_replace(m.name, 'Cargo\.toml$', ''))) = regexp_replace(m.name, 'Cargo\.toml$', '')
     WHERE p.project_id = $1 AND p.kind = 'protocol-item' AND p.detail ~ '^pub enum (Op|EventMsg)\y'
     ORDER BY p.name, length(m.name) DESC) x)
SELECT 'датчик «protocol-variant» ' || fact_gap($1, 'protocol-variant') || ': что умеет протокол — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-variant')
UNION ALL
SELECT 'датчик «event-wildcard-arm» ' || fact_gap($1, 'event-wildcard-arm') || ': глотает ли кто-то событие — неизвестно' AS detail WHERE NOT fact_fresh($1, 'event-wildcard-arm')
UNION ALL
SELECT 'датчик «event-caused-by» ' || fact_gap($1, 'event-caused-by') || ': несёт ли событие caused_by — неизвестно' AS detail WHERE NOT fact_fresh($1, 'event-caused-by')
UNION ALL
SELECT 'датчик «op-spontaneous» ' || fact_gap($1, 'op-spontaneous') || ': объявлено ли фоновое событие — неизвестно' AS detail WHERE NOT fact_fresh($1, 'op-spontaneous')
UNION ALL
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': где исходники крейтов — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'протокол — enum Op или EventMsg не разобран: протокол не прочитан' WHERE NOT EXISTS (SELECT 1 FROM опы) OR NOT EXISTS (SELECT 1 FROM события)
UNION ALL
SELECT 'документ протокола не объявлен: протокол в коде есть, а сверять его не с чем — объявите дверью scheme-term-set role=doc.protocol value=«вид:имя»' WHERE NOT EXISTS (SELECT 1 FROM scheme($1) s WHERE s.role = 'doc.protocol')
UNION ALL
SELECT 'документ протокола ' || s.value || ' — в наборе его нет либо в нём нет §4 и §5: сверять не с чем' FROM scheme($1) s WHERE s.role = 'doc.protocol' AND NOT EXISTS (SELECT 1 FROM раздел)
UNION ALL
SELECT 'protocol — §4.14 не объявляет перечень длительных операций' WHERE EXISTS (SELECT 1 FROM раздел) AND NOT EXISTS (SELECT 1 FROM длит)
UNION ALL
SELECT 'Op::' || о.вариант || ' — нет в protocol §4' FROM опы о WHERE EXISTS (SELECT 1 FROM раздел) AND о.вариант NOT IN (SELECT оп FROM строки)
UNION ALL
SELECT 'Op::' || s.оп || ' — §4 обещает операцию, которой нет в коде' FROM (SELECT DISTINCT оп FROM строки) s WHERE EXISTS (SELECT 1 FROM опы) AND s.оп NOT IN (SELECT вариант FROM опы)
UNION ALL
SELECT 'Op::' || s.оп || ' — строка §4 не называет ни одного события' FROM (SELECT DISTINCT оп FROM строки) s WHERE s.оп IN (SELECT вариант FROM опы) AND NOT EXISTS (SELECT 1 FROM пары p WHERE p.оп = s.оп)
UNION ALL
SELECT 'EventMsg::' || p.событие || ' — §4 называет событие, которого нет в EventMsg' FROM (SELECT DISTINCT событие FROM пары) p WHERE EXISTS (SELECT 1 FROM события) AND p.событие NOT IN (SELECT вариант FROM события) AND p.событие NOT IN (SELECT вариант FROM опы)
UNION ALL
SELECT 'EventMsg::' || e.вариант || ' — не назван в §1–§4 protocol: контракт не знает события, которое крейт шлёт' FROM события e, раздел р WHERE р.норм !~ ('\y' || e.вариант || '\y')
UNION ALL
SELECT 'Op::' || д.оп || ' — §4.14 называет длительной операцию, которой нет в таблицах §4' FROM длит д WHERE д.оп NOT IN (SELECT оп FROM строки)
UNION ALL
SELECT 'Op::' || д.оп || ' — объявлена длительной, а несёт одно событие: начала и завершения у неё нет' FROM длит д WHERE д.оп IN (SELECT оп FROM строки) AND (SELECT count(*) FROM строки s, regexp_matches(s.хвост, '`([A-Z][A-Za-z]*)', 'g') e WHERE s.оп = д.оп) < 2 AND NOT EXISTS (SELECT 1 FROM строки s WHERE s.оп = д.оп AND (s.строка LIKE '%…%' OR s.строка LIKE '%далее%' OR s.строка LIKE '%затем%'))
UNION ALL
SELECT f.name || ' — матчит EventMsg рукавом _ => (И-5)' FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'event-wildcard-arm' AND EXISTS (SELECT 1 FROM исходники и WHERE left(f.name, length(и.src)) = и.src) AND NOT EXISTS (SELECT 1 FROM протокол п WHERE left(f.name, length(п.каталог)) = п.каталог)
UNION ALL
SELECT 'Op::' || вариант || ' — записывает страницу, а она производна (И-2, ART-18)' FROM опы WHERE вариант ~ '^Page(Store|Save|Cache|Write|Put)'
UNION ALL
SELECT 'Op::' || вариант || ' — создаёт receipt, а его чеканит движок (И-3, ART-15)' FROM опы WHERE вариант LIKE '%Receipt%' AND вариант NOT LIKE '%Request'
UNION ALL
SELECT 'Event — без поля caused_by: парность нечем проверить' WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'event-caused-by')
UNION ALL
SELECT 'OpId::spontaneous — фоновое событие не объявлено явно' WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'op-spontaneous')
ORDER BY 1
