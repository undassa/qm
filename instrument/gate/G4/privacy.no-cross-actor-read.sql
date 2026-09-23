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
     AND substring(w.name from ':(\d+)$')::int BETWEEN тело.от AND coalesce(тело.до, 2147483647))
SELECT 'датчик «protocol-variant» ' || fact_gap($1, 'protocol-variant') || ': что умеет протокол — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-variant')
UNION ALL
SELECT 'датчик «op-body-word» ' || fact_gap($1, 'op-body-word') || ': что принимает Op — неизвестно' AS detail WHERE NOT fact_fresh($1, 'op-body-word')
UNION ALL
SELECT 'протокол — enum Op или EventMsg не разобран: протокол не прочитан' WHERE NOT EXISTS (SELECT 1 FROM опы) OR NOT EXISTS (SELECT 1 FROM события)
UNION ALL
-- ГРАНИЦА ПО РОЛИ ПОЛЯ, А НЕ ПО ЕГО ТИПУ. Статья запрещает читать чужой след по
-- чужому идентификатору; поле, называющее НАЗНАЧАЕМОГО, следа не читает — оно
-- говорит, кому документ отдан. Решение владельца 2026-09-23 провело границу
-- именно так, и отвергло вариант «вписать имя операции в запрос»: прошитое имя
-- прошло бы проверку за счёт имени, а не за счёт предмета.
--
-- Имена назначаемых полей объявляет НАБОР ролью `field.assignee`, и потому
-- исключение видно одной строкой словаря, а не россыпью по запросам. Замер на
-- день объявления: полей типа актора в протоколе `tot-ade` ровно два, оба
-- назначаемые (`Op::DocOwnerAssign`, `EventMsg::DocOwnerAssigned`), и оба зовутся
-- `owner`. Первое новое поле придётся объявить явно — молча оно не пройдёт.
SELECT 'Op — принимает ActorId: чужой след стал бы читаемым (' || w.name || ')' FROM слова w
 WHERE w.detail ~ '\yActorId\y'
   AND NOT EXISTS (SELECT 1 FROM scheme($1) s
                    WHERE s.role = 'field.assignee'
                      AND w.detail ~ ('^[[:space:]]*(pub[[:space:]]+)?' || s.value || '[[:space:]]*:'))
UNION ALL
SELECT 'MemoryConfirm — операции публикации нет: подтверждение кандидата и есть акт публикации' WHERE EXISTS (SELECT 1 FROM опы) AND NOT EXISTS (SELECT 1 FROM опы WHERE вариант = 'MemoryConfirm')
ORDER BY 1
