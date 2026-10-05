-- СВЕЖЕСТЬ ПОДАЧИ — НЕ ОТВЕТ О ПРЕДМЕТЕ. Четвёртая ветка добавлена 22.09 по
-- замеру: набор `tot-ade` объявил заморозку `vendor/rio-vt`, фактов вида
-- `frozen-tree` в проекте было НОЛЬ, и пункт стоял зелёным с нулём нарушений.
--
-- Вышло так: датчик снимался ТОГДА, КОГДА ЗАМОРОЗКИ ЕЩЁ НЕ ОБЪЯВИЛИ. Он честно
-- отдал пустой перечень, подача записалась свежей и чистой — и третья ветка,
-- сторожащая несвежесть, замолчала навсегда. Пустая подача стала значить
-- «дерево стоит», хотя не значит ничего.
--
-- Поэтому объявленное дерево требует ФАКТА О СЕБЕ ПОИМЁННО, а не отметки
-- времени у датчика вообще. Заморозка без своего факта — «неизвестно», и это
-- не зелёное.
SELECT t.path || ' — заморозка объявлена без хэша: держать ей нечего, и это не зелёное' AS detail
  FROM project_frozen_tree t
 WHERE t.project_id = $1 AND t.tree_hash = ''
UNION ALL
SELECT 'заморозка не объявлена ни для одного дерева: стоит ли донор — неизвестно' AS detail
 WHERE NOT EXISTS (SELECT 1 FROM project_frozen_tree WHERE project_id = $1)
UNION ALL
SELECT 'датчик заморозки ' || fact_gap($1, 'frozen-tree') || ': стоит ли донор — неизвестно'
 WHERE EXISTS (SELECT 1 FROM project_frozen_tree WHERE project_id = $1)
   AND NOT fact_fresh($1, 'frozen-tree')
UNION ALL
SELECT t.path || ' — заморозка объявлена, а датчик о ней не сказал НИЧЕГО: подача свежа, '
       || 'но этого дерева в ней нет. Харнес переснимет его со ствола после слияния' AS detail
  FROM project_frozen_tree t
 WHERE t.project_id = $1
   AND NOT EXISTS (SELECT 1 FROM code_fact f
                    WHERE f.project_id = t.project_id
                      AND f.kind = 'frozen-tree' AND f.name = t.path)
UNION ALL
SELECT f.name || '  —  ' || f.detail
  FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'frozen-tree' AND f.detail NOT LIKE 'стоит на%'
 ORDER BY 1
