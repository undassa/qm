WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-ready-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'READY-PROBE', v.id, 9990, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'READY-PROBE', m.id, 1, 'проба самотеста', '', 'closed', 'dev', 'task', 'READY-PROBE' FROM m RETURNING id),
-- ЗАЯВЛЕНИЕ, КОТОРОЕ НЕ ПРОШЛО. Вердикт кладётся прямо: проба обязана
-- подсадить СОСТОЯНИЕ, а не гонять исполнителя — иначе она проверяла бы его,
-- а не ветку правила.
з AS (INSERT INTO readiness_method
        (project_id, owner_kind, owner_id, ord, method_kind, method, declared_by, item_text, verdict, verdict_at)
      SELECT $1, 'task', t.id, 3, 'checks-green', 'проба_несуществующая_проверка', 'проба',
             'проба самотеста: заявление не прошло', 'failed', 1 FROM t RETURNING ord)
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
SELECT $1, t.id, 1, 'probe_check', 'проба самотеста', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 2, '', 'проба самотеста: пункт без названной проверки', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 3, '', 'проба самотеста: заявление не прошло', false, 'проба' FROM t, з
