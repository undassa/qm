WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-ready-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'READY-PROBE', v.id, 9990, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'READY-PROBE', m.id, 1, 'проба самотеста', '', 'closed', 'dev', 'task', 'READY-PROBE' FROM m RETURNING id),
-- ЗАЯВЛЕНИЕ, КОТОРОЕ НЕ ПРОШЛО. Вердикт кладётся прямо: проба обязана
-- подсадить СОСТОЯНИЕ, а не гонять исполнителя — иначе она проверяла бы его,
-- а не ветку правила.
--
-- Номер у заявления — СТАРЫЙ (1, а пункт стоит на 3): так выглядит заявление
-- после правки выше чек-листа, и правило обязано найти его по тексту пункта.
-- РАЗНИЦЫ КЛЮЧЕЙ ЭТА ПРОБА НЕ ДОКАЗЫВАЕТ: самотест видит лишь «пункт
-- покраснел», а первые две строки краснят его при любой третьей ветке.
-- Ключ по тексту держит тест `a_shift_keeps_the_binding_and_a_new_subject_drops_it`
-- в проекторе — возврат соединения по номеру краснит его.
з AS (INSERT INTO readiness_method
        (project_id, owner_kind, owner_id, ord, method_kind, method, declared_by, item_text, verdict, verdict_at)
      SELECT $1, 'task', t.id, 1, 'checks-green', 'проба_несуществующая_проверка', 'проба',
             'проба самотеста: заявление не прошло', 'failed', 1 FROM t RETURNING ord),
-- ЯКОРЮ НУЖЕН ПУНКТ В `readiness_item`, И ТЕКСТ ТАМ БЕЗ МЕТКИ `- [ ]`.
--
-- Правило сличает `item_text` с текстом из `readiness_item`, а не из
-- `task_ready_item`: там строка лежит целиком, с меткой и склеенными
-- продолжениями. Подсадка, не знающая об этой разнице, породила бы форму,
-- которой проектор не создаёт, и перестала бы что-либо ловить — ровно это и
-- случилось с первой редакцией правила: проба осталась зелёной и с условием,
-- и без него, потому что подсаживала один и тот же текст в обе таблицы.
и AS (INSERT INTO readiness_item (project_id, owner_kind, owner_id, ord, text, declared, method_kind)
      SELECT $1, 'task', t.id, 3, 'проба самотеста: заявление не прошло', false, 'checks-green'
        FROM t RETURNING ord)
INSERT INTO task_ready_item (project_id, task_id, ord, check_id, text, done, origin)
-- ТРИ СТРОКИ, ПОТОМУ ЧТО ВЕТКИ ТРИ. Первая несёт имя проверки, которой нет в
-- дереве, — её ловит ветка «проверка не написана». Вторая имени не несёт
-- вовсе — её ловит ветка «пункт без названной проверки». Третья несёт
-- ЗАЯВЛЕНИЕ, которое не прошло, — её ловит ветка «заявлений не прошло».
-- Проба с одной строкой оставляла остальные ветки без самотеста: перевернув
-- их условие, гейт остался бы зелёным на подсадке.
--
-- ПОТОЛОК НАЗВАН: самотест видит «пункт покраснел», а не «покраснела эта
-- ветка». И того, что ПРОШЕДШЕЕ заявление находку СНИМАЕТ, проба не проверяет
-- вовсе — подсадить зелёное она не умеет. Это проверяется замером до и после
-- на живом наборе, и иначе сегодня никак.
SELECT $1, t.id, 1, 'probe_check', '- [ ] проба самотеста', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 2, '', '- [ ] проба самотеста: пункт без названной проверки', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 3, '', '- [ ] проба самотеста: заявление не прошло', false, 'проба' FROM t, з, и
