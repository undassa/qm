-- АДРЕС БЕЗ ЦИТАТЫ НЕ ПРОВЕРЯЕТСЯ НИЧЕМ, И ПУНКТ БОЛЬШЕ НЕ МОЛЧИТ ОБ ЭТОМ.
-- Расхождение видно только там, где рядом с адресом написан текст строки:
-- сравнивать иначе не с чем, и ветка ниже отвечает «сходится» всякому адресу,
-- у которого цитаты нет. Сходится при этом одно — что в файле столько строк.
--
-- Что это стоило, измерено на живом наборе: вставка в `crates/tot-protocol/src/event.rs`
-- сдвинула ДЕСЯТЬ объявленных адресов, автор запроса знал о двух — ровно о тех,
-- у которых цитата была, — и собирался чинить две. Восемь остальных уехали бы
-- молча и навсегда. Цитат нет у 104 адресов из 136 на обоих наборах, 76%.
--
-- Считается ОДНОЙ строкой, а не находкой на каждый адрес. Сто четыре находки
-- разом — это долг, который не разгребают, а перестают читать; одна строка
-- называет число и действие, и закрывается тем же действием.
--
-- Решение владельца 2026-09-23: пункт говорит правду и краснеет, пока цитаты не
-- допишут. Вариант «дописать цитаты машиной из нынешнего содержимого» отвергнут
-- тем же решением: адрес, который врёт уже сегодня, закрепился бы как верный —
-- один такой на стволе уже найден (`red-task:V4-T5` :406).
SELECT 'датчик «code-line» ' || fact_gap($1, 'code-line') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'code-line') UNION ALL SELECT 'датчик строк ' || fact_gap($1, 'code-line') || ': сходится ли адрес — неизвестно' AS detail WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'code-line') UNION ALL SELECT a.entity_kind || ' ' || coalesce(nullif(a.entity_name, ''), '(одиночный)') || ' — ' || a.path || ':' || a.line || ': ' || CASE WHEN f.name IS NULL THEN 'строка не снята датчиком' WHEN f.detail LIKE 'файла нет%' OR f.detail LIKE 'в файле строк%' OR f.detail LIKE 'имя неоднозначно%' THEN f.detail WHEN a.anchor <> '' AND position(a.anchor in f.detail) = 0 THEN 'якорь «' || a.anchor || '» уехал, там теперь «' || f.detail || '»' ELSE 'сходится' END AS detail FROM project_code_address a LEFT JOIN code_fact f ON f.project_id = a.project_id AND f.kind = 'code-line' AND f.name = a.path || ':' || a.line WHERE a.project_id = $1 AND EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'code-line') AND (f.name IS NULL OR f.detail LIKE 'файла нет%' OR f.detail LIKE 'в файле строк%' OR f.detail LIKE 'имя неоднозначно%' OR (a.anchor <> '' AND position(a.anchor in f.detail) = 0))
UNION ALL
SELECT 'адресов без цитаты строки: ' || count(*) || ' — проверить их нечем. Сходится лишь то, что в файле столько строк; вставка выше уводит адрес молча. Цитата пишется рядом с адресом: `путь:строка` — «текст строки»' AS detail
  FROM project_code_address a
 WHERE a.project_id = $1 AND a.anchor = ''
HAVING count(*) > 0
ORDER BY 1
