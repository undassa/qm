WITH v AS (INSERT INTO project_plan_versions (project_id, id) VALUES ($1, 'v-ready-probe') RETURNING id),
m AS (INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title) SELECT $1, 'READY-PROBE', v.id, 9990, 'проба самотеста' FROM v RETURNING id),
t AS (INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind, entity_kind, entity_name) SELECT $1, 'READY-PROBE', m.id, 1, 'проба самотеста', '', 'closed', 'dev', 'task', 'READY-PROBE' FROM m RETURNING id),
-- ОБЪЯВЛЕННЫЙ ЗАПРОС, КОТОРЫЙ НЕ ПРОХОДИТ: `SELECT $1` отдаёт строку, то есть
-- нарушение. Его ловит ветка «объявил и не прошёл» — единственная, ради
-- которой послабление про прошедший запрос вообще можно было давать.
--
-- Запрос обязан поминать `$1`: довод подставляется всегда, и запросу без
-- места под него база откажет. Так же ведёт себя исполнитель на сервере, и
-- отказ там и здесь читается одинаково — «неизвестно», а не «прошло».
sp AS (INSERT INTO readiness_method (project_id, owner_kind, owner_id, ord, method_kind, method, declared_by)
       SELECT $1, 'task', t.id, 3, 'query', 'SELECT $1 AS detail', 'проба' FROM t RETURNING ord)
INSERT INTO task_ready_item (project_id, task_id, ord, check_id, text, done, origin)
-- ТРИ СТРОКИ, ПОТОМУ ЧТО ВЕТКИ ТРИ. Первая несёт имя проверки, которой нет в
-- дереве, — её ловит ветка «проверка не написана». Вторая имени не несёт
-- вовсе — её ловит ветка «пункт без названной проверки». Третья несёт
-- объявленный запрос, который не прошёл. Проба с одной строкой оставляла
-- остальные ветки без самотеста: перевернув их условие, гейт остался бы
-- зелёным на подсадке.
--
-- ПОТОЛОК: самотест видит «пункт покраснел», а не «покраснела эта ветка».
-- Три подсадки живут в одной пробе, и какая из них сработала, он не
-- различает. Что послабление про ПРОШЕДШИЙ запрос снимает находку, проба не
-- проверяет вовсе: подсадить зелёное она не умеет.
SELECT $1, t.id, 1, 'probe_check', 'проба самотеста', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 2, '', 'проба самотеста: пункт без названной проверки', false, 'проба' FROM t
UNION ALL
SELECT $1, t.id, 3, '', 'проба самотеста: объявленный запрос не прошёл', false, 'проба' FROM t, sp
