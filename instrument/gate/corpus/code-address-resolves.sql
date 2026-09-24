-- АДРЕС БЕЗ ЦИТАТЫ НЕ ПРОВЕРЯЕТСЯ НИЧЕМ, И ЭТО ПОКА НЕ ГОВОРИТСЯ ВСЛУХ.
-- Расхождение видно только там, где рядом с адресом написан текст строки; без
-- цитаты сходится лишь то, что в файле столько строк. Цитат нет у 104 адресов
-- из 136 на обоих наборах.
--
-- Строка, называвшая это число, была написана и снята обратно, и вот почему.
-- Красный `corpus` приходит в лестницу второй дорогой — через `ladder_holds`,
-- где имя `corpus` из отбора не исключается никогда, — и держит ступень 6.
-- Замер: у `tot-ade` `next-task` ответил бы `null` на весь набор, из
-- пятнадцати доступных задач осталось бы ноль. То есть правило, чинящее
-- удобство чтения ссылок, остановило бы разработку продукта.
--
-- Я померил это неверно и сказал владельцу, что работу оно не держит; решение
-- стояло на моём замере. Строка вернётся вместе с новой формой ссылки («имя
-- элемента плюс отпечаток»), когда станет закрываемой, а не просто красной.
SELECT 'датчик «code-line» ' || fact_gap($1, 'code-line') || ': проверить нечем, и это не зелёное' AS detail WHERE NOT fact_fresh($1, 'code-line') UNION ALL SELECT 'датчик строк ' || fact_gap($1, 'code-line') || ': сходится ли адрес — неизвестно' AS detail WHERE NOT EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'code-line') UNION ALL SELECT a.entity_kind || ' ' || coalesce(nullif(a.entity_name, ''), '(одиночный)') || ' — ' || a.path || ':' || a.line || ': ' || CASE WHEN f.name IS NULL THEN 'строка не снята датчиком' WHEN f.detail LIKE 'файла нет%' OR f.detail LIKE 'в файле строк%' OR f.detail LIKE 'имя неоднозначно%' THEN f.detail WHEN a.anchor <> '' AND position(a.anchor in f.detail) = 0 THEN 'якорь «' || a.anchor || '» уехал, там теперь «' || f.detail || '»' ELSE 'сходится' END AS detail FROM project_code_address a LEFT JOIN code_fact f ON f.project_id = a.project_id AND f.kind = 'code-line' AND f.name = a.path || ':' || a.line WHERE a.project_id = $1 AND EXISTS (SELECT 1 FROM code_fact WHERE project_id = $1 AND kind = 'code-line') AND (f.name IS NULL OR f.detail LIKE 'файла нет%' OR f.detail LIKE 'в файле строк%' OR f.detail LIKE 'имя неоднозначно%' OR (a.anchor <> '' AND position(a.anchor in f.detail) = 0))
UNION ALL
-- ФРАЗА СИЛЬНЕЕ ЯКОРЯ, И ОТВЕЧАЕТ О НЕЙ ДАТЧИК. Якорь сверяет одну строку;
-- фраза документа почти всегда называет предмет целиком — «событие
-- `EventMsg::Reverted`» плюс адрес. Сравнение делает датчик: файл у него в
-- руках, а правилу пришлось бы тащить в базу цепочку охватывающих элементов и
-- тело внутреннего ради одного сравнения.
--
-- Прочитано на четырнадцати живых адресах: восемь съехали, и якорь молчал бы о
-- всех восьми — цитаты у них нет. Ответ трёхзначный, и это не педантизм:
-- «съехал» лечится номером строки, «уехал из файла» — правкой самой фразы.
SELECT a.entity_kind || ' ' || coalesce(nullif(a.entity_name, ''), '(одиночный)') || ' — '
       || a.path || ':' || a.line || ': ' || f.detail AS detail
  FROM project_code_address a
  JOIN code_fact f ON f.project_id = a.project_id AND f.kind = 'code-line'
   AND f.name = a.path || ':' || a.line || '!named'
 WHERE a.project_id = $1 AND f.detail NOT LIKE 'имя %на месте'

ORDER BY 1
