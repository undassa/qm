//! Проекции, которых у донора нет.
//!
//! Донор разбирает документ на блоки, разделы, ячейки и поля — это его работа, и
//! трогать её нельзя. Отсюда и граница: здесь из **структуры** считаются те
//! таблицы, которых в доноре не заведено. Каждая — по правилу, названному вслух;
//! правило, не записанное рядом, назавтра разойдётся с набором незаметно.

use crate::entities::Miss;
use deadpool_postgres::Pool;
use serde_json::{json, Value};

/// Выводимое живёт представлением, а не колонкой.
///
/// Сохранённый вывод протухает в тот миг, когда меняется то, из чего он выведен:
/// задача становится готовой оттого, что закрылась соседняя, и никто не
/// переписал ей колонку. Поэтому «готова», «покрыто», «выполнен» — это `select`.
const VIEWS: &str = r#"
CREATE OR REPLACE VIEW task_ready AS
  SELECT t.project_id, t.id AS task_id, t.milestone_id, t.state,
         NOT EXISTS (SELECT 1 FROM project_plan_task_deps d
                       JOIN project_plan_tasks p ON p.project_id = d.project_id AND p.id = d.depends_on
                      WHERE d.project_id = t.project_id AND d.task_id = t.id AND p.state <> 'closed')
     AND NOT EXISTS (SELECT 1 FROM task_milestone_dep md
                      WHERE md.project_id = t.project_id AND md.task_id = t.id
                        AND EXISTS (SELECT 1 FROM project_plan_tasks t2
                                     WHERE t2.project_id = md.project_id
                                       AND t2.milestone_id = md.milestone_id AND t2.state <> 'closed'))
         AS ready
    FROM project_plan_tasks t;

CREATE OR REPLACE VIEW requirement_covered AS
  SELECT q.project_id, q.id AS requirement_id, q.kind, q.area, q.satisfied,
         (SELECT count(*) FROM project_checks c
           WHERE c.project_id = q.project_id AND c.requirement_id = q.id) AS checks,
         EXISTS (SELECT 1 FROM project_checks c
                  WHERE c.project_id = q.project_id AND c.requirement_id = q.id) AS covered
    FROM project_requirements q;

CREATE OR REPLACE VIEW corpus_progress AS
  -- Три числа, никогда одно. Процент считается ОТ ОТВЕЧАЕМОГО и показывается
  -- рядом с `unknown`; плитка, у которой отвечаемого нет, показывает не «0 %»,
  -- а «пока не измеряется». Полоска, сложившая `unknown` в любую из сторон, —
  -- тот же дефект, что зелёный чек-лист, который ничего не мерит.
  SELECT project_id, 'задачи' AS tile,
         count(*) FILTER (WHERE state = 'closed') AS done,
         count(*) FILTER (WHERE state <> 'closed') AS open,
         0::bigint AS unknown
    FROM project_plan_tasks GROUP BY project_id
  UNION ALL
  SELECT project_id, 'вопросы',
         count(*) FILTER (WHERE state = 'closed'),
         count(*) FILTER (WHERE state <> 'closed'), 0
    FROM project_questions GROUP BY project_id
  UNION ALL
  SELECT project_id, 'требования покрыты',
         count(*) FILTER (WHERE covered), count(*) FILTER (WHERE NOT covered), 0
    FROM requirement_covered GROUP BY project_id
  UNION ALL
  SELECT project_id, 'пункты гейтов',
         count(*) FILTER (WHERE state = 'passed'),
         count(*) FILTER (WHERE state = 'failed'),
         count(*) FILTER (WHERE state NOT IN ('passed','failed'))
    FROM project_gates GROUP BY project_id
  UNION ALL
  SELECT i.project_id, 'пункты готовности',
         count(*) FILTER (WHERE i.method_kind <> 'unknown' AND NOT EXISTS (
            SELECT 1 FROM readiness_item x WHERE false)),
         0,
         count(*) FILTER (WHERE i.method_kind = 'unknown')
    FROM readiness_item i GROUP BY i.project_id
  UNION ALL
  SELECT $$__ladder__$$, 'ступени лестницы',
         count(*) FILTER (WHERE method_kind <> 'unknown'), 0,
         count(*) FILTER (WHERE method_kind = 'unknown')
    FROM harness_process_step GROUP BY 1;

CREATE OR REPLACE VIEW process_state AS
  SELECT s.set_name, s.process, s.ord, s.question, s.method_kind, s.owner_kind, s.owner, s.touches,
         CASE WHEN s.method_kind = 'unknown' THEN 'unknown' ELSE 'computable' END AS answerable
    FROM harness_process_step s;
CREATE OR REPLACE VIEW readiness_state AS
  SELECT i.project_id, i.owner_kind, i.owner_id, i.ord, i.text, i.declared, i.method_kind,
         CASE WHEN i.method_kind = 'unknown' THEN 'unknown' ELSE 'computable' END AS state
    FROM readiness_item i;
"#;

const DDL: &str = r#"
-- Летопись сущности. Две породы записей, и они не смешиваются:
--
--   `declared` — то, что документ говорит о себе сам (журнал внутри вопроса).
--     Выводится из текста и пересобирается вместе с ним;
--   `edit`     — то, что случилось: правка, её ревизия и имя правившего.
--     Пересборка её НЕ ТРОГАЕТ, иначе след правки жил бы до первой пересборки.
--
-- Источник входит в ключ: у одного вопроса бывает и десятая строка журнала, и
-- десятая правка, и это разные события.
CREATE TABLE IF NOT EXISTS entity_event (
  project_id text NOT NULL, entity_kind text NOT NULL, entity_id text NOT NULL,
  ord integer NOT NULL,
  at date, event text NOT NULL, actor text NOT NULL DEFAULT '',
  by_decision text, by_commit text,
  PRIMARY KEY (project_id, entity_kind, entity_id, ord));
ALTER TABLE entity_event ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT 'declared';
DO $$ BEGIN
  ALTER TABLE entity_event DROP CONSTRAINT entity_event_pkey;
  ALTER TABLE entity_event ADD PRIMARY KEY (project_id, entity_kind, entity_id, source, ord);
EXCEPTION WHEN others THEN NULL; END $$;

CREATE TABLE IF NOT EXISTS norm_version (
  project_id text NOT NULL, entity_kind text NOT NULL, entity_id text NOT NULL,
  version text NOT NULL, at date NOT NULL,
  changed text NOT NULL, by_decision text,
  PRIMARY KEY (project_id, entity_kind, entity_id, version));

CREATE TABLE IF NOT EXISTS measurement (
  project_id text NOT NULL, subject text NOT NULL, at date NOT NULL,
  value text NOT NULL, method text NOT NULL DEFAULT '',
  stated_in_kind text, stated_in_id text,
  PRIMARY KEY (project_id, subject, at));

-- Разрыв пути и требование, которым он закрыт.
--
-- Таблица заводится ПУСТОЙ и пустой остаётся. В `cjm.md` семь разрывов, и
-- колонка «Чем закрываем» у всех семи — проза: «ведущий по восьми шагам»,
-- «приёмка смены». Идентификатора требования нет ни у одного, вычислить его
-- нельзя, и догадка здесь была бы связью, которой набор не объявлял.
--
-- Место держится, чтобы связь было куда записать, когда владелец её назовёт.
-- Пустая таблица честнее заполненной догадками: по ней видно, что работа не
-- сделана, а не что её сделали неверно.
-- Фазы проекта — цепочка, объявленная самим набором:
--   Ф0 рамка ─G0─► Ф1 требования ─G1─► Ф2 проект ─G2─► Ф3 тесты ─G3─►
--   Ф4 генерация ─G4─► Ф5 выпуск
--
-- Каждой фазе принадлежат три разных вещи, и они считаются по-разному:
-- документы (уровнем плана), гейт (своим номером) и задачи (видом). До сих пор
-- в плане были только вехи кода, и трек проверок в картине не участвовал вовсе —
-- хотя 82 задачи и целая фаза Ф3 держатся на нём.
CREATE TABLE IF NOT EXISTS project_phase (
  project_id text NOT NULL,
  id text NOT NULL,                      -- Ф0…Ф5
  ord integer NOT NULL,
  title text NOT NULL,
  gate text NOT NULL DEFAULT '',         -- гейт, которым фаза закрывается
  plan_level text NOT NULL DEFAULT '',   -- уровень плана документов
  task_kind text NOT NULL DEFAULT '',    -- вид задач фазы: red · dev
  PRIMARY KEY (project_id, id));

-- Словарь статусов — таблица, а не `CHECK`.
--
-- «Какие статусы бывают у задачи» обязано отвечаться запросом: пока набор зашит
-- в ограничение DDL, интерфейс зашьёт конвейер у себя, и две копии разойдутся
-- молча — ровно как разошлись `red-task` в раскладке, `red` в проекции и `test`
-- в запросе гейта.
--
-- `fact` — ЗАПРОС, которым статус достигнут. Статус без факта — `unknown`, и
-- никогда «достигнут»: тот же закон, что у пункта гейта, пункта готовности и
-- ступени лестницы, и исполнитель у него тот же самый.
CREATE TABLE IF NOT EXISTS kind_status (
  kind text NOT NULL,
  ord integer NOT NULL,
  name text NOT NULL,
  title text NOT NULL DEFAULT '',
  fact text NOT NULL DEFAULT '',
  -- Пишет ли КТО-НИБУДЬ этот факт вообще. Запрос определён, а источник пуст —
  -- значит статус неизвестен, а не «не достигнут». Разница существенная:
  -- «не достигнут» читается как «не сделано», и работа, прошедшая мимо записи,
  -- выглядит несделанной.
  source text NOT NULL DEFAULT '',
  terminal boolean NOT NULL DEFAULT false,
  PRIMARY KEY (kind, name));
ALTER TABLE kind_status ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT '';
-- Почему факта нет. Пустой статус без причины читается как «забыли»; с
-- причиной — как решение, которое кто-то принял и записал.
ALTER TABLE kind_status ADD COLUMN IF NOT EXISTS why text NOT NULL DEFAULT '';

-- Подача факта: кто и когда подал. Без этой записи пустая таблица деревьев
-- значит и «никто не работает», и «никто ни разу не подавал» — а это разные
-- ответы, и второй обязан быть «неизвестно».
-- Наблюдение датчика о РЕПОЗИТОРИИ: таблицы миграций, операции контракта,
-- крейты. Сервер репозитория не видит и видеть не будет; харнес видит и
-- **молчит о выводах** — он подаёт факт, судит сервер.
-- Расхождение порождённого файла с тем, что считает сервер. Пишется сборкой,
-- читается гейтом: правило гейта — запрос, и вычисление на Rust должно оставить
-- ему след в базе, а не проситься исключением в исполнитель.
-- Как подсадить нарушение этому пункту. Без этого самотест угадывал бы, чем
-- испортить набор под каждый запрос, — а угадывание с видом проверки хуже
-- отсутствующей проверки. Способ ОБЪЯВЛЕН, как и способ пункта готовности.
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS probe text NOT NULL DEFAULT '';
-- Почему у пункта нет способа. «Требуется, но не проверяется» — состояние, и
-- пустотой его не выразить: пустой пункт читается как забытый.
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS why text NOT NULL DEFAULT '';
-- Родов у пункта четыре, а было три.
--
--   `manual` — проиграно человеком: онбординг на чистой установке — это
--     ДЕЙСТВИЕ с исходом, а не утверждение о тексте. Назвав его подписью, мы
--     получали гейт, которому будто нужны две подписи;
--   `unknown` — пункт плана, машинного способа у которого пока нет. Он обязан
--     существовать: отсутствие пункта нельзя ни показать, ни посчитать.
DO $$ BEGIN
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_kind_check;
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_check;
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_check1;
  ALTER TABLE project_gates ADD CONSTRAINT project_gates_kind_check
    CHECK (kind IN ('query','command','signed','manual','unknown'));
  ALTER TABLE project_gates ADD CONSTRAINT project_gates_query_check
    CHECK ((kind = 'query') = (query IS NOT NULL));
  ALTER TABLE project_gates ADD CONSTRAINT project_gates_owner_check
    CHECK ((kind = 'signed') = (owner IS NOT NULL));
EXCEPTION WHEN others THEN NULL; END $$;

-- Объявленный способ СТУПЕНИ живёт отдельно от самой ступени — ровно по тому
-- же образцу, что способ пункта готовности. Причина та же и уже измеренная:
-- способ, лежащий в строке ступени, стирается всяким проходом, который эту
-- строку трогает, и объявленное исчезает молча. Семь способов так и пропали.
CREATE TABLE IF NOT EXISTS harness_process_method (
  set_name text NOT NULL, process text NOT NULL, ord integer NOT NULL,
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','signed','unknown')),
  method text NOT NULL DEFAULT '', declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (set_name, process, ord));

-- Ответ вопроса — ПОЛЕ, а не догадка по заголовку раздела. Рядом флаг для
-- быстрого поиска и сортировки: отвечен · искали и не нашли · не сказано.
-- Третье состояние обязательно: «искали, набор молчит» зовёт владельца, а
-- «не сказано» — автора вопроса, и это разные работы.
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS answer text NOT NULL DEFAULT '';
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS answer_state text NOT NULL DEFAULT 'unsaid';
CREATE INDEX IF NOT EXISTS project_questions_by_answer
  ON project_questions(project_id, answer_state);

-- Подпись — утверждение ЧЕЛОВЕКА, и запись о ней обязана нести всё, что делает
-- её проверяемой годы спустя: кто, когда, под какой формулировкой, под какими
-- документами и их содержанием на тот момент, и чем снята.
--
-- Прежняя `project_gate_signatures` держала путь и хеш — и ни кто, ни когда, ни
-- что именно утверждено. Хеш есть, подписи нет.
--
-- Подпись одна на ГЕЙТ, а не на пункт: замер — у каждого гейта ровно один
-- подписной пункт, а место для неё было выбрано как для многих.
-- Гейт — СТРОКА со своей формулировкой из плана. Прежде строкой был пункт, а у
-- самого гейта строки не было: «что такое G2» отвечалось перечнем проверок, а
-- не тем, чего гейт требует.
--
-- Состояния здесь нет и не будет: оно выводится из проверок и подписи.
-- Сохранённое протухает в тот момент, когда меняется то, из чего выведено, —
-- и уже протухало: плитка показывала 10/2/6 по сохранённому при 9/4/5 счётом.
CREATE TABLE IF NOT EXISTS gate (
  project_id text NOT NULL, phase text NOT NULL,
  title text NOT NULL DEFAULT '',
  needs_signature boolean NOT NULL DEFAULT true,
  PRIMARY KEY (project_id, phase));

CREATE TABLE IF NOT EXISTS gate_signature (
  project_id text NOT NULL, phase text NOT NULL,
  signed_at date NOT NULL, signed_by text NOT NULL,
  wording text NOT NULL,             -- формулировка, под которой стоит подпись
  note text NOT NULL DEFAULT '',     -- обстоятельства: чем подтверждена, что было спорным
  lifted_by text NOT NULL DEFAULT '',-- правка либо решение, снявшее подпись
  PRIMARY KEY (project_id, phase));

-- Под чем именно подписано: документ и его содержание НА ТОТ МОМЕНТ. Правка
-- подписанного делает подпись устаревшей — и умеет назвать, какая именно.
CREATE TABLE IF NOT EXISTS gate_signature_doc (
  project_id text NOT NULL, phase text NOT NULL,
  entity_kind text NOT NULL, entity_id text NOT NULL,
  content_hash text NOT NULL, revision bigint,
  PRIMARY KEY (project_id, phase, entity_kind, entity_id));

-- Имя сущности — в самой таблице документов, рядом с путём.
--
-- Путь остаётся полем ПРОИСХОЖДЕНИЯ («откуда приехало»), но опознаётся документ
-- видом и именем. Пока ключ был файловым, два `onboarding.md` были неразличимы
-- не потому, что набор чего-то не сказал, а потому что спрашивали не о том.
--
-- Шаг добавляющий: ни одна из тридцати трёх колонок с путём не тронута, и
-- ничего на него опирающееся не ломается. Ключ переносится следующим заходом,
-- когда имя будет заполнено у всех и проверено.
ALTER TABLE project_documents ADD COLUMN IF NOT EXISTS entity_kind text NOT NULL DEFAULT '';
ALTER TABLE project_documents ADD COLUMN IF NOT EXISTS entity_name text NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS project_documents_by_entity
  ON project_documents(project_id, entity_kind, entity_name)
  WHERE entity_kind <> '';

CREATE TABLE IF NOT EXISTS generated_drift (
  project_id text NOT NULL, name text NOT NULL, detail text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

CREATE TABLE IF NOT EXISTS code_fact (
  project_id text NOT NULL, kind text NOT NULL, name text NOT NULL,
  detail text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, kind, name));

CREATE TABLE IF NOT EXISTS fact_push (
  project_id text NOT NULL, fact text NOT NULL,
  at bigint NOT NULL, actor text NOT NULL DEFAULT '', rows integer NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, fact));

-- Снятый термин — строка, а не regexp внутри правила.
--
-- Список жил в `docs-lint.mjs` образцом `RETIRED`. Пока он там, списков два:
-- один в правиле, другой в голове у читающего. Здесь он один, и «снятый термин
-- вернулся» становится соединением.
CREATE TABLE IF NOT EXISTS term_retired (
  project_id text NOT NULL, term text NOT NULL,
  retired_by text NOT NULL DEFAULT '',
  declared_in text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, term));

-- Признак раздела «Состояния» у экрана. Карта проекта просит проверять ПО
-- ОТСУТСТВИЮ ЗАГОЛОВКА, а не по перечню: перечень она сама объявляет неполным.
CREATE TABLE IF NOT EXISTS screen_states (
  project_id text NOT NULL, screen_id text NOT NULL,
  has_states boolean NOT NULL,
  PRIMARY KEY (project_id, screen_id));

-- Статус решения: четыре константы и слова документа рядом.
--
-- Свободного текста в статусе не бывает. Слова не пропадают — они в
-- `state_text`, и расхождение между словом и константой ловится.
CREATE TABLE IF NOT EXISTS decision_status (
  project_id text NOT NULL, decision_id text NOT NULL,
  status text NOT NULL,
  state_text text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, decision_id));

-- Скиллы харнеса — той же формы, что и субагенты. Пока их нет в базе,
-- `owner` ступени указывает на то, чего база не подтвердит: переименованный
-- скилл ломает предложение, а не связь.
CREATE TABLE IF NOT EXISTS harness_skill (
  set_name text NOT NULL, name text NOT NULL,
  description text NOT NULL DEFAULT '', body text NOT NULL DEFAULT '',
  content_hash text NOT NULL DEFAULT '',
  updated_at bigint NOT NULL DEFAULT 0, updated_by text NOT NULL DEFAULT '',
  PRIMARY KEY (set_name, name));

-- Лестница входа. Ключ — НАБОР, а не проект: она принадлежит харнесу и
-- одинакова везде, где он привязан.
CREATE TABLE IF NOT EXISTS harness_process (
  set_name text NOT NULL, name text NOT NULL, title text NOT NULL DEFAULT '',
  PRIMARY KEY (set_name, name));

CREATE TABLE IF NOT EXISTS harness_process_step (
  set_name text NOT NULL,
  process text NOT NULL,
  ord integer NOT NULL,
  question text NOT NULL,
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','signed','unknown')),
  method text NOT NULL DEFAULT '',
  -- Когда ступень вообще в игре. Пусто — всегда. Условная ступень объявляется,
  -- а не подразумевается: пропуск обязан быть виден с причиной.
  when_query text NOT NULL DEFAULT '',
  when_why text NOT NULL DEFAULT '',
  owner_kind text NOT NULL CHECK (owner_kind IN ('skill','agent','none')),
  owner text NOT NULL DEFAULT '',
  -- Сторона: ступень читает набор либо пишет в репозиторий. Диспетчер обязан
  -- отказаться от репозиторной, пока фаза набора открыта.
  touches text NOT NULL CHECK (touches IN ('corpus','repository')),
  PRIMARY KEY (set_name, process, ord),
  FOREIGN KEY (set_name, process) REFERENCES harness_process(set_name, name) ON DELETE CASCADE);

-- Владелец-скилл проверяется внешним ключом, а не надеждой.
CREATE TABLE IF NOT EXISTS harness_process_step_skill (
  set_name text NOT NULL, process text NOT NULL, ord integer NOT NULL, skill text NOT NULL,
  PRIMARY KEY (set_name, process, ord),
  FOREIGN KEY (set_name, process, ord)
    REFERENCES harness_process_step(set_name, process, ord) ON DELETE CASCADE,
  FOREIGN KEY (set_name, skill) REFERENCES harness_skill(set_name, name));

-- Каждый проход диспетчера записывается: «почему выбрал это» отвечается после,
-- а ступень, которая скачет, видна запросом.
CREATE TABLE IF NOT EXISTS process_run (
  project_id text NOT NULL, process text NOT NULL, at bigint NOT NULL,
  ord integer NOT NULL, state text NOT NULL, detail text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, process, at, ord));

-- Состояние задачи приходит ИЗ ИСТОРИИ, а не из документа.
--
-- Красная задача говорит о себе прямо: состояние не записано в ней намеренно,
-- оно выводится из закрывающего трейлера `Task: R-M1-T1 closed`. Документ,
-- несущий состояние, становится вторым источником истины и расходится молча.
--
-- Читатель трейлеров живёт у харнеса: у него есть репозиторий, у сервера его
-- нет и быть не должно. Поэтому харнес подаёт, база принимает.
CREATE TABLE IF NOT EXISTS task_state (
  project_id text NOT NULL, task_id text NOT NULL,
  state text NOT NULL CHECK (state IN ('not_started','claimed','closed')),
  closing_commit text NOT NULL DEFAULT '',
  seen_at bigint NOT NULL,
  PRIMARY KEY (project_id, task_id));

-- Объявленное отсутствие требований — факт, и у факта есть строка.
--
-- `M0-T12` пишет «нет собственных; исполняет решения Q-277 и Q-282 (ADR-0150)».
-- Разбор вытащил оттуда Q-277 и Q-282 и записал их ТРЕБОВАНИЯМИ задачи — то
-- есть ровно обратное написанному. Из объяснения отсутствия связи не
-- добываются: имена в такой фразе суть упоминания.
CREATE TABLE IF NOT EXISTS task_requirements_declared (
  project_id text NOT NULL, task_id text NOT NULL,
  has_own boolean,                       -- NULL: поле не заполнено вовсе, и это дефект документа
  note text NOT NULL DEFAULT '',         -- фраза целиком, как написана
  PRIMARY KEY (project_id, task_id));

CREATE TABLE IF NOT EXISTS task_requirement (
  project_id text NOT NULL, task_id text NOT NULL, requirement_id text NOT NULL,
  PRIMARY KEY (project_id, task_id, requirement_id));

-- Красная задача: имя несёт родителя, поле — перечень проверок, которые она пишет.
CREATE TABLE IF NOT EXISTS red_task (
  project_id text NOT NULL, id text NOT NULL,
  parent_task text NOT NULL, milestone text NOT NULL,
  checks integer NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS red_task_check (
  project_id text NOT NULL, red_task_id text NOT NULL, check_id text NOT NULL,
  PRIMARY KEY (project_id, red_task_id, check_id));

-- Пункт готовности. Одна таблица на все виды: форма у них одна — владелец,
-- порядок, текст, способ. (У истории формы были РАЗНЫЕ, и там три таблицы правы.)
CREATE TABLE IF NOT EXISTS readiness_item (
  project_id text NOT NULL,
  owner_kind text NOT NULL, owner_id text NOT NULL, ord integer NOT NULL,
  text text NOT NULL,
  declared boolean,                      -- стояла ли галочка в документе
  method_kind text NOT NULL DEFAULT 'unknown'
    CHECK (method_kind IN ('query','command','signed','unknown')),
  method text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, owner_kind, owner_id, ord));

-- Исключение из правила — строка, а не абзац.
-- Объявленный способ живёт ОТДЕЛЬНО от пункта.
--
-- Пункты пересобираются из документов — удаляются и пишутся заново. Способ,
-- лежащий в той же строке, каждой пересборкой стирался бы: объявил человек,
-- забрала машина, и никто не заметил. Способ объявлен, а не выведен, значит
-- переживать пересчёт обязан.
CREATE TABLE IF NOT EXISTS readiness_method (
  project_id text NOT NULL, owner_kind text NOT NULL, owner_id text NOT NULL, ord integer NOT NULL,
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','signed','unknown')),
  method text NOT NULL DEFAULT '',
  declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, owner_kind, owner_id, ord));

-- Связи, вынутые из фактов и полей документов. Каждая — своя таблица, потому
-- что соединять их иначе значит искать подстроку в чужом тексте: такая связь
-- рвётся от правки прозы и чинится молча.
CREATE TABLE IF NOT EXISTS project_requirement_op (
  project_id text NOT NULL, requirement_id text NOT NULL, operation text NOT NULL,
  PRIMARY KEY (project_id, requirement_id, operation));

CREATE TABLE IF NOT EXISTS project_task_operation (
  project_id text NOT NULL, task_id text NOT NULL, operation text NOT NULL,
  PRIMARY KEY (project_id, task_id, operation));

CREATE TABLE IF NOT EXISTS project_written_check (
  project_id text NOT NULL, check_id text NOT NULL,
  PRIMARY KEY (project_id, check_id));

CREATE TABLE IF NOT EXISTS project_task_check (
  project_id text NOT NULL, task_id text NOT NULL, check_id text NOT NULL,
  -- Откуда названа: доказательство родителя или перечень красной фазы.
  said_as text NOT NULL,
  PRIMARY KEY (project_id, task_id, check_id, said_as));

-- Файл кода, разобранный ОДИН раз: каталог, имя, приставка до подчёркивания.
-- Правило «однокоренные файлы — это подкаталог» группирует по колонкам, а не
-- режет имя в запросе на каждом прогоне.
CREATE TABLE IF NOT EXISTS project_code_file (
  project_id text NOT NULL, path text NOT NULL,
  dir text NOT NULL, base text NOT NULL, prefix text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, path));

CREATE TABLE IF NOT EXISTS project_code_dir (
  project_id text NOT NULL, dir text NOT NULL,
  PRIMARY KEY (project_id, dir));

-- Из чего состоит поверхность. Строка на каждый источник: вид документа и имя,
-- пустое имя — весь вид целиком. Объявляется дверью, а не выводится из формы
-- имени: имя, разобранное как путь, — это данные, взятые из прозы, и меняются
-- они молча. Множество поверхностей закрыто решением, и здесь оно видно.
CREATE TABLE IF NOT EXISTS project_surface_source (
  project_id text NOT NULL, surface text NOT NULL,
  entity_kind text NOT NULL, entity_name text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, surface, entity_kind, entity_name));

-- Замороженное дерево донора: путь и хэш, на котором его заморозили.
CREATE TABLE IF NOT EXISTS project_frozen_tree (
  project_id text NOT NULL, path text NOT NULL,
  tree_hash text NOT NULL DEFAULT '', why text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, path));

-- Шапка этапа и его же перечень: два числа об одном, и правят их порознь.
CREATE TABLE IF NOT EXISTS project_milestone_count (
  project_id text NOT NULL, milestone_id text NOT NULL, what text NOT NULL,
  said integer NOT NULL, listed integer NOT NULL,
  PRIMARY KEY (project_id, milestone_id, what));

-- Голое число в строке-перечне: запись, прячущая имя.
CREATE TABLE IF NOT EXISTS project_hidden_name (
  project_id text NOT NULL, entity_kind text NOT NULL, entity_name text NOT NULL,
  number text NOT NULL, line text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, entity_kind, entity_name, number, line));

-- Адрес «файл:строка» и ЯКОРЬ рядом с ним: набор пишет `waker.rs:31` «pub trait
-- Hints». Номер строки уезжает от любой правки выше, а документ продолжает
-- утверждать прежнее. Якорь — то, чем адрес проверяется.
CREATE TABLE IF NOT EXISTS project_code_address (
  project_id text NOT NULL,
  entity_kind text NOT NULL, entity_name text NOT NULL,
  path text NOT NULL, line integer NOT NULL,
  anchor text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, entity_kind, entity_name, path, line));

-- Имя, названное документом: раскрытое перечнем, с оговоркой и с ответом на
-- вопрос «есть ли такое». Одна таблица на все виды имён: правило «указатель в
-- никуда» одно, а не семь одинаковых с разными именами таблиц.
CREATE TABLE IF NOT EXISTS project_named_id (
  project_id text NOT NULL,
  entity_kind text NOT NULL, entity_name text NOT NULL,
  said_id text NOT NULL,
  -- Оговорка строки: имя названо ради истории, а не как действующее.
  caveated boolean NOT NULL DEFAULT false,
  PRIMARY KEY (project_id, entity_kind, entity_name, said_id));

-- Числа, СКАЗАННЫЕ документом прослеживаемости. Правило сверяет их с
-- измеренным; чтобы сверять равенством, сказанное обязано быть значением, а не
-- ячейкой, из которой его вынимают на каждом прогоне.
CREATE TABLE IF NOT EXISTS project_traceability_said (
  project_id text NOT NULL, block_ord integer NOT NULL,
  subject text NOT NULL, column_name text NOT NULL, said integer NOT NULL,
  -- Строка итога — не предмет: сверять её с числом предмета значит требовать,
  -- чтобы итог равнялся одному из слагаемых.
  is_total boolean NOT NULL DEFAULT false,
  PRIMARY KEY (project_id, block_ord, subject, column_name));

-- Держатель инварианта: требование, которое не видно ни на одной поверхности,
-- но названо МЕСТОМ В КОДЕ. Строка держателя без места — не держатель, а
-- обещание; место, где стоит заглушка, — тоже не держатель.
CREATE TABLE IF NOT EXISTS project_requirement_holder (
  project_id text NOT NULL, requirement_id text NOT NULL, path text NOT NULL,
  PRIMARY KEY (project_id, requirement_id, path));

-- Поверхности, названные РЕШЕНИЕМ. Множество закрыто им, и закрытость обязана
-- сверяться на равных: гейт, читающий свой перечень и ничей больше, закрытым
-- множество не делает — он делает закрытым СВОЙ список.
CREATE TABLE IF NOT EXISTS project_surface_declared (
  project_id text NOT NULL, surface text NOT NULL, said_in text NOT NULL,
  PRIMARY KEY (project_id, surface));

-- Поверхность, на которую приземлилось требование. Множество поверхностей
-- закрыто решением: требование либо видно на одной из них, либо названо
-- держателем инварианта. Третьего нет, и «нигде» — находка, а не состояние.
CREATE TABLE IF NOT EXISTS project_requirement_surface (
  project_id text NOT NULL, requirement_id text NOT NULL, surface text NOT NULL,
  said_in text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, requirement_id, surface));

-- Листья блока «Что меняется в дереве»: место, куда ляжет код задачи.
-- Не таблица ради таблицы: без неё правило «лист обязан быть путём» пришлось
-- бы считать разбором прозы внутри запроса, а разбор — не дело запроса.
CREATE TABLE IF NOT EXISTS project_task_tree_leaf (
  project_id text NOT NULL, task_id text NOT NULL, ord integer NOT NULL,
  dir text NOT NULL DEFAULT '', leaf text NOT NULL,
  is_path boolean NOT NULL,
  -- Оговорка каталога: строка называет вопрос реестра, то есть место ещё не
  -- решено. Это не побег на будущее — решится вопрос, и оговорка уйдёт сама.
  exempt boolean NOT NULL DEFAULT false,
  PRIMARY KEY (project_id, task_id, ord));

CREATE TABLE IF NOT EXISTS rule_exception (
  project_id text NOT NULL, rule text NOT NULL,
  entity_kind text NOT NULL, entity_id text NOT NULL,
  reason text NOT NULL, decided_by text NOT NULL DEFAULT '',
  -- Побег бывает двух родов, и разница — не в тоне причины. Один снимает
  -- находку навсегда: значение подтверждено не секретом. Другой терпит её до
  -- задачи, которая её уберёт. Второй без имени этой задачи вырождается в
  -- первый: причина «пока так» стареет молча. Отсюда колонка, а не проза.
  closes text NOT NULL DEFAULT '',
  -- Откуда строка: 'projected' — вынута из документа, 'declared' — объявлена
  -- дверью. Без этого пересборка сносила таблицу целиком и объявленные побеги
  -- исчезали молча: гейт наутро краснел находками, которые вчера разобрали.
  origin text NOT NULL DEFAULT 'declared',
  PRIMARY KEY (project_id, rule, entity_kind, entity_id));

-- Вердикт предполёта знает ревизию, на которой получен: «предполёт устарел» —
-- сравнение ревизий, а не память.
-- Открытое рабочее дерево — факт репозитория, а не набора: его подаёт харнес.
CREATE TABLE IF NOT EXISTS task_worktree (
  project_id text NOT NULL, task_id text NOT NULL,
  branch text NOT NULL DEFAULT '', since bigint NOT NULL,
  PRIMARY KEY (project_id, task_id));

CREATE TABLE IF NOT EXISTS preflight_verdict (
  project_id text NOT NULL, task_id text NOT NULL, at bigint NOT NULL,
  task_revision bigint NOT NULL,
  verdict text NOT NULL CHECK (verdict IN ('ready','ready-with-risks','blocked')),
  findings integer NOT NULL DEFAULT 0, body text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, task_id, at));

-- Что именно считает заявленное число: запрос, дающий факт, и оговорка.
CREATE TABLE IF NOT EXISTS claim_subject (
  project_id text NOT NULL, name text NOT NULL, subject text NOT NULL,
  counts text NOT NULL,
  note text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name, subject));

CREATE TABLE IF NOT EXISTS cjm_gap (
  project_id text NOT NULL, ord integer NOT NULL,
  gap text NOT NULL, phase text NOT NULL, closes_with text NOT NULL,
  PRIMARY KEY (project_id, ord));

CREATE TABLE IF NOT EXISTS cjm_gap_requirement (
  project_id text NOT NULL, gap_ord integer NOT NULL,
  gap text NOT NULL, phase text NOT NULL,
  closed_by_requirement text NOT NULL,
  PRIMARY KEY (project_id, gap_ord, closed_by_requirement));

CREATE TABLE IF NOT EXISTS task_milestone_dep (
  project_id text NOT NULL, task_id text NOT NULL, milestone_id text NOT NULL,
  said text NOT NULL,
  PRIMARY KEY (project_id, task_id, milestone_id));

-- Имя сущности в структурных таблицах.
--
-- Разбор до сих пор адресуется путём — тем самым `path`, которым документ
-- перестаёт опознаваться. Колонки добавляются РЯДОМ со старым ключом, а не
-- вместо него: набор, у которого половина таблиц на новом ключе, а половина на
-- старом, не читается ничем. Ключ переставляется отдельным шагом, когда обе
-- стороны — чтение и запись — уже адресуют именем.
--
-- Имя у одиночки пустое, и это не пропуск: `corpus constitution` спрашивают без
-- имени, вид сам его заменяет. Непустым обязан быть ВИД.
DO $$
DECLARE t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['project_document_blocks','project_document_sections',
                           'project_document_cells','project_document_fields',
                           'project_document_links','project_document_revisions'] LOOP
    EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS entity_kind text NOT NULL DEFAULT %L', t, '');
    EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS entity_name text NOT NULL DEFAULT %L', t, '');
    EXECUTE format('CREATE INDEX IF NOT EXISTS %I ON %I (project_id, entity_kind, entity_name)',
                   t || '_by_entity', t);
  END LOOP;
END $$;

-- Цель ссылки — тоже сущность, а не файл.
--
-- `target_document` держит путь, и обратные ссылки сверялись путями: ссылка
-- `[M0-T12](task:M0-T12)` при записи РАЗРЕШАЛАСЬ ОБРАТНО в путь, чтобы лечь в
-- колонку. Разрешение обратно — это и есть тот шов, на котором набор терял
-- переписанные ссылки: разборщик, не понявший новую форму, обнулил 5748 связей
-- разом, и увидели это не проверкой, а обратными ссылками, ставшими пустыми.
ALTER TABLE project_document_links ADD COLUMN IF NOT EXISTS target_kind text NOT NULL DEFAULT '';
ALTER TABLE project_document_links ADD COLUMN IF NOT EXISTS target_name text NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS project_document_links_by_target_entity
  ON project_document_links (project_id, target_kind, target_name);

-- ─── Ключ переезжает на имя ───────────────────────────────────────────────────
--
-- Всё ниже одноразово и идемпотентно: сделанное второй раз не делается. Порядок
-- существен и потому записан подряд, а не разложен по вызовам: назвать, снести
-- пережившее свой документ, переставить ключ, запереть внешним ключом.

-- Всё до «Путь исчезает» — ОДНОРАЗОВЫЙ переезд, и он читает `path`. Колонку
-- снимает тот же прогон, ниже; на втором запуске её уже нет, и без сторожа
-- `ensure` падал бы при каждом старте на первом же `UPDATE`. Сторож один на
-- весь переезд, потому что и предмет у него один.
DO $$
DECLARE t text; tail text; i int;
  spec text[][] := ARRAY[
    ARRAY['project_documents', ''],
    ARRAY['project_document_blocks', 'ord'],
    ARRAY['project_document_sections', 'ord'],
    ARRAY['project_document_cells', 'block_ord, row_ord, col'],
    ARRAY['project_document_fields', 'ord'],
    ARRAY['project_document_links', 'block_ord, ord']];
BEGIN
IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'project_documents'
                  AND column_name = 'path') THEN
  RETURN;
END IF;

-- 1. Документ без вида — «документ», названный тем, чем был. Таких пятнадцать,
--    и все в песочных проектах донора: раскладка у сервера одна, myack-ова.
--    Ключом станет пара «вид · имя», а `p6` держит четыре безымянных документа —
--    под ключом они столкнулись бы в одну строку.
UPDATE project_documents SET entity_kind = 'document', entity_name = path WHERE entity_kind = '';

-- 2. Разбор берёт имя у своего документа; летопись снятого — от пути.
FOREACH t IN ARRAY ARRAY['project_document_blocks','project_document_sections',
                         'project_document_cells','project_document_fields',
                         'project_document_links','project_document_revisions'] LOOP
  EXECUTE format(
    'UPDATE %I b SET entity_kind = d.entity_kind, entity_name = d.entity_name
       FROM project_documents d
      WHERE d.project_id = b.project_id AND d.path = b.path AND b.entity_kind = %L', t, '');
END LOOP;
UPDATE project_document_revisions SET entity_kind = 'document', entity_name = path WHERE entity_kind = '';

-- 3. Предметные таблицы называют документ именем, а не адресом. Каждая тащила
--    `path` рядом с собственным именем: у требования есть `id = FR-SIT-09` И
--    `path = 10-intent/srs.md`. Второе — тот же дефект, что был у
--    `readiness_item.owner_id`: «путь возвращается в базу идентификатором».
FOREACH t IN ARRAY ARRAY['project_article_references','project_articles','project_checks',
                         'project_db_tables','project_decisions','project_document_plan',
                         'project_features','project_gate_signatures','project_needs',
                         'project_plan_milestones','project_plan_status','project_plan_tasks',
                         'project_plan_versions','project_questions','project_requirements',
                         'project_risks','project_runs_log','project_screens','project_stories',
                         'project_terms','project_traceability_claims'] LOOP
  EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS entity_kind text NOT NULL DEFAULT %L', t, '');
  EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS entity_name text NOT NULL DEFAULT %L', t, '');
  EXECUTE format(
    'UPDATE %I x SET entity_kind = d.entity_kind, entity_name = d.entity_name
       FROM project_documents d
      WHERE d.project_id = x.project_id AND d.path = x.path AND x.entity_kind = %L', t, '');
  EXECUTE format('CREATE INDEX IF NOT EXISTS %I ON %I (project_id, entity_kind, entity_name)',
                 t || '_by_entity', t);
END LOOP;

-- 4. Ключ на имя.
FOR i IN 1 .. array_length(spec, 1) LOOP
  t := spec[i][1]; tail := spec[i][2];
  IF EXISTS (SELECT 1 FROM pg_constraint c
               JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
              WHERE c.conrelid = t::regclass AND c.contype = 'p' AND a.attname = 'path') THEN
    EXECUTE format('ALTER TABLE %I DROP CONSTRAINT %I', t, t || '_pkey');
    EXECUTE format('ALTER TABLE %I ADD PRIMARY KEY (project_id, entity_kind, entity_name%s)',
                   t, CASE WHEN tail = '' THEN '' ELSE ', ' || tail END);
  END IF;
END LOOP;
DROP INDEX IF EXISTS project_documents_by_entity;

IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'project_document_revisions_entity_revision_key') THEN
  ALTER TABLE project_document_revisions DROP CONSTRAINT IF EXISTS project_document_revisions_project_id_path_revision_key;
  ALTER TABLE project_document_revisions ADD CONSTRAINT project_document_revisions_entity_revision_key
    UNIQUE (project_id, entity_kind, entity_name, revision);
END IF;

FOREACH t IN ARRAY ARRAY['project_document_blocks','project_document_sections',
                         'project_document_cells','project_document_fields',
                         'project_document_links'] LOOP
  EXECUTE format('DROP INDEX IF EXISTS %I', t || '_by_entity');
  -- 5. Разбор не переживает свой документ, и это теперь запрещено, а не
  --    соблюдено. Переименование он берёт за собой: имя, ошибочно проставленное
  --    однажды, иначе нечем было бы исправить. Летопись ключом НЕ заперта — правки
  --    снятого документа единственное, что от него остаётся.
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = t || '_document_fk') THEN
    EXECUTE format(
      'DELETE FROM %I b WHERE NOT EXISTS (SELECT 1 FROM project_documents d
         WHERE d.project_id = b.project_id AND d.entity_kind = b.entity_kind
           AND d.entity_name = b.entity_name)', t);
    EXECUTE format(
      'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (project_id, entity_kind, entity_name)
         REFERENCES project_documents (project_id, entity_kind, entity_name)
         ON DELETE CASCADE ON UPDATE CASCADE', t, t || '_document_fk');
  END IF;
END LOOP;
END $$;

-- ─── Объявленные проверки переезжают вместе со схемой ────────────────────────
--
-- Часть проверок живёт ЗАПРОСАМИ в самой базе: пункт гейта несёт `query`, чем
-- он меряет, и `probe`, чем его роняют; статус вида несёт `fact`; заявленное
-- число несёт `counts`. Это данные, а не код, и переписать их обязан тот же
-- переезд — иначе они молча начнут отвечать «запрос не выполнился», а гейт
-- покажет `unknown` там, где раньше мерил.
--
-- Проверяется это самотестом: `gate-selftest` роняет каждый запросный пункт
-- своей пробой, и пункт, чей запрос сломан, там сразу виден.
UPDATE project_gates SET query = replace(query,
  'SELECT r.path || '' → Article ''',
  'SELECT r.entity_kind || '' '' || r.entity_name || '' → Article ''')
 WHERE query LIKE '%r.path%';

UPDATE project_gates SET query =
  'SELECT ''проектных видов '' || count(*)::text || '' из 8'' AS detail
     FROM project_documents WHERE project_id = $1
      AND entity_kind IN (''design-view'', ''data-model'') HAVING count(*) <> 8'
 WHERE query LIKE '%30-design/[^/]+%' OR query LIKE '%^30-design%';

UPDATE project_gates SET probe = replace(probe,
  '(project_id, path, number) VALUES ($1,''00-frame/constitution.md'',999)',
  '(project_id, entity_kind, entity_name, number) VALUES ($1,''constitution'','''',999)')
 WHERE probe LIKE '%00-frame/constitution.md%';
UPDATE project_gates SET probe = replace(probe,
  'text, path, satisfied) VALUES ($1,''FR-PROBE-01'',''FR'',''PROBE'',''проба самотеста'',''10-intent/srs.md'',false)',
  'text, entity_kind, entity_name, satisfied) VALUES ($1,''FR-PROBE-01'',''FR'',''PROBE'',''проба самотеста'',''srs'','''',false)')
 WHERE probe LIKE '%FR-PROBE-01%';
UPDATE project_gates SET probe = replace(probe,
  'text, path, satisfied) VALUES ($1,''NFR-PROBE-02'',''NFR'','''',''проба самотеста'',''10-intent/srs.md'',false)',
  'text, entity_kind, entity_name, satisfied) VALUES ($1,''NFR-PROBE-02'',''NFR'','''',''проба самотеста'',''srs'','''',false)')
 WHERE probe LIKE '%NFR-PROBE-02%';
UPDATE project_gates SET probe =
  'DELETE FROM project_documents WHERE project_id=$1 AND entity_kind=''design-view'' AND entity_name=''sdd'''
 WHERE probe LIKE '%30-design/sdd.md%';
UPDATE project_gates SET probe = replace(probe,
  'title, path, status) VALUES ($1,''ADR-9998'',9998,''проба'',''30-design/decisions/accepted/9998.md'',''accepted'')',
  'title, entity_kind, entity_name, status) VALUES ($1,''ADR-9998'',9998,''проба'',''decision'',''ADR-9998'',''accepted'')')
 WHERE probe LIKE '%ADR-9998%';
UPDATE project_gates SET probe = replace(probe,
  'spec, path) VALUES ($1,''TC-PROBE-01'',''PROBE'',''FR-НЕТ-99'',''проба'',''40-proof/test-cases.md'')',
  'spec, entity_kind, entity_name) VALUES ($1,''TC-PROBE-01'',''PROBE'',''FR-НЕТ-99'',''проба'',''test-cases'','''')')
 WHERE probe LIKE '%TC-PROBE-01%';
UPDATE project_gates SET probe = replace(probe,
  'columns, path) VALUES ($1,''проба_модели'','''','''','''',''30-design/data-model.md'')',
  'columns, entity_kind, entity_name) VALUES ($1,''проба_модели'','''','''','''',''data-model'','''')')
 WHERE probe LIKE '%проба_модели%';
UPDATE project_gates SET probe = replace(probe,
  'title, path, area) VALUES ($1,''SCR-PROBE-01'',''проба самотеста'',''20-surface/probe.md'',''probe'')',
  'title, entity_kind, entity_name, area) VALUES ($1,''SCR-PROBE-01'',''проба самотеста'',''screen'',''SCR-PROBE-01'',''probe'')')
 WHERE probe LIKE '%SCR-PROBE-01%';

UPDATE kind_status SET fact = replace(fact,
  'JOIN project_documents d ON d.project_id=t.project_id AND d.path=t.path',
  'JOIN project_documents d ON d.project_id=t.project_id AND d.entity_kind=t.entity_kind AND d.entity_name=t.entity_name')
 WHERE fact LIKE '%d.path=t.path%';

UPDATE claim_subject SET counts =
  'SELECT count(*) FROM project_documents WHERE project_id = $1
     AND entity_kind IN (''decision'', ''decision-template'')'
 WHERE counts LIKE '%30-design/decisions/%';

-- ─── У документа есть автор ──────────────────────────────────────────────────
--
-- `updated_by` отвечает на другой вопрос — «кто правил последним», — и после
-- машинного прохода отвечает на него именем прохода: `links-retarget` значится
-- последним правившим у 670 документов. Спрашивать у него автора бессмысленно.
--
-- Автор ставится один раз и машинными проходами НЕ ТРОГАЕТСЯ. Пусто — автор не
-- объявлен, и это третье состояние: не «ничей» и не «чей-то», а «не сказано».
ALTER TABLE project_documents ADD COLUMN IF NOT EXISTS author text NOT NULL DEFAULT '';

-- Заполняется тем, кто документ ЗАВЁЛ, — первой правкой летописи. Где заводил
-- инструмент переноса, там имя инструмента и стоит: выдумывать человека нельзя,
-- а «перенесено волтом» — честный ответ, который владелец заменит, когда назовёт
-- настоящего автора.
UPDATE project_documents d SET author = r.written_by
  FROM project_document_revisions r
 WHERE r.project_id = d.project_id AND r.entity_kind = d.entity_kind
   AND r.entity_name = d.entity_name AND r.revision = 1 AND d.author = '';

-- ─── Кто может войти ─────────────────────────────────────────────────────────
--
-- Край (Caddy) утверждает личность двумя заголовками: общий секрет и имя
-- вошедшего. Секрет отвечает на вопрос «это точно край», но НЕ на вопрос «этому
-- человеку сюда можно» — имя край подставляет какое настроено, и сервер прежде
-- верил любому непустому. «Кто дотянулся до порта — владелец» было правдой.
--
-- Допущенные объявляются здесь. Список пуст — сервер пускает, но КАЖДЫЙ вход
-- записывает как незаявленный, и `principals` их называет. Список непуст —
-- незаявленному отказ. Пустой список не запирает свежую установку и при этом
-- не молчит о дыре.
CREATE TABLE IF NOT EXISTS edge_principal (
  principal text PRIMARY KEY,
  note text NOT NULL DEFAULT '',
  declared_at bigint NOT NULL,
  declared_by text NOT NULL DEFAULT '');

-- Кто входил и когда. Не журнал запросов, а последний след на каждого: вопрос
-- «кто сюда ходит» обязан отвечаться и через неделю, а не жить в памяти
-- процесса, которую стирает каждая выкатка.
CREATE TABLE IF NOT EXISTS edge_seen (
  principal text PRIMARY KEY,
  first_at bigint NOT NULL,
  last_at bigint NOT NULL,
  requests bigint NOT NULL DEFAULT 0,
  refused bigint NOT NULL DEFAULT 0);

-- Заморозка набора на начало выпуска.
--
-- Скилл `godzy-version` делал её обходом файлов с `shasum` и складывал в
-- `freeze.md`. База знает то же самое сама: у каждого документа есть хеш и
-- ревизия. Заморозка — снимок этого, а разница выпуска — сравнение с ним.
--
-- `fate` заполняется сравнением, а не рукой: `unchanged` · `changed` ·
-- `removed` · `new`. Удалённый документ виден именно этой колонкой — прежде
-- пропущенная строка означала требование, исчезнувшее молча.
CREATE TABLE IF NOT EXISTS version_freeze (
  project_id text NOT NULL,
  version text NOT NULL,
  entity_kind text NOT NULL,
  entity_name text NOT NULL,
  content_hash text NOT NULL,
  revision bigint NOT NULL,
  frozen_at bigint NOT NULL,
  frozen_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, version, entity_kind, entity_name));

-- Область экрана — ОБЪЯВЛЕННАЯ величина, а не вывод.
--
-- `configure`, `mobile`, `shell` — раздел интерфейса, к которому принадлежит
-- экран. Набор о ней нигде не говорит: она была видна только по каталогу, в
-- котором лежал файл. Вывести её из имени (`SCR-CFG-01` → `configure`) можно —
-- соответствие взаимно однозначно на всех 68, — но такого правила набор не
-- объявлял, и завести его значило бы выдумать.
--
-- Поэтому она становится тем, чем и является: ДАННЫМИ. Своя таблица, потому что
-- `project_screens` пересборка удаляет и пишет заново, а объявленное переживает
-- пересборку — тем же устройством, что и состояния задач в `task_state`.
CREATE TABLE IF NOT EXISTS screen_area (
  project_id text NOT NULL,
  screen_id text NOT NULL,
  area text NOT NULL,
  PRIMARY KEY (project_id, screen_id));

-- Заполняется однажды из того, что каталог успел сказать, пока он был. Дальше
-- меняется только ручкой `screen-area-set`.
INSERT INTO screen_area (project_id, screen_id, area)
SELECT project_id, id, area FROM project_screens WHERE area <> ''
ON CONFLICT DO NOTHING;

-- ─── Путь исчезает ───────────────────────────────────────────────────────────
--
-- Снимается последним и только тогда, когда его не читает никто: разбор и
-- предметные таблицы адресуют именем, ссылки набора переписаны в `вид:имя`
-- проходом `links-retarget`, сверка порта спрашивает именем.
--
-- Порядок внутри блока значения не имеет — колонка ни на что не ссылается, —
-- но `project_documents` стоит последним, потому что от него имя брали все
-- остальные.
--
-- Сюда НЕ входят `acl_grants`, `file_artifacts` и `project_task_workspaces`:
-- их `path` — путь ACL, путь файла и путь рабочего дерева на диске. Одинаковое
-- имя колонки не делает их одним предметом.
DO $$
DECLARE t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['project_document_blocks','project_document_sections',
                           'project_document_cells','project_document_fields',
                           'project_document_links','project_document_revisions',
                           'project_article_references','project_articles','project_checks',
                           'project_db_tables','project_decisions','project_document_plan',
                           'project_features','project_gate_signatures','project_needs',
                           'project_plan_milestones','project_plan_status','project_plan_tasks',
                           'project_plan_versions','project_questions','project_requirements',
                           'project_risks','project_runs_log','project_screens','project_stories',
                           'project_terms','project_traceability_claims',
                           'project_documents'] LOOP
    EXECUTE format('DROP INDEX IF EXISTS %I', t || '_by_path');
    EXECUTE format('ALTER TABLE %I DROP COLUMN IF EXISTS path', t);
  END LOOP;
END $$;
DROP INDEX IF EXISTS project_documents_by_path;

-- Гейт считается ПРИ ИЗМЕНЕНИИ, а не при открытии доски.
--
-- Прежде каждый заход на страницу исполнял тридцать восемь запросов заново, а
-- в колонке `state` лежало что-то другое — записанное когда-то руками и с тех
-- пор не пересчитанное. Доска показывала оба числа рядом и звала расхождением
-- то, что было просто устаревшей записью.
--
-- Теперь запись и есть замер: `result` держит целиком то, что вышло, `checked_at`
-- — когда. Пустой `checked_at` значит «ещё не мерили», и это ответ, а не «пройден».
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS result jsonb;

-- Чем уронить ступень. То же, что `probe` у пункта гейта: запрос, подсаживающий
-- нарушение в откатываемой транзакции. Ступень без пробы не «прошла самотест» —
-- про неё просто не сказано, чем её ронять, и это разные ответы.
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS probe text NOT NULL DEFAULT '';

-- Что планируется — ВИДОМ И ИМЕНЕМ, а не строкой «constitution.md».
--
-- План перечислял документы файлами, и проекция копировала их как есть. Свести
-- такую строку с документом было нечем: сверка «объявлено есть, а его нет»
-- сравнивала план сам с собой и отвечала ноль вхолостую.
--
-- Пусто у строки — план назвал что-то, чего в наборе нет: файл репозитория либо
-- имя, которому вида не досталось. Это находка, а не пропуск.
ALTER TABLE project_document_plan ADD COLUMN IF NOT EXISTS planned_kind text NOT NULL DEFAULT '';
ALTER TABLE project_document_plan ADD COLUMN IF NOT EXISTS planned_name text NOT NULL DEFAULT '';

-- Почему у решения нет отвергнутых вариантов — СЛОВОМ, а не догадкой.
--
-- Отвергнутый вариант бывает не всегда: часть записей сделана до того, как
-- скелет решения стал требованием, и дописать им варианты значило бы выдумать
-- нерассмотренное. Набор это и говорит — пометкой `godzy-decision:` в самом
-- документе. Пункт гейта её не читал и требовал вариантов у всех подряд.
ALTER TABLE project_decisions ADD COLUMN IF NOT EXISTS waiver text NOT NULL DEFAULT '';

-- Чем умению разрешено пользоваться — КОЛОНКОЙ, а не строкой в шапке файла.
--
-- Формат скилла объявляет `allowed-tools` и `disable-model-invocation` у всех
-- двадцати четырёх, а таблица не держала ни того, ни другого. Установка с
-- сервера снимала бы ограничение инструментов молча: скилл, которому разрешено
-- читать, получал бы право писать, и заметить это было бы нечем.
--
-- Пусто у `allowed_tools` — не объявлено; `NULL` у признака — то же самое.
-- Ложь и «не сказано» здесь разные ответы.
ALTER TABLE harness_skill ADD COLUMN IF NOT EXISTS allowed_tools text NOT NULL DEFAULT '';
ALTER TABLE harness_skill ADD COLUMN IF NOT EXISTS disable_model_invocation boolean;

-- Откуда сущность взялась: ВЫВЕДЕНА из документа или ОБЪЯВЛЕНА прямо.
--
-- До сих пор предметные таблицы были только проекциями: пересборка стирала их
-- целиком и писала заново из текста документов. Пока набор жил файлами, это было
-- верно — источником был файл. Теперь источник сам набор, и сущность, внесённую
-- явно, стирать нельзя: она не выводится ниоткуда, она и есть запись.
--
-- Пересборка удаляет только выведенное. Объявленное переживает её — как
-- переживают состояние выпуска, снятые требования и пометки решений.
ALTER TABLE project_articles ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_terms ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_stories ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_decisions ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_checks ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';

-- План тоже бывает объявленным, а не выведенным.
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_plan_versions ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';

-- Отвергнутый вариант тоже бывает объявленным.
ALTER TABLE project_decision_alternatives ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_risks ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';



-- Состояние решения бывает не только «принято».
--
-- Ограничение знало три слова, и предложенное решение — написанное, но ещё не
-- подписанное — в набор не принималось вовсе. Отказ выглядел поломкой ввоза, а
-- был отказом признать законное состояние: «предложено» и «отвергнуто» такие же
-- части жизни решения, как «принято» и «отменено».
DO $$ BEGIN
  ALTER TABLE project_decisions DROP CONSTRAINT IF EXISTS project_decisions_status_check;
  ALTER TABLE project_decisions ADD CONSTRAINT project_decisions_status_check
    CHECK (status IN ('accepted','superseded','template','proposed','rejected','unknown'));
END $$;



-- ОБЪЯВЛЕНИЕ отдельно от ЗАМЕРА — и для гейтов, и для фаз.
--
-- Так уже устроена лестница: ступени объявлены раз на весь сервер, а «где мы»
-- лежит у каждого проекта своё. Гейты и фазы жили иначе: пункт объявлялся внутри
-- проекта, и второй проект начинал с пустого места. Гейт отвечает на вопрос
-- «можно ли идти дальше», и ответ не должен зависеть от того, кто как завёл
-- проверки у себя; особый случай закрывается объявленным исключением, и его
-- видно.
--
-- Запросы пунктов уже написаны через `$1 = проект` — они и так меряли любой
-- проект, просто лежали в чужом.
-- Пункт, неприменимый К ЭТОМУ проекту.
--
-- Не исключение из правила и не прощённое нарушение: `.sqlx` и HTTP-контракт
-- есть у одного проекта и не будет у другого никогда — там нечего проверять и
-- нечего прощать. Прежде такой пункт вечно отвечал «неизвестно», и это верно
-- ровно до тех пор, пока никто не сказал, что ответа не будет.
--
-- Причина обязательна, и объявивший назван: неприменимость — решение, а не
-- умолчание, и однажды её придётся пересмотреть.
-- Цели проекта. Цель без способа измерить — намерение, и потому у неё три
-- обязательные части: чем измеряется, когда проверяется и что считается
-- провалом. Уровень отличает состоятельность замысла от продуктовой ценности:
-- первую проверяют машиной до всякого пользователя, вторую — только на людях.
-- Сценарий приёмки: чем закрывается пункт приёмки истории.
--
-- У проверки набора четыре обязательные части: предусловия, шаги, наблюдаемый
-- результат и провал. Сценарий без наблюдаемого результата — не проверка, а
-- пожелание; сценарий без провала не отличает «прошло» от «не смотрели».
-- Что фича объявляет своим: требования, которые она несёт, и статьи, на которые
-- опирается. Смежные, «проверяемые в другой фиче», её связью не считаются —
-- иначе всякое требование окажется у всех, и покрытие перестанет что-то значить.
-- Требование бывает не только функциональным и нефункциональным.
--
-- Ограничение знало два вида, и пятьдесят три требования к интерфейсу — `UI-01`
-- и далее — в набор не принимались вовсе. Отказ выглядел поломкой ввоза, а был
-- отказом признать законный вид: интерфейсное требование такое же требование,
-- просто проверяется иначе.
-- У требования заголовок и тело — РАЗНЫЕ вещи, и колонки у них разные.
--
-- Прежде они склеивались в одну: «Индекс строится инкрементально. Индекс —
-- производное: он…». Так нельзя ни показать перечень, ни сравнить формулировку
-- с источником, ни заметить, что тело потерялось, — заголовок его подменял.
--
-- `measured_by` был заведён и стоял пустым, хотя источник у каждого требования
-- пишет «Проверяется: тест такой-то». Способ доказательства — часть требования,
-- а не примечание к нему.
ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS title text NOT NULL DEFAULT '';

-- Требование бывает вне выпуска и бывает сквозным, и оба состояния надо
-- объявить, а не вычислять отсутствием связи.
--
-- `FR-104`, `FR-107…112` не взяты ни в один этап — потому что план прямо пишет
-- «Вне v1». `NFR-06` не принадлежит ни одной фиче — потому что «один асинхронный
-- рантайм» есть свойство всей сборки. Без объявления оба читаются одинаково:
-- «связи нет», то есть как недоработка.
-- На что требование опирается: решения, статьи, экраны, соседние требования.
--
-- Источник пишет их хвостом строки — `**[ADR-0053, `66`]**`, — и до сих пор они
-- ехали внутрь формулировки. Ссылка в тексте требования не проверяется ничем:
-- отменённое решение остаётся стоять в скобках, и никто не узнает.
-- Чем статья конституции исполняется. Конституция объявляет это сама —
-- машиночитаемым реестром внизу документа: статья, имя гейта, состояние.
--
-- Состояние существенно: `enforced` значит «гейт работает», `planned` — «гейт
-- назван и не написан». Статья с одним лишь запланированным гейтом держится
-- текстом, а не проверкой, и цель Ц-01 требует различать эти два случая.
-- Контракт протокола: операция и парные ей события.
--
-- У tot-ade нет HTTP-контракта — связь идёт операциями и событиями крейта
-- `tot-protocol`, и объявлены они таблицей документа протокола. Пока это проза,
-- гейт `protocol:event-coverage` проверить нечем: он обещает, что у каждой
-- операции есть парное событие, а перечня операций в наборе нет.
-- Крейты репозитория, как их объявляет архитектура: что крейт делает и —
-- отдельной колонкой — чего НЕ делает.
--
-- Второе важнее первого: «не делает» есть проведённая граница, и нарушают её
-- молча. У датчика репозитория крейты уже перечислены фактом; здесь лежит то,
-- чем они обязаны быть.
-- Объявленный стенд: чем именно выполнены правила эталонного размера.
--
-- Пока стенд не объявлен, числовые бюджеты сравнимы только сами с собой: «60 мс»
-- на неназванной машине не значит ничего. Документ стенда сам это и говорит.
-- Алгоритм истории: как она проходится шагами, и чем каждый шаг закреплён.
--
-- Алгоритм — единственное место, где история, фичи, страницы, требования и
-- операции протокола названы вместе. Без него связь «сценарий приёмки ⇄ шаг
-- алгоритма» существует только в прозе, и проверить её нечем.
-- Происхождение справочного документа: откуда взят, когда и с каким отпечатком.
--
-- Раскладка объявляет виду `reference` проекцию `provenance` — «ценность в том,
-- откуда взято и когда». До сих пор это лежало шапкой в тексте: сто восемь
-- документов, у каждого источник, дата снятия и sha, и ни одного способа
-- спросить «что у нас из tot-dev и не устарело ли».
-- Токены оформления: имя, значение в тёмной и светлой теме, назначение.
--
-- Токен — то единственное в макете, что проверяется машиной: цвет, названный
-- дважды разными значениями, есть расхождение, а не вкус. `ART` про контраст
-- опирается ровно на эту таблицу.
-- Разбор случившегося: сводка, хронология, корневая причина, урок.
--
-- Риски смотрят вперёд, постмортемы назад, предмет один. Корневая причина —
-- главное поле: без неё разбор есть рассказ о неприятности, а не знание,
-- которым можно не повторить.
-- Что поставляется: артефакт выпуска и куда он ставится.
--
-- Поставка — последняя стадия, и её долг виден только перечнем: артефакт,
-- объявленный поставляемым, но не собираемый ничем, есть обещание без сборки.
CREATE TABLE IF NOT EXISTS project_release_artifact (
  project_id text NOT NULL,
  name text NOT NULL,
  what text NOT NULL DEFAULT '',
  installed_to text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

CREATE TABLE IF NOT EXISTS project_postmortem (
  project_id text NOT NULL,
  id text NOT NULL,
  title text NOT NULL DEFAULT '',
  summary text NOT NULL DEFAULT '',
  timeline text NOT NULL DEFAULT '',
  root_cause text NOT NULL DEFAULT '',
  lesson text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS project_token (
  project_id text NOT NULL,
  name text NOT NULL,
  dark text NOT NULL DEFAULT '',
  light text NOT NULL DEFAULT '',
  purpose text NOT NULL DEFAULT '',
  section text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

CREATE TABLE IF NOT EXISTS project_reference_source (
  project_id text NOT NULL,
  entity_name text NOT NULL,
  source text NOT NULL DEFAULT '',
  note text NOT NULL DEFAULT '',
  taken text NOT NULL DEFAULT '',
  sha text NOT NULL DEFAULT '',
  ref_type text NOT NULL DEFAULT '',
  from_project text NOT NULL DEFAULT '',
  repo text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, entity_name));

CREATE TABLE IF NOT EXISTS project_algorithm (
  project_id text NOT NULL,
  id text NOT NULL,
  story_id text NOT NULL DEFAULT '',
  title text NOT NULL DEFAULT '',
  preconditions text NOT NULL DEFAULT '',
  flow text NOT NULL DEFAULT '',
  failure_branches text NOT NULL DEFAULT '',
  not_covered text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS project_algorithm_links (
  project_id text NOT NULL,
  algorithm_id text NOT NULL,
  kind text NOT NULL,
  target text NOT NULL,
  PRIMARY KEY (project_id, algorithm_id, kind, target));

CREATE TABLE IF NOT EXISTS project_stand (
  project_id text NOT NULL,
  section text NOT NULL,
  name text NOT NULL,
  value text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, section, name));

CREATE TABLE IF NOT EXISTS project_crate (
  project_id text NOT NULL,
  name text NOT NULL,
  does text NOT NULL DEFAULT '',
  does_not text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

CREATE TABLE IF NOT EXISTS project_protocol_op (
  project_id text NOT NULL,
  op text NOT NULL,
  op_group text NOT NULL DEFAULT '',
  events text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, op));

CREATE TABLE IF NOT EXISTS project_protocol_requirements (
  project_id text NOT NULL,
  op_group text NOT NULL,
  requirement_id text NOT NULL,
  PRIMARY KEY (project_id, op_group, requirement_id));

CREATE TABLE IF NOT EXISTS project_article_gates (
  project_id text NOT NULL,
  article integer NOT NULL,
  gate text NOT NULL,
  state text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, article, gate));

CREATE TABLE IF NOT EXISTS project_requirement_sources (
  project_id text NOT NULL,
  requirement_id text NOT NULL,
  kind text NOT NULL,
  target text NOT NULL,
  PRIMARY KEY (project_id, requirement_id, kind, target));

ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS out_of_version text NOT NULL DEFAULT '';
ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS crosscutting text NOT NULL DEFAULT '';


DO $$ BEGIN
  ALTER TABLE project_requirements DROP CONSTRAINT IF EXISTS project_requirements_kind_check;
  ALTER TABLE project_requirements ADD CONSTRAINT project_requirements_kind_check
    CHECK (kind IN ('FR','NFR','UI','ST'));
END $$;

-- Что история объявляет своим: кто в ней действует и какими страницами она
-- закрывается. Роль записана в разделе «Кто и что хочет» и до сих пор терялась:
-- истории лежали именем и заголовком, а того, ради кого они написаны, в наборе
-- не было вовсе.
-- Что экран объявляет своим: требования, которые он держит, когда открывается и
-- что показывает, когда пусто или сломано. Последнее — не украшение: пустое
-- состояние, не описанное заранее, реализация выдумает сама.
-- Что этап объявляет своим: что делается, чем блокирован, какие требования
-- закрывает и какие гейты включает. Раньше от этапа хранились имя и заголовок,
-- а «чем блокирован» и «какие гейты включаются» — то, ради чего этап и пишут, —
-- оставались прозой.
-- Стадии работы и чем каждая закрывается.
--
-- Фазы стандарта их не различают, и от этого долг попадал не в свою колонку:
-- «нет кода» на стадии, которая кода не пишет, долгом не является. Стадия
-- названа вместе с тем, что производит и чем закрывается, — иначе «фаза без
-- критерия» остаётся намерением.
-- Гарантии продукта и правила внешних заявлений.
--
-- Гарантия — то, что продукт обещает наружу, и держится она гейтом, а не
-- текстом: заявленная гарантия без работающего гейта есть продажа пожелания
-- (`10-intent/goals.md` Ц-01). Оттого у гарантии два поля — чем держится и
-- работает ли это сейчас.
-- Связь решения с требованием — тоже связь.
--
-- Ограничение знало пять видов: уточняет, отменяет, закрывает, правит статью,
-- связано. Того, что решение ТРОГАЕТ требование, среди них не было — а решение
-- почти всегда пишется про требование, и эта связь самая частая.
DO $$ BEGIN
  ALTER TABLE project_decision_links DROP CONSTRAINT IF EXISTS project_decision_links_kind_check;
  ALTER TABLE project_decision_links ADD CONSTRAINT project_decision_links_kind_check
    CHECK (kind IN ('refines','related','relates','relates-to-decision','supersedes',
                    'closes','amends-article','touches'));
END $$;

-- Прогон: что на самом деле произошло с задачей.
--
-- Документ прогона — единственное место, где записано, чем задача закончилась:
-- диапазон коммитов, даты, ревью и — главное — что осталось открытым. Последнее
-- терялось целиком: задача закрывалась, а её открытый хвост оставался прозой.
CREATE TABLE IF NOT EXISTS project_run_record (
  project_id text NOT NULL,
  id text NOT NULL,
  task_id text NOT NULL DEFAULT '',
  milestone_id text NOT NULL DEFAULT '',
  title text NOT NULL DEFAULT '',
  commits text NOT NULL DEFAULT '',
  dates text NOT NULL DEFAULT '',
  review text NOT NULL DEFAULT '',
  appeared text NOT NULL DEFAULT '',
  left_open text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS project_guarantee (
  project_id text NOT NULL,
  number integer NOT NULL,
  title text NOT NULL,
  held_by text NOT NULL DEFAULT '',
  body text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, number));

-- Правило внешнего заявления: что разрешено утверждать и чем это закрепляется.
CREATE TABLE IF NOT EXISTS project_claim_rule (
  project_id text NOT NULL,
  number integer NOT NULL,
  title text NOT NULL,
  body text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, number));

CREATE TABLE IF NOT EXISTS project_stage (
  project_id text NOT NULL,
  ord integer NOT NULL,
  name text NOT NULL,
  produces text NOT NULL DEFAULT '',
  closed_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

-- Раскладка набора по фазам: какой артефакт фазы есть, какой частично, какого
-- нет. Состояние объявлено самим документом, а не выведено из наличия файла:
-- документ может лежать и быть заглушкой.
CREATE TABLE IF NOT EXISTS project_phase_artifact (
  project_id text NOT NULL,
  phase text NOT NULL,
  artifact text NOT NULL,
  we_have text NOT NULL DEFAULT '',
  state text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, phase, artifact));

ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS what text NOT NULL DEFAULT '';
ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS blocked_by text NOT NULL DEFAULT '';
-- Закрытие вехи — дата, а не абзац. Веха `M0` закрылась до того, как завели
-- дисциплину задач-документов, и потому не имеет ни одной: ступень «у каждого
-- этапа есть задачи» спрашивала с неё разбивку через полгода после закрытия.
ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS closed text NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS project_milestone_requirements (
  project_id text NOT NULL,
  milestone_id text NOT NULL,
  requirement_id text NOT NULL,
  PRIMARY KEY (project_id, milestone_id, requirement_id));

CREATE TABLE IF NOT EXISTS project_milestone_gates (
  project_id text NOT NULL,
  milestone_id text NOT NULL,
  gate text NOT NULL,
  PRIMARY KEY (project_id, milestone_id, gate));

CREATE TABLE IF NOT EXISTS project_screen_requirements (
  project_id text NOT NULL,
  screen_id text NOT NULL,
  requirement_id text NOT NULL,
  PRIMARY KEY (project_id, screen_id, requirement_id));

ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS opens_when text NOT NULL DEFAULT '';
ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS empty_and_broken text NOT NULL DEFAULT '';
ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS purpose text NOT NULL DEFAULT '';
-- Экран, объявленный отложенным, задачи в текущем выпуске не имеет — и не
-- должен. Без колонки он неотличим от забытого.
ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS out_of_version text NOT NULL DEFAULT '';

-- Предполёт у самой задачи, а не только отдельной таблицей вердиктов.
--
-- Вердикт — свойство задачи, и читатель плана обязан видеть его там же, где
-- состояние: без этих колонок «готова ли задача к работе» отвечалось join-ом,
-- и потому не отвечалось вовсе — ни доска, ни `waves`, ни `next-task` его не
-- показывали. `preflight_revision` держит правку документа, НА КОТОРОЙ вердикт
-- снят: он относится к тексту, а не к имени, и правка задачи его отменяет.
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS preflight text NOT NULL DEFAULT '';
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS preflight_at bigint;
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS preflight_revision bigint;
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS preflight_findings integer;
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS preflight_fresh boolean;

-- Проект как сущность: у него есть имя и репозиторий.
--
-- Проектов не существовало таблицей — только строкой `project_id` в двух
-- десятках других, вперемешку с двенадцатью огрызками проб (`s1`…`p6`,
-- `mine`). Из-за этого клиент не мог спросить «чей это репозиторий» и держал
-- ответ файлом в самом репозитории: `.harness/mh.json`. Знание о том, какому
-- проекту принадлежит дерево, — знание сервера, а не дерева.
CREATE TABLE IF NOT EXISTS project (
  id text PRIMARY KEY,
  name text NOT NULL DEFAULT '',
  repo text NOT NULL DEFAULT '',
  declared_at bigint NOT NULL DEFAULT 0,
  declared_by text NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS project_by_repo ON project (repo);

-- Чем сторож ловит: путь, содержимое, текст команды.
--
-- Само правило («донор заморожен») уже лежало строкой, а поймать по нему было
-- нечем: ловля жила отдельным файлом `.harness/probity.ts`, который сервер не
-- читает. Объявление без способа поймать — пожелание.
ALTER TABLE project_guard ADD COLUMN IF NOT EXISTS acts_on text NOT NULL DEFAULT 'write';
ALTER TABLE project_guard ADD COLUMN IF NOT EXISTS path_re text NOT NULL DEFAULT '';
ALTER TABLE project_guard ADD COLUMN IF NOT EXISTS content_re text NOT NULL DEFAULT '';
ALTER TABLE project_guard ADD COLUMN IF NOT EXISTS command_re text NOT NULL DEFAULT '';

-- Чем датчик снимает факт с репозитория.
--
-- Пять датчиков жили пятью скриптами на другом языке: каждый ходил по своим
-- файлам своим образцом и подавал свой род факта. Общего у них ровно это —
-- «где искать, что вынимать, чем назвать», — и оно объявляется, а исполняет
-- клиент, у которого есть репозиторий.
CREATE TABLE IF NOT EXISTS project_sensor_spec (
  project_id text NOT NULL,
  fact text NOT NULL,
  reads text NOT NULL DEFAULT '',
  extract_re text NOT NULL DEFAULT '',
  note text NOT NULL DEFAULT '',
  how text NOT NULL DEFAULT 'extract',
  PRIMARY KEY (project_id, fact));
-- Чем снимается факт, тремя способами, и третий — не выемка.
--
-- `extract` — образец по содержимому: имена требований в контракте, таблицы в
-- миграциях. `files` — сам факт существования файла: дисковая сторона сверки
-- с объявленным деревом; сравнивает потом сервер, потому что объявленная
-- сторона уже лежит у него таблицей документа. `secret-fields` — обход
-- объявлений Rust: `#[derive(Debug)]`, за ним поля, и голое секрето-подобное
-- имя среди них. Последнее не выражается образцом и потому названо родом.
ALTER TABLE project_sensor_spec ADD COLUMN IF NOT EXISTS how text NOT NULL DEFAULT 'extract';
-- Строка, которую датчик НЕ считает находкой. Комментарий — не код: правило
-- «не кричать прописными» ловит слово `uppercase` в объяснении самого правила,
-- и без этого пропуска у каждого правила появляется столько ложных находок,
-- сколько раз о нём написали. Пустая строка значит «пропускать нечего».
-- ИМЯ ПУНКТА ГЕЙТА — устойчивый ярлык, а не его заголовок.
--
-- Ключом было предложение по-русски: «таблица миграции описана в модели
-- данных». Всякая правка формулировки рвала отметки, отмены и исключения —
-- строка с прежним заголовком оставалась сиротой, а пункт заводился заново
-- пустым. Заголовок теперь просто текст, который можно переписать; адресует
-- пункт `id`.
ALTER TABLE gate_item ADD COLUMN IF NOT EXISTS id text NOT NULL DEFAULT '';
-- ИСПОЛНЯЕТСЯ ЛИ ПРОБА. `null` — не проверяли; `false` — проба есть, но она не
-- запрос, и пункт не роняли НИ РАЗУ. Зелёное у такого пункта не значит ничего,
-- и молчать об этом нельзя: гейт для того и написан, чтобы не было зелёного,
-- которое никто не проверял.
ALTER TABLE gate_item ADD COLUMN IF NOT EXISTS probe_ok boolean;
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS probe_ok boolean;
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS id text NOT NULL DEFAULT '';
ALTER TABLE gate_item_waiver ADD COLUMN IF NOT EXISTS id text NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS gate_item_by_id ON gate_item (phase, id) WHERE id <> '';
CREATE UNIQUE INDEX IF NOT EXISTS project_gates_by_id ON project_gates (project_id, phase, id);
-- Прежний ключ отметки — заголовок. Переписали формулировку, и рядом со старой
-- отметкой легла вторая: гейт считал один пункт дважды. Ключ теперь имя.
DELETE FROM project_gates g WHERE NOT EXISTS
  (SELECT 1 FROM gate_item i WHERE i.phase = g.phase AND i.item = g.item);
ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_pkey;
ALTER TABLE project_gates ADD PRIMARY KEY (project_id, phase, id);
ALTER TABLE project_sensor_spec ADD COLUMN IF NOT EXISTS skip_re text NOT NULL DEFAULT '';
-- Места, где правило не действует, перечнем образцов пути через пробел.
-- Шкала размеров живёт в одном файле, и запрещать ей называть размеры значит
-- запрещать шкале быть шкалой.
ALTER TABLE project_sensor_spec ADD COLUMN IF NOT EXISTS allow text NOT NULL DEFAULT '';

-- ХРАПОВИК: объявленный потолок долга, который может только опускаться.
--
-- Гейт, краснеющий в день, когда его завели, назавтра выключают. Долг, который
-- уже есть, не должен никого держать; РОСТ долга — должен. Потолок называет,
-- сколько находок сегодня терпимо, и почему.
--
-- Красным становится и счёт НИЖЕ потолка: долг погасили, а потолок не опустили —
-- значит завтра он молча вырастет обратно, и никто не заметит.
CREATE TABLE IF NOT EXISTS rule_ceiling (
  project_id text NOT NULL,
  rule text NOT NULL,
  ceiling integer NOT NULL,
  why text NOT NULL DEFAULT '',
  origin text NOT NULL DEFAULT 'declared',
  PRIMARY KEY (project_id, rule));
ALTER TABLE rule_exception ADD COLUMN IF NOT EXISTS closes text NOT NULL DEFAULT '';
ALTER TABLE rule_exception ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';
ALTER TABLE project_feature_requirements ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';
ALTER TABLE project_milestone_requirements ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';
ALTER TABLE project_screen_requirements ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';
-- Родитель красной задачи — КОЛОНКА, а не первые два знака её имени. Имя
-- разбирают глазами, колонку соединяют равенством.
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS parent_task_id text NOT NULL DEFAULT '';
-- Каталог, куда ляжет лист: разбор пути — дело проекции, а не запроса.
ALTER TABLE project_task_tree_leaf ADD COLUMN IF NOT EXISTS target_dir text NOT NULL DEFAULT '';
-- Порядок задачи внутри этапа — её НОМЕР, а не место в алфавите: `ord` ставит
-- `T10` вторым, до `T9`, и правило «зависит от более поздней» врёт на каждой
-- второй паре. Номер ставится проекцией один раз, запрос сравнивает числа.
ALTER TABLE project_plan_tasks ADD COLUMN IF NOT EXISTS number integer NOT NULL DEFAULT 0;
-- «Описано вперёд»: каталог назван документом дерева как отсутствующий, и
-- задача вправе класть туда код — она его и заведёт. Считается по ПРЕДКУ:
-- объявлено `mobile`, значит и `mobile/src/offline` описан вперёд.
ALTER TABLE project_task_tree_leaf ADD COLUMN IF NOT EXISTS forward_declared boolean NOT NULL DEFAULT false;
-- Ключ фазы у строки процесса. Прежде фаза жила только прозой («Фаза 2 ·
-- Проектирование»), и правило искало её подстрокой: переименовали заголовок —
-- правило перестало находить и позеленело, ничего не сказав.
ALTER TABLE project_phase_artifact ADD COLUMN IF NOT EXISTS phase_id text NOT NULL DEFAULT '';
-- Крейт репозитория и сколько в нём тестовых функций. Колонка, а не счёт на
-- лету: правило сверяет число из документа прослеживаемости с этим, и обе
-- стороны обязаны быть значениями.
ALTER TABLE project_crate ADD COLUMN IF NOT EXISTS in_repo boolean NOT NULL DEFAULT false;
ALTER TABLE project_crate ADD COLUMN IF NOT EXISTS test_functions integer NOT NULL DEFAULT 0;
ALTER TABLE project_traceability_said ADD COLUMN IF NOT EXISTS is_total boolean NOT NULL DEFAULT false;
-- Имя, ВЗЯТОЕ ИЗ ПЕРЕЧНЯ, а не написанное целиком: `SCR-CFG-01…18` называет
-- восемнадцать экранов. Для покрытия это имя названо, для указателя — нет:
-- указатель обязан написать имя, а перечень вправе его сократить.
ALTER TABLE project_named_id ADD COLUMN IF NOT EXISTS from_range boolean NOT NULL DEFAULT false;
-- На чём держится отмена пункта: род факта, который обязан оставаться ПУСТЫМ.
-- Отмена «у проекта нет sqlx» верна, пока датчик sqlx ничего не подаёт; появился
-- хоть один факт — довод отмены исчез. Без этой колонки отмена живёт прозой и
-- тухнет молча: у myack пункт про крейты был отменён доводом «крейтов нет», а
-- их было четырнадцать.
ALTER TABLE gate_item_waiver ADD COLUMN IF NOT EXISTS holds_while_empty text NOT NULL DEFAULT '';

-- Донорский код: чужая реализация, замороженная на запись.
--
-- Набор говорит «донор» одиннадцатью документами прозой, и ни одна колонка не
-- отличала донорское дерево от своего. Считать по нему строки, крейты и
-- покрытие — считать чужую работу своей; правило «донор заморожен» при этом
-- живёт хуком, который о наборе не знает.
CREATE TABLE IF NOT EXISTS project_donor (
  project_id text NOT NULL,
  path text NOT NULL,
  what text NOT NULL DEFAULT '',
  frozen_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, path));

-- Чем правило принуждается ДО действия: хук, а не замер.
--
-- Гейт меряет случившееся; сторож не даёт случиться. Статья 14 и статья 15
-- держатся обоими, и в базе была видна только половина — та, что меряется.
CREATE TABLE IF NOT EXISTS project_guard (
  project_id text NOT NULL,
  name text NOT NULL,
  enforces text NOT NULL DEFAULT '',
  scope text NOT NULL DEFAULT '',
  refuses text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name));

-- Раскладка видов — знание СЕРВЕРА, а не одного репозитория.
--
-- Она общая для всех проектов, а лежала файлом в `.harness/` одного из них:
-- сервер не поднимался без чужого репозитория, и «работаем только с сервером»
-- было неправдой в первой же строке запуска. Файл остаётся способом ЗАВЕСТИ
-- раскладку, но не местом, где она живёт.
CREATE TABLE IF NOT EXISTS kind_layout (
  name text PRIMARY KEY,
  spec jsonb NOT NULL,
  declared_at bigint NOT NULL DEFAULT 0,
  declared_by text NOT NULL DEFAULT '');

-- Одна проверка доказывает НЕСКОЛЬКО требований: гейт `criterion:agent-write-denied`
-- назван и в FR-05, и в FR-22. Колонка `requirement_id` держит одно имя, и
-- второе требование выглядело непокрытым при названном доказательстве.
CREATE TABLE IF NOT EXISTS project_check_requirements (
  project_id text NOT NULL,
  check_id text NOT NULL,
  requirement_id text NOT NULL,
  PRIMARY KEY (project_id, check_id, requirement_id));
-- Своё у справки — откуда снята; чужое — что о себе говорил исходный документ.
-- Эти пять повторяются у десятков справок и потому колонки, а не проза в шапке.
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS written text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS updated text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS tags text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS role text NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS project_story_screens (
  project_id text NOT NULL,
  story_id text NOT NULL,
  screen_id text NOT NULL,
  PRIMARY KEY (project_id, story_id, screen_id));

CREATE TABLE IF NOT EXISTS project_feature_requirements (
  project_id text NOT NULL,
  feature_id text NOT NULL,
  requirement_id text NOT NULL,
  PRIMARY KEY (project_id, feature_id, requirement_id));

CREATE TABLE IF NOT EXISTS project_feature_articles (
  project_id text NOT NULL,
  feature_id text NOT NULL,
  article integer NOT NULL,
  PRIMARY KEY (project_id, feature_id, article));

CREATE TABLE IF NOT EXISTS project_acceptance (
  project_id text NOT NULL,
  id text NOT NULL,
  story_id text NOT NULL DEFAULT '',
  number integer NOT NULL DEFAULT 0,
  title text NOT NULL,
  preconditions text NOT NULL DEFAULT '',
  steps text NOT NULL DEFAULT '',
  observed text NOT NULL DEFAULT '',
  fails_when text NOT NULL DEFAULT '',
  origin text NOT NULL DEFAULT 'declared',
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS project_goal (
  project_id text NOT NULL,
  id text NOT NULL,
  number integer NOT NULL DEFAULT 0,
  level text NOT NULL DEFAULT '',
  title text NOT NULL,
  measured_by text NOT NULL DEFAULT '',
  checked_when text NOT NULL DEFAULT '',
  fails_when text NOT NULL DEFAULT '',
  state_now text NOT NULL DEFAULT '',
  origin text NOT NULL DEFAULT 'declared',
  PRIMARY KEY (project_id, id));

CREATE TABLE IF NOT EXISTS gate_item_waiver (
  project_id text NOT NULL,
  phase text NOT NULL,
  item text NOT NULL,
  why text NOT NULL,
  declared_at bigint NOT NULL,
  declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, phase, item));

-- Словарь схемы: РОЛЬ, которую знает код, и ЗНАЧЕНИЕ, которым её зовёт набор.
--
-- Общий на все проекты, как гейты и фазы: разрабатываем по одной схеме, и
-- «раздел доказательства» зовётся в ней одинаково везде. Но зовётся он словом,
-- а слово — данные. Зашитое в бинарник, оно делает бинарник знающим один
-- проект: другой набор, назвавший раздел иначе, получил бы молчаливый ноль.
--
-- Роль стабильна и на латинице — её пишет код. Значение меняется правкой
-- строки, а не сборкой.
CREATE TABLE IF NOT EXISTS scheme_term (
  role text NOT NULL,
  value text NOT NULL,
  ord integer NOT NULL DEFAULT 0,
  why text NOT NULL DEFAULT '',
  PRIMARY KEY (role, value));

CREATE TABLE IF NOT EXISTS gate_item (
  phase text NOT NULL,
  item text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('query','command','signed','manual','unknown')),
  query text,
  owner text,
  probe text NOT NULL DEFAULT '',
  why text NOT NULL DEFAULT '',
  article integer,
  PRIMARY KEY (phase, item));

CREATE TABLE IF NOT EXISTS gate_head (
  phase text PRIMARY KEY,
  title text NOT NULL DEFAULT '',
  needs_signature boolean NOT NULL DEFAULT true);

CREATE TABLE IF NOT EXISTS phase (
  id text PRIMARY KEY,
  ord integer NOT NULL,
  title text NOT NULL,
  gate text NOT NULL DEFAULT '',
  plan_level text NOT NULL DEFAULT '',
  task_kind text NOT NULL DEFAULT '');

-- Переноса объявлений из проектных таблиц здесь БОЛЬШЕ НЕТ, и это не упущение.
-- Он был нужен однажды, когда объявление переезжало из проектных таблиц в
-- общую. Оставшись, он воскрешал снятое: пункт удаляли из объявления, его
-- строка измерения оставалась у проекта, и следующий запуск объявлял пункт
-- заново. Снятое возвращалось само, и выглядело это как чужая правка.
-- Перенос завершён: измерений без объявления нет ни одного.

-- ЗДЕСЬ БЫЛА ЧИСТКА, И ОНА ЕЛА ОБЪЯВЛЕНИЯ.
--
-- Первый перенос занёс семь тестовых пунктов, и правило снимало всё, чего нет в
-- замерах живого проекта. Но у ТОЛЬКО ЧТО объявленного пункта замера ещё нет —
-- он появляется первым `gate-measure`, — и перезапуск между объявлением и
-- замером стирал объявление молча. Так исчезли четыре подписных пункта, только
-- что заведённых: `gate_item` показывал 36 вместо 40, и ни одного слова о том,
-- куда делись четыре.
--
-- Своё дело чистка сделала давно: тестовых пунктов нет. Правило, стирающее
-- новое ради уборки старого, вредно чистым итогом — снято.

INSERT INTO gate_head (phase, title, needs_signature)
SELECT DISTINCT ON (phase) phase, title, needs_signature FROM gate ORDER BY phase, project_id
ON CONFLICT (phase) DO NOTHING;

INSERT INTO phase (id, ord, title, gate, plan_level, task_kind)
SELECT DISTINCT ON (id) id, ord, title, gate, plan_level, task_kind
  FROM project_phase ORDER BY id, project_id
ON CONFLICT (id) DO NOTHING;


-- Снятые требования: имя, которого больше нет, и почему.
--
-- Устроено как `term_retired` у словаря. Решение, написанное когда требование
-- ещё было, ссылается на него и после снятия — и это история, а не обрыв.
-- Отличить историю от опечатки можно только объявлением: снятое названо, всё
-- прочее остаётся находкой.
CREATE TABLE IF NOT EXISTS requirement_retired (
  project_id text NOT NULL,
  id text NOT NULL,
  why text NOT NULL DEFAULT '',
  retired_by text NOT NULL DEFAULT '',
  declared_at bigint NOT NULL,
  declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, id));

-- Где мы на лестнице — СОХРАНЁННОЕ, а не посчитанное при вопросе.
--
-- Одиннадцать запросов на каждый «где мы» — та же цена, что была у гейтов, и с
-- тем же изъяном: агент, спросивший дважды подряд, платил дважды, а между
-- правкой документа и вопросом никто не считал вообще ничего.
--
-- Колонки названы, а не свалены в один свёрток: «на какой мы ступени» и «открыта
-- ли фаза набора» спрашивают запросом, а не разбором json. Свёрток рядом держит
-- то, у чего своей колонки быть не может, — списки найденного.
CREATE TABLE IF NOT EXISTS process_position (
  project_id text NOT NULL,
  process text NOT NULL,
  at_ord integer,
  at_state text NOT NULL DEFAULT '',
  at_question text NOT NULL DEFAULT '',
  at_owner text NOT NULL DEFAULT '',
  at_owner_kind text NOT NULL DEFAULT '',
  at_touches text NOT NULL DEFAULT '',
  passed integer NOT NULL DEFAULT 0,
  skipped integer NOT NULL DEFAULT 0,
  unanswerable integer NOT NULL DEFAULT 0,
  corpus_phase_open boolean NOT NULL DEFAULT true,
  result jsonb,
  checked_at bigint,
  PRIMARY KEY (project_id, process));

-- Фаза: сколько её документов написано, сколько её задач закрыто, что говорит
-- её гейт. Всё числами в своих колонках — ими и спрашивают.
-- Закрыт ли выпуск. Своя таблица, а не колонка в `project_plan_versions`: та —
-- проекция, её переписывает каждая пересборка, и поставленное руками исчезло бы
-- молча.
--
-- Нужна ради слепка. `version_freeze` снимается ОДИН раз и служит той чертой, от
-- которой считают, что изменилось после выпуска. Снять её посреди работы значит
-- провести черту не там: дельта навсегда останется пустой, а второй раз
-- заморозить нельзя — это стёрло бы то, с чем сравнивают.
-- Умение ступени едет за ней при перенумерации.
--
-- Связь была объявлена только `ON DELETE CASCADE`: снятая ступень уносила своё
-- умение, а переехавшая — оставляла его показывать на пустое место. Вставка
-- ступени в середину лестницы на этом и споткнулась.
DO $$ BEGIN
  ALTER TABLE harness_process_step_skill
    DROP CONSTRAINT IF EXISTS harness_process_step_skill_set_name_process_ord_fkey;
  ALTER TABLE harness_process_step_skill
    ADD CONSTRAINT harness_process_step_skill_set_name_process_ord_fkey
    FOREIGN KEY (set_name, process, ord)
    REFERENCES harness_process_step(set_name, process, ord)
    ON DELETE CASCADE ON UPDATE CASCADE;
END $$;

-- Датчики репозитория: кто обязан подавать факты.
--
-- `fact_push` помнит, КТО подал. Кто подать обязан — не помнил никто, и оттого
-- молчание датчика было неотличимо от его отсутствия. Пустой список тут врёт
-- ровно так же, как врал он у сверки порождённого.
--
-- `stale_after_ms` пусто — проверяется только «подавал ли когда-нибудь». Срок
-- годности объявляется отдельно и осознанно: выдумать его за проект нельзя.
CREATE TABLE IF NOT EXISTS sensor (
  project_id text NOT NULL,
  fact text NOT NULL,
  about text NOT NULL DEFAULT '',
  stale_after_ms bigint,
  declared_at bigint NOT NULL,
  declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, fact));

CREATE TABLE IF NOT EXISTS version_state (
  project_id text NOT NULL,
  version text NOT NULL,
  state text NOT NULL CHECK (state IN ('open','closed')),
  changed_at bigint NOT NULL,
  changed_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, version));

CREATE TABLE IF NOT EXISTS phase_state (
  project_id text NOT NULL,
  phase text NOT NULL,
  ord integer NOT NULL,
  title text NOT NULL DEFAULT '',
  gate text NOT NULL DEFAULT '',
  gate_state text NOT NULL DEFAULT '',
  -- Пусто, если фаза документов не объявляет: ноль сказал бы «объявила и нет их».
  documents_present integer,
  documents_absent integer,
  documents_declared integer,
  tasks_closed integer,
  tasks_open integer,
  tasks_total integer,
  checked_at bigint,
  PRIMARY KEY (project_id, phase));


-- Что изменилось и требует пересчёта. Одна строка на проект: пересчёт всё равно
-- общий, а десять правок подряд обязаны стоить одного прогона, а не десяти.
--
-- Отметка лежит В БАЗЕ, а не в памяти процесса: пишут в набор двое — сервер по
-- HTTP и `mh-server mcp`, запускаемый отдельным процессом. Флаг в памяти одного
-- из них второй бы не увидел, и правка через MCP осталась бы непосчитанной.
CREATE TABLE IF NOT EXISTS gate_dirty (
  project_id text PRIMARY KEY,
  dirty_at bigint NOT NULL,
  reason text NOT NULL DEFAULT '',
  ran_at bigint,
  ran_ms integer);
"#;

/// Завести таблицы, если их ещё нет. Зовётся один раз при старте.
///
/// Не при каждой пересборке: `CREATE TABLE IF NOT EXISTS` печатает NOTICE, а
/// печатать что-либо в разговоре по stdio нельзя.
pub async fn ensure(pool: &Pool) -> Result<(), tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    client.batch_execute(DDL).await?;
    // Представления заводятся после таблиц: они их читают.
    client.batch_execute(VIEWS).await
}

/// Проекции, которые обязаны быть готовы ДО донорской пересборки.
///
/// Донорские проекции читают `task_requirement`; посчитанная после них, она
/// накормила бы их данными прошлого круга — расхождение на один шаг, невидимое
/// глазом и оттого худшее.
pub async fn rebuild_before(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;

    tx.execute("DELETE FROM task_requirements_declared WHERE project_id = $1", &[&project]).await?;
    let declared = tx
        .execute(
            "INSERT INTO task_requirements_declared (project_id, task_id, has_own, note)
             SELECT t.project_id, t.id,
                    CASE WHEN f.value IS NULL OR btrim(f.value) = '' THEN NULL
                         WHEN f.value ~* '^\\s*(нет собственных|нет\\M|—|–)' THEN false
                         ELSE true END,
                    coalesce(f.value, '')
               FROM project_plan_tasks t
               LEFT JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name AND f.name ILIKE 'Требовани%'
              WHERE t.project_id = $1
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;
    tx.execute("DELETE FROM task_requirement WHERE project_id = $1", &[&project]).await?;
    let links = tx
        .execute(
            "INSERT INTO task_requirement (project_id, task_id, requirement_id)
             SELECT d.project_id, d.task_id, m[1]
               FROM task_requirements_declared d,
                    LATERAL regexp_matches(d.note, '((?:FR|NFR)-[A-Z0-9]+(?:-[0-9]+)?)', 'g') m
              WHERE d.project_id = $1 AND d.has_own IS NOT false
                AND EXISTS (SELECT 1 FROM project_requirements q
                             WHERE q.project_id = d.project_id AND q.id = m[1])
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;
    tx.commit().await?;
    Ok(json!({ "task_requirements_declared": declared, "task_requirement": links }))
}

/// Пересобрать проекции этого сервера. Возвращает счёт по каждой.
pub async fn rebuild(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    // Отладка: какой из запросов упал, видно по порядку в логе.
    // Пересборка идёт ОДНОЙ транзакцией.
    //
    // Каждая проекция считается как «удалить и записать заново». Без транзакции
    // между этими двумя шагами есть окно, в котором таблица пуста, — и читатель,
    // попавший в него, видит не «пересчитывается», а «ничего нет». Интерфейс на
    // этом уже показал 20 неизвестных вместо 1758 и выглядел правдой.
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;

    // ── Событие сущности ─────────────────────────────────────────────────────
    // Правило: вопрос ведёт журнал таблицей с шапкой «Дата · Событие · Кем».
    // Опознаётся шапкой, а не положением: таблица в документе не единственная.
    // Прочерк в дате — законное «даты нет», и он становится пустым значением, а
    // не выдуманным днём.
    // Стирается только выведенное из текста. След правки — не проекция: его
    // никто не пересчитает, и пересборка, сносящая его, стирает единственную
    // запись о том, кто и когда правил.
    tx
        .execute("DELETE FROM entity_event WHERE project_id = $1 AND source = 'declared'", &[&project])
        .await?;
    let events = tx
        .execute(
            "INSERT INTO entity_event (project_id, entity_kind, entity_id, ord, at, event, actor)
             SELECT q.project_id, 'question', q.id, c.row_ord,
                    CASE WHEN max(CASE WHEN c.col = 0 THEN c.value END) ~ '^\\d{4}-\\d{2}-\\d{2}$'
                         THEN max(CASE WHEN c.col = 0 THEN c.value END)::date END,
                    coalesce(max(CASE WHEN c.col = 1 THEN c.value END), ''),
                    coalesce(max(CASE WHEN c.col = 2 THEN c.value END), '')
               FROM project_questions q
               JOIN project_document_cells c
                 ON c.project_id = q.project_id AND c.entity_kind = q.entity_kind AND c.entity_name = q.entity_name
              WHERE q.project_id = $1 AND c.row_ord > 0
                AND c.block_ord IN (SELECT block_ord FROM project_document_cells h
                                     WHERE h.project_id = q.project_id AND h.entity_kind = q.entity_kind AND h.entity_name = q.entity_name
                                       AND h.row_ord = 0 AND h.col = 0 AND h.value = 'Дата')
              GROUP BY q.project_id, q.id, c.row_ord
             HAVING coalesce(max(CASE WHEN c.col = 1 THEN c.value END), '') <> ''
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Версия нормы ─────────────────────────────────────────────────────────
    // Правило: документ объявляет номер строкой «**Constitution version: X.Y
    // (дата)** — что изменилось». Номер цитируют другие документы, поэтому он и
    // есть идентификатор. Решение берётся первой ссылкой на ADR в той же строке.
    tx
        .execute("DELETE FROM norm_version WHERE project_id = $1", &[&project])
        .await?;
    let versions = tx
        .execute(
            "INSERT INTO norm_version (project_id, entity_kind, entity_id, version, at, changed, by_decision)
             SELECT d.project_id, 'constitution', 'constitution', m[1], m[2]::date, m[3],
                    substring(m[3] from 'ADR-\\d{4}')
               FROM project_documents d,
                    LATERAL regexp_matches(d.content,
                      '\\*\\*Constitution version: ([0-9.]+) \\(([0-9-]{10})\\)\\*\\* — ([^\\n]*(?:\\n(?!\\n)[^\\n]*)*)', 'g') m
              WHERE d.project_id = $1 AND d.entity_kind = 'constitution'
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Замер ────────────────────────────────────────────────────────────────
    // Правило: строка несёт слово «замер» и дату видом ГГГГ-ММ-ДД. Предмет —
    // заголовок раздела, под которым строка стоит; значение — сама строка.
    //
    // Строка НЕ разбирается на число и способ: формы записи в наборе разные, и
    // разложение здесь было бы догадкой. Догадка в замере хуже его отсутствия —
    // по ней потом сверят факт.
    tx
        .execute("DELETE FROM measurement WHERE project_id = $1", &[&project])
        .await?;
    let measurements = tx
        .execute(
            "INSERT INTO measurement (project_id, subject, at, value, method, stated_in_kind, stated_in_id)
             SELECT project_id, subject, at, value, '', entity_kind, entity_name FROM (
               SELECT b.project_id, b.entity_kind, b.entity_name,
                      coalesce((SELECT s.title FROM project_document_sections s
                                 WHERE s.project_id = b.project_id AND s.entity_kind = b.entity_kind AND s.entity_name = b.entity_name
                                   AND b.ord > s.first_block AND b.ord <= s.last_block
                                 ORDER BY s.level DESC LIMIT 1), b.entity_name) AS subject,
                      substring(m[1] from '\\d{4}-\\d{2}-\\d{2}')::date AS at,
                      left(m[1], 400) AS value,
                      row_number() OVER (
                        PARTITION BY coalesce((SELECT s.title FROM project_document_sections s
                                                WHERE s.project_id = b.project_id AND s.entity_kind = b.entity_kind AND s.entity_name = b.entity_name
                                                  AND b.ord > s.first_block AND b.ord <= s.last_block
                                                ORDER BY s.level DESC LIMIT 1), b.entity_name),
                                     substring(m[1] from '\\d{4}-\\d{2}-\\d{2}')) AS n
                 FROM project_document_blocks b,
                      LATERAL regexp_matches(b.raw, '[^\\n]*[Зз]амер[^\\n]*\\d{4}-\\d{2}-\\d{2}[^\\n]*', 'g') m
                WHERE b.project_id = $1) x
              WHERE n = 1
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Красная задача ───────────────────────────────────────────────────────
    // Имя `R-M1-T1` несёт родителя `M1-T1`; правило составления записано здесь:
    // отбрасывается приставка `R-`. Проверки — перечень из поля «Какие».
    tx.execute("DELETE FROM red_task_check WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM red_task WHERE project_id = $1", &[&project]).await?;
    let red = tx
        .execute(
            "INSERT INTO red_task (project_id, id, parent_task, milestone, checks)
             SELECT $1, d.entity_name,
                    -- Пару и веху набор называет своими словами: «Пара» и
                    -- «Веха». Прежние имена полей знал только один набор, и у
                    -- второго все 77 красных задач выходили без вехи — а
                    -- значит, и без места в плане.
                    coalesce(max(CASE WHEN f.name ILIKE 'Родительская%' OR f.name = 'Пара'
                                      THEN f.value END), ''),
                    coalesce(max(CASE WHEN f.name ILIKE 'Этап родителя%' OR f.name = 'Веха'
                                      THEN f.value END), ''),
                    -- Число проверок — ЧИСЛО в начале значения, а не всё
                    -- значение: набор пишет «8, и все восемь названы парой…»,
                    -- и приведение целиком роняло пересчёт.
                    coalesce(max(CASE WHEN f.name ILIKE 'Проверок%'
                                      THEN substring(f.value from '^[0-9]+') END), '0')::integer
               FROM project_documents d
               LEFT JOIN project_document_fields f
                 ON f.project_id = d.project_id AND f.entity_kind = d.entity_kind AND f.entity_name = d.entity_name
              WHERE d.project_id = $1 AND d.entity_kind = 'red-task'
              GROUP BY d.entity_name
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;
    let red_checks = tx
        .execute(
            "INSERT INTO red_task_check (project_id, red_task_id, check_id)
             SELECT $1, d.entity_name, m[1]
               FROM project_documents d
               JOIN project_document_fields f
                 ON f.project_id = d.project_id AND f.entity_kind = d.entity_kind AND f.entity_name = d.entity_name AND f.name ILIKE 'Какие%',
                    LATERAL regexp_matches(f.value, '(TC-[A-Z]+-[0-9]+[a-z]?)', 'g') m
              WHERE d.project_id = $1 AND d.entity_kind = 'red-task'
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Красные задачи — в ту же таблицу, что и остальные ────────────────────
    //
    // Не в свою: «следующая задача», «чего ждёт», «что закрыто» обязаны видеть
    // весь план разом. Отдельная таблица для красных означала бы, что каждый
    // читатель помнит про вторую, — а забывший её прочтёт план неполным и не
    // заметит.
    //
    // Вставка идёт ПОСЛЕ донорской пересборки, которая свои строки удаляет и
    // пишет заново; поэтому красные добавляются здесь, в том же вызове.
    let red_in_plan = tx
        .execute(
            "INSERT INTO project_plan_tasks
                (project_id, id, milestone_id, ord, title, entity_kind, entity_name, size, kind, state, closing_commit)
             SELECT r.project_id, r.id, r.milestone, 0,
                    r.id || ' · проверки для ' || r.parent_task,
                    'red-task', r.id, '', 'red', 'not_started', ''
               FROM red_task r
              WHERE r.project_id = $1
                AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t
                                 WHERE t.project_id = r.project_id AND t.id = r.id)
                -- Красная задача, не назвавшая существующей вехи, роняла ВЕСЬ
                -- пересчёт о внешний ключ: набор tot объявляет её вехой ноль
                -- раз из семидесяти семи. Пропуск здесь виден в числе строк, а
                -- падение не видно ничем, кроме отказа всей ручки.
                AND EXISTS (SELECT 1 FROM project_plan_milestones ms
                             WHERE ms.project_id = r.project_id AND ms.id = r.milestone)",
            &[&project],
        )
        .await?;

    // Зависимости красных задач — здесь же, и по той же причине, по которой
    // здесь их строки: донорская пересборка знает только вид `task`. Имя
    // приводится по таблице задач, а не сравнивается буквально: набор пишет
    // `V1-T2`, а задача зовётся `v1-t2`.
    tx.execute(
        "INSERT INTO project_plan_task_deps (project_id, task_id, depends_on)
         SELECT $1, x.red, t.id
           FROM (SELECT d.entity_name AS red, m[1] AS named
                   FROM project_documents d
                   JOIN project_document_fields f
                     ON f.project_id = d.project_id AND f.entity_kind = d.entity_kind
                    AND f.entity_name = d.entity_name AND f.name = 'Зависит от',
                        LATERAL regexp_matches(f.value, '([MmVv][0-9]+-[Tt][0-9a-z]+)', 'g') m
                  WHERE d.project_id = $1 AND d.entity_kind = 'red-task') x
           JOIN project_plan_tasks t
             ON t.project_id = $1 AND lower(t.id) = lower(x.named)
          WHERE t.id <> x.red
            AND EXISTS (SELECT 1 FROM project_plan_tasks s
                         WHERE s.project_id = $1 AND s.id = x.red)
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;

    // Предполёт кладётся на задачу: последний вердикт, его находки и то, на
    // какой правке документа он снят. `preflight_fresh` — ответ на вопрос,
    // который задаёт ступень 10: вердикт относится к НЫНЕШНЕМУ тексту задачи.
    tx.execute(
        "UPDATE project_plan_tasks t SET
           preflight = coalesce(v.verdict, ''), preflight_at = v.at,
           preflight_revision = v.task_revision, preflight_findings = v.findings,
           preflight_fresh = (v.task_revision IS NOT NULL AND v.task_revision = d.revision
                              AND v.verdict <> 'blocked')
           FROM project_documents d
           LEFT JOIN LATERAL (
                SELECT p.verdict, p.at, p.task_revision, p.findings
                  FROM preflight_verdict p
                 WHERE p.project_id = d.project_id AND p.task_id = (
                       SELECT t2.id FROM project_plan_tasks t2
                        WHERE t2.project_id = d.project_id AND t2.entity_kind = d.entity_kind
                          AND t2.entity_name = d.entity_name)
                 ORDER BY p.at DESC LIMIT 1) v ON true
          WHERE d.project_id = t.project_id AND d.entity_kind = t.entity_kind
            AND d.entity_name = t.entity_name AND t.project_id = $1",
        &[&project],
    )
    .await?;

    // Состояние из истории кладётся поверх объявленного документом. Документ
    // здесь не власть: он второй источник, и расхождение видно отдельным
    // запросом, а не примиряется молча.
    let stated = tx
        .execute(
            "UPDATE project_plan_tasks t SET state = s.state, closing_commit = s.closing_commit
               FROM task_state s
              WHERE s.project_id = t.project_id AND s.task_id = t.id AND t.project_id = $1",
            &[&project],
        )
        .await?;

    // ── Пункт готовности ─────────────────────────────────────────────────────
    // Переносятся ВСЕ пункты со способом `unknown`. Это ничего не проверяет — и
    // сразу даёт число, которого нет: сколько пунктов готовности не имеют
    // способа. Выводить выполненность из прозы запрещено: зелёный чек-лист,
    // который ничего не мерит, хуже отсутствующего, потому что ему верят.
    tx.execute("DELETE FROM readiness_item WHERE project_id = $1", &[&project]).await?;
    let readiness = tx
        .execute(
            "INSERT INTO readiness_item (project_id, owner_kind, owner_id, ord, text, declared, method_kind)
             SELECT $1, o.kind, o.id, x.n, btrim(regexp_replace(x.line, '^\\s*[-*]\\s+\\[[ xX]\\]\\s*', '')),
                    x.line ~ '\\[[xX]\\]', 'unknown'
               -- `50-plan/v1/order.md` исключён, и это ВЫБОР, а не пропуск.
               -- Доска волны порождается скриптом, который пишет `- [ ]`
               -- безусловно при каждой пересборке: её 145 «пунктов» не
               -- объявленная готовность, а отображение, и позеленеть они не
               -- могут по устройству. Вида у доски набор не объявляет; завести
               -- его значило бы узаконить полторы сотни пунктов, которые никто
               -- никогда не отметит.
               FROM (SELECT d.entity_kind, d.entity_name, l.line, l.n
                       FROM project_documents d,
                            LATERAL (SELECT line, row_number() OVER () AS n
                                       FROM regexp_split_to_table(d.content, E'\n') line) l
                      WHERE d.project_id = $1 AND l.line ~ '^\\s*[-*]\\s+\\[[ xX]\\]') x
               JOIN LATERAL (
                     SELECT 'task' AS kind, t.id FROM project_plan_tasks t
                      WHERE t.project_id = $1 AND t.entity_kind = x.entity_kind AND t.entity_name = x.entity_name
                      UNION ALL
                     SELECT 'red-task', r.id FROM red_task r
                      WHERE r.project_id = $1 AND x.entity_kind = 'red-task' AND x.entity_name = r.id
                      UNION ALL
                     SELECT 'question', q.id FROM project_questions q
                      WHERE q.project_id = $1 AND q.entity_kind = x.entity_kind AND q.entity_name = x.entity_name
                      UNION ALL
                     -- Одиночка адресуется своим видом, а имени у неё нет: путь
                     -- в `owner_id` вернул бы адрес в базу идентификатором.
                     SELECT 'acceptance', '' WHERE x.entity_kind = 'acceptance'
                      UNION ALL
                     SELECT 'test-plan', '' WHERE x.entity_kind = 'test-plan'
                      UNION ALL
                     SELECT 'document-plan', '' WHERE x.entity_kind = 'document-plan'
                      UNION ALL
                     -- Владелец пункта — ВИД документа и его имя, а не слово
                     -- «документ» с путём вместо имени. Слово одно на всех
                     -- свело бы `feature onboarding` и `after onboarding` в
                     -- одного владельца — тех самых два `onboarding.md`, ради
                     -- различения которых ключ и переезжал.
                     SELECT x.entity_kind, x.entity_name WHERE NOT EXISTS (
                        SELECT 1 FROM project_plan_tasks t WHERE t.project_id = $1 AND t.entity_kind = x.entity_kind AND t.entity_name = x.entity_name
                        UNION ALL SELECT 1 FROM project_questions q WHERE q.project_id = $1 AND q.entity_kind = x.entity_kind AND q.entity_name = x.entity_name
                        UNION ALL SELECT 1 FROM red_task r WHERE r.project_id = $1 AND x.entity_kind = 'red-task' AND x.entity_name = r.id)
                        AND x.entity_kind NOT IN ('acceptance', 'test-plan', 'document-plan')
                        AND NOT (x.entity_kind = 'board' AND x.entity_name = 'order')
                     LIMIT 1) o ON true
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Исключение из правила ────────────────────────────────────────────────
    // Правило: таблица `COVERAGE.md` с шапкой «История · Почему». Исключение,
    // объявленное прозой, неотличимо от дыры для всякой считающей ручки: она
    // либо соврёт про четыре дыры, либо спрячет настоящую пятую, когда та
    // появится.
    tx.execute("DELETE FROM rule_exception WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    let exceptions = tx
        .execute(
            "INSERT INTO rule_exception (project_id, rule, entity_kind, entity_id, reason, decided_by, origin)
             SELECT $1, 'история без требования', 'story',
                    max(CASE WHEN c.col = 0 THEN c.value END),
                    coalesce(max(CASE WHEN c.col = 1 THEN c.value END), ''),
                    'COVERAGE', 'projected'
               FROM project_document_cells c
              WHERE c.project_id = $1 AND c.entity_kind = 'coverage'
                AND c.row_ord > 0
                AND c.block_ord IN (SELECT block_ord FROM project_document_cells h
                                     WHERE h.project_id = $1 AND h.entity_kind = 'coverage'
                                       AND h.row_ord = 0 AND h.col = 0 AND h.value = 'История')
              GROUP BY c.row_ord
             HAVING max(CASE WHEN c.col = 0 THEN c.value END) ~ '^US-[A-Z]+-[0-9]+$'
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Над чем считано заявленное число ─────────────────────────────────────
    // Без этого сверка кричит волком: `172` считано по ФАЙЛАМ решений, а таблица
    // держит записи; `272` — по функциональным, а таблица держит FR и NFR
    // вместе. Два ложных крика утопят единственный настоящий.
    tx.execute("DELETE FROM claim_subject WHERE project_id = $1", &[&project]).await?;
    let subjects = tx
        .execute(
            "INSERT INTO claim_subject (project_id, name, subject, counts, note) VALUES
               ($1, 'constitution.md', 'articles',
                'SELECT count(*) FROM project_articles WHERE project_id = $1', ''),
               ($1, 'strs.md', 'needs',
                'SELECT count(*) FROM project_needs WHERE project_id = $1', ''),
               ($1, 'srs.md', 'requirements',
                'SELECT count(*) FROM project_requirements WHERE project_id = $1 AND id NOT LIKE $$NFR-%$$',
                'заявлено про функциональные; таблица держит FR и NFR вместе'),
               ($1, 'srs.md', 'nfr',
                'SELECT count(*) FROM project_requirements WHERE project_id = $1 AND id LIKE $$NFR-%$$', ''),
               ($1, 'ui-spec.md', 'screens',
                'SELECT count(*) FROM project_screens WHERE project_id = $1', ''),
               ($1, 'adr/', 'decisionFiles',
                'SELECT count(*) FROM project_documents WHERE project_id = $1
                   AND entity_kind IN ($$decision$$, $$decision-template$$)',
                'считано по файлам вместе с шаблоном; записей решений на одну меньше'),
               ($1, 'test-cases.md', 'checks',
                'SELECT count(*) FROM project_checks WHERE project_id = $1', '')
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // Статус «в работе» получает свой факт: открытое рабочее дерево. Источник —
    // была ли подача вообще: пустая таблица деревьев без подачи значит
    // «неизвестно», а не «никто не работает». Заполняется только пустое:
    // объявленное человеком не перетирается.
    tx.execute(
        r#"UPDATE kind_status SET
             fact = 'SELECT 1 FROM task_worktree WHERE project_id = $1 AND task_id = $2',
             source = 'SELECT 1 FROM fact_push WHERE project_id = $1 AND fact = ''worktree'''
           WHERE kind = 'task' AND name = 'в работе' AND fact = ''"#,
        &[],
    )
    .await?;
    // «Имплементирована» отложена, и это ОБЪЯВЛЕНО. Пустой статус без причины
    // читается как «забыли»; с причиной — как решение, которое кто-то принял.
    tx.execute(
        r#"UPDATE kind_status SET
             why = 'факт не подаётся: нужна запись о прогоне проверок на коммите задачи — какие, чем запущены, что вышло. Отложено сознательно (HARNESS-PLAN-7 §3)'
           WHERE kind = 'task' AND name = 'имплементирована' AND fact = '' AND why = ''"#,
        &[],
    )
    .await?;

    // ── Порождённый порядок против вычисленного ─────────────────────────────
    // Файл — производное сервера; отстав, он врёт исполнителю. Сверка идёт
    // здесь, а гейт читает её след: правило гейта — запрос.
    // Объявленный способ ступени возвращается на ступень — как у пунктов.
    let step_methods = tx
        .execute(
            "UPDATE harness_process_step s SET method_kind = m.method_kind, method = m.method
               FROM harness_process_method m
              WHERE m.set_name = s.set_name AND m.process = s.process AND m.ord = s.ord",
            &[],
        )
        .await?;

    // Объявленные способы возвращаются на пересобранные пункты.
    let methods = tx
        .execute(
            "UPDATE readiness_item i SET method_kind = m.method_kind, method = m.method
               FROM readiness_method m
              WHERE m.project_id = i.project_id AND m.owner_kind = i.owner_kind
                AND m.owner_id = i.owner_id AND m.ord = i.ord AND i.project_id = $1",
            &[&project],
        )
        .await?;

    // ── Снятые термины ───────────────────────────────────────────────────────
    // Источник — список, который сегодня носит правило `docs-lint`. Он назван
    // здесь один раз, и правило может читать его отсюда вместо своей копии.
    tx.execute("DELETE FROM term_retired WHERE project_id = $1", &[&project]).await?;
    let retired = tx
        .execute(
            "INSERT INTO term_retired (project_id, term, retired_by, declared_in)
             SELECT $1, t.term, '', 'docs-lint RETIRED + Article 12'
               FROM (VALUES ('Incident'), ('AlertState'), ('watchdog'), ('data_source'),
                            ('SituationState.merged'), ('Alert')) AS t(term)
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Есть ли у экрана раздел «Состояния» ──────────────────────────────────
    // Проверка ПО ОТСУТСТВИЮ ЗАГОЛОВКА: перечень из карты она сама объявляет
    // неполным, и считать по нему значит считать по устаревшей копии.
    tx.execute("DELETE FROM screen_states WHERE project_id = $1", &[&project]).await?;
    let screens = tx
        .execute(
            "INSERT INTO screen_states (project_id, screen_id, has_states)
             SELECT s.project_id, s.id,
                    EXISTS (SELECT 1 FROM project_document_sections d
                             WHERE d.project_id = s.project_id AND d.entity_kind = s.entity_kind AND d.entity_name = s.entity_name
                               AND d.title ILIKE '%состояни%')
               FROM project_screens s WHERE s.project_id = $1
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Статус решения: константа плюс слова ─────────────────────────────────
    // Свободный текст из поля уходит в `state_text`; константа выводится по
    // первому слову, на обоих языках, которыми набор пишет статус.
    tx.execute("DELETE FROM decision_status WHERE project_id = $1", &[&project]).await?;
    let decisions = tx
        .execute(
            "INSERT INTO decision_status (project_id, decision_id, status, state_text)
             SELECT d.project_id, d.id,
                    CASE
                      WHEN d.status_text ~* '^\\\\s*(предлож|proposed)' THEN 'proposed'
                      WHEN d.status_text ~* '^\\\\s*(отмен|declined|rejected)' THEN 'declined'
                      WHEN d.status_text ~* '^\\\\s*(заменен|заменён|superseded)' THEN 'superseded'
                      WHEN d.status_text ~* '^\\\\s*(принят|accepted)' THEN 'accepted'
                      ELSE d.status
                    END,
                    d.status_text
               FROM project_decisions d WHERE d.project_id = $1
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Разрыв пути ──────────────────────────────────────────────────────────
    // Правило: таблица `cjm.md` с шапкой, где четвёртая колонка — «Чем
    // закрываем». Сам разрыв объявлен и потому хранится; связь с требованием —
    // нет, и её таблица остаётся пустой.
    tx.execute("DELETE FROM cjm_gap WHERE project_id = $1", &[&project]).await?;
    let gaps = tx
        .execute(
            "INSERT INTO cjm_gap (project_id, ord, gap, phase, closes_with)
             SELECT $1, c.row_ord,
                    max(CASE WHEN c.col = 1 THEN c.value END),
                    coalesce(max(CASE WHEN c.col = 2 THEN c.value END), ''),
                    coalesce(max(CASE WHEN c.col = 3 THEN c.value END), '')
               FROM project_document_cells c
              WHERE c.project_id = $1 AND c.entity_kind = 'cjm' AND c.row_ord > 0
                AND c.block_ord IN (SELECT block_ord FROM project_document_cells h
                                     WHERE h.project_id = $1 AND h.entity_kind = 'cjm'
                                       AND h.row_ord = 0 AND h.value ILIKE '%закрываем%')
              GROUP BY c.row_ord
             HAVING max(CASE WHEN c.col = 1 THEN c.value END) <> ''
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    // ── Зависимость задачи от этапа ──────────────────────────────────────────
    // Правило: в поле «Зависит от» стоит имя этапа `M<n>`, НЕ являющееся началом
    // имени задачи (`M8-T10`), и строка не отрицает зависимость.
    //
    // Отрицание проверяется отдельно и намеренно: «ничего в M1» содержит имя
    // этапа и без этой проверки завело бы ребро, которого документ не объявлял, —
    // задача ждала бы этап, о котором сказано ровно обратное.
    tx
        .execute("DELETE FROM task_milestone_dep WHERE project_id = $1", &[&project])
        .await?;
    let milestone_deps = tx
        .execute(
            "INSERT INTO task_milestone_dep (project_id, task_id, milestone_id, said)
             SELECT t.project_id, t.id, m[1], left(f.value, 200)
               FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name AND f.name LIKE 'Зависит%',
                    LATERAL regexp_matches(f.value, '(?<!-)\\m(M\\d+)\\M(?!-T)', 'g') m
              WHERE t.project_id = $1
                AND f.value !~ '[Нн]ичего в'
                AND EXISTS (SELECT 1 FROM project_plan_milestones ms
                             WHERE ms.project_id = t.project_id AND ms.id = m[1])
             ON CONFLICT DO NOTHING",
            &[&project],
        )
        .await?;

    tx.commit().await?;
    // Связи считаются ПОСЛЕДНИМИ и здесь, а не в пересборке: красные задачи
    // кладёт эта сборка, и до неё их в плане нет. Связь, посчитанная раньше,
    // легла бы на пустоту и показала бы ноль как «сошлось».
    let relations = crate::reproject::relations::project(pool, project).await?;

    Ok(json!({
        "relations": relations,
        "entity_event": events,
        "norm_version": versions,
        "measurement": measurements,
        "rule_exception": exceptions,
        "claim_subject": subjects,
        "red_task": red,
        "red_in_plan": red_in_plan,
        "state_from_history": stated,
        "red_task_check": red_checks,
        "readiness_item": readiness,
        "readiness_method": methods,
        "harness_process_method": step_methods,
        "term_retired": retired,
        "screen_states": screens,
        "decision_status": decisions,
        "cjm_gap": gaps,
        // Связь разрыва с требованием не считается ниоткуда: в наборе её нет.
        // Число сообщается, чтобы пустота была видна, а не подразумевалась.
        "cjm_gap_requirement": 0,
        "task_milestone_dep": milestone_deps,
    }))
}

/// Следующая незакрытая задача, у которой закрыты зависимости.
///
/// Зависимость от этапа проверяется **связью с этапом**, а не раскрытым списком
/// задач: раскрытое при записи протухнет, как только в этап добавят задачу, и
/// задача прочтётся готовой, хотя ждёт новичка. Именно на этом `M8-T10` сегодня
/// читается свободной: рёбер у неё ноль, а документ говорит «все задачи M8».
pub async fn next_task(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "WITH open AS (
               SELECT t.id, t.milestone_id, t.title, t.ord, t.kind, t.size, t.entity_kind, t.entity_name
                 FROM project_plan_tasks t
                WHERE t.project_id = $1 AND t.state <> 'closed'),
             blocked_by_task AS (
               SELECT d.task_id FROM project_plan_task_deps d
                 JOIN project_plan_tasks p
                   ON p.project_id = d.project_id AND p.id = d.depends_on
                WHERE d.project_id = $1 AND p.state <> 'closed'),
             blocked_by_milestone AS (
               SELECT md.task_id FROM task_milestone_dep md
                WHERE md.project_id = $1
                  AND EXISTS (SELECT 1 FROM project_plan_tasks t2
                               WHERE t2.project_id = md.project_id
                                 AND t2.milestone_id = md.milestone_id
                                 AND t2.state <> 'closed'))
             SELECT o.id, o.milestone_id, o.title, o.kind, o.size,
                    (SELECT count(*) FROM task_requirement r
                      WHERE r.project_id = $1 AND r.task_id = o.id) AS requirements,
                    coalesce((SELECT string_agg(r.requirement_id || ' · ' || coalesce(rq.text, '—'), E'\\n')
                                FROM task_requirement r
                                LEFT JOIN project_requirements rq
                                  ON rq.project_id = r.project_id AND rq.id = r.requirement_id
                               WHERE r.project_id = $1 AND r.task_id = o.id), '') AS requirement_text,
                    coalesce((SELECT string_agg(c.id, ' · ' ORDER BY c.id)
                                FROM project_checks c
                                JOIN task_requirement r
                                  ON r.project_id = c.project_id AND r.requirement_id = c.requirement_id
                               WHERE c.project_id = $1 AND r.task_id = o.id), '') AS checks
               FROM open o
              WHERE o.id NOT IN (SELECT task_id FROM blocked_by_task)
                AND o.id NOT IN (SELECT task_id FROM blocked_by_milestone)
              ORDER BY o.milestone_id, o.ord
              LIMIT 1",
            &[&project],
        )
        .await?;
    // Барьер красной фазы. Правило проекта: весь трек проверок предшествует
    // всему коду. Состояние красной задачи в базе не лежит — оно выводится из
    // закрывающего трейлера git, а трейлеров база не знает. Значит закрытость
    // красной фазы НЕ ПОДТВЕРЖДЕНА, и задачу кода выдавать нельзя: выдать её —
    // значит поручиться за то, чего не проверял.
    // Барьер красной фазы: весь трек проверок предшествует всему коду. Теперь
    // он проверяем — состояние приходит из истории, и открытые красные задачи
    // называются поимённо. «Состояние неизвестно» больше не ответ: это был не
    // барьер, а отсутствие источника.
    // Открытые красные читаются ИЗ ПЛАНА, а не из поданных состояний.
    //
    // Читая `task_state` напрямую, барьер завёл бы второй источник того же
    // факта: подача говорила бы «закрыто», план — «не начата», и барьер снялся
    // бы при непересобранном плане. Это ровно тот дефект, ради которого
    // состояние и переносится в одно место. Пойман на себе.
    let open_red = client
        .query(
            "SELECT t.id FROM project_plan_tasks t
              WHERE t.project_id = $1 AND t.kind = 'red' AND t.state <> 'closed'
              ORDER BY t.id",
            &[&project],
        )
        .await?;
    let pushed: i64 = client
        .query_one("SELECT count(*) FROM task_state WHERE project_id = $1", &[&project])
        .await?
        .get(0);
    if !open_red.is_empty() {
        let names: Vec<String> = open_red.iter().take(20).map(|r| r.get::<_, String>(0)).collect();
        return Ok(json!({
            "task": null,
            "why": if pushed == 0 {
                "барьер красной фазы не проверить: состояний из истории никто не подавал"
            } else {
                "барьер красной фазы держит: есть незакрытые красные задачи"
            },
            "redPhase": {
                "open": open_red.len(),
                "openNamed": names,
                "stateSource": if pushed == 0 { "нет подачи" } else { "история, поданная харнесом" },
            },
            "candidate": rows.first().map(|r| r.get::<_, String>(0)),
        }));
    }

    let Some(r) = rows.first() else {
        return Ok(json!({ "task": null, "why": "незакрытых задач с закрытыми зависимостями нет" }));
    };
    Ok(json!({
        "task": {
            "id": r.get::<_, String>(0),
            "milestone": r.get::<_, String>(1),
            "title": r.get::<_, String>(2),
            "kind": r.get::<_, Option<String>>(3),
            "size": r.get::<_, Option<String>>(4),
            "requirements": r.get::<_, i64>(5),
            "requirementText": r.get::<_, String>(6),
            "checks": r.get::<_, String>(7),
        }
    }))
}

/// Почему задача не берётся: перечень того, чего она ждёт.
pub async fn task_blockers(pool: &Pool, project: &str, task: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let tasks = client
        .query(
            "SELECT d.depends_on FROM project_plan_task_deps d
               JOIN project_plan_tasks p ON p.project_id = d.project_id AND p.id = d.depends_on
              WHERE d.project_id = $1 AND d.task_id = $2 AND p.state <> 'closed' ORDER BY 1",
            &[&project, &task],
        )
        .await?;
    let milestones = client
        .query(
            "SELECT md.milestone_id, md.said,
                    (SELECT count(*) FROM project_plan_tasks t
                      WHERE t.project_id = md.project_id AND t.milestone_id = md.milestone_id
                        AND t.state <> 'closed') AS open_tasks
               FROM task_milestone_dep md
              WHERE md.project_id = $1 AND md.task_id = $2",
            &[&project, &task],
        )
        .await?;
    Ok(json!({
        "task": task,
        "waitsForTasks": tasks.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
        "waitsForMilestones": milestones.iter().map(|r| json!({
            "milestone": r.get::<_, String>(0),
            "said": r.get::<_, String>(1),
            "openTasks": r.get::<_, i64>(2),
        })).collect::<Vec<_>>(),
    }))
}

/// Измерить пункты гейтов и СОХРАНИТЬ измеренное.
///
/// Это единственное место, где запросы гейта исполняются. Доска их не исполняет:
/// она читает то, что здесь записано, вместе со временем замера. Пересчёт зовут
/// при изменении набора — правкой документа, пересборкой проекций, поданным
/// фактом, — а не при каждом открытии страницы.
///
/// Прежде было наоборот: страница считала всё заново при каждом заходе, а в
/// колонке `state` лежало заявление, написанное когда-то руками. Рядом они
/// читались как два мнения, и «расхождением» звалось то, что было просто
/// непересчитанной записью.
/// Объявить пункт гейта неприменимым к проекту — с причиной.
pub async fn waive_gate_item(
    pool: &Pool, project: &str, phase: &str, item: &str, why: &str, actor: &str, drop: bool,
    holds_while_empty: &str,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let n = client.execute(
            "DELETE FROM gate_item_waiver WHERE project_id = $1 AND phase = $2 AND id = $3",
            &[&project, &phase, &item]).await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" } }));
    }
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_reason",
                          "why": "неприменимость без причины не объявляется: это решение, а не умолчание" }));
    }
    let known: i64 = client
        .query_one("SELECT count(*) FROM gate_item WHERE phase = $1 AND id = $2", &[&phase, &item])
        .await?.get(0);
    if known == 0 {
        return Ok(json!({ "status": "not_found", "why": format!("пункта «{item}» у гейта {phase} нет") }));
    }
    client.execute(
        "INSERT INTO gate_item_waiver (project_id, phase, id, item, why, declared_at, declared_by,
                                       holds_while_empty)
         VALUES ($1,$2,$3,$3,$4,$5,$6,$7)
         ON CONFLICT (project_id, phase, item) DO UPDATE SET why = EXCLUDED.why, id = EXCLUDED.id,
           declared_at = EXCLUDED.declared_at, declared_by = EXCLUDED.declared_by,
           holds_while_empty = EXCLUDED.holds_while_empty",
        &[&project, &phase, &item, &why, &now_ms(), &actor, &holds_while_empty]).await?;
    Ok(json!({ "status": "waived", "phase": phase, "item": item, "why": why,
               "holdsWhileEmpty": holds_while_empty }))
}

pub async fn measure_gates(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Объявление — общее, замер — проектный. Пункты берутся из `gate_item`, и
    // проект, у которого их ещё не было, получает все сразу: гейт отвечает на
    // вопрос «можно ли идти дальше», и ответ не должен зависеть от того, кто как
    // завёл проверки у себя.
    let rows = client
        .query("SELECT phase, item, kind, query, why, owner, id FROM gate_item ORDER BY phase, id", &[])
        .await?;
    let waived: std::collections::HashSet<(String, String)> = client
        .query("SELECT phase, id FROM gate_item_waiver WHERE project_id = $1", &[&project])
        .await?
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .collect();
    let why_waived: std::collections::HashMap<(String, String), String> = client
        .query("SELECT phase, id, why FROM gate_item_waiver WHERE project_id = $1", &[&project])
        .await?
        .iter()
        .map(|r| ((r.get::<_, String>(0), r.get::<_, String>(1)), r.get::<_, String>(2)))
        .collect();
    let now = now_ms();
    let mut measured = 0usize;
    let mut failed = 0usize;
    for r in &rows {
        let phase: String = r.get(0);
        let item: String = r.get(1);
        let kind: String = r.get(2);
        let query: Option<String> = r.get(3);
        // Отмена и отметка адресуются ИМЕНЕМ, а не заголовком: заголовок
        // переписывают, и прежде всякая правка формулировки роняла отмену.
        let id: String = r.get(6);
        // Неприменимый пункт НЕ ИСПОЛНЯЕТСЯ. Исполнить и прощать значило бы
        // считать нарушением то, чего в этом проекте не существует: `.sqlx` у
        // проекта без sqlx не «не снята» — её тут не бывает.
        let entry = if waived.contains(&(phase.clone(), id.clone())) {
            json!({ "item": item, "kind": kind, "computed": "waived",
                    "why": why_waived.get(&(phase.clone(), id.clone())).cloned().unwrap_or_default(),
                    "means": r.get::<_, String>(4) })
        } else {
            measure_item(&client, project, &phase, &item, &kind, query.as_deref(), r).await?
        };
        let computed = entry["computed"].as_str().unwrap_or("unknown");
        // В колонке `state` живут только четыре слова — так объявлено ограничением
        // таблицы. Полное слово («не подписан», «подпись устарела») лежит в
        // `result`: сузить его до `unknown` в колонке можно, потерять — нельзя.
        let flat = match computed {
            "passed" => "passed",
            "failed" => "failed",
            _ => "unknown",
        };
        if flat == "failed" {
            failed += 1;
        }
        let violations = entry["violations"].as_i64().unwrap_or(0) as i32;
        let detail = entry["detail"]
            .as_array()
            .map(|d| d.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        // Строка замера заводится, если её ещё нет: у нового проекта её и не
        // было. Объявление копируется рядом — им пользуются запросы ступеней,
        // читающие состояние пункта.
        let query_col: Option<String> = r.get(3);
        let why_col: String = r.get(4);
        let owner_col: Option<String> = r.get(5);
        client
            .execute(
                // Подписант переносится вместе с родом: у подписного пункта
                // ограничение таблицы требует имени, и замер без него падал —
                // «new row violates check constraint», четыре пункта из сорока
                // не мерились вовсе.
                "INSERT INTO project_gates (project_id, phase, item, kind, query, why, owner,
                                            state, violations, detail, result, checked_at, id)
                 VALUES ($1,$2,$3,$9,$10,$11,$12,$4,$5,$6,$7,$8,$13)
                 ON CONFLICT (project_id, phase, id) DO UPDATE SET
                   item = EXCLUDED.item, kind = EXCLUDED.kind, query = EXCLUDED.query, why = EXCLUDED.why,
                   owner = EXCLUDED.owner, id = EXCLUDED.id,
                   state = EXCLUDED.state, violations = EXCLUDED.violations,
                   detail = EXCLUDED.detail, result = EXCLUDED.result,
                   checked_at = EXCLUDED.checked_at",
                &[&project, &phase, &item, &flat, &violations, &detail, &entry, &now,
                  &kind, &query_col, &why_col, &owner_col, &id],
            )
            .await?;
        measured += 1;
    }
    Ok(json!({ "measured": measured, "failed": failed, "at": now }))
}

/// Один пункт: чем меряют — тем и меряется.
async fn measure_item(
    client: &deadpool_postgres::Client,
    project: &str,
    phase: &str,
    item: &str,
    kind: &str,
    query: Option<&str>,
    r: &tokio_postgres::Row,
) -> Result<Value, tokio_postgres::Error> {
    let item = item.to_owned();
    let kind = kind.to_owned();
    let phase = phase.to_owned();
        let why_col: String = r.try_get("why").unwrap_or_default();
        let entry = if kind == "unknown" || kind == "manual" {
            json!({ "item": item, "kind": kind, "computed": "unknown",
                    "why": if why_col.is_empty() {
                        if kind == "manual" { "проиграно ли — не записано" } else { "требуется, но машинного способа нет" }
                    } else { why_col.as_str() } })
        } else if kind == "query" {
            match query.as_deref() {
                // Запрос гейта исполняется ОБЩИМ исполнителем — тем же, что у
                // пункта готовности и ступени лестницы. Свой здесь мерил бы не
                // то, что обещано пунктом, и разошёлся бы молча.
                sql => {
                    let v = execute_method_upto(client, project, "query", sql.unwrap_or(""), 200).await;
                    // Объявленные исключения ВЫЧИТАЮТСЯ — и остаются видны.
                    //
                    // Молча вычесть нельзя: исключение, которого не видно, — это
                    // дыра с разрешением. Поэтому чисел три: сколько нарушений
                    // осталось, сколько прощено объявленным исключением и
                    // сколько исключений объявлено на то, чего уже нет, —
                    // последнее значит, что исключение пора снять.
                    let allowed = client
                        .query(
                            "SELECT entity_id, reason FROM rule_exception
                              WHERE project_id = $1 AND rule = $2",
                            &[&project, &item],
                        )
                        .await
                        .unwrap_or_default();
                    let names: std::collections::HashMap<String, String> = allowed
                        .iter()
                        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
                        .collect();
                    let (excepted, rest): (Vec<&String>, Vec<&String>) =
                        v.detail.iter().partition(|d| names.contains_key(&violator(d)));
                    // Нарушения показываются пятью строками, а исключения — все:
                    // их немного, и каждое кто-то однажды объявил.
                    let hit: std::collections::HashSet<String> =
                        v.detail.iter().map(|d| violator(d)).collect();
                    let stale: Vec<&String> = names.keys().filter(|n| !hit.contains(*n)).collect();
                    let left = v.violations.saturating_sub(excepted.len());
                    let state = if v.state == "failed" && left == 0 { "passed" } else { v.state };
                    json!({
                        "item": item, "kind": kind,
                        "computed": state, "violations": left, "detail": rest,
                        "excepted": excepted.len(),
                        "exceptedNames": excepted,
                        "staleExceptions": stale.len(),
                        "staleExceptionNames": stale,
                        // `why` исполнителя — про то, ПОЧЕМУ запрос не выполнился;
                        // `means` — про то, что пункт вообще меряет. Второе
                        // объявляется вместе с пунктом и до сих пор наружу не
                        // выходило: доска показывала имя пункта и знак, а чем он
                        // меряет и зачем — знал только тот, кто заводил.
                        "why": v.why,
                        "means": why_col,
                    })
                }
            }
        } else {
            // Подпись у гейта ОДНА, и лежит она в одном месте.
            //
            // Прежде их было два: ручка `gate-sign` писала `gate_signature` и
            // `gate_signature_doc` (кто, когда, под какой формулировкой и под
            // какими документами), а пункт читал `project_gate_signatures` —
            // таблицу, которая держала хеш и не держала ни подписавшего, ни
            // даты. Подписанный гейт оставался с пунктом `unsigned`: подпись
            // была, а пункт её не видел.
            let signature = client
                .query(
                    "SELECT g.signed_by, g.signed_at::text, g.lifted_by,
                            count(*) FILTER (WHERE d.content_hash IS DISTINCT FROM p.content_hash),
                            count(*),
                            coalesce(string_agg(d.entity_kind || ' ' || d.entity_id, ', ')
                                     FILTER (WHERE d.content_hash IS DISTINCT FROM p.content_hash), '')
                       FROM gate_signature g
                       LEFT JOIN gate_signature_doc d
                         ON d.project_id = g.project_id AND d.phase = g.phase
                       LEFT JOIN project_documents p
                         ON p.project_id = d.project_id AND p.entity_kind = d.entity_kind
                        AND p.entity_name = d.entity_id
                      WHERE g.project_id = $1 AND g.phase = $2
                      GROUP BY g.signed_by, g.signed_at, g.lifted_by",
                    &[&project, &phase],
                )
                .await?;
            match signature.first() {
                Some(sig) => {
                    let lifted: String = sig.get(2);
                    let changed: i64 = sig.get(3);
                    let under: i64 = sig.get(4);
                    let which: String = sig.get(5);
                    let state = if !lifted.is_empty() {
                        "lifted"
                    } else if changed > 0 {
                        "stale"
                    } else {
                        "passed"
                    };
                    json!({
                        "item": item, "kind": kind, "computed": state,
                        "signedBy": sig.get::<_, String>(0),
                        "signedAt": sig.get::<_, Option<String>>(1),
                        "under": under,
                        // Вот ради чего хеш и хранится: подписано было одно, лежит другое.
                        "changedSinceSigned": changed,
                        "changedDocuments": which,
                        "liftedBy": lifted,
                    })
                }
                None => json!({
                    "item": item, "kind": kind, "computed": "unsigned",
                    "why": "подписи нет: пункт не закрыт, а не пройден молча",
                }),
            }
        };
        Ok(entry)
}

pub async fn gate(pool: &Pool, project: &str, phase: Option<&str>) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // ЧИТАЕТСЯ сохранённое, а не считается заново. Замер делает `measure_gates`,
    // и делает его при изменении набора; страница показывает результат и время,
    // когда он получен. Непосчитанный пункт называется непосчитанным.
    let rows = client
        .query(
           // Правило отдаётся ЦЕЛИКОМ: имя, запрос, проба, владелец. Прежде ответ
           // нёс только замер, и починить пункт, не имея прямого доступа к базе,
           // было нельзя: имена восстанавливались сопоставлением заголовков
           // вручную, а запросов не видел никто. Гейт, который нельзя прочесть,
           // нельзя и проверить.
            "SELECT phase, item, kind, result, checked_at, why, id, query, probe, owner, probe_ok
               FROM project_gates
              WHERE project_id = $1 AND ($2 = '' OR phase = $2)
              ORDER BY phase, id, item",
               &[&project, &phase.unwrap_or("")],
        )
        .await?;

    // Сам гейт: формулировка из плана и нужна ли ему подпись.
    let heads = client
        .query(
            "SELECT g.phase, g.title, g.needs_signature,
                    s.signed_at::text, s.signed_by, s.wording, s.lifted_by
               FROM gate_head g
               LEFT JOIN gate_signature s ON s.project_id = $1 AND s.phase = g.phase",
            &[&project],
        )
        .await?;
    let head_of: std::collections::HashMap<String, (String, bool, Option<String>, String, String, String)> =
        heads
            .iter()
            .map(|r| {
                (
                    r.get::<_, String>(0),
                    (r.get(1), r.get(2), r.get(3), r.get::<_, Option<String>>(4).unwrap_or_default(),
                     r.get::<_, Option<String>>(5).unwrap_or_default(),
                     r.get::<_, Option<String>>(6).unwrap_or_default()),
                )
            })
            .collect();

    let mut gates: std::collections::BTreeMap<String, Vec<Value>> = std::collections::BTreeMap::new();
    let mut checked_at: std::collections::BTreeMap<String, Option<i64>> = std::collections::BTreeMap::new();
    for r in &rows {
        let phase: String = r.get(0);
        let item: String = r.get(1);
        let kind: String = r.get(2);
        let stored: Option<Value> = r.get(3);
        let at: Option<i64> = r.get(4);
        let why_col: String = r.get(5);
        // Пустой замер — не «пройден» и не «провален». Он значит, что пункт ещё
        // ни разу не мерили, и сказать это надо словом.
        let mut entry = stored.unwrap_or_else(|| {
            json!({ "item": item, "kind": kind, "computed": "unknown",
                    "why": "ещё не мерили: пересчёт с заведения пункта не запускался",
                    "means": why_col })
        });

        // Само правило — рядом с замером. Отдельной ручкой это было бы вторым

        // местом, где надо помнить имя пункта; здесь оно там же, где число.

        if let Some(m) = entry.as_object_mut() {

            m.insert("id".into(), json!(r.get::<_, Option<String>>(6).unwrap_or_default()));

            m.insert("query".into(), json!(r.get::<_, Option<String>>(7).unwrap_or_default()));

            m.insert("probe".into(), json!(r.get::<_, Option<String>>(8).unwrap_or_default()));

            m.insert("owner".into(), json!(r.get::<_, Option<String>>(9).unwrap_or_default()));

            m.insert("phase".into(), json!(phase));
            // «Пробу не проверяли» и «проба не роняет» — разное, и оба не
            // «зелёное»: пункт, который ни разу не уронили, никто не проверял.
            m.insert("probeRuns".into(), json!(r.get::<_, Option<bool>>(10)));

        }
        let slot = checked_at.entry(phase.clone()).or_insert(at);
        // У гейта одно время замера — самое старое из его пунктов. Показывать
        // самое свежее значило бы прикрыть непосчитанный пункт посчитанным.
        if at.is_none() || slot.map(|s| at.map(|a| a < s).unwrap_or(true)).unwrap_or(false) {
            *slot = at;
        }
        gates.entry(phase).or_default().push(entry);
    }

    if gates.is_empty() {
        // Гейта нет ни одной строкой — это не «пройден». «Пройден» означает, что
        // мерили и сошлось; здесь мерить нечем, и ответ должен это говорить.
        return Ok(json!({
            "gates": [],
            "why": match phase {
                Some(p) => format!("у гейта {p} нет ни одного пункта: отвечать нечем, и это не «пройден»"),
                None => "у проекта нет ни одного пункта гейта".to_owned(),
            }
        }));
    }

    let mut out = Vec::new();
    for (phase, items) in gates {
        let failed = items.iter().filter(|i| i["computed"] == "failed").count();
        let unknown = items
            .iter()
            .filter(|i| i["computed"] == "unknown" || i["computed"] == "unsigned" || i["computed"] == "stale")
            .count();
        // Состояние гейта ВЫВОДИТСЯ: все проверки прошли **и** подпись стоит и не
        // снята. Пройти проверки, не подписав, — это не «пройден»: подпись и есть
        // то, чем человек берёт на себя сказанное.
        let checked = checked_at.get(&phase).copied().flatten();
        let head = head_of.get(&phase);
        let needs = head.map(|h| h.1).unwrap_or(true);
        let signed_at = head.and_then(|h| h.2.clone());
        let lifted = head.map(|h| h.5.clone()).unwrap_or_default();
        let signature_ok = !needs || (signed_at.is_some() && lifted.is_empty());
        let computed = if failed > 0 {
            "failed"
        } else if unknown > 0 {
            "open"
        } else if !signature_ok {
            "unsigned"
        } else {
            "passed"
        };
        out.push(json!({
            "gate": phase,
            "title": head.map(|h| h.0.clone()).unwrap_or_default(),
            "needsSignature": needs,
            "signature": head.map(|h| json!({ "at": h.2, "by": h.3, "wording": h.4, "liftedBy": h.5 })),
            "computed": computed,
            "items": items,
            "failedItems": failed,
            "openItems": unknown,
            "checkedAt": checked,
        }));
    }
    Ok(json!({ "gates": out }))
}

/// Сверка заявленного числа с фактом — но сперва сказав, что именно считается.
///
/// Без оговорки сверка кричит волком дважды и топит единственный настоящий крик.
pub async fn claims(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT c.name, c.subject, c.claimed, s.counts, s.note
               FROM project_document_plan_counts c
               LEFT JOIN claim_subject s
                 ON s.project_id = c.project_id AND s.name = c.name AND s.subject = c.subject
              WHERE c.project_id = $1 ORDER BY c.name, c.subject",
            &[&project],
        )
        .await?;
    let mut out = Vec::new();
    let mut disagree = 0;
    for r in &rows {
        let claimed: i32 = r.get(2);
        let counts: Option<String> = r.get(3);
        let fact = match counts.as_deref() {
            Some(sql) => match client.query_one(&sql.replace("$1", "$1"), &[&project]).await {
                Ok(row) => Some(row.get::<_, i64>(0)),
                Err(_) => None,
            },
            None => None,
        };
        let agrees = fact.map(|f| f == claimed as i64);
        if agrees == Some(false) {
            disagree += 1;
        }
        out.push(json!({
            "name": r.get::<_, String>(0),
            "subject": r.get::<_, String>(1),
            "claimed": claimed,
            "fact": fact,
            "agrees": agrees,
            // Пусто — не «сошлось»: над чем считано, никто не объявил, и
            // сравнивать нечего. Это дефект объявления, а не согласие.
            "countsOver": counts,
            "note": r.get::<_, Option<String>>(4),
        }));
    }
    Ok(json!({ "claims": out, "disagree": disagree }))
}

/// Чек-лист сущности с честным `unknown` там, где способа нет.
pub async fn readiness(pool: &Pool, project: &str, kind: &str, id: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT ord, text, declared, method_kind, state FROM readiness_state
              WHERE project_id = $1 AND owner_kind = $2 AND owner_id = $3 ORDER BY ord",
            &[&project, &kind, &id],
        )
        .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "ord": r.get::<_, i32>(0), "text": r.get::<_, String>(1),
                "declared": r.get::<_, Option<bool>>(2),
                "methodKind": r.get::<_, String>(3), "state": r.get::<_, String>(4),
            })
        })
        .collect();
    let unmeasurable = items.iter().filter(|i| i["state"] == "unknown").count();
    let declared_done = items.iter().filter(|i| i["declared"] == json!(true)).count();
    Ok(json!({
        "owner": { "kind": kind, "id": id },
        "items": items,
        "total": items.len(),
        "declaredDone": declared_done,
        "unmeasurable": unmeasurable,
        // Самое опасное сочетание: галочка стоит, а мерить нечем.
        "declaredDoneButUnmeasurable": items.iter()
            .filter(|i| i["declared"] == json!(true) && i["state"] == "unknown").count(),
    }))
}

/// Требования задачи — с объявленным отсутствием как ответом, а не пустотой.
pub async fn requirements_of(pool: &Pool, project: &str, task: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let declared = client
        .query(
            "SELECT has_own, note FROM task_requirements_declared WHERE project_id = $1 AND task_id = $2",
            &[&project, &task],
        )
        .await?;
    let Some(d) = declared.first() else {
        return Ok(json!({ "task": task, "known": false, "why": "такой задачи в наборе нет" }));
    };
    let has_own: Option<bool> = d.get(0);
    let rows = client
        .query(
            "SELECT r.requirement_id, coalesce(q.text, ''), q.priority, q.satisfied, v.checks, v.covered
               FROM task_requirement r
               LEFT JOIN project_requirements q ON q.project_id = r.project_id AND q.id = r.requirement_id
               LEFT JOIN requirement_covered v ON v.project_id = r.project_id AND v.requirement_id = r.requirement_id
              WHERE r.project_id = $1 AND r.task_id = $2 ORDER BY r.requirement_id",
            &[&project, &task],
        )
        .await?;
    Ok(json!({
        "task": task,
        "hasOwnRequirements": has_own,
        // Фраза целиком: «нет собственных» — это ответ, и он должен доехать
        // словами, а не превратиться в пустой список.
        "says": d.get::<_, String>(1),
        "requirements": rows.iter().map(|r| json!({
            "id": r.get::<_, String>(0), "text": r.get::<_, String>(1),
            "priority": r.get::<_, Option<String>>(2), "satisfied": r.get::<_, Option<bool>>(3),
            "checks": r.get::<_, Option<i64>>(4), "covered": r.get::<_, Option<bool>>(5),
        })).collect::<Vec<_>>(),
    }))
}

/// Задачи истории — через требования, с исключением как ответом.
pub async fn tasks_of_story(pool: &Pool, project: &str, story: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let known = client
        .query("SELECT id FROM project_stories WHERE project_id = $1 AND id = $2", &[&project, &story])
        .await?;
    if known.is_empty() {
        return Ok(json!({ "story": story, "known": false, "why": "такой истории в наборе нет" }));
    }
    let rows = client
        .query(
            "SELECT DISTINCT r.task_id FROM project_story_requirements sr
               JOIN task_requirement r ON r.project_id = sr.project_id AND r.requirement_id = sr.requirement_id
              WHERE sr.project_id = $1 AND sr.story_id = $2 ORDER BY 1",
            &[&project, &story],
        )
        .await?;
    let exception = client
        .query(
            "SELECT reason, decided_by FROM rule_exception
              WHERE project_id = $1 AND entity_kind = 'story' AND entity_id = $2",
            &[&project, &story],
        )
        .await?;
    let requirements: i64 = client
        .query_one(
            "SELECT count(*) FROM project_story_requirements WHERE project_id = $1 AND story_id = $2",
            &[&project, &story],
        )
        .await?
        .get(0);
    Ok(json!({
        "story": story,
        "tasks": rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
        "requirements": requirements,
        // Исключение — это ответ, а не дыра и не пустота. Ручка, которая
        // молчит здесь, заставит агента считать историю незакрытой.
        "exception": exception.first().map(|e| json!({
            "reason": e.get::<_, String>(0), "declaredIn": e.get::<_, String>(1)
        })),
    }))
}

/// Очередь предполёта: чего ещё не смотрели или смотрели до правки.
pub async fn preflight_queue(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            // Очередь — ВСЕ незакрытые задачи, а не только готовые к работе.
            //
            // Предполёт — стадия, а не шаг перед взятием задачи: описание
            // проверяют попыткой выполнить его вслух, и делают это по всему
            // плану, прежде чем начинается код. Фильтр по готовности отвечал
            // «в очереди три» там, где ступень лестницы насчитывала семьдесят
            // шесть, — два числа на один вопрос.
            //
            // Готовность при этом не теряется: она стоит полем у строки, и
            // порядок обхода по ней виден.
            "SELECT t.id, t.title, d.revision,
                    (SELECT max(v.task_revision) FROM preflight_verdict v
                      WHERE v.project_id = t.project_id AND v.task_id = t.id) AS seen_revision,
                    coalesce(r.ready, false) AS ready
               FROM project_plan_tasks t
               JOIN project_documents d ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
               LEFT JOIN task_ready r ON r.project_id = t.project_id AND r.task_id = t.id
              WHERE t.project_id = $1 AND t.state <> 'closed'
              ORDER BY coalesce(r.ready, false) DESC, t.milestone_id, t.ord",
            &[&project],
        )
        .await?;
    let mut queue = Vec::new();
    let (mut never, mut unknown, mut stale) = (0, 0, 0);
    for r in &rows {
        let revision: i64 = r.get(2);
        let seen: Option<i64> = r.get(3);
        // «Предполёт устарел» — сравнение ревизий, а не память. Причин попасть в
        // очередь ТРИ, и они разные:
        //   предполёта не было вовсе;
        //   он был, но ревизия на тот момент неизвестна — летопись началась
        //     позже вердикта, и покрывает ли он нынешний текст, сказать нечем;
        //   он был до правки задачи.
        // Слить их в одну — значит объявить неизвестное устаревшим.
        if seen != Some(revision) {
            let why = match seen {
                None => { never += 1; "предполёта не было" }
                Some(0) => { unknown += 1; "предполёт был, но ревизия на тот момент неизвестна" }
                Some(_) => { stale += 1; "предполёт был до правки задачи" }
            };
            queue.push(json!({
                "task": r.get::<_, String>(0), "title": r.get::<_, String>(1),
                "revision": revision,
                "seenAtRevision": seen.filter(|v| *v > 0),
                "ready": r.get::<_, bool>(4),
                "why": why,
            }));
        }
    }
    let ready = queue.iter().filter(|q| q["ready"] == json!(true)).count();
    Ok(json!({ "queue": queue, "count": queue.len(), "ready": ready,
               "never": never, "revisionUnknown": unknown, "stale": stale }))
}

/// Приём состояний задач от харнеса.
///
/// Подаёт тот, у кого есть репозиторий: состояние выводится из закрывающего
/// трейлера, а трейлеры знает история. Сервер их не читает и читать не должен —
/// он принимает поданное и запоминает, когда видел.
pub async fn push_task_state(
    pool: &Pool,
    project: &str,
    states: &[(String, String, String)],
    seen_at: i64,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    // Подача полная, а не добавочная: задача, исчезнувшая из подачи, потеряла
    // трейлер, и держать её прежнее состояние значило бы помнить отменённое.
    tx.execute("DELETE FROM task_state WHERE project_id = $1", &[&project]).await?;
    let mut written = 0;
    for (task, state, commit) in states {
        written += tx
            .execute(
                "INSERT INTO task_state (project_id, task_id, state, closing_commit, seen_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (project_id, task_id) DO UPDATE
                   SET state = EXCLUDED.state, closing_commit = EXCLUDED.closing_commit,
                       seen_at = EXCLUDED.seen_at",
                &[&project, task, state, commit, &seen_at],
            )
            .await?;
    }
    // След ПОДАЧИ, а не только её содержимого.
    //
    // `task_state` держала 99 строк и ни слова о том, кто и когда их подал:
    // отличить «харнес подал сегодня» от «лежит с августа, а датчик молчит уже
    // месяц» было нечем. Всякий вывод о состоянии задач при этом выглядел
    // свежим. Отметка о проходе — то же лекарство, что у сверки порождённого.
    tx.execute(
        "INSERT INTO fact_push (project_id, fact, at, actor, rows) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at,
           actor = EXCLUDED.actor, rows = EXCLUDED.rows",
        &[&project, &"task-state", &seen_at, &"харнес", &(states.len() as i32)],
    )
    .await?;
    tx.commit().await?;
    Ok(json!({ "accepted": written, "pushedAt": seen_at }))
}

/// Где документ и история расходятся о состоянии задачи.
///
/// **Молчание документа — не расхождение.** Красная задача говорит о себе
/// дословно: «Состояние не записано здесь намеренно. Оно вычисляется из истории
/// по закрывающему трейлеру, как у всех задач набора: документ, несущий
/// состояние, становится вторым источником истины и расходится молча».
///
/// Считать это расхождением значило бы читать ОБЪЯВЛЕННОЕ ОТСУТСТВИЕ как
/// находку — и получать восемьдесят два крика, после которых ручке перестают
/// верить, а сквозь неё проходит настоящее.
///
/// Расхождение — это когда документ говорит ДРУГОЕ.
pub async fn state_disagreements(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT t.id, f.value, coalesce(s.state, 'not_started'), coalesce(s.closing_commit, '')
               FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name AND f.name = 'Состояние'
                AND btrim(f.value) <> ''
               LEFT JOIN task_state s ON s.project_id = t.project_id AND s.task_id = t.id
              WHERE t.project_id = $1 AND t.entity_kind <> ''
                AND (CASE WHEN f.value ILIKE '%закрыт%' THEN 'closed' ELSE 'not_started' END)
                    <> coalesce(s.state, 'not_started')
              ORDER BY t.id",
            &[&project],
        )
        .await?;
    // Сколько документов молчат по устройству вида — сказать надо, иначе
    // «расхождений ноль» будет читаться как «все документы состояние несут».
    let silent = client
        .query_one(
            "SELECT count(*) FROM project_plan_tasks t
               JOIN task_state s ON s.project_id = t.project_id AND s.task_id = t.id
              WHERE t.project_id = $1 AND t.entity_kind <> ''
                AND NOT EXISTS (SELECT 1 FROM project_document_fields f
                                 WHERE f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name
                                   AND f.name = 'Состояние' AND btrim(f.value) <> '')",
            &[&project],
        )
        .await?
        .get::<_, i64>(0);
    Ok(json!({
        "count": rows.len(),
        "silentByDesign": silent,
        "why": "молчание документа о состоянии — объявленное устройство вида, а не расхождение; расхождение — когда документ говорит другое",
        "disagreements": rows.iter().map(|r| json!({
            "task": r.get::<_, String>(0),
            "documentSays": r.get::<_, Option<String>>(1),
            "historySays": r.get::<_, String>(2),
            "closingCommit": r.get::<_, String>(3),
        })).collect::<Vec<_>>(),
    }))
}

/// Объявить способ проверки пункта готовности.
///
/// Способ **объявляется**, а не добывается разбором. Я пробовал добыть: правило
/// «пункт называет проверяющего в обратных кавычках» дало два совпадения на
/// весь набор, и одно из двух называло задачу, а не проверку. Правило, которое
/// ломается на половине своих же совпадений, — не правило; выведенный по нему
/// зелёный пункт хуже отсутствующего.
///
/// `query` сервер считает сам. `command` — не считает: у него нет ни
/// репозитория, ни оболочки, и заводить их ради чек-листа значит менять, чем
/// сервер является. Команду выполняет харнес и подаёт итог — тем же путём, что
/// состояния задач.
pub async fn set_method(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
    ord: i32,
    method_kind: &str,
    method: &str,
    declared_by: &str,
    drop: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let n = client
            .execute(
                "DELETE FROM readiness_method
                  WHERE project_id = $1 AND owner_kind = $2 AND owner_id = $3 AND ord = $4",
                &[&project, &kind, &id, &ord],
            )
            .await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" }, "ord": ord }));
    }
    // Способ объявляется ЧЬЕМУ-ТО пункту. Вызов без вида и без номера писал
    // строку с пустым владельцем и `ord = -1` — объявление, которое ничему не
    // принадлежит и никогда не сработает.
    if kind.trim().is_empty() || ord < 0 {
        return Ok(json!({ "status": "no_owner",
                          "why": "способ объявляется пункту: нужен вид владельца и номер пункта" }));
    }
    // Кто объявил — записывается. Это не украшение: способ живёт дольше
    // пересборки, и без имени объявившего его нельзя ни спросить, ни убрать
    // выборочно. Проверка, снимающая чужие объявления заодно со своими, уже
    // стёрла один настоящий способ.
    client
        .execute(
            "INSERT INTO readiness_method (project_id, owner_kind, owner_id, ord, method_kind, method, declared_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (project_id, owner_kind, owner_id, ord)
               DO UPDATE SET method_kind = EXCLUDED.method_kind, method = EXCLUDED.method,
                             declared_by = EXCLUDED.declared_by",
            &[&project, &kind, &id, &ord, &method_kind, &method, &declared_by],
        )
        .await?;
    let n = client
        .execute(
            "UPDATE readiness_item SET method_kind = $5, method = $6
              WHERE project_id = $1 AND owner_kind = $2 AND owner_id = $3 AND ord = $4",
            &[&project, &kind, &id, &ord, &method_kind, &method],
        )
        .await?;
    Ok(json!({ "updated": n, "methodKind": method_kind, "declaredBy": declared_by,
               "survivesRebuild": true }))
}

/// Сейчас в миллисекундах — время подачи, а не время события.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// След правки в летописи сущности.
///
/// Пишется на КАЖДУЮ принятую запись: вид, имя, ревизия, кто и что изменилось.
/// Прежде правка меняла `content` и `revision`, не оставляя следа «кто и
/// когда»; пока был волт, на это отвечал `git log`, а после его удаления — никто.
pub async fn record_edit(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
    revision: i64,
    actor: &str,
    event: &str,
) -> Result<u64, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute(
            "INSERT INTO entity_event (project_id, entity_kind, entity_id, ord, at, event, actor, source)
             VALUES ($1, $2, $3, $4, current_date, $5, $6, 'edit')
             ON CONFLICT (project_id, entity_kind, entity_id, source, ord)
               DO UPDATE SET event = EXCLUDED.event, actor = EXCLUDED.actor",
            &[&project, &kind, &id, &(revision as i32), &event, &actor],
        )
        .await
}

/// Приём наблюдений о репозитории. Подача полная в пределах вида факта.
///
/// Датчик подаёт то, что видит: «такие таблицы есть в миграциях», «в контракте
/// столько операций». Ни одного вывода: сошлось ли это с набором — вопрос к
/// серверу, у которого лежит и то и другое.
pub async fn push_code_facts(
    pool: &Pool,
    project: &str,
    kind: &str,
    facts: &[(String, String)],
    actor: &str,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let before = tx
        .query_one("SELECT count(*) FROM code_fact WHERE project_id = $1 AND kind = $2", &[&project, &kind])
        .await?
        .get::<_, i64>(0);
    tx.execute("DELETE FROM code_fact WHERE project_id = $1 AND kind = $2", &[&project, &kind]).await?;
    for (name, detail) in facts {
        tx.execute(
            "INSERT INTO code_fact (project_id, kind, name, detail) VALUES ($1,$2,$3,$4)
             ON CONFLICT (project_id, kind, name) DO UPDATE SET detail = EXCLUDED.detail",
            &[&project, &kind, name, detail],
        )
        .await?;
    }
    tx.execute(
        "INSERT INTO fact_push (project_id, fact, at, actor, rows) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at, actor = EXCLUDED.actor, rows = EXCLUDED.rows",
        &[&project, &kind, &now_ms(), &actor, &(facts.len() as i32)],
    )
    .await?;
    tx.commit().await?;
    Ok(json!({ "kind": kind, "was": before, "now": facts.len() }))
}

/// Приём скиллов харнеса. Подача полная, как у агентов и состояний задач.
///
/// Пока скиллы лежали только на диске, `owner` ступени указывал на то, чего
/// база не подтвердит, а переименованный скилл ломал предложение, а не связь.
pub async fn push_skills(
    pool: &Pool,
    set_name: &str,
    skills: &[(String, String, String)],
    actor: &str,
    dry: bool,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let before = tx
        .query("SELECT name, content_hash FROM harness_skill WHERE set_name = $1", &[&set_name])
        .await?;
    let was: std::collections::HashMap<String, String> =
        before.iter().map(|r| (r.get(0), r.get(1))).collect();

    let now = now_ms();
    let mut changed: Vec<Value> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (name, description, body) in skills {
        let hash = crate::store::hash_document(body);
        names.push(name.clone());
        match was.get(name) {
            Some(old) if *old == hash => {}
            Some(_) => changed.push(json!({ "skill": name, "what": "тело изменилось" })),
            None => changed.push(json!({ "skill": name, "what": "новый" })),
        }
        if dry {
            continue;
        }
        tx.execute(
            "INSERT INTO harness_skill (set_name, name, description, body, content_hash, updated_at, updated_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (set_name, name) DO UPDATE SET description = EXCLUDED.description,
               body = EXCLUDED.body, content_hash = EXCLUDED.content_hash,
               updated_at = EXCLUDED.updated_at, updated_by = EXCLUDED.updated_by",
            &[&set_name, name, description, body, &hash, &now, &actor],
        )
        .await?;
    }
    // Снятый скилл исчезает тем, что не пришёл, — но только если на него никто
    // не ссылается: ступень с владельцем-скиллом держит его внешним ключом, и
    // отказ здесь честнее тихого удаления.
    let mut kept: Vec<Value> = Vec::new();
    for (name, _) in &was {
        if names.contains(name) {
            continue;
        }
        if dry {
            changed.push(json!({ "skill": name, "what": "снят" }));
            continue;
        }
        match tx
            .execute("DELETE FROM harness_skill WHERE set_name = $1 AND name = $2", &[&set_name, name])
            .await
        {
            Ok(_) => changed.push(json!({ "skill": name, "what": "снят" })),
            Err(e) => kept.push(json!({ "skill": name, "why": format!("на него ссылаются: {e}") })),
        }
    }
    if dry {
        // Сверка ничего не пишет: расхождение считает тот, кто владеет хешем.
        // Считать его второй раз на стороне харнеса — завести второй способ
        // мерить одно и то же, и разойтись они смогут молча.
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(json!({ "was": was.len(), "now": names.len(), "dry": dry,
               "drift": changed.len(), "changed": changed, "kept": kept }))
}

/// Приём вердиктов предполёта. Подача полная: харнес — держатель истории, база
/// — приёмник, как у состояний задач.
///
/// Ревизия задачи берётся из подачи, а не из текущего документа: вердикт
/// получен на той ревизии, которая была в тот момент, и «предполёт устарел»
/// считается сравнением, а не памятью.
pub async fn push_preflight(
    pool: &Pool,
    project: &str,
    verdicts: &[(String, i64, i64, String, i32, String)],
    actor: &str,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let before = tx
        .query_one("SELECT count(*) FROM preflight_verdict WHERE project_id = $1", &[&project])
        .await?
        .get::<_, i64>(0);
    tx.execute("DELETE FROM preflight_verdict WHERE project_id = $1", &[&project]).await?;
    let mut written = 0u64;
    let mut refused: Vec<Value> = Vec::new();
    let mut stale: Vec<Value> = Vec::new();
    for (task, at, revision, verdict, findings, body) in verdicts {
        // Задача, которой в плане нет, отвергается поимённо: молча принятый
        // вердикт о несуществующей задаче — это ложь, которую потом никто не
        // найдёт.
        let known = tx
            .query_one(
                "SELECT count(*) FROM project_plan_tasks WHERE project_id = $1 AND id = $2",
                &[&project, task],
            )
            .await?
            .get::<_, i64>(0);
        if known == 0 {
            refused.push(json!({ "task": task, "why": "такой задачи в плане нет" }));
            continue;
        }
        // Ревизию задачи на момент вердикта знает БАЗА, а не подающий: у харнеса
        // репозиторий, у базы летопись. Если на тот день летописи ещё не было —
        // ревизия остаётся нулём, и это значит «неизвестна», а не «первая».
        let revision = if *revision > 0 {
            *revision
        } else {
            tx.query_one(
                "SELECT coalesce(max(r.revision), 0) FROM project_document_revisions r
                   JOIN project_plan_tasks t ON t.project_id = r.project_id AND t.entity_kind = r.entity_kind AND t.entity_name = r.entity_name
                  WHERE r.project_id = $1 AND t.id = $2 AND r.written_at <= $3",
                &[&project, task, at],
            )
            .await?
            .get::<_, i64>(0)
        };
        // Вердикт, поданный не на нынешнюю правку, принимается — но НАЗЫВАЕТСЯ.
        //
        // Предполёт вписывает свои находки в саму задачу, и правка документа
        // отменяет вердикт, который её же и вызвал: у myack так обесценились
        // все 128 вердиктов до одного. Молча принять такую подачу значит
        // выдать самоотменившийся вердикт за свежий. Порядок, который это
        // снимает, один: сперва находки в документ, потом вердикт на ту правку,
        // которая получилась.
        let current: i64 = tx
            .query_one(
                "SELECT coalesce(max(d.revision), 0) FROM project_documents d
                   JOIN project_plan_tasks t ON t.project_id = d.project_id
                    AND t.entity_kind = d.entity_kind AND t.entity_name = d.entity_name
                  WHERE d.project_id = $1 AND t.id = $2",
                &[&project, task],
            )
            .await?
            .get(0);
        if revision > 0 && current > 0 && revision != current {
            stale.push(json!({ "task": task, "verdictAt": revision, "documentAt": current,
                               "why": "вердикт снят не на нынешней правке задачи и потому уже устарел" }));
        }
        written += tx
            .execute(
                "INSERT INTO preflight_verdict (project_id, task_id, at, task_revision, verdict, findings, body)
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
                &[&project, task, at, &revision, verdict, findings, body],
            )
            .await?;
    }
    tx.execute(
        "INSERT INTO fact_push (project_id, fact, at, actor, rows) VALUES ($1, 'preflight', $2, $3, $4)
         ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at, actor = EXCLUDED.actor, rows = EXCLUDED.rows",
        &[&project, &now_ms(), &actor, &(written as i32)],
    )
    .await?;
    tx.commit().await?;
    Ok(json!({ "was": before, "now": written, "refused": refused.len(), "refusedTasks": refused,
               "staleOnArrival": stale.len(), "stale": stale }))
}

/// Приём открытых рабочих деревьев — статус «в работе».
///
/// Единственный статус, который меняется В ТЕЧЕНИЕ волны, и ради него в
/// интерфейс и смотрят. Факт знает только тот, у кого есть репозиторий:
/// `git worktree list`. Подача полная — закрытое дерево исчезает тем, что не
/// пришло.
pub async fn push_worktrees(
    pool: &Pool,
    project: &str,
    open: &[(String, String, i64)],
    actor: &str,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let before = tx
        .query_one("SELECT count(*) FROM task_worktree WHERE project_id = $1", &[&project])
        .await?
        .get::<_, i64>(0);
    tx.execute("DELETE FROM task_worktree WHERE project_id = $1", &[&project]).await?;
    let mut written = 0u64;
    for (task, branch, since) in open {
        written += tx
            .execute(
                "INSERT INTO task_worktree (project_id, task_id, branch, since)
                 VALUES ($1,$2,$3,$4) ON CONFLICT (project_id, task_id) DO UPDATE
                   SET branch = EXCLUDED.branch, since = EXCLUDED.since",
                &[&project, task, branch, since],
            )
            .await?;
    }
    tx.execute(
        "INSERT INTO fact_push (project_id, fact, at, actor, rows) VALUES ($1, 'worktree', $2, $3, $4)
         ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at, actor = EXCLUDED.actor, rows = EXCLUDED.rows",
        &[&project, &now_ms(), &actor, &(written as i32)],
    )
    .await?;
    tx.commit().await?;
    Ok(json!({ "was": before, "now": written }))
}

/// Вопрос и его задача-держатель: расхождение в обе стороны.
///
/// **Машинно объявленной связи вопрос→держатель в наборе НЕТ.** Первая версия
/// этой ручки брала за связь всякое упоминание вопроса в поле требований задачи
/// — и попалась ровно в ту яму, о которой набор предупреждает: пять таких полей
/// оказались фразами об ОТСУТСТВИИ собственных требований («нет собственных;
/// исполняет решения Q-277 и Q-282», «собственных нет — задача про оснастку
/// проверок (Q-305)»). Из объяснения отсутствия связи не добываются: так уже
/// однажды родились 86 ложных связей задача→требование.
///
/// Поэтому здесь два разных ответа, и они помечены разными словами:
///
///   `declared` — связь, объявленная машинно. Сегодня таких ноль, и это
///     сказано числом, а не выдано за «расхождений нет»;
///   `byText` — вопрос, чьё ЗАКРЫТИЕ обосновано фразой «держатель написан»,
///     при том что ни одна названная в тексте задача не закрыта. Это чтение
///     текста, а не связь, и оно так и подписано: находка для человека.
pub async fn question_holders(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");

    // 1. Объявленная связь — ПОЛЕ «Держатель» в шапке вопроса, и только оно.
    //    Проза не разбирается: держатель, добытый регуляркой из абзаца, — это
    //    угадывание с видом проверки, и один раз оно уже завело 86 ложных связей.
    let declared = client
        .query(
            "SELECT q.id, q.state, btrim(replace(replace(f.value, '`', ''), '\u{00a0}', ' ')) AS holder,
                    coalesce(t.state, 'нет в плане')
               FROM project_questions q
               JOIN project_document_fields f
                 ON f.project_id = q.project_id AND f.entity_kind = q.entity_kind AND f.entity_name = q.entity_name AND f.name = 'Держатель'
                AND btrim(f.value) <> ''
               LEFT JOIN project_plan_tasks t
                 ON t.project_id = q.project_id
                AND t.id = btrim(replace(replace(f.value, '`', ''), '\u{00a0}', ' '))
              WHERE q.project_id = $1 AND q.state <> 'open'
              ORDER BY q.id",
            &[&project],
        )
        .await?;
    let (mut ready, mut early, mut waiting) = (Vec::new(), Vec::new(), Vec::new());
    for r in &declared {
        let (id, state, task, task_state): (String, String, String, String) =
            (r.get(0), r.get(1), r.get(2), r.get(3));
        let row = json!({ "question": id, "holder": task, "holderState": task_state });
        match (state.as_str(), task_state.as_str()) {
            // Держатель закрылся — ответ исполнен, вопросу пора в «закрыт».
            ("decided", "closed") => ready.push(row),
            // Решено, а держатель ещё не закрыт — ШТАТНОЕ состояние, ради
            // которого `decided` и заведено: ответ есть, исполнения нет.
            // Это работа впереди, а не находка.
            ("decided", _) => waiting.push(row),
            // Закрыт, а держатель не закрылся — «исполнено» сказано раньше
            // исполнения. Вот это находка.
            ("closed", s) if s != "closed" => early.push(row),
            _ => {}
        }
    }

    // 2. Чтение текста, и оно узко нарочно: задача берётся из ПОЛЯ «Закрыт» —
    //    того самого, где записано основание закрытия («исполнено: `M0-T12`
    //    заведена»). Не из всего документа: текст вопроса поминает по пять
    //    задач, и среди них всегда найдётся закрытая — такая проверка молчала
    //    бы всегда. Именно поле основания называет исполнителя.
    let by_text = client
        .query(
            "SELECT q.id, q.state_text, f.value_raw,
                    (SELECT coalesce(string_agg(DISTINCT m[1] || ' ' || coalesce(t.state, 'нет в плане'), ' · '), '')
                       FROM regexp_matches(f.value_raw, '([MV][0-9]+-T[0-9a-z]+)', 'g') AS m
                       LEFT JOIN project_plan_tasks t
                         ON t.project_id = q.project_id AND t.id = m[1]) AS named
               FROM project_questions q
               JOIN project_document_fields f
                 ON f.project_id = q.project_id AND f.entity_kind = q.entity_kind AND f.entity_name = q.entity_name AND f.name = 'Закрыт'
              WHERE q.project_id = $1 AND q.state = 'closed'
              ORDER BY q.id",
            &[&project],
        )
        .await?;
    // Два разных основания, и набор различает их сам.
    //
    // Реестр объявляет правило вслух: «вопрос, ответ которого „это несёт задача
    // X“, отвечён в момент, когда X его называет». По этому правилу закрытие
    // при незакрытой задаче ЗАКОННО, и трогать такие строки нельзя.
    //
    // А «**исполнено**: `X` **заведена**» — не то же самое: заведение задачи
    // выдано за её исполнение. Это дефект, и он отделяется от первого одним
    // словом в самом основании.
    let mut said: Vec<Value> = Vec::new();
    let mut by_rule: Vec<Value> = Vec::new();
    for r in &by_text {
        let named: String = r.get(3);
        if named.is_empty() || named.contains("closed") {
            continue;
        }
        let basis: String = r.get(2);
        let row = json!({ "question": r.get::<_, String>(0),
                          "state": r.get::<_, String>(1),
                          "closedBy": basis.chars().take(90).collect::<String>(),
                          "named": named });
        if basis.contains("заведена") || basis.contains("заведён") {
            said.push(row);
        } else {
            by_rule.push(row);
        }
    }

    let blind = client
        .query_one(
            "SELECT count(*) FROM project_questions q
              WHERE q.project_id = $1 AND q.state <> 'open'
                AND NOT EXISTS (SELECT 1 FROM project_document_fields f
                                 WHERE f.project_id = q.project_id AND f.entity_kind = q.entity_kind AND f.entity_name = q.entity_name
                                   AND f.name = 'Держатель' AND btrim(f.value) <> '')",
            &[&project],
        )
        .await?
        .get::<_, i64>(0);

    Ok(json!({
        "declaredLinks": declared.len(),
        "readyToClose": ready.len(), "ready": ready,
        "closedTooEarly": early.len(), "tooEarly": early,
        "waitingForHolder": waiting.len(), "waiting": waiting,
        "unjudgeable": blind,
        "closedOnCreation": said.len(), "onCreation": said,
        "closedByRegistryRule": by_rule.len(), "byRule": by_rule,
        "why": "держатель берётся ТОЛЬКО из поля «Держатель»; проза не разбирается. `waitingForHolder` — решено и ждёт исполнения, это не находка. `closedOnCreation` — заведение задачи выдано за исполнение, дефект; `closedByRegistryRule` — закрытие по объявленному правилу реестра, законно",
    }))
}

/// Сводка по виду: перечень сущностей с колонками-числами.
///
/// **Одна ручка на все разделы интерфейса.** Страница требований спрашивает
/// «сколько у требования проверок», страница решений — «сколько отвергнутых
/// вариантов», страница историй — «сколько требований у истории». Это один
/// вопрос с разным предметом; если каждая страница заведёт свой запрос, они
/// разойдутся молча — как разошлись две таблицы связей задача→требование.
///
/// Форма ответа у всех видов одна: имя, заголовок, четыре словесных колонки и
/// четыре числовых. Единая форма — не экономия, а условие: разбор ответа в
/// интерфейсе тоже один, и новый вид не требует новой страницы.
///
/// Вид, для которого сводка не написана, отвечает ОТКАЗОМ с именем вида, а не
/// пустым перечнем: пустой список тут читался бы как «сущностей нет».
pub async fn summary(pool: &Pool, project: &str, kind: &str) -> Result<Value, Miss> {
    let client = pool.get().await.expect("пул отдал соединение");
    let (sql, words, numbers) = match kind {
        "requirement" => (
            "SELECT r.id, r.text, r.kind, r.area, r.priority,
                    CASE WHEN r.satisfied THEN 'уже есть' ELSE '' END,
                    (SELECT count(*) FROM project_checks c
                      WHERE c.project_id = r.project_id AND c.requirement_id = r.id),
                    (SELECT count(*) FROM project_story_requirements s
                      WHERE s.project_id = r.project_id AND s.requirement_id = r.id),
                    (SELECT count(*) FROM task_requirement t
                      WHERE t.project_id = r.project_id AND t.requirement_id = r.id),
                    (SELECT count(*) FROM project_requirement_needs n
                      WHERE n.project_id = r.project_id AND n.requirement_id = r.id)
               FROM project_requirements r WHERE r.project_id = $1 ORDER BY r.id",
            ["kind", "area", "priority", "satisfied"],
            ["checks", "stories", "tasks", "needs"],
        ),
        "decision" => (
            "SELECT d.id, d.title, d.status, d.date, d.deciders,
                    CASE WHEN d.consequences <> '' THEN 'записаны' ELSE '' END,
                    (SELECT count(*) FROM project_decision_alternatives a
                      WHERE a.project_id = d.project_id AND a.decision_id = d.id),
                    (SELECT count(*) FROM project_decision_links l
                      WHERE l.project_id = d.project_id AND l.decision_id = d.id),
                    (SELECT count(*) FROM project_decision_links l
                      WHERE l.project_id = d.project_id AND l.decision_id = d.id AND l.kind = 'closes'),
                    (SELECT count(*) FROM project_article_references f
                      WHERE f.project_id = d.project_id AND f.entity_kind = d.entity_kind AND f.entity_name = d.entity_name)
               FROM project_decisions d WHERE d.project_id = $1 ORDER BY d.number",
            ["status", "date", "deciders", "consequences"],
            ["alternatives", "links", "closes", "articles"],
        ),
        "story" => (
            "SELECT s.id, s.title, s.area, s.persona, s.phase, s.feature,
                    (SELECT count(*) FROM project_story_requirements r
                      WHERE r.project_id = s.project_id AND r.story_id = s.id),
                    (SELECT count(*) FROM project_screen_references f
                      WHERE f.project_id = s.project_id AND f.source = s.id),
                    (SELECT count(*) FROM project_need_stories n
                      WHERE n.project_id = s.project_id AND n.story_id = s.id),
                    (SELECT count(*) FROM project_feature_stories f
                      WHERE f.project_id = s.project_id AND f.story_id = s.id)
               FROM project_stories s WHERE s.project_id = $1 ORDER BY s.id",
            ["area", "persona", "phase", "feature"],
            ["requirements", "screens", "needs", "features"],
        ),
        "screen" => (
            "SELECT s.id, s.title, s.area, '', '',
                    CASE WHEN EXISTS (SELECT 1 FROM screen_states t
                                       WHERE t.project_id = s.project_id AND t.screen_id = s.id
                                         AND t.has_states) THEN 'есть' ELSE '' END,
                    (SELECT count(*) FROM project_screen_references r
                      WHERE r.project_id = s.project_id AND r.screen_id = s.id),
                    (SELECT count(*) FROM project_screen_references r
                      WHERE r.project_id = s.project_id AND r.screen_id = s.id AND r.source_kind = 'story'),
                    (SELECT count(*) FROM project_screen_references r
                      WHERE r.project_id = s.project_id AND r.screen_id = s.id AND r.source_kind = 'task'),
                    0::bigint
               FROM project_screens s WHERE s.project_id = $1 ORDER BY s.id",
            ["area", "", "", "states"],
            ["referenced", "stories", "tasks", ""],
        ),
        "question" => (
            "SELECT q.id, q.title, q.state, q.gate, q.closed_at,
                    q.answer_state,
                    (SELECT count(*) FROM project_decision_links l
                      WHERE l.project_id = q.project_id AND l.kind = 'closes' AND l.target = q.id),
                    (SELECT count(*) FROM entity_event e
                      WHERE e.project_id = q.project_id AND e.entity_kind = 'question' AND e.entity_id = q.id),
                    (SELECT count(*) FROM project_document_revisions v
                      WHERE v.project_id = q.project_id AND v.entity_kind = q.entity_kind AND v.entity_name = q.entity_name),
                    0::bigint
               FROM project_questions q WHERE q.project_id = $1 ORDER BY q.number",
            // Колонка называется тем же именем, каким её спрашивает поверхность.
            // Прежде ручка отдавала `answer` со словом «есть», а страница искала
            // `answerState` — не находила и честно писала «не сказано» у вопроса
            // с записанным ответом.
            ["state", "gate", "closedAt", "answerState"],
            ["decisions", "journal", "edits", ""],
        ),
        "need" => (
            "SELECT n.id, n.text, n.priority, n.theme, n.sides, n.sources,
                    (SELECT count(*) FROM project_need_stories s
                      WHERE s.project_id = n.project_id AND s.need_id = n.id AND s.kind = 'declared'),
                    (SELECT count(*) FROM project_requirement_needs r
                      WHERE r.project_id = n.project_id AND r.need_id = n.id),
                    0::bigint, 0::bigint
               FROM project_needs n WHERE n.project_id = $1 ORDER BY n.number",
            ["priority", "theme", "sides", "sources"],
            ["stories", "requirements", "", ""],
        ),
        "term" => (
            "SELECT t.id, t.term, t.area, t.meaning, '',
                    CASE WHEN EXISTS (SELECT 1 FROM term_retired r
                                       WHERE r.project_id = t.project_id AND r.term = t.id) THEN 'снято' ELSE '' END,
                    (SELECT count(*) FROM project_document_cells c
                      WHERE c.project_id = t.project_id AND c.value LIKE '%' || t.id || '%'),
                    0::bigint, 0::bigint, 0::bigint
               FROM project_terms t WHERE t.project_id = $1 ORDER BY t.id",
            ["area", "meaning", "", "retired"],
            ["mentions", "", "", ""],
        ),
        "feature" => (
            "SELECT f.id, f.title, '', '', '', '',
                    (SELECT count(*) FROM project_feature_stories s
                      WHERE s.project_id = f.project_id AND s.feature_id = f.id),
                    (SELECT count(DISTINCT r.requirement_id) FROM project_feature_stories s
                       JOIN project_story_requirements r
                         ON r.project_id = s.project_id AND r.story_id = s.story_id
                      WHERE s.project_id = f.project_id AND s.feature_id = f.id),
                    (SELECT count(DISTINCT c.requirement_id) FROM project_feature_stories s
                       JOIN project_story_requirements r
                         ON r.project_id = s.project_id AND r.story_id = s.story_id
                       JOIN project_checks c
                         ON c.project_id = r.project_id AND c.requirement_id = r.requirement_id
                      WHERE s.project_id = f.project_id AND s.feature_id = f.id),
                    0::bigint
               FROM project_features f WHERE f.project_id = $1 ORDER BY f.id",
            ["", "", "", ""],
            ["stories", "requirements", "covered", ""],
        ),
        "article" => (
            "SELECT a.number::text, a.title, '', '', '', '',
                    (SELECT count(*) FROM project_article_references r
                      WHERE r.project_id = a.project_id AND r.number = a.number),
                    length(a.body)::bigint, 0::bigint, 0::bigint
               FROM project_articles a WHERE a.project_id = $1 ORDER BY a.number",
            ["", "", "", ""],
            ["citedBy", "chars", "", ""],
        ),
        other => return Err(Miss::Unprojected(other.to_owned())),
    };

    let rows = client.query(sql, &[&project]).await.map_err(|e| Miss::Db(e.to_string()))?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut row = serde_json::Map::new();
            row.insert("id".into(), json!(r.get::<_, String>(0)));
            row.insert("title".into(), json!(r.get::<_, String>(1)));
            for (i, name) in words.iter().enumerate() {
                if !name.is_empty() {
                    row.insert((*name).into(), json!(r.get::<_, String>(i + 2)));
                }
            }
            for (i, name) in numbers.iter().enumerate() {
                if !name.is_empty() {
                    row.insert((*name).into(), json!(r.get::<_, i64>(i + 6)));
                }
            }
            Value::Object(row)
        })
        .collect();
    let columns: Vec<&str> = words.iter().chain(numbers.iter()).copied().filter(|c| !c.is_empty()).collect();
    Ok(json!({ "kind": kind, "count": out.len(), "words": words.iter().filter(|c| !c.is_empty()).collect::<Vec<_>>(),
               "numbers": numbers.iter().filter(|c| !c.is_empty()).collect::<Vec<_>>(),
               "columns": columns, "rows": out }))
}

/// Связи одной сущности — всё, что панель раздела показывает под ней.
///
/// Одна ручка, как и сводка: панель требования спрашивает «какие проверки,
/// какие истории, какие задачи», панель истории — «какие требования, какие
/// экраны». Разные предметы, один вопрос. Вид без описанных связей отвечает
/// отказом, а не пустотой.
pub async fn links_of(pool: &Pool, project: &str, kind: &str, id: &str) -> Result<Value, Miss> {
    let client = pool.get().await.expect("пул отдал соединение");
    if id.is_empty() {
        return Err(Miss::NoEntity(kind.to_owned(), String::new()));
    }
    // Каждая связь — свой запрос: имя, подпись, вид на той стороне.
    let sets: Vec<(&str, &str)> = match kind {
        "requirement" => vec![
            ("checks", "SELECT c.id, left(c.spec, 90), 'check' FROM project_checks c
                         WHERE c.project_id = $1 AND c.requirement_id = $2 ORDER BY c.id"),
            ("stories", "SELECT s.story_id, coalesce(left(t.title, 90), ''), 'story'
                           FROM project_story_requirements s
                           LEFT JOIN project_stories t ON t.project_id = s.project_id AND t.id = s.story_id
                          WHERE s.project_id = $1 AND s.requirement_id = $2 ORDER BY s.story_id"),
            ("tasks", "SELECT r.task_id, coalesce(left(t.title, 90), ''), 'task'
                         FROM task_requirement r
                         LEFT JOIN project_plan_tasks t ON t.project_id = r.project_id AND t.id = r.task_id
                        WHERE r.project_id = $1 AND r.requirement_id = $2 ORDER BY r.task_id"),
            ("needs", "SELECT n.need_id, coalesce(left(d.text, 90), ''), 'need'
                         FROM project_requirement_needs n
                         LEFT JOIN project_needs d ON d.project_id = n.project_id AND d.id = n.need_id
                        WHERE n.project_id = $1 AND n.requirement_id = $2 ORDER BY n.need_id"),
            ("decisions", "SELECT l.decision_id, coalesce(left(d.title, 90), ''), 'decision'
                             FROM project_decision_links l
                             LEFT JOIN project_decisions d ON d.project_id = l.project_id AND d.id = l.decision_id
                            WHERE l.project_id = $1 AND l.target = $2 ORDER BY l.decision_id"),
        ],
        "story" => vec![
            ("requirements", "SELECT s.requirement_id, coalesce(left(r.text, 90), ''), 'requirement'
                                FROM project_story_requirements s
                                LEFT JOIN project_requirements r ON r.project_id = s.project_id AND r.id = s.requirement_id
                               WHERE s.project_id = $1 AND s.story_id = $2 ORDER BY s.requirement_id"),
            ("screens", "SELECT f.screen_id, coalesce(left(s.title, 90), ''), 'screen'
                           FROM project_screen_references f
                           LEFT JOIN project_screens s ON s.project_id = f.project_id AND s.id = f.screen_id
                          WHERE f.project_id = $1 AND f.source = $2 ORDER BY f.screen_id"),
            ("needs", "SELECT n.need_id, coalesce(left(d.text, 90), '') || ' · ' || n.kind, 'need'
                         FROM project_need_stories n
                         LEFT JOIN project_needs d ON d.project_id = n.project_id AND d.id = n.need_id
                        WHERE n.project_id = $1 AND n.story_id = $2 ORDER BY n.need_id"),
            ("features", "SELECT f.feature_id, coalesce(left(t.title, 90), ''), 'feature'
                            FROM project_feature_stories f
                            LEFT JOIN project_features t ON t.project_id = f.project_id AND t.id = f.feature_id
                           WHERE f.project_id = $1 AND f.story_id = $2 ORDER BY f.feature_id"),
        ],
        "article" => vec![
            ("citedBy", "SELECT r.entity_name, '', r.entity_kind FROM project_article_references r
                          WHERE r.project_id = $1 AND r.number::text = $2
                          ORDER BY r.entity_kind, r.entity_name"),
        ],
        "feature" => vec![
            ("stories", "SELECT f.story_id, coalesce(left(s.title, 90), ''), 'story'
                           FROM project_feature_stories f
                           LEFT JOIN project_stories s ON s.project_id = f.project_id AND s.id = f.story_id
                          WHERE f.project_id = $1 AND f.feature_id = $2 ORDER BY f.story_id"),
        ],
        "decision" => vec![
            ("alternatives", "SELECT a.title, left(a.body, 160), 'alternative'
                                FROM project_decision_alternatives a
                               WHERE a.project_id = $1 AND a.decision_id = $2 ORDER BY a.ord"),
            ("links", "SELECT l.target, l.kind, 'link' FROM project_decision_links l
                        WHERE l.project_id = $1 AND l.decision_id = $2 ORDER BY l.kind, l.target"),
            ("incoming", "SELECT l.decision_id, l.kind, 'decision' FROM project_decision_links l
                           WHERE l.project_id = $1 AND l.target = $2 ORDER BY l.decision_id"),
        ],
        other => return Err(Miss::Unprojected(other.to_owned())),
    };

    let mut out = serde_json::Map::new();
    for (name, sql) in sets {
        let rows = client.query(sql, &[&project, &id]).await.map_err(|e| Miss::Db(e.to_string()))?;
        out.insert(
            name.to_owned(),
            json!(rows
                .iter()
                .map(|r| json!({ "id": r.get::<_, String>(0), "title": r.get::<_, String>(1),
                                 "kind": r.get::<_, String>(2) }))
                .collect::<Vec<_>>()),
        );
    }
    Ok(json!({ "kind": kind, "id": id, "sets": out }))
}
/// Объявить автора документа. Пустое имя снимает объявление.
///
/// Автор — не «кто правил последним»: машинный проход правит сотни документов и
/// автором от этого не становится. Поэтому запись отдельная и проходами не
/// трогается.
pub async fn set_author(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    author: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Снятие — очистка имени, а не удаление документа: автор — свойство
    // документа, и убрать его значит сказать «не назван», а не снести сам
    // документ вместе с текстом.
    if drop_it {
        let gone = client
            .execute("UPDATE project_documents SET author = '' \
                      WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                     &[&project, &kind, &name])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
    }
    let n = client
        .execute(
            "UPDATE project_documents SET author = $4
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name, &author],
        )
        .await?;
    if n == 0 {
        return Ok(json!({ "entity": format!("{kind} {name}").trim().to_owned(), "status": "not_found" }));
    }
    Ok(json!({ "entity": format!("{kind} {name}").trim().to_owned(),
               "author": if author.is_empty() { Value::Null } else { json!(author) } }))
}

/// Кто за какими документами стоит.
pub async fn authors(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT coalesce(nullif(author, ''), '(не объявлен)'), count(*)::bigint,
                    string_agg(DISTINCT entity_kind, ', ' ORDER BY entity_kind)
               FROM project_documents WHERE project_id = $1 GROUP BY 1 ORDER BY 2 DESC",
            &[&project],
        )
        .await?;
    let nameless: i64 = client
        .query_one(
            "SELECT count(*) FROM project_documents WHERE project_id = $1 AND author = ''",
            &[&project],
        )
        .await?
        .get(0);
    Ok(json!({
        "authors": rows.iter().map(|r| json!({
            "author": r.get::<_, String>(0),
            "documents": r.get::<_, i64>(1),
            "kinds": r.get::<_, Option<String>>(2),
        })).collect::<Vec<_>>(),
        "undeclared": nameless,
        "why": "автор — тот, кто за документ отвечает; «кто правил последним» отвечает на другой вопрос и после машинного прохода зовётся именем прохода"
    }))
}

/// Пускать ли этого человека и записать, что он приходил.
///
/// Один вызов на запрос, и он же ведёт след: отдельный «журнал входов» рядом с
/// проверкой разошёлся бы с ней в первый же отказ.
pub async fn edge_admits(pool: &Pool, principal: &str) -> bool {
    let Ok(client) = pool.get().await else { return false };
    let declared: i64 = match client.query_one("SELECT count(*) FROM edge_principal", &[]).await {
        Ok(r) => r.get(0),
        Err(_) => return false,
    };
    let allowed = if declared == 0 {
        true
    } else {
        client
            .query_one(
                "SELECT count(*) FROM edge_principal WHERE principal = $1",
                &[&principal],
            )
            .await
            .map(|r| r.get::<_, i64>(0) > 0)
            .unwrap_or(false)
    };
    let now = now_ms();
    let (ok, no): (i64, i64) = if allowed { (1, 0) } else { (0, 1) };
    let _ = client
        .execute(
            "INSERT INTO edge_seen (principal, first_at, last_at, requests, refused)
             VALUES ($1,$2,$2,$3,$4)
             ON CONFLICT (principal) DO UPDATE SET last_at = EXCLUDED.last_at,
               requests = edge_seen.requests + $3, refused = edge_seen.refused + $4",
            &[&principal, &now, &ok, &no],
        )
        .await;
    allowed
}

/// Объявить, что этому человеку можно войти. Пустая пометка снимает объявление.
pub async fn allow_principal(
    pool: &Pool,
    principal: &str,
    note: Option<&str>,
    drop: bool,
    actor: &str,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let gone = client
            .execute("DELETE FROM edge_principal WHERE principal = $1", &[&principal])
            .await?;
        return Ok(json!({ "principal": principal, "removed": gone }));
    }
    client
        .execute(
            "INSERT INTO edge_principal (principal, note, declared_at, declared_by)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (principal) DO UPDATE SET note = EXCLUDED.note,
               declared_at = EXCLUDED.declared_at, declared_by = EXCLUDED.declared_by",
            &[&principal, &note.unwrap_or(""), &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "principal": principal, "allowed": true }))
}

/// Кто объявлен допущенным и кто на самом деле ходит.
pub async fn principals(pool: &Pool) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let declared = client
        .query("SELECT principal, note FROM edge_principal ORDER BY principal", &[])
        .await?;
    let seen = client
        .query(
            "SELECT s.principal, s.requests, s.refused,
                    to_timestamp(s.first_at/1000)::date::text, to_timestamp(s.last_at/1000)::date::text,
                    EXISTS (SELECT 1 FROM edge_principal p WHERE p.principal = s.principal)
               FROM edge_seen s ORDER BY s.last_at DESC",
            &[],
        )
        .await?;
    Ok(json!({
        "declared": declared.iter().map(|r| json!({
            "principal": r.get::<_, String>(0), "note": r.get::<_, String>(1) })).collect::<Vec<_>>(),
        "seen": seen.iter().map(|r| json!({
            "principal": r.get::<_, String>(0),
            "requests": r.get::<_, i64>(1),
            "refused": r.get::<_, i64>(2),
            "first": r.get::<_, String>(3),
            "last": r.get::<_, String>(4),
            "declared": r.get::<_, bool>(5),
        })).collect::<Vec<_>>(),
        "why": if declared.is_empty() {
            "допущенные не объявлены: сервер пускает всякого, кого назовёт край, и записывает это"
        } else {
            "пускают только объявленных; остальным отказ, и он записан"
        }
    }))
}

/// Заморозить набор на начало выпуска: что он говорил в этот миг.
///
/// Не обход файлов и не `shasum`: хеш и ревизия у документа уже есть, и снимок
/// — это они. Повторная заморозка того же выпуска отказывает, а не переписывает:
/// заморозка, сдвинутая задним числом, отвечает на вопрос «что изменилось» так,
/// будто ничего.
pub async fn freeze_version(
    pool: &Pool,
    project: &str,
    version: &str,
    actor: &str,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let already: i64 = client
        .query_one(
            "SELECT count(*) FROM version_freeze WHERE project_id = $1 AND version = $2",
            &[&project, &version],
        )
        .await?
        .get(0);
    if already > 0 {
        return Ok(json!({ "version": version, "status": "already", "documents": already,
                          "why": "выпуск уже заморожен; вторая заморозка стёрла бы то, с чем сравнивают" }));
    }
    let n = client
        .execute(
            "INSERT INTO version_freeze
                (project_id, version, entity_kind, entity_name, content_hash, revision, frozen_at, frozen_by)
             SELECT project_id, $2, entity_kind, entity_name, content_hash, revision, $3, $4
               FROM project_documents WHERE project_id = $1",
            &[&project, &version, &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "version": version, "status": "frozen", "documents": n }))
}

/// Что выпуск сделал с набором: изменил, удалил, добавил.
pub async fn version_delta(pool: &Pool, project: &str, version: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT coalesce(f.entity_kind, d.entity_kind), coalesce(f.entity_name, d.entity_name),
                    CASE WHEN f.entity_kind IS NULL THEN 'new'
                         WHEN d.entity_kind IS NULL THEN 'removed'
                         WHEN d.content_hash <> f.content_hash THEN 'changed'
                         ELSE 'unchanged' END
               FROM version_freeze f
               FULL JOIN project_documents d
                 ON d.project_id = f.project_id AND d.entity_kind = f.entity_kind
                AND d.entity_name = f.entity_name
              WHERE coalesce(f.project_id, d.project_id) = $1
                AND (f.version = $2 OR f.version IS NULL)
              ORDER BY 3, 1, 2",
            &[&project, &version],
        )
        .await?;
    let mut counts: std::collections::BTreeMap<String, i64> = Default::default();
    let mut examples: Vec<Value> = Vec::new();
    for r in &rows {
        let (kind, name, fate): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        *counts.entry(fate.clone()).or_default() += 1;
        if fate != "unchanged" && examples.len() < 20 {
            examples.push(json!({ "entity": format!("{kind} {name}").trim().to_owned(), "fate": fate }));
        }
    }
    let frozen: i64 = client
        .query_one(
            "SELECT count(*) FROM version_freeze WHERE project_id = $1 AND version = $2",
            &[&project, &version],
        )
        .await?
        .get(0);
    if frozen == 0 {
        return Ok(json!({ "version": version, "known": false,
                          "why": "выпуск не заморожен: сравнивать не с чем, и это не «ничего не изменилось»" }));
    }
    Ok(json!({ "version": version, "frozen": frozen, "fates": counts, "examples": examples }))
}

/// Записать один скилл. Подача целым набором (`skills-push`) для этого не годится:
/// она снимает всё, что не пришло, а исходников скиллов на машине больше нет —
/// база единственное место, где они есть.
/// Объявить субагента: тело, описание, инструменты, модель.
///
/// Ручки записи у субагента не было вовсе — только чтение. Из-за этого правку
/// тела делали ручкой УМЕНИЯ, и она заводила умение с именем субагента, а одно
/// такое имя совпало и перезаписало настоящее умение. Дверь, которой нет,
/// заставляет ходить в соседнюю.
#[allow(clippy::too_many_arguments)]
pub async fn set_agent(
    pool: &Pool,
    set_name: &str,
    name: &str,
    description: Option<&str>,
    body: &str,
    tools: Option<&str>,
    model: Option<&str>,
    actor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "субагент без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM harness_agent WHERE set_name = $1 AND name = $2", &[&set_name, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    let hash = crate::store::hash_document(body);
    let was = client
        .query("SELECT content_hash, description, tools, model FROM harness_agent
                 WHERE set_name = $1 AND name = $2", &[&set_name, &name])
        .await?;
    let (old_hash, old_desc, old_tools, old_model) = was
        .first()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2),
                  r.get::<_, String>(3)))
        .unwrap_or_default();
    // Незаданное поле не стирает заданное прежде: ручку зовут и ради одного тела.
    let description = description.map(str::to_owned).unwrap_or(old_desc);
    let tools = tools.map(str::to_owned).unwrap_or(old_tools);
    let model = model.map(str::to_owned).unwrap_or(old_model);
    let now = now_ms();
    client
        .execute(
            "INSERT INTO harness_agent (set_name, name, description, tools, model, body,
                                        content_hash, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (set_name, name) DO UPDATE SET description = EXCLUDED.description,
               tools = EXCLUDED.tools, model = EXCLUDED.model, body = EXCLUDED.body,
               content_hash = EXCLUDED.content_hash, updated_at = EXCLUDED.updated_at",
            &[&set_name, &name, &description, &tools, &model, &body, &hash, &now],
        )
        .await?;
    let _ = actor;
    Ok(json!({ "status": if old_hash.is_empty() { "created" } else if old_hash == hash { "unchanged" } else { "written" },
               "agent": name, "bytes": body.len(), "was": old_hash, "now": hash }))
}

pub async fn set_skill(
    pool: &Pool,
    set_name: &str,
    name: &str,
    description: Option<&str>,
    body: &str,
    allowed_tools: Option<&str>,
    disable_model_invocation: Option<bool>,
    actor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM harness_skill WHERE set_name = $1 AND name = $2", &[&set_name, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    let hash = crate::store::hash_document(body);
    let was = client
        .query(
            "SELECT content_hash, description, allowed_tools, disable_model_invocation
               FROM harness_skill WHERE set_name = $1 AND name = $2",
            &[&set_name, &name],
        )
        .await?;
    let (old_hash, old_description, old_tools, old_flag) = was
        .first()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2),
                  r.get::<_, Option<bool>>(3)))
        .unwrap_or_default();
    // Незаписанное не считается изменением: `skill-set` зовут и ради одного
    // описания, и незаданное поле не должно стирать заданное прежде.
    let tools = allowed_tools.map(str::to_owned).unwrap_or(old_tools.clone());
    let flag = disable_model_invocation.or(old_flag);
    if old_hash == hash
        && description.map_or(true, |d| d == old_description)
        && tools == old_tools
        && flag == old_flag
    {
        return Ok(json!({ "skill": name, "status": "unchanged" }));
    }
    let description = description.unwrap_or(&old_description).to_owned();
    client
        .execute(
            "INSERT INTO harness_skill (set_name, name, description, body, content_hash,
                                        allowed_tools, disable_model_invocation, updated_at, updated_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (set_name, name) DO UPDATE SET description = EXCLUDED.description,
               body = EXCLUDED.body, content_hash = EXCLUDED.content_hash,
               allowed_tools = EXCLUDED.allowed_tools,
               disable_model_invocation = EXCLUDED.disable_model_invocation,
               updated_at = EXCLUDED.updated_at, updated_by = EXCLUDED.updated_by",
            &[&set_name, &name, &description, &body, &hash, &tools, &flag, &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "skill": name, "status": if old_hash.is_empty() { "created" } else { "written" },
               "bytes": body.len(), "was": old_hash, "now": hash }))
}

/// Где скиллы ещё водят агента по файлам набора.
///
/// Проверка машинная и потому годная: корни набора и `.md` рядом с ними — то,
/// чего в теле скилла быть не должно. Своя машинерия харнеса (`.harness/`,
/// `core/`) под правило не подпадает: это не набор.
pub async fn skills_with_paths(pool: &Pool, set_name: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT s.name, count(*)::bigint, min(l.line)
               FROM harness_skill s,
                    LATERAL unnest(string_to_array(s.body, E'\n')) AS l(line)
              WHERE s.set_name = $1
                AND l.line ~ '(^|[^A-Za-z0-9._/-])(00-frame|10-intent|20-surface|30-design|40-proof|50-plan|60-runs|70-after|80-harness|90-reference)/'
              GROUP BY s.name ORDER BY 2 DESC, 1",
            &[&set_name],
        )
        .await?;
    Ok(json!({
        "set": set_name,
        "skills": rows.len(),
        "found": rows.iter().map(|r| json!({
            "skill": r.get::<_, String>(0),
            "lines": r.get::<_, i64>(1),
            "example": r.get::<_, String>(2).trim().chars().take(110).collect::<String>(),
        })).collect::<Vec<_>>(),
        "why": "скилл, ведущий к файлу набора, приведёт исполнителя в пустоту: файлов нет"
    }))
}

/// Объявить область экрана: раздел интерфейса, к которому он принадлежит.
///
/// Область ниоткуда не выводится — ни из имени экрана, ни из текста документа.
/// Это ДАННЫЕ, и ставятся они прямо. Пересборку переживают, потому что живут в
/// своей таблице, а не в той, которую пересборка удаляет и пишет заново.
pub async fn set_screen_area(
    pool: &Pool,
    project: &str,
    screen: &str,
    area: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM screen_area WHERE project_id = $1 AND screen_id = $2", &[&project, &screen])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if area.is_empty() {
        let gone = client
            .execute(
                "DELETE FROM screen_area WHERE project_id = $1 AND screen_id = $2",
                &[&project, &screen],
            )
            .await?;
        return Ok(json!({ "screen": screen, "area": Value::Null, "removed": gone,
                          "why": "область снята: у экрана её больше не объявлено" }));
    }
    client
        .execute(
            "INSERT INTO screen_area (project_id, screen_id, area) VALUES ($1,$2,$3)
             ON CONFLICT (project_id, screen_id) DO UPDATE SET area = EXCLUDED.area",
            &[&project, &screen, &area],
        )
        .await?;
    // Проекция экранов пересобирается отдельно; здесь колонка правится сразу,
    // чтобы объявленное было видно тем же вызовом, а не следующей пересборкой.
    let touched = client
        .execute(
            "UPDATE project_screens SET area = $3 WHERE project_id = $1 AND id = $2",
            &[&project, &screen, &area],
        )
        .await?;
    Ok(json!({ "screen": screen, "area": area, "screens": touched }))
}

/// Сверить порождённые файлы с тем, что считает сервер, и оставить след.
///
/// Гейт читает запросом; вычисление живёт на Rust. Мост между ними — строка в
/// `generated_drift`: есть строка — файл отстал.
pub async fn check_generated(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let computed = order(pool, project).await?;
    let want = computed["content"].as_str().unwrap_or("");
    let client = pool.get().await.expect("пул отдал соединение");
    let have: Option<String> = client
        .query(
            "SELECT content FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &"board", &"order"],
        )
        .await?
        .first()
        .map(|r| r.get(0));
    client
        .execute("DELETE FROM generated_drift WHERE project_id = $1 AND name = $2",
                 &[&project, &"50-plan/v1/order.md"])
        .await?;
    let same = have.as_deref() == Some(want);
    if !same {
        let detail = match &have {
            None => "файла нет".to_owned(),
            Some(text) => {
                let (a, b) = (text.lines().count(), want.lines().count());
                format!("файл отстал от вычисленного: строк {a} против {b}, знаков {} против {}",
                        text.len(), want.len())
            }
        };
        client
            .execute(
                "INSERT INTO generated_drift (project_id, name, detail) VALUES ($1, $2, $3)
                 ON CONFLICT (project_id, name) DO UPDATE SET detail = EXCLUDED.detail",
                &[&project, &"50-plan/v1/order.md", &detail],
            )
            .await?;
    }
    // След СВЕРКИ, а не только её находки.
    //
    // Пустая `generated_drift` значила две вещи разом: «сверили, и совпало» и
    // «ни разу не сверяли». Пункт гейта читал её и зеленел на второй так же, как
    // на первой. Отметка о проходе разделяет их: нет отметки — «неизвестно».
    client
        .execute(
            "INSERT INTO fact_push (project_id, fact, at, actor, rows) VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (project_id, fact) DO UPDATE SET at = EXCLUDED.at,
               actor = EXCLUDED.actor, rows = EXCLUDED.rows",
            &[&project, &"generated-order", &now_ms(), &"сверка порождённого",
              &(if same { 0i32 } else { 1i32 })],
        )
        .await?;
    Ok(json!({ "name": "board order", "matches": same,
               "waves": computed["waves"], "tasks": computed["tasks"] }))
}

/// Объявить способ проверки ступени лестницы.
///
/// Способ хранится ОТДЕЛЬНО от ступени и возвращается на неё сборкой. Пока он
/// лежал в самой строке, его стирал всякий проход, эту строку трогавший: семь
/// объявленных способов исчезли за двадцать минут, и заметно это стало только
/// потому, что их пересчитали.
/// Ответ проверки как набор строк — то, что сравнивают до подсадки и после.
///
/// Сравнивается ОТВЕТ, а не его длина. Есть проверки, у которых нарушение
/// всегда одно, а меняется формулировка: «датчик ни разу не подавал» против
/// «датчик подал, схема не снята». Длина у обеих единица, и по ней такая
/// проверка выглядела бы глухой, будучи вполне зрячей.
async fn answer_of(
    tx: &deadpool_postgres::Transaction<'_>,
    sql: &str,
    project: &str,
) -> Result<Vec<String>, String> {
    match tx.query(sql, &[&project]).await {
        Ok(rows) => {
            let mut out: Vec<String> =
                rows.iter().map(|r| r.try_get::<_, String>(0).unwrap_or_default()).collect();
            out.sort();
            Ok(out)
        }
        Err(e) => Err(db_says(&e)),
    }
}

/// Ошибка базы словами, а не «db error».
///
/// `Display` у ошибки tokio-postgres печатает ровно «db error» и прячет причину
/// в поле, которое надо спросить. Самотест, отвечающий «проба не исполнилась:
/// db error», не отличим от отсутствия ответа: чинить по такому нечего.
pub fn db_says(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => format!("{}: {}", d.severity(), d.message()),
        None => e.to_string(),
    }
}

/// Снять ступень и сдвинуть номера следом идущих.
///
/// Обратная сторона `add_step`, и с той же заботой: номер ступени — ссылка на
/// неё в способе, умении и журнале прогонов, и все они едут вместе. Способ и
/// проба снятой ступени уходят вместе с ней: оставленные, они однажды
/// достанутся чужой ступени, въехавшей на освободившийся номер.
pub async fn remove_step(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
    ord: i32,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let question: Option<String> = tx
        .query_opt(
            "SELECT question FROM harness_process_step
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord],
        )
        .await?
        .map(|r| r.get(0));
    let Some(question) = question else {
        return Ok(json!({ "status": "not_found", "why": "ступени с таким номером нет" }));
    };
    tx.execute(
        "DELETE FROM harness_process_step WHERE set_name = $1 AND process = $2 AND ord = $3",
        &[&set_name, &process, &ord],
    )
    .await?;
    tx.execute(
        "DELETE FROM harness_process_method WHERE set_name = $1 AND process = $2 AND ord = $3",
        &[&set_name, &process, &ord],
    )
    .await?;
    tx.execute(
        "DELETE FROM process_run WHERE project_id = $1 AND process = $2 AND ord = $3",
        &[&project, &process, &ord],
    )
    .await?;
    for (table, keyed_by_project) in [
        ("harness_process_step", false),
        ("harness_process_method", false),
        ("process_run", true),
    ] {
        let (down, back) = if keyed_by_project {
            (
                format!("UPDATE {table} SET ord = -(ord - 1) WHERE project_id = $1 AND process = $2 AND ord > $3"),
                format!("UPDATE {table} SET ord = -ord WHERE project_id = $1 AND process = $2 AND ord < 0"),
            )
        } else {
            (
                format!("UPDATE {table} SET ord = -(ord - 1) WHERE set_name = $1 AND process = $2 AND ord > $3"),
                format!("UPDATE {table} SET ord = -ord WHERE set_name = $1 AND process = $2 AND ord < 0"),
            )
        };
        let first: &(dyn tokio_postgres::types::ToSql + Sync) =
            if keyed_by_project { &project } else { &set_name };
        tx.execute(down.as_str(), &[first, &process, &ord]).await?;
        tx.execute(back.as_str(), &[first, &process]).await?;
    }
    tx.commit().await?;
    Ok(json!({ "status": "removed", "ord": ord, "question": question }))
}

/// Объявить требование снятым: имя, причина и чем снято.
/// Объявить статью конституции — прямо, а не выводом из текста.
///
/// Это и есть «внести в харнес»: файл читают, понимают и кладут сущностью.
/// Учить вывод чужим записям можно без конца — каждый набор пишет по-своему, и
/// разбор, выучивший второй формат, завтра встретит третий. Запись же одна на всех.
/// Объявить решение: имя, заголовок, состояние, дата, решающие.
/// Объявить выпуск, этап и задачу — прямо.
/// Объявить отвергнутый вариант решения.
///
/// Отвергнутый вариант — единственное место, где видно, что выбор вообще был.
/// Без него решение читается как единственно возможное.
/// Объявить связь задачи с требованием и с экраном — прямо.
///
/// Связь у задачи записана её же таблицей: «Требования | FR-01, FR-02», «Экраны
/// | `50`, `66`». Пока связи нет, гейт «у экрана есть история или задача» видит
/// экран ничьим, а требование — непокрытым, и обе находки ложны.
/// Объявить вопрос: имя, заголовок, состояние и чем закрыт.
/// Объявить риск либо расхождение: чем оно, чем смягчается и по чему видно.
///
/// Признак срабатывания — то, по чему будет видно, что риск реализовался, а не
/// мнение задним числом. Без него реестр превращается в список опасений.
/// Объявить цель проекта: чем измеряется, когда проверяется, что есть провал.
/// Объявить сценарий приёмки: предусловия, шаги, наблюдаемый результат, провал.
/// Объявить, что фича несёт требование либо опирается на статью.
/// Объявить, что история закрывается экраном, и кто в ней действует.
/// Объявить подробности экрана: зачем он, когда открывается, что показывает
/// пустым и сломанным, и какие требования держит.
/// Объявить подробности этапа: что делается, чем блокирован, что закрывает.
/// Объявить стадию работы либо артефакт фазы — из разбора процесса.
/// Объявить гарантию продукта либо правило внешнего заявления.
/// Объявить связь решения: что оно закрывает, отменяет, уточняет или трогает.
///
/// Вид связи назван, а не выведен из соседства: «дополняет ADR-0079» и «отменяет
/// ADR-0037» — разные отношения, и сливать их в «связано» значит терять то
/// единственное, ради чего связь записывают.
/// Объявить запись прогона: чем задача закончилась и что осталось открытым.
pub async fn declare_run_record(
    pool: &Pool, project: &str, id: &str, task: &str, milestone: &str, title: &str,
    commits: &str, dates: &str, review: &str, appeared: &str, left_open: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "прогон без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_run_record WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_run_record (project_id, id, task_id, milestone_id, title,
                                         commits, dates, review, appeared, left_open)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (project_id, id) DO UPDATE SET task_id = EXCLUDED.task_id,
           milestone_id = EXCLUDED.milestone_id, title = EXCLUDED.title,
           commits = EXCLUDED.commits, dates = EXCLUDED.dates, review = EXCLUDED.review,
           appeared = EXCLUDED.appeared, left_open = EXCLUDED.left_open",
        &[&project, &id, &task, &milestone, &title, &commits, &dates, &review, &appeared, &left_open]).await?;
    // Открытый хвост назван отдельно: прогон, закрывший задачу и оставивший
    // что-то открытым, — не то же, что прогон, закрывший её целиком.
    Ok(json!({ "status": "declared", "id": id, "leftOpen": !left_open.trim().is_empty() }))
}

pub async fn declare_decision_link(
    pool: &Pool, project: &str, decision: &str, kind: &str, target: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if decision.trim().is_empty() || target.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без решения или без цели не объявляется" }));
    }
    if !matches!(kind, "closes" | "supersedes" | "refines" | "touches" | "relates") {
        return Ok(json!({ "status": "bad_kind",
                          "why": "связь бывает: closes · supersedes · refines · touches · relates" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_decision_links WHERE project_id = $1 AND decision_id = $2 AND kind = $3 AND target = $4", &[&project, &decision, &kind, &target])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_decision_links (project_id, decision_id, kind, target)
         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
        &[&project, &decision, &kind, &target]).await?;
    Ok(json!({ "status": "declared", "decision": decision, "kind": kind, "target": target }))
}

pub async fn declare_frame_rule(
    pool: &Pool, project: &str, kind: &str, number: i32, title: &str, body: &str, held_by: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "без формулировки не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_guarantee WHERE project_id = $1 AND number = $2", &[&project, &kind])
                .await?;
            gone += client
                .execute("DELETE FROM project_claim_rule WHERE project_id = $1 AND number = $2", &[&project, &kind])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    match kind {
        "guarantee" => {
            client.execute(
                "INSERT INTO project_guarantee (project_id, number, title, held_by, body)
                 VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (project_id, number) DO UPDATE SET title = EXCLUDED.title,
                   held_by = EXCLUDED.held_by, body = EXCLUDED.body",
                &[&project, &number, &title, &held_by, &body]).await?;
            Ok(json!({ "status": "declared", "guarantee": number, "heldBy": held_by }))
        }
        _ => {
            client.execute(
                "INSERT INTO project_claim_rule (project_id, number, title, body)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT (project_id, number) DO UPDATE SET title = EXCLUDED.title,
                   body = EXCLUDED.body",
                &[&project, &number, &title, &body]).await?;
            Ok(json!({ "status": "declared", "rule": number }))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn declare_process_row(
    pool: &Pool, project: &str, kind: &str, a: &str, b: &str, c: &str, d: &str, ord: i32,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Снятие — той же ручкой, что и объявление: строка, заведённая по ошибке
    // (шапка таблицы, прочитанная как данные), иначе снимается только руками в
    // обход сервера.
    if drop_it {
        let gone = match kind {
            "stage" => client.execute(
                "DELETE FROM project_stage WHERE project_id = $1 AND name = $2", &[&project, &a]).await?,
            _ => client.execute(
                "DELETE FROM project_phase_artifact WHERE project_id = $1 AND phase = $2 AND artifact = $3",
                &[&project, &a, &b]).await?,
        };
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "rows": gone }));
    }
    match kind {
        "stage" if !a.trim().is_empty() => {
            client.execute(
                "INSERT INTO project_stage (project_id, ord, name, produces, closed_by)
                 VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (project_id, name) DO UPDATE SET ord = EXCLUDED.ord,
                   produces = EXCLUDED.produces, closed_by = EXCLUDED.closed_by",
                &[&project, &ord, &a, &b, &c]).await?;
            Ok(json!({ "status": "declared", "stage": a }))
        }
        "artifact" if !a.trim().is_empty() && !b.trim().is_empty() => {
            client.execute(
                "INSERT INTO project_phase_artifact (project_id, phase, artifact, we_have, state)
                 VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (project_id, phase, artifact) DO UPDATE SET we_have = EXCLUDED.we_have,
                   state = EXCLUDED.state",
                &[&project, &a, &b, &c, &d]).await?;
            Ok(json!({ "status": "declared", "phase": a, "artifact": b }))
        }
        _ => Ok(json!({ "status": "empty", "why": "строка процесса без имени не объявляется" })),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn declare_milestone_detail(
    pool: &Pool, project: &str, milestone: &str, what: &str, blocked_by: &str,
    requirement: &str, gate: &str, closed: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if milestone.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без этапа не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_milestone_requirements WHERE project_id = $1 AND milestone_id = $2 AND requirement_id = $3", &[&project, &milestone, &what])
                .await?;
            gone += client
                .execute("DELETE FROM project_milestone_gates WHERE project_id = $1 AND milestone_id = $2 AND gate = $3", &[&project, &milestone, &what])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !what.trim().is_empty() || !blocked_by.trim().is_empty() || !closed.trim().is_empty() {
        client.execute(
            "UPDATE project_plan_milestones SET
               what = CASE WHEN $3 = '' THEN what ELSE $3 END,
               blocked_by = CASE WHEN $4 = '' THEN blocked_by ELSE $4 END,
               closed = CASE WHEN $5 = '' THEN closed ELSE $5 END
             WHERE project_id = $1 AND id = $2",
            &[&project, &milestone, &what, &blocked_by, &closed]).await?;
    }
    if !requirement.trim().is_empty() {
        client.execute(
            "INSERT INTO project_milestone_requirements (project_id, milestone_id, requirement_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &milestone, &requirement]).await?;
    }
    if !gate.trim().is_empty() {
        client.execute(
            "INSERT INTO project_milestone_gates (project_id, milestone_id, gate)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &milestone, &gate]).await?;
    }
    Ok(json!({ "status": "declared", "milestone": milestone }))
}

pub async fn declare_screen_detail(
    pool: &Pool, project: &str, screen: &str, purpose: &str, opens_when: &str,
    empty_and_broken: &str, requirement: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if screen.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без экрана не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_screen_requirements WHERE project_id = $1 AND screen_id = $2 AND requirement_id = $3", &[&project, &screen, &purpose])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !requirement.trim().is_empty() {
        client.execute(
            "INSERT INTO project_screen_requirements (project_id, screen_id, requirement_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &screen, &requirement]).await?;
    }
    if !purpose.trim().is_empty() || !opens_when.trim().is_empty() || !empty_and_broken.trim().is_empty() {
        client.execute(
            "UPDATE project_screens SET
               purpose = CASE WHEN $3 = '' THEN purpose ELSE $3 END,
               opens_when = CASE WHEN $4 = '' THEN opens_when ELSE $4 END,
               empty_and_broken = CASE WHEN $5 = '' THEN empty_and_broken ELSE $5 END
             WHERE project_id = $1 AND id = $2",
            &[&project, &screen, &purpose, &opens_when, &empty_and_broken]).await?;
    }
    Ok(json!({ "status": "declared", "screen": screen }))
}

pub async fn declare_story_detail(
    pool: &Pool, project: &str, story: &str, screen: &str, persona: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if story.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без истории не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_story_screens WHERE project_id = $1 AND story_id = $2 AND screen_id = $3", &[&project, &story, &screen])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !screen.trim().is_empty() {
        // Экран назван номером, а зовётся именем — сводим здесь, у того, кто
        // знает имена; иначе связь ляжет на несуществующий экран.
        let full = client
            .query_opt("SELECT id FROM project_screens WHERE project_id = $1
                          AND (id = $2 OR id LIKE $2 || '-%') ORDER BY length(id) LIMIT 1",
                       &[&project, &screen]).await?
            .map(|r| r.get::<_, String>(0)).unwrap_or_else(|| screen.to_owned());
        client.execute(
            "INSERT INTO project_story_screens (project_id, story_id, screen_id) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING", &[&project, &story, &full]).await?;
    }
    if !persona.trim().is_empty() {
        client.execute(
            "UPDATE project_stories SET persona = $3 WHERE project_id = $1 AND id = $2",
            &[&project, &story, &persona]).await?;
    }
    Ok(json!({ "status": "declared", "story": story }))
}

pub async fn declare_feature_link(
    pool: &Pool, project: &str, feature: &str, requirement: &str, article: i32,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if feature.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без фичи не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_feature_requirements WHERE project_id = $1 AND feature_id = $2 AND requirement_id = $3", &[&project, &feature, &requirement])
                .await?;
            gone += client
                .execute("DELETE FROM project_feature_articles WHERE project_id = $1 AND feature_id = $2 AND article = $3", &[&project, &feature, &requirement])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !requirement.trim().is_empty() {
        client.execute(
            "INSERT INTO project_feature_requirements (project_id, feature_id, requirement_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &feature, &requirement]).await?;
    }
    if article > 0 {
        client.execute(
            "INSERT INTO project_feature_articles (project_id, feature_id, article)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &feature, &article]).await?;
    }
    Ok(json!({ "status": "declared", "feature": feature }))
}

pub async fn declare_acceptance(
    pool: &Pool, project: &str, id: &str, story: &str, number: i32, title: &str,
    preconditions: &str, steps: &str, observed: &str, fails_when: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "сценарий без имени или без названия не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_acceptance WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_acceptance (project_id, id, story_id, number, title,
                                         preconditions, steps, observed, fails_when, origin)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET story_id = EXCLUDED.story_id,
           number = EXCLUDED.number, title = EXCLUDED.title,
           preconditions = EXCLUDED.preconditions, steps = EXCLUDED.steps,
           observed = EXCLUDED.observed, fails_when = EXCLUDED.fails_when",
        &[&project, &id, &story, &number, &title, &preconditions, &steps, &observed, &fails_when]).await?;
    // Проверкой считается сценарий с наблюдаемым результатом. Без него это
    // пожелание, и оно так и называется.
    Ok(json!({ "status": "declared", "id": id,
               "isCheck": !observed.trim().is_empty() && !fails_when.trim().is_empty() }))
}

pub async fn declare_goal(
    pool: &Pool, project: &str, id: &str, number: i32, level: &str, title: &str,
    measured_by: &str, checked_when: &str, fails_when: &str, state_now: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "цель без имени или без формулировки не объявляется" }));
    }
    // Цель без способа измерить — намерение, и так и называется.
    let state = if measured_by.trim().is_empty() { "намерение" } else { "цель" };
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_goal WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_goal (project_id, id, number, level, title, measured_by,
                                   checked_when, fails_when, state_now, origin)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET number = EXCLUDED.number, level = EXCLUDED.level,
           title = EXCLUDED.title, measured_by = EXCLUDED.measured_by,
           checked_when = EXCLUDED.checked_when, fails_when = EXCLUDED.fails_when,
           state_now = EXCLUDED.state_now",
        &[&project, &id, &number, &level, &title, &measured_by, &checked_when, &fails_when, &state_now]).await?;
    Ok(json!({ "status": "declared", "id": id, "kind": state }))
}

pub async fn declare_risk(
    pool: &Pool, project: &str, id: &str, number: i32, title: &str, state: &str,
    mitigation: &str, trigger: &str, owner: &str, source: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "риск без имени не объявляется" }));
    }
    let state = if matches!(state, "open" | "accepted" | "closed") { state } else { "open" };
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_risks WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_risks (project_id, id, number, title, state, impact, probability,
                                    owner, trigger_sign, source, settled_by, entity_kind, entity_name, origin)
         VALUES ($1,$2,$3,$4,$5,'','',$6,$7,$8,$9,'register','','declared')
         ON CONFLICT (project_id, id) DO UPDATE SET number = EXCLUDED.number, title = EXCLUDED.title,
           state = EXCLUDED.state, owner = EXCLUDED.owner, trigger_sign = EXCLUDED.trigger_sign,
           source = EXCLUDED.source, settled_by = EXCLUDED.settled_by, origin = 'declared'",
        &[&project, &id, &number, &title, &state, &owner, &trigger, &source, &mitigation]).await?;
    Ok(json!({ "status": "declared", "id": id, "state": state }))
}

#[allow(clippy::too_many_arguments)]
pub async fn declare_question(
    pool: &Pool, project: &str, id: &str, number: i32, title: &str, state: &str,
    answer: &str, closed_by: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "вопрос без имени не объявляется" }));
    }
    // Снятие — той же ручкой. Без него вопрос, заведённый по ошибке (занятым
    // номером, чужим именем), снимался только запросом в базу мимо сервера — и
    // так уже вышло дважды за один разбор.
    if drop_it {
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute("DELETE FROM project_questions WHERE project_id = $1 AND id = $2",
                     &[&project, &id])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let state = if matches!(state, "open" | "decided" | "closed") { state } else { "open" };
    let client = pool.get().await.expect("пул отдал соединение");
    // Колонки вопроса объявлены проекцией; здесь заполняются те, что есть у
    // объявленного: остальное остаётся пустым и видно как пустое.
    client.execute(
        // Ответ и СОСТОЯНИЕ ответа — разные колонки. Прежде текст ответа
        // укладывался в состояние: вопрос выходил с состоянием ответа длиной в
        // полторы тысячи знаков и с пустым ответом.
        "INSERT INTO project_questions (project_id, id, number, title, state, entity_kind, entity_name,
                                        answer, answer_state, has_answer, origin)
         VALUES ($1,$2,$3,$4,$5,'register','',$6,$7,$8,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET number = EXCLUDED.number, title = EXCLUDED.title,
           state = EXCLUDED.state, answer = EXCLUDED.answer,
           answer_state = EXCLUDED.answer_state, has_answer = EXCLUDED.has_answer,
           origin = 'declared'",
        &[&project, &id, &number, &title, &state, &answer,
          &(if answer.trim().is_empty() { "unanswered" } else { "answered" }),
          &!answer.trim().is_empty()]).await?;
    if !closed_by.is_empty() {
        client.execute(
            "INSERT INTO project_decision_links (project_id, decision_id, kind, target)
             VALUES ($1,$2,'closes',$3) ON CONFLICT DO NOTHING",
            &[&project, &closed_by, &id]).await.ok();
    }
    Ok(json!({ "status": "declared", "id": id, "state": state, "closedBy": closed_by }))
}

pub async fn declare_task_requirement(
    pool: &Pool, project: &str, task: &str, requirement: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if task.trim().is_empty() || requirement.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без задачи или без требования не объявляется" }));
    }
    // Снятие той же дверью: связь, оставшаяся без задачи, — сирота, и находит
    // её только тот, кто знал, что она была.
    if drop_it {
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM task_requirement WHERE project_id = $1 AND task_id = $2
                   AND requirement_id = $3",
                &[&project, &task, &requirement],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                          "task": task, "requirement": requirement }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    client.execute(
        "INSERT INTO task_requirement (project_id, task_id, requirement_id) VALUES ($1,$2,$3)
         ON CONFLICT DO NOTHING", &[&project, &task, &requirement]).await?;
    Ok(json!({ "status": "declared", "task": task, "requirement": requirement }))
}

pub async fn declare_screen_reference(
    pool: &Pool, project: &str, source: &str, source_kind: &str, screen: &str, drop: bool,
) -> Result<Value, tokio_postgres::Error> {
    if source.trim().is_empty() || screen.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "ссылка без источника или без экрана не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let n = client.execute(
            "DELETE FROM project_screen_references WHERE project_id = $1 AND source = $2 AND screen_id = $3",
            &[&project, &source, &screen]).await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" } }));
    }
    // Экран называется своим именем. Задача зовёт его номером — `50`, — и это
    // приставка имени, а не имя: экран `50-cold-start` от сокращения не
    // становится другим. Разрешается здесь, у единственного, кто знает имена.
    let screen = match client
        .query_opt(
            "SELECT id FROM project_screens WHERE project_id = $1
              AND (id = $2 OR id LIKE $2 || '-%') ORDER BY length(id) LIMIT 1",
            &[&project, &screen])
        .await?
    {
        Some(r) => r.get::<_, String>(0),
        None => screen.to_owned(),
    };
    let screen = screen.as_str();
    client.execute(
        "INSERT INTO project_screen_references (project_id, source, source_kind, screen_id)
         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
        &[&project, &source, &source_kind, &screen]).await?;
    Ok(json!({ "status": "declared", "source": source, "screen": screen }))
}

pub async fn declare_alternative(
    pool: &Pool, project: &str, decision: &str, ord: i32, title: &str, body: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if decision.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "вариант без решения или без названия не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_decision_alternatives WHERE project_id = $1 AND decision_id = $2 AND ord = $3", &[&project, &decision, &ord])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_decision_alternatives (project_id, decision_id, ord, title, body, origin)
         VALUES ($1,$2,$3,$4,$5,'declared')
         ON CONFLICT (project_id, decision_id, ord) DO UPDATE SET title = EXCLUDED.title,
           body = EXCLUDED.body, origin = 'declared'",
        &[&project, &decision, &ord, &title, &body]).await?;
    Ok(json!({ "status": "declared", "decision": decision, "ord": ord }))
}

pub async fn declare_version(
    pool: &Pool, project: &str, id: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "выпуск без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_plan_versions WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_plan_versions (project_id, id, entity_kind, entity_name, origin)
         VALUES ($1,$2,'version',$2,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET origin = 'declared'",
        &[&project, &id]).await?;
    Ok(json!({ "status": "declared", "id": id }))
}

pub async fn declare_milestone(
    pool: &Pool, project: &str, id: &str, version: &str, ord: i32, title: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "этап без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_plan_milestones WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title,
                                              entity_kind, entity_name, origin)
         VALUES ($1,$2,$3,$4,$5,'milestone',$2,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET version_id = EXCLUDED.version_id,
           ord = EXCLUDED.ord, title = EXCLUDED.title, origin = 'declared'",
        &[&project, &id, &version, &ord, &title]).await?;
    Ok(json!({ "status": "declared", "id": id }))
}

pub async fn declare_task(
    pool: &Pool, project: &str, id: &str, milestone: &str, ord: i32, title: &str,
    kind: &str, state: &str, size: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    // Снятие идёт той же дверью, и только объявленное: спроецированная задача
    // уходит со своим документом, а не отдельной рукой.
    if drop_it {
        if id.trim().is_empty() {
            return Ok(json!({ "status": "nameless", "why": "снимать задачу без имени нечего" }));
        }
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM project_plan_tasks WHERE project_id = $1 AND id = $2
                   AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }

    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "задача без имени не объявляется" }));
    }
    // Состояние объявляется тремя словами и только ими: незнание называется
    // «не начата», а не выдумывается четвёртым словом.
    let state = if matches!(state, "not_started" | "claimed" | "closed") { state } else { "not_started" };
    let client = pool.get().await.expect("пул отдал соединение");
    client.execute(
        "INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state,
                                         closing_commit, kind, entity_kind, entity_name, origin)
         VALUES ($1,$2,$3,$4,$5,$6,$7,'',$8,'task',$2,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET milestone_id = EXCLUDED.milestone_id,
           ord = EXCLUDED.ord, title = EXCLUDED.title, size = EXCLUDED.size,
           state = EXCLUDED.state, kind = EXCLUDED.kind, origin = 'declared'",
        &[&project, &id, &milestone, &ord, &title, &size, &state, &kind]).await?;
    Ok(json!({ "status": "declared", "id": id, "state": state }))
}

pub async fn declare_decision(
    pool: &Pool, project: &str, id: &str, number: i32, title: &str, status: &str,
    status_text: &str, date: &str, deciders: &str, context: &str, decision: &str,
    consequences: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "решение без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_decisions WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client
        .execute(
            "INSERT INTO project_decisions (project_id, id, number, title, entity_kind, entity_name,
                        status, status_text, date, deciders, context, decision, consequences, waiver, origin)
             VALUES ($1,$2,$3,$4,'decision',$2,$5,$6,$7,$8,$9,$10,$11,'','declared')
             ON CONFLICT (project_id, id) DO UPDATE SET number = EXCLUDED.number,
               title = EXCLUDED.title, status = EXCLUDED.status, status_text = EXCLUDED.status_text,
               date = EXCLUDED.date, deciders = EXCLUDED.deciders, context = EXCLUDED.context,
               decision = EXCLUDED.decision, consequences = EXCLUDED.consequences, origin = 'declared'",
            &[&project, &id, &number, &title, &status, &status_text, &date, &deciders,
              &context, &decision, &consequences],
        )
        .await?;
    Ok(json!({ "status": "declared", "id": id, "state": status }))
}

/// Объявить историю.
pub async fn declare_story(
    pool: &Pool, project: &str, id: &str, title: &str, area: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "история без имени не объявляется" }));
    }
    // Снятие той же дверью: без него объявленное убирается только запросом
    // мимо сервера — второй дверью, о которой сервер не знает.
    if drop_it {
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM project_stories WHERE project_id = $1 AND id = $2 AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute(
            "INSERT INTO project_stories (project_id, id, title, entity_kind, entity_name, area,
                                           persona, phase, feature, origin)
             VALUES ($1,$2,$3,'story',$2,$4,'','','','declared')
             ON CONFLICT (project_id, id) DO UPDATE SET title = EXCLUDED.title,
               area = EXCLUDED.area, origin = 'declared'",
            &[&project, &id, &title, &area],
        )
        .await?;
    Ok(json!({ "status": "declared", "id": id }))
}

/// Объявить экран.
pub async fn declare_screen(
    pool: &Pool, project: &str, id: &str, title: &str, area: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "экран без имени не объявляется" }));
    }
    // Снятие той же дверью.
    if drop_it {
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM project_screens WHERE project_id = $1 AND id = $2 AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute(
            "INSERT INTO project_screens (project_id, id, title, entity_kind, entity_name, area, origin)
             VALUES ($1,$2,$3,'screen',$2,$4,'declared')
             ON CONFLICT (project_id, id) DO UPDATE SET title = EXCLUDED.title,
               area = EXCLUDED.area, origin = 'declared'",
            &[&project, &id, &title, &area],
        )
        .await?;
    Ok(json!({ "status": "declared", "id": id }))
}

pub async fn declare_article(
    pool: &Pool, project: &str, number: i32, title: &str, body: &str, anchor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if title.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "статья без заголовка не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_articles WHERE project_id = $1 AND number = $2", &[&project, &number])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client
        .execute(
            "INSERT INTO project_articles (project_id, number, title, anchor, body,
                                           entity_kind, entity_name, origin)
             VALUES ($1,$2,$3,$4,$5,'constitution','','declared')
             ON CONFLICT (project_id, number) DO UPDATE SET title = EXCLUDED.title,
               anchor = EXCLUDED.anchor, body = EXCLUDED.body, origin = 'declared'",
            &[&project, &number, &title, &anchor, &body],
        )
        .await?;
    Ok(json!({ "status": "declared", "number": number, "title": title }))
}

/// Объявить требование.
/// Объявить требование вне выпуска либо сквозным — с причиной.
/// Объявить, на что требование опирается: решение, статью, экран, требование.
///
/// Вид опоры называется, а не угадывается по виду имени: `ADR-0053` — решение,
/// `ART-13` — статья, `66` — экран, `UI-32` — соседнее требование.
/// Объявить, каким гейтом исполняется статья и работает ли он.
/// Объявить операцию протокола и парные ей события.
/// Объявить крейт: что делает и чего не делает.
/// Объявить строку стенда: раздел, имя, значение.
/// Объявить алгоритм истории и его связи.
/// Объявить происхождение справочного документа.
/// Объявить токен оформления.
/// Объявить разбор случившегося.
/// Внести строку объявленного слепка: документ и его отпечаток на заморозку.
///
/// Отличается от `freeze_version` тем, что снимает не сейчас, а вносит уже
/// снятое: набор пришёл с манифестом заморозки, и черта, от которой считают
/// дельту, проведена там, а не в момент ввоза.
/// Объявить артефакт поставки: что это и куда ставится.
/// Объявить зависимость задачи от другой.
///
/// Без неё порядок работ схлопывается в одну волну: сто пятьдесят четыре задачи
/// объявляются готовыми к запуску разом, хотя сами документы называют, кто кого
/// ждёт. Волна есть топологический слой, и без рёбер слой один.
pub async fn declare_task_dep(
    pool: &Pool, project: &str, task: &str, depends_on: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if task.trim().is_empty() || depends_on.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "зависимость без задачи или без цели не объявляется" }));
    }
    // Снятие идёт той же дверью. Без него объявленное ребро убирается только
    // запросом мимо сервера — второй дверью, о которой сервер не знает, и
    // разойдутся они молча.
    if drop_it {
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM project_plan_task_deps WHERE project_id = $1 AND task_id = $2
                   AND depends_on = $3",
                &[&project, &task, &depends_on],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                          "task": task, "dependsOn": depends_on }));
    }
    if task == depends_on {
        return Ok(json!({ "status": "self", "why": "задача не зависит от себя" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    // Обе стороны обязаны существовать: ребро в несуществующую задачу тихо
    // выпадает из порядка и делает волну шире, чем она есть.
    let known: i64 = client
        .query_one("SELECT count(*) FROM project_plan_tasks WHERE project_id = $1 AND id = ANY($2)",
                   &[&project, &vec![task.to_owned(), depends_on.to_owned()]]).await?.get(0);
    if known < 2 {
        return Ok(json!({ "status": "unknown_task",
                          "why": format!("нет задачи: {task} либо {depends_on}") }));
    }
    client.execute(
        "INSERT INTO project_plan_task_deps (project_id, task_id, depends_on) VALUES ($1,$2,$3)
         ON CONFLICT DO NOTHING", &[&project, &task, &depends_on]).await?;
    Ok(json!({ "status": "declared", "task": task, "dependsOn": depends_on }))
}

pub async fn declare_release_artifact(
    pool: &Pool, project: &str, name: &str, what: &str, installed_to: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "артефакт без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_release_artifact WHERE project_id = $1 AND name = $2", &[&project, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_release_artifact (project_id, name, what, installed_to)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, name) DO UPDATE SET what = EXCLUDED.what,
           installed_to = EXCLUDED.installed_to",
        &[&project, &name, &what, &installed_to]).await?;
    Ok(json!({ "status": "declared", "artifact": name }))
}

pub async fn declare_freeze_row(
    pool: &Pool, project: &str, version: &str, kind: &str, name: &str, hash: &str, actor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if version.trim().is_empty() || kind.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "строка слепка без выпуска или без вида не вносится" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM version_freeze WHERE project_id = $1 AND version = $2 AND entity_kind = $3 AND entity_name = $4", &[&project, &version, &kind, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO version_freeze (project_id, version, entity_kind, entity_name,
                                     content_hash, revision, frozen_at, frozen_by)
         VALUES ($1,$2,$3,$4,$5,0,$6,$7)
         ON CONFLICT DO NOTHING",
        &[&project, &version, &kind, &name, &hash, &now_ms(), &actor]).await?;
    Ok(json!({ "status": "declared", "version": version, "entity": format!("{kind} {name}") }))
}

pub async fn declare_postmortem(
    pool: &Pool, project: &str, id: &str, title: &str, summary: &str, timeline: &str,
    root_cause: &str, lesson: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "разбор без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_postmortem WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_postmortem (project_id, id, title, summary, timeline, root_cause, lesson)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (project_id, id) DO UPDATE SET title = EXCLUDED.title,
           summary = EXCLUDED.summary, timeline = EXCLUDED.timeline,
           root_cause = EXCLUDED.root_cause, lesson = EXCLUDED.lesson",
        &[&project, &id, &title, &summary, &timeline, &root_cause, &lesson]).await?;
    // Разбор без корневой причины — рассказ о неприятности, и это разные вещи.
    Ok(json!({ "status": "declared", "id": id, "hasRootCause": !root_cause.trim().is_empty() }))
}

pub async fn declare_token(
    pool: &Pool, project: &str, name: &str, dark: &str, light: &str, purpose: &str, section: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "токен без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_token WHERE project_id = $1 AND name = $2", &[&project, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_token (project_id, name, dark, light, purpose, section)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (project_id, name) DO UPDATE SET dark = EXCLUDED.dark, light = EXCLUDED.light,
           purpose = EXCLUDED.purpose, section = EXCLUDED.section",
        &[&project, &name, &dark, &light, &purpose, &section]).await?;
    // Токен, объявленный только в одной теме, во второй будет выдуман.
    Ok(json!({ "status": "declared", "token": name,
               "bothThemes": !dark.trim().is_empty() && !light.trim().is_empty() }))
}

#[allow(clippy::too_many_arguments)]
pub async fn declare_reference_source(
    pool: &Pool, project: &str, name: &str, source: &str, note: &str, taken: &str,
    sha: &str, ref_type: &str, from_project: &str, repo: &str,
    written: &str, updated: &str, status: &str, tags: &str, role: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "происхождение без документа не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_reference_source WHERE project_id = $1 AND entity_name = $2", &[&project, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_reference_source (project_id, entity_name, source, note, taken,
                                               sha, ref_type, from_project, repo,
                                               written, updated, status, tags, role)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
         ON CONFLICT (project_id, entity_name) DO UPDATE SET source = EXCLUDED.source,
           note = EXCLUDED.note, taken = EXCLUDED.taken, sha = EXCLUDED.sha,
           ref_type = EXCLUDED.ref_type, from_project = EXCLUDED.from_project, repo = EXCLUDED.repo,
           written = EXCLUDED.written, updated = EXCLUDED.updated, status = EXCLUDED.status,
           tags = EXCLUDED.tags, role = EXCLUDED.role",
        &[&project, &name, &source, &note, &taken, &sha, &ref_type, &from_project, &repo,
          &written, &updated, &status, &tags, &role]).await?;
    // Снятая копия без отпечатка не сверяется с originalом ничем.
    Ok(json!({ "status": "declared", "name": name, "hasSha": !sha.trim().is_empty() }))
}

pub async fn declare_algorithm(
    pool: &Pool, project: &str, id: &str, story: &str, title: &str, preconditions: &str,
    flow: &str, failure: &str, not_covered: &str, link_kind: &str, link_target: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "алгоритм без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_algorithm_links WHERE project_id = $1 AND algorithm_id = $2 AND kind = $3 AND target = $4", &[&project, &id, &story, &title])
                .await?;
            gone += client
                .execute("DELETE FROM project_algorithm WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !link_kind.trim().is_empty() && !link_target.trim().is_empty() {
        client.execute(
            "INSERT INTO project_algorithm_links (project_id, algorithm_id, kind, target)
             VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&project, &id, &link_kind, &link_target]).await?;
        return Ok(json!({ "status": "declared", "id": id, "link": link_kind }));
    }
    client.execute(
        "INSERT INTO project_algorithm (project_id, id, story_id, title, preconditions, flow,
                                        failure_branches, not_covered)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (project_id, id) DO UPDATE SET story_id = EXCLUDED.story_id,
           title = EXCLUDED.title, preconditions = EXCLUDED.preconditions, flow = EXCLUDED.flow,
           failure_branches = EXCLUDED.failure_branches, not_covered = EXCLUDED.not_covered",
        &[&project, &id, &story, &title, &preconditions, &flow, &failure, &not_covered]).await?;
    // Алгоритм без веток отказа описывает только удачный путь — и это находка,
    // а не подробность: отказ реализация выдумает сама.
    Ok(json!({ "status": "declared", "id": id, "hasFailures": !failure.trim().is_empty() }))
}

pub async fn declare_stand_row(
    pool: &Pool, project: &str, section: &str, name: &str, value: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "строка стенда без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_stand WHERE project_id = $1 AND section = $2 AND name = $3", &[&project, &section, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_stand (project_id, section, name, value) VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, section, name) DO UPDATE SET value = EXCLUDED.value",
        &[&project, &section, &name, &value]).await?;
    Ok(json!({ "status": "declared", "name": name }))
}

/// Объявить датчик: где искать, чем вынимать, как назвать факт.
#[allow(clippy::too_many_arguments)]
pub async fn declare_sensor_spec(
    pool: &Pool, project: &str, fact: &str, reads: &str, extract: &str, note: &str, how: &str,
    skip: &str, allow: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if fact.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "датчик без имени факта не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if drop_it {
        let gone = client.execute("DELETE FROM project_sensor_spec WHERE project_id = $1 AND fact = $2",
                                  &[&project, &fact]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "fact": fact }));
    }
    // Незнакомый род прежде молча становился `extract`: датчик объявляли одним,
    // он снимал другое и говорил «снято». Отказ называет допустимые роды.
    const KINDS: [&str; 13] = ["extract", "files", "secret-fields", "declared-paths", "lines",
                              "domain-vs-check", "contract-vs-schema", "contract-ops", "contract-body", "declared-lines", "contract-marks", "contract-head", "frozen-tree"];
    let how = if how.trim().is_empty() { "extract" } else { how };
    if !KINDS.contains(&how) {
        return Ok(json!({ "status": "unknown_how", "how": how,
                          "why": format!("род датчика не из этих: {}", KINDS.join(" · ")) }));
    }
    client.execute(
        "INSERT INTO project_sensor_spec (project_id, fact, reads, extract_re, note, how, skip_re, allow)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (project_id, fact) DO UPDATE SET reads = EXCLUDED.reads,
           extract_re = EXCLUDED.extract_re, note = EXCLUDED.note, how = EXCLUDED.how,
           skip_re = EXCLUDED.skip_re, allow = EXCLUDED.allow",
        &[&project, &fact, &reads, &extract, &note, &how, &skip, &allow]).await?;
    Ok(json!({ "status": "declared", "fact": fact, "reads": reads, "how": how }))
}

/// Пункты готовности без способа проверки — разбивкой по владельцам.
///
/// Их у myack тысяча шестьсот девяносто пять из тысячи семисот незнаний: почти
/// всё, чего проект о себе не знает, — это одно и то же. Плоский список из
/// двадцати строк, где эта тысяча стоит одной, врёт соразмерностью: глаз читает
/// двадцать равных бед вместо одной большой и девятнадцати мелких.
///
/// Разбивка идёт по ВИДУ ВЛАДЕЛЬЦА: пункт живёт строкой внутри задачи, вопроса
/// или документа приёмки, и чинится он там же. «Тысяча шестьсот» не говорит, с
/// чего начать; «тысяча пятьсот восемьдесят два у задач» — говорит.
pub async fn readiness_gaps(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT owner_kind, count(*)::bigint,
                    count(*) FILTER (WHERE method_kind = 'unknown')::bigint,
                    count(DISTINCT owner_id)::bigint
               FROM readiness_item
              WHERE project_id = $1
              GROUP BY owner_kind
              ORDER BY count(*) FILTER (WHERE method_kind = 'unknown') DESC, owner_kind",
            &[&project],
        )
        .await?;
    let by: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "ownerKind": r.get::<_, String>(0),
                "items": r.get::<_, i64>(1),
                "withoutMethod": r.get::<_, i64>(2),
                "owners": r.get::<_, i64>(3),
            })
        })
        .collect();
    let total: i64 = by.iter().map(|x| x["items"].as_i64().unwrap_or(0)).sum();
    let bare: i64 = by.iter().map(|x| x["withoutMethod"].as_i64().unwrap_or(0)).sum();
    Ok(json!({
        "items": total,
        "withoutMethod": bare,
        "byOwner": by,
        // Дверь названа здесь, а не в интерфейсе: чем закрывается пробел, знает
        // сервер. Интерфейс, знающий имя двери, — вторая запись о том же.
        "closedBy": "method-set",
    }))
}

/// Объявить потолок долга: сколько находок правила сегодня терпимо и почему.
///
/// Гейт, краснеющий в день, когда его завели, назавтра выключают. Потолок даёт
/// правилу приехать в проект, где долг уже есть: держит он не долг, а его РОСТ.
pub async fn set_ceiling(
    pool: &Pool, project: &str, rule: &str, ceiling: i32, why: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if rule.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "потолок без правила не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if drop_it {
        let gone = client
            .execute("DELETE FROM rule_ceiling WHERE project_id = $1 AND rule = $2", &[&project, &rule])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "rule": rule }));
    }
    // ПРАВИЛО ОБЯЗАНО ЧИТАТЬ ПОТОЛОК. `ceiling-set rule=column-input ceiling=41`
    // отвечал «declared» и не делал ничего: запрос пункта не соединялся с
    // `rule_ceiling`. Потолок, который никто не читает, — обещание, данное в
    // пустоту.
    let reads: i64 = client
        .query_one(
            "SELECT count(*) FROM gate_item
              WHERE kind = 'query' AND query LIKE '%' || $1 || '%' AND query LIKE '%rule_ceiling%'",
            &[&rule],
        )
        .await?
        .get(0);
    if reads == 0 {
        return Ok(json!({
            "status": "rule_ignores_ceiling",
            "why": format!(
                "правило «{rule}» потолка не читает: ни один пункт гейта не соединяет свой запрос с \
                 `rule_ceiling` по этому имени. Записанное здесь ничего бы не держало."
            ),
        }));
    }
    // Потолок без причины — это разрешение, выданное неизвестно кем и зачем.
    // Через полгода его никто не решится опустить: непонятно, что он держит.
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_reason",
                          "why": "потолок без причины — дыра с разрешением: назовите, что он держит" }));
    }
    client.execute(
        "INSERT INTO rule_ceiling (project_id, rule, ceiling, why, origin)
         VALUES ($1,$2,$3,$4,'declared')
         ON CONFLICT (project_id, rule) DO UPDATE SET ceiling = EXCLUDED.ceiling, why = EXCLUDED.why",
        &[&project, &rule, &ceiling, &why]).await?;
    Ok(json!({ "status": "declared", "rule": rule, "ceiling": ceiling }))
}

/// Чем снимать факты: перечень объявленных датчиков для клиента.
pub async fn sensor_specs(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query("SELECT fact, reads, extract_re, note, how, skip_re, allow FROM project_sensor_spec
                 WHERE project_id = $1 ORDER BY fact", &[&project]).await?;
    Ok(json!({ "specs": rows.iter().map(|r| json!({
        "fact": r.get::<_, String>(0), "reads": r.get::<_, String>(1),
        "extract": r.get::<_, String>(2), "note": r.get::<_, String>(3),
        "how": r.get::<_, String>(4), "skip": r.get::<_, String>(5),
        "allow": r.get::<_, String>(6) })).collect::<Vec<_>>() }))
}

/// Объявить проект: имя и репозиторий, которому он принадлежит.
pub async fn declare_project(
    pool: &Pool, id: &str, name: &str, repo: &str, actor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "проект без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project WHERE id = $1", &[&id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project (id, name, repo, declared_at, declared_by) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, repo = EXCLUDED.repo,
           declared_at = EXCLUDED.declared_at, declared_by = EXCLUDED.declared_by",
        &[&id, &name, &repo, &now_ms(), &actor]).await?;
    Ok(json!({ "status": "declared", "project": id, "repo": repo }))
}

/// Чей это репозиторий: путь дерева → проект.
///
/// Отвечает БЕЗ знания проекта — в этом весь смысл: клиент, стоящий в дереве,
/// спрашивает, кому оно принадлежит, вместо того чтобы читать ответ файлом,
/// лежащим в том же дереве.
pub async fn whose_repo(pool: &Pool, path: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query("SELECT id, name, repo FROM project WHERE repo <> '' ORDER BY length(repo) DESC", &[])
        .await?;
    for r in &rows {
        let repo: String = r.get(2);
        // Подкаталог принадлежит тому же проекту: команду набирают и из глубины.
        if path == repo || path.starts_with(&format!("{repo}/")) {
            return Ok(json!({ "project": r.get::<_, String>(0), "name": r.get::<_, String>(1),
                              "repo": repo }));
        }
    }
    Ok(json!({ "project": Value::Null,
               "why": format!("ни один проект не объявлен репозиторием {path}"),
               "known": rows.iter().map(|r| r.get::<_, String>(2)).collect::<Vec<_>>() }))
}

/// Доноры проекта и сторожа — читаются вместе: чужое дерево и то, что не даёт
/// его тронуть, отвечают на один вопрос — «что здесь не наше и чем это держится».
pub async fn donors_and_guards(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let d = client
        .query("SELECT path, what, frozen_by FROM project_donor WHERE project_id = $1 ORDER BY path",
               &[&project]).await?;
    let g = client
        .query("SELECT name, enforces, scope, refuses, acts_on, path_re, content_re, command_re
                  FROM project_guard WHERE project_id = $1 ORDER BY name", &[&project]).await?;
    Ok(json!({
        "donors": d.iter().map(|r| json!({ "path": r.get::<_, String>(0),
            "what": r.get::<_, String>(1), "frozenBy": r.get::<_, String>(2) })).collect::<Vec<_>>(),
        "guards": g.iter().map(|r| json!({ "guard": r.get::<_, String>(0),
            "enforces": r.get::<_, String>(1), "scope": r.get::<_, String>(2),
            "refuses": r.get::<_, String>(3), "actsOn": r.get::<_, String>(4),
            "pathRe": r.get::<_, String>(5), "contentRe": r.get::<_, String>(6),
            "commandRe": r.get::<_, String>(7) })).collect::<Vec<_>>(),
    }))
}

/// Донорское дерево: чужая реализация, замороженная на запись.
pub async fn declare_donor(
    pool: &Pool, project: &str, path: &str, what: &str, frozen_by: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if path.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "донор без пути не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if drop_it {
        let gone = client.execute("DELETE FROM project_donor WHERE project_id = $1 AND path = $2",
                                  &[&project, &path]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "path": path }));
    }
    client.execute(
        "INSERT INTO project_donor (project_id, path, what, frozen_by) VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, path) DO UPDATE SET what = EXCLUDED.what,
           frozen_by = EXCLUDED.frozen_by",
        &[&project, &path, &what, &frozen_by]).await?;
    // Донор без названного правила заморозки — просто чужой каталог: спорить
    // о правке в нём будет нечем.
    Ok(json!({ "status": "declared", "path": path, "frozen": !frozen_by.trim().is_empty() }))
}

/// Сторож: чем правило принуждается ДО действия, а не меряется после.
#[allow(clippy::too_many_arguments)]
pub async fn declare_guard(
    pool: &Pool, project: &str, name: &str, enforces: &str, scope: &str, refuses: &str,
    acts_on: &str, path_re: &str, content_re: &str, command_re: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "сторож без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if drop_it {
        let gone = client.execute("DELETE FROM project_guard WHERE project_id = $1 AND name = $2",
                                  &[&project, &name]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "name": name }));
    }
    let acts_on = if matches!(acts_on, "write" | "command") { acts_on } else { "write" };
    client.execute(
        "INSERT INTO project_guard (project_id, name, enforces, scope, refuses,
                                    acts_on, path_re, content_re, command_re)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (project_id, name) DO UPDATE SET enforces = EXCLUDED.enforces,
           scope = EXCLUDED.scope, refuses = EXCLUDED.refuses, acts_on = EXCLUDED.acts_on,
           path_re = EXCLUDED.path_re, content_re = EXCLUDED.content_re,
           command_re = EXCLUDED.command_re",
        &[&project, &name, &enforces, &scope, &refuses, &acts_on, &path_re, &content_re,
          &command_re]).await?;
    // Сторож без способа поймать — пожелание: он объявлен и ничего не отвергает.
    let catches = !path_re.trim().is_empty() || !content_re.trim().is_empty()
        || !command_re.trim().is_empty();
    Ok(json!({ "status": "declared", "guard": name, "enforces": enforces, "catches": catches }))
}

pub async fn declare_crate(
    pool: &Pool, project: &str, name: &str, does: &str, does_not: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "крейт без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_crate WHERE project_id = $1 AND name = $2", &[&project, &name])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_crate (project_id, name, does, does_not) VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, name) DO UPDATE SET does = EXCLUDED.does,
           does_not = EXCLUDED.does_not",
        &[&project, &name, &does, &does_not]).await?;
    // Граница «чего не делает» — то, что нарушают молча; её отсутствие называется.
    Ok(json!({ "status": "declared", "crate": name, "hasBoundary": !does_not.trim().is_empty() }))
}

pub async fn declare_protocol_op(
    pool: &Pool, project: &str, op: &str, group: &str, events: &str, requirement: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_protocol_requirements WHERE project_id = $1 AND op_group = $2 AND requirement_id = $3", &[&project, &op, &group])
                .await?;
            gone += client
                .execute("DELETE FROM project_protocol_op WHERE project_id = $1 AND op = $2", &[&project, &op])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    if !requirement.trim().is_empty() && !group.trim().is_empty() {
        client.execute(
            "INSERT INTO project_protocol_requirements (project_id, op_group, requirement_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, &group, &requirement]).await?;
        if op.trim().is_empty() {
            return Ok(json!({ "status": "declared", "group": group, "requirement": requirement }));
        }
    }
    if op.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "операция без имени не объявляется" }));
    }
    client.execute(
        "INSERT INTO project_protocol_op (project_id, op, op_group, events)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, op) DO UPDATE SET op_group = EXCLUDED.op_group,
           events = EXCLUDED.events",
        &[&project, &op, &group, &events]).await?;
    // Операция без парного события — то самое, что обещает ловить гейт
    // `protocol:event-coverage`. Ответ называет это сразу.
    Ok(json!({ "status": "declared", "op": op, "hasEvent": !events.trim().is_empty() }))
}

pub async fn declare_article_gate(
    pool: &Pool, project: &str, article: i32, gate: &str, state: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if article <= 0 || gate.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без статьи или без гейта не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_article_gates WHERE project_id = $1 AND article = $2 AND gate = $3", &[&project, &article, &gate])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_article_gates (project_id, article, gate, state)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (project_id, article, gate) DO UPDATE SET state = EXCLUDED.state",
        &[&project, &article, &gate, &state]).await?;
    // Названный, но не написанный гейт — не исполнение. Ответ это говорит.
    Ok(json!({ "status": "declared", "article": article, "gate": gate,
               "enforced": state == "enforced" }))
}

pub async fn declare_requirement_source(
    pool: &Pool, project: &str, id: &str, kind: &str, target: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() || target.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "опора без требования или без цели не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM project_requirement_sources WHERE project_id = $1 AND requirement_id = $2 AND kind = $3 AND target = $4", &[&project, &id, &kind, &target])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    client.execute(
        "INSERT INTO project_requirement_sources (project_id, requirement_id, kind, target)
         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
        &[&project, &id, &kind, &target]).await?;
    Ok(json!({ "status": "declared", "id": id, "kind": kind, "target": target }))
}

pub async fn declare_requirement_scope(
    pool: &Pool, project: &str, id: &str, out_of_version: &str, crosscutting: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "область без требования не объявляется" }));
    }
    if out_of_version.trim().is_empty() && crosscutting.trim().is_empty() {
        return Ok(json!({ "status": "no_reason",
                          "why": "и «вне выпуска», и «сквозное» объявляются причиной, а не флагом" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — очистка объявленного, а не удаление строки: строка тут
        // принадлежит не этому объявлению. Пустое значение и есть «не
        // объявлено», и читатель обязан звать это словом, а не пустотой.
        if drop_it {
            let gone = client
                .execute("UPDATE project_requirements SET out_of_version = '', crosscutting = '' \
                          WHERE project_id = $1 AND id = $2", &[&project, &id])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }

    let n = client.execute(
        "UPDATE project_requirements SET
           out_of_version = CASE WHEN $3 = '' THEN out_of_version ELSE $3 END,
           crosscutting = CASE WHEN $4 = '' THEN crosscutting ELSE $4 END
         WHERE project_id = $1 AND id = $2",
        &[&project, &id, &out_of_version, &crosscutting]).await?;
    Ok(json!({ "status": if n > 0 { "declared" } else { "not_found" }, "id": id }))
}

pub async fn declare_requirement(
    pool: &Pool, project: &str, id: &str, kind: &str, area: &str, title: &str, text: &str,
    measured_by: &str, priority: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    // Снятие идёт той же дверью. Без него объявленное требование убирается
    // только запросом мимо сервера — а это уже вторая дверь, о которой сервер
    // не знает, и разойдутся они молча.
    if drop_it {
        if id.trim().is_empty() {
            return Ok(json!({ "status": "nameless", "why": "снимать требование без имени нечего" }));
        }
        let client = pool.get().await.expect("пул отдал соединение");
        let gone = client
            .execute(
                "DELETE FROM project_requirements WHERE project_id = $1 AND id = $2
                   AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id,
                          "why": if gone > 0 { Value::Null }
                                 else { json!("снимается только объявленное: спроецированное уходит с документом") } }));
    }
    if id.trim().is_empty() || (title.trim().is_empty() && text.trim().is_empty()) {
        return Ok(json!({ "status": "empty", "why": "требование без имени и без формулировки не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute(
            "INSERT INTO project_requirements (project_id, id, kind, area, title, text, satisfied,
                                               priority, measured_by, entity_kind, entity_name, origin)
             VALUES ($1,$2,$3,$4,$5,$6,false,$7,$8,'srs','','declared')
             ON CONFLICT (project_id, id) DO UPDATE SET kind = EXCLUDED.kind, area = EXCLUDED.area,
               title = EXCLUDED.title, text = EXCLUDED.text, priority = EXCLUDED.priority,
               measured_by = EXCLUDED.measured_by, origin = 'declared'",
            &[&project, &id, &kind, &area, &title, &text, &priority, &measured_by],
        )
        .await?;
    // Требование без тела — заголовок, выданный за требование; без способа
    // проверки — намерение. Оба состояния называются, а не прячутся.
    Ok(json!({ "status": "declared", "id": id, "kind": kind,
               "hasBody": !text.trim().is_empty(), "hasCheck": !measured_by.trim().is_empty() }))
}

/// Объявить термин словаря.
pub async fn declare_term(
    pool: &Pool, project: &str, term: &str, meaning: &str, area: &str, drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if term.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "термин без имени не объявляется" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    // Снятие — той же ручкой. Термин, оказавшийся лишним, иначе убирается
    // только запросом мимо сервера, и сервер перестаёт быть единственной дверью.
    if drop_it {
        let gone = client
            .execute("DELETE FROM project_terms WHERE project_id = $1 AND id = $2", &[&project, &term])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "term": term }));
    }
    client
        .execute(
            "INSERT INTO project_terms (project_id, id, term, meaning, area, entity_kind, entity_name, origin)
             VALUES ($1,$2,$2,$3,$4,'glossary','','declared')
             ON CONFLICT (project_id, id) DO UPDATE SET meaning = EXCLUDED.meaning,
               area = EXCLUDED.area, origin = 'declared'",
            &[&project, &term, &meaning, &area],
        )
        .await?;
    Ok(json!({ "status": "declared", "term": term }))
}

pub async fn retire_requirement(
    pool: &Pool,
    project: &str,
    id: &str,
    why: &str,
    retired_by: &str,
    actor: &str,
    drop: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Заслон стоит ПОСЛЕ снятия с учёта, а не до: пустое имя объявлять нельзя,
    // но однажды записанную с пустым именем строку убрать надо чем-то, и это
    // единственная дверь к ней.
    if drop {
        let n = client
            .execute("DELETE FROM requirement_retired WHERE project_id = $1 AND id = $2", &[&project, &id])
            .await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    // Пустое имя — не требование. Прежде такой вызов заводил строку с пустым
    // именем, и она молча оставалась в наборе снятых.
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "требование без имени снятым не объявляется" }));
    }
    // Живое требование снятым не объявляется: это спрятало бы его от всех
    // проверок разом, а не рассказало бы о его судьбе.
    let alive: i64 = client
        .query_one(
            "SELECT count(*) FROM project_requirements WHERE project_id = $1 AND id = $2",
            &[&project, &id],
        )
        .await?
        .get(0);
    if alive > 0 {
        return Ok(json!({ "status": "still_alive",
                          "why": format!("требование «{id}» есть в наборе: снятым его звать нельзя") }));
    }
    client
        .execute(
            "INSERT INTO requirement_retired (project_id, id, why, retired_by, declared_at, declared_by)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (project_id, id) DO UPDATE SET why = EXCLUDED.why,
               retired_by = EXCLUDED.retired_by, declared_at = EXCLUDED.declared_at,
               declared_by = EXCLUDED.declared_by",
            &[&project, &id, &why, &retired_by, &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "status": "retired", "id": id, "retiredBy": retired_by }))
}

/// Объявить датчик репозитория: что он подаёт и через сколько его молчание
/// считается устареванием.
pub async fn declare_sensor(
    pool: &Pool,
    project: &str,
    fact: &str,
    about: &str,
    stale_after_ms: Option<i64>,
    actor: &str,
    drop: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let n = client
            .execute("DELETE FROM sensor WHERE project_id = $1 AND fact = $2", &[&project, &fact])
            .await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" }, "fact": fact }));
    }
    if fact.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "датчик без имени факта не объявляется" }));
    }
    client
        .execute(
            "INSERT INTO sensor (project_id, fact, about, stale_after_ms, declared_at, declared_by)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (project_id, fact) DO UPDATE SET about = EXCLUDED.about,
               stale_after_ms = EXCLUDED.stale_after_ms, declared_at = EXCLUDED.declared_at,
               declared_by = EXCLUDED.declared_by",
            &[&project, &fact, &about, &stale_after_ms, &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "status": "declared", "fact": fact, "staleAfterMs": stale_after_ms }))
}

/// Объявленные датчики и когда каждый подавал в последний раз.
pub async fn sensors(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT s.fact, s.about, s.stale_after_ms, f.at, f.rows, f.actor
               FROM sensor s LEFT JOIN fact_push f
                 ON f.project_id = s.project_id AND f.fact = s.fact
              WHERE s.project_id = $1 ORDER BY s.fact",
            &[&project],
        )
        .await?;
    let undeclared = client
        .query(
            "SELECT f.fact, f.at FROM fact_push f
              WHERE f.project_id = $1
                AND NOT EXISTS (SELECT 1 FROM sensor s WHERE s.project_id = f.project_id AND s.fact = f.fact)
              ORDER BY f.fact",
            &[&project],
        )
        .await?;
    // Часы здесь — НЕ настенные. Датчики запускает харнес, а не расписание, и
    // «час назад никто не подавал» значило бы всего лишь «час никто не работал».
    // Отсчёт идёт от самого свежего из объявленных датчиков: отстал тот, кто
    // промолчал, когда остальные отчитались.
    let clock: Option<i64> = client
        .query_one(
            "SELECT max(f.at) FROM fact_push f JOIN sensor s
                     ON s.project_id = f.project_id AND s.fact = f.fact
              WHERE f.project_id = $1",
            &[&project],
        )
        .await?
        .get(0);
    Ok(json!({
        "clock": clock,
        "sensors": rows.iter().map(|r| {
            let at: Option<i64> = r.get(3);
            let stale_after: Option<i64> = r.get(2);
            json!({
                "fact": r.get::<_, String>(0),
                "about": r.get::<_, String>(1),
                "lastAt": at,
                "rows": r.get::<_, Option<i32>>(4),
                "by": r.get::<_, Option<String>>(5),
                "silent": at.is_none(),
                "stale": match (at, stale_after, clock) {
                    (Some(at), Some(after), Some(clock)) => clock - at > after,
                    _ => false,
                },
            })
        }).collect::<Vec<_>>(),
        // Подающий, которого никто не объявлял, — не ошибка и не порядок: это
        // факт, о котором надо знать, чтобы решить.
        "undeclared": undeclared.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
    }))
}

/// Завести ступень лестницы на указанное место.
///
/// Место — не украшение: лестница читается сверху вниз, и ступень «объявлен ли
/// открытый выпуск» обязана стоять ДО «снят ли с него слепок». Оттого заведение
/// умеет вставлять в середину, раздвигая номера.
///
/// Раздвигаются все таблицы, где номер ступени — ссылка на неё: способ, умение,
/// и ЖУРНАЛ ПРОГОНОВ тоже. Оставить журнал непередвинутым значило бы, что его
/// прошлые записи начнут показывать на соседнюю ступень: запись сама не менялась,
/// а рассказывать станет о другом.
///
/// Сдвиг идёт через отрицательные номера. Прямое `ord = ord + 1` натыкается на
/// собственный первичный ключ на первой же строке: третья ступень становится
/// четвёртой, а четвёртая ещё на месте.
pub async fn add_step(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
    ord: i32,
    question: &str,
    owner_kind: &str,
    owner: &str,
    touches: &str,
) -> Result<Value, tokio_postgres::Error> {
    if !matches!(owner_kind, "skill" | "agent" | "none") {
        return Ok(json!({ "status": "bad_owner_kind", "why": "закрывает ступень скилл, субагент либо человек" }));
    }
    if !matches!(touches, "corpus" | "repository") {
        return Ok(json!({ "status": "bad_touches", "why": "ступень читает набор либо пишет в репозиторий" }));
    }
    if question.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "ступень без условия не заводится" }));
    }
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let known: i64 = tx
        .query_one(
            "SELECT count(*) FROM harness_process WHERE set_name = $1 AND name = $2",
            &[&set_name, &process],
        )
        .await?
        .get(0);
    if known == 0 {
        return Ok(json!({ "status": "unknown_process",
                          "why": format!("процесса «{process}» в наборе «{set_name}» нет") }));
    }
    // Умение ступени здесь не двигают: связь объявлена `ON UPDATE CASCADE`, и
    // база переносит его сама. Сдвинуть его руками значило бы сдвинуть дважды.
    for (table, keyed_by_project) in [
        ("harness_process_step", false),
        ("harness_process_method", false),
        ("process_run", true),
    ] {
        let (up, back) = if keyed_by_project {
            (
                format!("UPDATE {table} SET ord = -(ord + 1) WHERE project_id = $1 AND process = $2 AND ord >= $3"),
                format!("UPDATE {table} SET ord = -ord WHERE project_id = $1 AND process = $2 AND ord < 0"),
            )
        } else {
            (
                format!("UPDATE {table} SET ord = -(ord + 1) WHERE set_name = $1 AND process = $2 AND ord >= $3"),
                format!("UPDATE {table} SET ord = -ord WHERE set_name = $1 AND process = $2 AND ord < 0"),
            )
        };
        let first: &(dyn tokio_postgres::types::ToSql + Sync) =
            if keyed_by_project { &project } else { &set_name };
        tx.execute(up.as_str(), &[first, &process, &ord]).await?;
        tx.execute(back.as_str(), &[first, &process]).await?;
    }
    tx.execute(
        "INSERT INTO harness_process_step
            (set_name, process, ord, question, method_kind, method, owner_kind, owner, touches)
         VALUES ($1,$2,$3,$4,'unknown','',$5,$6,$7)",
        &[&set_name, &process, &ord, &question, &owner_kind, &owner, &touches],
    )
    .await?;
    tx.commit().await?;
    // Способ и проба объявляются отдельно и после: заведённая без них ступень
    // честно зовётся «нечем ответить», а не молча считается пройденной.
    Ok(json!({ "status": "added", "ord": ord, "question": question,
               "why": "способ и проба не объявлены: ступень отвечает «нечем ответить», пока их не назовут" }))
}

/// Объявить выпуск закрытым или снова открытым.
///
/// Это решение человека, а не вывод из текста: «закрыт» значит, что от него
/// начинают считать. Оттого записывается кто и когда.
pub async fn set_version_state(
    pool: &Pool,
    project: &str,
    version: &str,
    state: &str,
    actor: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    if state != "open" && state != "closed" {
        return Ok(json!({ "status": "bad_state", "why": "выпуск бывает открыт либо закрыт" }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            let mut gone = 0u64;
            gone += client
                .execute("DELETE FROM version_state WHERE project_id = $1 AND version = $2", &[&project, &version])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }
    let known: i64 = client
        .query_one(
            "SELECT count(*) FROM project_plan_versions WHERE project_id = $1 AND id = $2",
            &[&project, &version],
        )
        .await?
        .get(0);
    if known == 0 {
        return Ok(json!({ "status": "unknown_version",
                          "why": format!("выпуска «{version}» в наборе нет") }));
    }
    client
        .execute(
            "INSERT INTO version_state (project_id, version, state, changed_at, changed_by)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (project_id, version) DO UPDATE SET state = EXCLUDED.state,
               changed_at = EXCLUDED.changed_at, changed_by = EXCLUDED.changed_by",
            &[&project, &version, &state, &now_ms(), &actor],
        )
        .await?;
    Ok(json!({ "version": version, "state": state, "by": actor }))
}

/// Привязать гейт к фазе.
pub async fn set_phase_gate(
    pool: &Pool,
    project: &str,
    phase: &str,
    gate: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — очистка объявленного, а не удаление строки: строка тут
        // принадлежит не этому объявлению. Пустое значение и есть «не
        // объявлено», и читатель обязан звать это словом, а не пустотой.
        if drop_it {
            let gone = client
                .execute("UPDATE project_phase SET gate = '' WHERE project_id = $1 AND id = $2",
                         &[&project, &phase])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }

    let n = client
        .execute(
            "UPDATE project_phase SET gate = $3 WHERE project_id = $1 AND id = $2",
            &[&project, &phase, &gate],
        )
        .await?;
    Ok(json!({ "updated": n, "phase": phase, "gate": gate,
               "why": if n == 0 { "фазы с таким именем нет" } else { "" } }))
}

/// Объявить, когда ступень вообще в игре.
pub async fn set_step_when(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    when_query: &str,
    when_why: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — очистка объявленного, а не удаление строки: строка тут
        // принадлежит не этому объявлению. Пустое значение и есть «не
        // объявлено», и читатель обязан звать это словом, а не пустотой.
        if drop_it {
            let gone = client
                .execute("UPDATE harness_process_step SET when_query = '', when_why = '' \
                          WHERE set_name = $1 AND process = $2 AND ord = $3",
                         &[&set_name, &process, &ord])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }

    let n = client
        .execute(
            "UPDATE harness_process_step SET when_query = $4, when_why = $5
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord, &when_query, &when_why],
        )
        .await?;
    Ok(json!({ "updated": n, "ord": ord,
               "why": if n == 0 { "ступени с таким номером нет" } else { "" } }))
}

/// Объявить, чем ронять ступень.
pub async fn set_step_probe(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    probe: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — очистка объявленного, а не удаление строки: строка тут
        // принадлежит не этому объявлению. Пустое значение и есть «не
        // объявлено», и читатель обязан звать это словом, а не пустотой.
        if drop_it {
            let gone = client
                .execute("UPDATE harness_process_step SET probe = '' \
                          WHERE set_name = $1 AND process = $2 AND ord = $3",
                         &[&set_name, &process, &ord])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }

    let n = client
        .execute(
            "UPDATE harness_process_step SET probe = $4
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord, &probe],
        )
        .await?;
    Ok(json!({ "updated": n, "ord": ord,
               "why": if n == 0 { "ступени с таким номером нет" } else { "" } }))
}

/// Самотест лестницы: каждую ступень роняют подсаженным нарушением.
///
/// Устроен строже, чем самотест гейта был сначала. Там подсадка делалась и
/// смотрелось, вернул ли запрос строки, — но ступень, уже красная сама по себе,
/// вернула бы строки и без подсадки, и тест на ней доказывал бы ровно ничего.
/// Здесь запрос исполняется ДВАЖДЫ, до подсадки и после, и живой считается та
/// ступень, у которой число выросло. Так проверяется, что запрос видит именно
/// подсаженное, а не то, что и так лежало.
///
/// Подсадка живёт внутри транзакции и умирает вместе с ней: набор после
/// самотеста обязан остаться тем же, чем был.
pub async fn step_selftest(pool: &Pool, project: &str, process: &str) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let steps = client
        .query(
            "SELECT ord, question, method_kind, method, probe FROM harness_process_step
              WHERE process = $1 ORDER BY ord",
            &[&process],
        )
        .await?;

    let (mut alive, mut broken, mut undeclared) = (Vec::new(), Vec::new(), Vec::new());
    for r in &steps {
        let ord: i32 = r.get(0);
        let question: String = r.get(1);
        let method_kind: String = r.get(2);
        let method: String = r.get(3);
        let probe: String = r.get(4);
        if method_kind != "query" || method.trim().is_empty() {
            undeclared.push(json!({ "ord": ord, "question": question,
                                    "why": "способ не запрос: ронять нечего" }));
            continue;
        }
        if probe.trim().is_empty() {
            undeclared.push(json!({ "ord": ord, "question": question,
                                    "why": "проба не объявлена: чем ронять эту ступень — не сказано" }));
            continue;
        }
        let tx = client.transaction().await?;
        let saw = match answer_of(&tx, &method, project).await {
            Err(e) => Err(format!("запрос ступени не исполнился: {e}")),
            Ok(before) => match tx.execute(probe.as_str(), &[&project]).await {
                Err(e) => Err(format!("проба не исполнилась: {}", db_says(&e))),
                Ok(0) => Err("проба ничего не подсадила".to_owned()),
                Ok(_) => match answer_of(&tx, &method, project).await {
                    Ok(after) => Ok((before, after)),
                    Err(e) => Err(format!("запрос ступени не исполнился после подсадки: {e}")),
                },
            },
        };
        tx.rollback().await?;
        match saw {
            Ok((before, after)) if after != before => alive.push(json!({ "ord": ord, "question": question,
                "was": before.len(), "became": after.len() })),
            Ok((before, _)) => broken.push(json!({ "ord": ord, "question": question,
                "why": format!("на подсаженном нарушении ответ ступени не изменился: те же {} строк, слово в слово", before.len()) })),
            Err(why) => broken.push(json!({ "ord": ord, "question": question, "why": why })),
        }
    }

    Ok(json!({
        "steps": steps.len(),
        "alive": alive.len(), "aliveSteps": alive,
        "broken": broken.len(), "brokenSteps": broken,
        "undeclared": undeclared.len(), "undeclaredSteps": undeclared,
        "why": "живой считается ступень, у которой на подсадке ИЗМЕНИЛСЯ ОТВЕТ: красная и без подсадки красна, и по одному её цвету ничего не докажешь",
    }))
}

/// Переименовать ступень: то, что должно быть верно, чтобы она была пройдена.
///
/// Ступени звались вопросами — «есть ли открытые вопросы», «полон ли набор», —
/// и зелёный знак на них читался наугад: у одной он значил «да», у соседней
/// «нет». Условие читается одинаково: зелено — значит верно.
///
/// Пишется через сервер, а не правкой строки в базе: у ступени своя таблица со
/// своей колонкой, и дверь к ней одна.
pub async fn set_step_question(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    question: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
        // Снятие — очистка объявленного, а не удаление строки: строка тут
        // принадлежит не этому объявлению. Пустое значение и есть «не
        // объявлено», и читатель обязан звать это словом, а не пустотой.
        if drop_it {
            let gone = client
                .execute("UPDATE harness_process_step SET question = '' \
                          WHERE set_name = $1 AND process = $2 AND ord = $3",
                         &[&set_name, &process, &ord])
                .await?;
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
        }

    let n = client
        .execute(
            "UPDATE harness_process_step SET question = $4
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord, &question],
        )
        .await?;
    // Ноль строк — не «сделано»: такой ступени нет, и молчание об этом выдало бы
    // промах за успех.
    Ok(json!({ "updated": n, "ord": ord,
               "why": if n == 0 { "ступени с таким номером нет" } else { "" } }))
}

pub async fn set_step_method(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    method_kind: &str,
    method: &str,
    declared_by: &str,
    drop: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    if drop {
        let n = client
            .execute(
                "DELETE FROM harness_process_method WHERE set_name = $1 AND process = $2 AND ord = $3",
                &[&set_name, &process, &ord],
            )
            .await?;
        return Ok(json!({ "status": if n > 0 { "dropped" } else { "not_found" }, "ord": ord }));
    }
    // Способ объявляется СУЩЕСТВУЮЩЕЙ ступени. Прежде вызов без номера писал
    // строку с `ord = -1`: способ, который никому не принадлежит и никогда не
    // исполнится, но лежит в наборе как объявленный.
    let known: i64 = client
        .query_one(
            "SELECT count(*) FROM harness_process_step WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord],
        )
        .await?
        .get(0);
    if known == 0 {
        return Ok(json!({ "status": "not_found",
                          "why": format!("ступени {ord} в процессе «{process}» нет: способ объявлять некому") }));
    }
    client
        .execute(
            "INSERT INTO harness_process_method (set_name, process, ord, method_kind, method, declared_by)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (set_name, process, ord) DO UPDATE SET method_kind = EXCLUDED.method_kind,
               method = EXCLUDED.method, declared_by = EXCLUDED.declared_by",
            &[&set_name, &process, &ord, &method_kind, &method, &declared_by],
        )
        .await?;
    let n = client
        .execute(
            "UPDATE harness_process_step SET method_kind = $4, method = $5
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord, &method_kind, &method],
        )
        .await?;
    Ok(json!({ "updated": n, "methodKind": method_kind, "declaredBy": declared_by,
               "survivesRebuild": true }))
}

/// Записать подпись гейта — утверждение человека, а не отметку машины.
pub async fn sign_gate(
    pool: &Pool,
    project: &str,
    phase: &str,
    signed_at: &str,
    signed_by: &str,
    wording: &str,
    note: &str,
    docs: &[(String, String)],
) -> Result<Value, Miss> {
    if signed_by.trim().is_empty() || wording.trim().is_empty() {
        return Err(Miss::Db("подпись без подписавшего или без формулировки — не подпись".into()));
    }
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await.map_err(|e| Miss::Db(e.to_string()))?;
    tx.execute(
        "INSERT INTO gate_signature (project_id, phase, signed_at, signed_by, wording, note)
         VALUES ($1,$2,to_date($3, 'YYYY-MM-DD'),$4,$5,$6)
         ON CONFLICT (project_id, phase) DO UPDATE SET signed_at = EXCLUDED.signed_at,
           signed_by = EXCLUDED.signed_by, wording = EXCLUDED.wording, note = EXCLUDED.note,
           lifted_by = ''",
        &[&project, &phase, &signed_at, &signed_by, &wording, &note],
    )
    .await
    .map_err(|e| Miss::Db(e.to_string()))?;
    tx.execute("DELETE FROM gate_signature_doc WHERE project_id=$1 AND phase=$2", &[&project, &phase])
        .await
        .map_err(|e| Miss::Db(e.to_string()))?;
    let mut named = Vec::new();
    for (kind, name) in docs {
        // Содержание берётся СЕЙЧАС и записывается как то, под чем подписано.
        // Дальше всякая правка этого документа делает подпись устаревшей, и
        // сказать можно будет не только «хеш разошёлся», но и какая правка.
        let row = tx
            .query(
                "SELECT content_hash, revision FROM project_documents
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                &[&project, kind, name],
            )
            .await
            .map_err(|e| Miss::Db(e.to_string()))?;
        let Some(r) = row.first() else {
            return Err(Miss::NoEntity(kind.clone(), name.clone()));
        };
        let (hash, revision): (String, i64) = (r.get(0), r.get(1));
        tx.execute(
            "INSERT INTO gate_signature_doc (project_id, phase, entity_kind, entity_id, content_hash, revision)
             VALUES ($1,$2,$3,$4,$5,$6)",
            &[&project, &phase, kind, name, &hash, &revision],
        )
        .await
        .map_err(|e| Miss::Db(e.to_string()))?;
        named.push(json!({ "kind": kind, "id": name, "hash": hash, "revision": revision }));
    }
    tx.commit().await.map_err(|e| Miss::Db(e.to_string()))?;
    Ok(json!({ "phase": phase, "signedAt": signed_at, "signedBy": signed_by, "under": named }))
}

/// Самотест гейтов: пункт, который не может провалиться, — не проверка.
///
/// Повод измерен: пункт `G0` «решение закрывает существующий вопрос» спрашивал
/// требования, чьё имя равно имени проекта. Таких нет и быть не может — пункт
/// проходил всегда и не мерил ничего. Зелень, которая не способна покраснеть,
/// дороже красноты: на ней стоит подпись, а держаться ей не на чем.
///
/// Каждому пункту объявляется **проба** — запрос, подсаживающий нарушение.
/// Самотест открывает транзакцию, исполняет пробу, спрашивает пункт и **всё
/// откатывает**. Ответов три, и третий обязателен: пункт молчит на подсаженном
/// (сломан), пункт увидел (жив), пробы нет (самотест не объявлен — и это не
/// «прошёл»).
pub async fn gate_selftest(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let items = client
        .query(
            "SELECT phase, id, query, probe FROM gate_item
              WHERE kind = 'query' ORDER BY phase, item",
            &[],
        )
        .await?;

    let (mut alive, mut broken, mut undeclared) = (Vec::new(), Vec::new(), Vec::new());
    // Самотест ЗАПИСЫВАЕТ приговор пробе: «ни разу не роняли» должен видеть
    // всякий, кто смотрит гейт, а не только тот, кто позвал самотест.
    let mut verdict: Vec<(String, String, bool)> = Vec::new();
    for r in &items {
        let (phase, item): (String, String) = (r.get(0), r.get(1));
        let query: Option<String> = r.get(2);
        let probe: String = r.get(3);
        let sql = query.unwrap_or_default();
        if probe.trim().is_empty() {
            undeclared.push(json!({ "phase": phase, "item": item,
                                    "why": "проба не объявлена: чем ронять этот пункт — не сказано" }));
            continue;
        }
        // Подсадка живёт внутри транзакции и умирает вместе с ней: набор после
        // самотеста обязан остаться тем же, чем был.
        let tx = client.transaction().await?;
        // Запрос исполняется ДВАЖДЫ, до подсадки и после, и живым считается
        // пункт, у которого число выросло.
        //
        // Прежде смотрели только «вернул ли запрос строки после подсадки», и на
        // четырнадцати уже красных пунктах это доказывало ровно ничего: они
        // вернули бы строки и с пустой пробой. Самотест отвечал «35 из 35
        // живы», а проверено было двадцать одно.
        let saw = match answer_of(&tx, &sql, project).await {
            Err(e) => Err(format!("запрос пункта не исполнился: {e}")),
            Ok(before) => match tx.execute(probe.as_str(), &[&project]).await {
                Err(e) => Err(format!("проба не исполнилась: {}", db_says(&e))),
                Ok(0) => Err("проба ничего не подсадила".to_owned()),
                Ok(_) => match answer_of(&tx, &sql, project).await {
                    Ok(after) => Ok((before, after)),
                    Err(e) => Err(format!("запрос пункта не исполнился после подсадки: {e}")),
                },
            },
        };
        tx.rollback().await?;
        match saw {
            Ok((was, became)) if became != was => {
                verdict.push((phase.clone(), item.clone(), true));
                alive.push(json!({ "phase": phase, "item": item,
                    "was": was.len(), "became": became.len() }));
            }
            Ok((was, _)) => {
                verdict.push((phase.clone(), item.clone(), false));
                broken.push(json!({ "phase": phase, "item": item,
                    "why": format!("на подсаженном нарушении ответ пункта не изменился: те же {} строк, слово в слово", was.len()) }));
            }
            Err(why) => {
                verdict.push((phase.clone(), item.clone(), false));
                broken.push(json!({ "phase": phase, "item": item, "why": why }));
            }
        }
    }

    // Приговор записывается: «ни разу не роняли» должен видеть всякий, кто
    // смотрит гейт, а не только тот, кто позвал самотест.
    for (phase, id, ok) in &verdict {
        client
            .execute("UPDATE gate_item SET probe_ok = $3 WHERE phase = $1 AND id = $2",
                     &[phase, id, ok])
            .await?;
        client
            .execute("UPDATE project_gates SET probe_ok = $3 WHERE phase = $1 AND id = $2",
                     &[phase, id, ok])
            .await?;
    }

    Ok(json!({
        "queryItems": items.len(),
        "alive": alive.len(), "aliveItems": alive,
        "broken": broken.len(), "brokenItems": broken,
        "undeclared": undeclared.len(), "undeclaredItems": undeclared,
        "why": "пункт без пробы не «прошёл самотест», а «самотест не объявлен» — это разные ответы",
    }))
}

/// Порядок выполнения задач — вывод, и делает его сервер.
///
/// **Почему не харнес.** Порядок считается из строк «Зависит от», а они лежат в
/// наборе; репозиторий для этого не нужен вовсе. Пока порядок считал скрипт,
/// его вычисление жило рядом с серверным (`waves`, `next-task`) — два порядка на
/// один план, и однажды они разошлись бы, оба выглядя правыми.
///
/// **Файл при этом остаётся.** Исполнителю нужен список, который открывают и по
/// которому ведут работу; вычисление в голове проверяющего таким списком не
/// является. Но файл теперь ПРОИЗВОДНОЕ: сервер считает, файл порождается,
/// гейт сверяет.
///
/// Волна здесь — топологический слой внутри этапа: задачи слоя не зависят друг
/// от друга, и это всё, что она утверждает. Параллельного хода она не обещает —
/// этап закрывается целиком.
pub async fn order(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Задачи и их зависимости — из плана, а не из файлов: связь уже разобрана
    // при записи набора.
    let tasks = client
        .query(
            "SELECT t.id, t.milestone_id FROM project_plan_tasks t
              WHERE t.project_id = $1 AND t.kind <> 'red' AND t.entity_kind <> ''
              ORDER BY t.milestone_id, t.id",
            &[&project],
        )
        .await?;
    // Зависимости берутся из СЫРОГО поля, а не из готовой таблицы связей.
    //
    // Таблица держит только те имена, что стоят в обратных кавычках; поле умеет
    // ещё две формы — «все задачи `M2`» и ссылку на этап целиком, — и обе
    // значат «все задачи этого этапа». Потеряв их, порядок разошёлся бы с
    // планом на две волны, а выглядел бы правым.
    let dep_fields = client
        .query(
            "SELECT t.id, f.value_raw FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind AND f.entity_name = t.entity_name AND f.name = 'Зависит от'
              WHERE t.project_id = $1 AND t.kind <> 'red'",
            &[&project],
        )
        .await?;
    static TASK_REF: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"`(M\d-T\d+[a-z]?)`").expect("образец задачи"));
    static ALL_OF: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"все задачи `?(M\d)`?").expect("образец этапа"));
    static MS_LINK: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"\[`(M\d)`\]").expect("образец ссылки на этап"));
    let of_milestone = |m: &str| -> Vec<String> {
        tasks
            .iter()
            .filter(|r| r.get::<_, String>(1) == m)
            .map(|r| r.get::<_, String>(0))
            .collect()
    };
    let mut deps: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();
    for r in &dep_fields {
        let id: String = r.get(0);
        let raw: String = r.get(1);
        let set = deps.entry(id.clone()).or_default();
        for m in TASK_REF.captures_iter(&raw) {
            set.insert(m[1].to_owned());
        }
        for m in ALL_OF.captures_iter(&raw).chain(MS_LINK.captures_iter(&raw)) {
            for t in of_milestone(&m[1]) {
                set.insert(t);
            }
        }
        set.remove(&id);
    }
    /// Номер задачи: `M0-T12` → 12. Внутри слоя порядок числовой, а не строковый:
    /// «T10» после «T9», а не между «T1» и «T2».
    fn num(id: &str) -> i64 {
        id.rsplit("-T").next().and_then(|t| t.trim_end_matches(|c: char| c.is_alphabetic()).parse().ok()).unwrap_or(0)
    }

    let mut milestones: Vec<String> = tasks.iter().map(|r| r.get::<_, String>(1)).collect();
    milestones.sort();
    milestones.dedup();

    let mut waves: Vec<(String, Vec<String>)> = Vec::new();
    for m in &milestones {
        let mut pend: Vec<String> = tasks
            .iter()
            .filter(|r| &r.get::<_, String>(1) == m)
            .map(|r| r.get::<_, String>(0))
            .collect();
        while !pend.is_empty() {
            let inside: std::collections::HashSet<&String> = pend.iter().collect();
            let empty = Default::default();
            let mut ready: Vec<String> = pend
                .iter()
                .filter(|t| !deps.get(*t).unwrap_or(&empty).iter().any(|d| inside.contains(d)))
                .cloned()
                .collect();
            // Цикл — дефект плана, а не повод остановиться: слой берётся по
            // номеру, и это видно тем, что волна вышла шире ожидаемой.
            if ready.is_empty() {
                ready = pend.clone();
            }
            ready.sort_by_key(|t| num(t));
            pend.retain(|t| !ready.contains(t));
            waves.push((m.clone(), ready));
        }
    }

    let total: usize = waves.iter().map(|(_, t)| t.len()).sum();
    let mut body: Vec<String> = Vec::new();
    let mut seen: std::collections::HashMap<&str, usize> = Default::default();
    for (m, list) in &waves {
        let n = seen.entry(m.as_str()).or_insert(0);
        *n += 1;
        body.push(String::new());
        body.push(format!("## {m} · волна {n}"));
        body.push(String::new());
        for t in list {
            body.push(format!("- [ ] `{t}`"));
        }
    }

    let head = vec![
        "# Порядок выполнения задач".to_owned(),
        String::new(),
        "**Вычислен из зависимостей**, а не написан рукой: порядок считает сервер набора"
            .to_owned(),
        "(ручка `order`) из строк «Зависит от» и сверяет с этим файлом. Расхождение роняет гейт —".to_owned(),
        "иначе список отстанет от плана молча (правило «сгенерированный артефакт расходится громко»)."
            .to_owned(),
        String::new(),
        "Порядок двухуровневый: **этапы идут по номеру**, внутри этапа — топологически по".to_owned(),
        "зависимостям. Этап закрывается целиком, «частично» не существует".to_owned(),
        // Ссылка адресует СУЩНОСТЬ, а не файл. Порождатель писал путь и потому
        // расходился с набором, который на имена уже перешёл: сверка порождённого
        // краснела на семнадцати знаках, и виноват был он, а не набор.
        "([`acceptance.md`](acceptance:) §1).".to_owned(),
        String::new(),
        "**Волна — слой этого топологического порядка**: задачи внутри неё не зависят друг от".to_owned(),
        "друга. Это всё, что она говорит: последовательность работы от неё не меняется, и".to_owned(),
        "параллельного хода она не обещает. У красной фазы (`red/order.md`) волна значит больше —".to_owned(),
        "там она ещё и раздвинута по общим файлам, потому что те задачи действительно садятся".to_owned(),
        "рядом, каждая в своём дереве.".to_owned(),
        String::new(),
        format!("Волн — **{}**, задач — **{}**.", waves.len(), total),
        String::new(),
    ];

    let mut text = head;
    text.extend(body);
    text.push(String::new());
    Ok(json!({
        "waves": waves.len(), "tasks": total,
        "content": text.join("\n"),
    }))
}

/// Ссылка на сущность вместо ссылки на файл.
///
/// В наборе 7825 ссылок вида `[текст](путь.md)` — самый крупный остаток файловой
/// эпохи, и он в прозе, а не в коде. Ярлык уже называет сущность (`M0-T12`,
/// `ADR-0004`), а цель ведёт по файловой системе, которой нет.
///
/// **Замена идёт ОТ ЯРЛЫКА, а не от пути.** Путь→сущность выводится только через
/// раскладку, а раскладка — вещь источника и уйдёт; ярлык же несёт имя сам.
/// Ссылка, чей ярлык имени не несёт, **не трогается и называется**: угадать её
/// значит завести ссылку, которой никто не писал.
/// Ссылка в разметке: ярлык и цель.
///
/// Ни перевода строки, ни черты столбца, ни пробела в цели: прежний образец
/// сшивал соседние ячейки таблицы в «ссылку» — 1905 таких обрывков висели в
/// остатке как нерешённые, ссылками не будучи.
static LINK: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
    // Закрывающая скобка не входит в цель. С `\S*` она входила: жадная часть
    // проглатывала `)` и сшивала две соседние ссылки в одну —
    // `M1.md)/[M2](../M2.md`, цель, которой нет ни у одной из них.
    // Длина ярлыка ограничена, но не коротко: 120 знаков отсекали настоящую
    // ссылку с ярлыком в 178 знаков — одну на весь набор, и она осталась бы
    // единственной, кого некому переписать. От сшивания соседних ячеек таблицы
    // бережёт запрет черты и перевода строки, а не короткая мера.
    regex::Regex::new(r"\[([^\]\n|]{1,400})\]\(([^)\s]*\.md[^)\s]*)\)").expect("образец ссылки")
});

/// Переписать ЦЕЛЬ ссылки: путь файла заменить на `вид:имя`.
///
/// Отличается от `rewrite_links` предметом. Тот идёт от ЯРЛЫКА и потому берёт
/// только ссылки, чей ярлык несёт имя (`[ADR-0157](…)`); этот идёт от ЦЕЛИ и
/// берёт всё, что разбор уже разрешил в документ. Ярлык не трогается вовсе.
///
/// Второго правила адресации здесь нет: пара «вид · имя» берётся из
/// `project_document_links`, куда её положила запись — та же, что кладёт разбор.
///
/// Зачем: пока цель — путь файла, она разрешается ОТНОСИТЕЛЬНО пути документа.
/// Снять `path` и оставить такие ссылки значит получить набор, где 2083 связи
/// молча перестанут находить документ при первом же перечитывании.
pub async fn retarget_links(
    pool: &Pool,
    project: &str,
    dry: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let known = client
        .query(
            "SELECT DISTINCT entity_kind, entity_name, target_path, target_kind, target_name
               FROM project_document_links
              WHERE project_id = $1 AND target_kind <> '' AND target_path NOT LIKE '%:%'",
            &[&project],
        )
        .await?;
    let mut by_doc: std::collections::HashMap<(String, String), std::collections::HashMap<String, String>> =
        Default::default();
    for r in &known {
        let (k, n, target, tk, tn): (String, String, String, String, String) =
            (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4));
        by_doc.entry((k, n)).or_default().insert(target, format!("{tk}:{tn}"));
    }

    let docs = client
        .query(
            "SELECT entity_kind, entity_name, content, revision FROM project_documents
              WHERE project_id = $1 ORDER BY entity_kind, entity_name",
            &[&project],
        )
        .await?;
    let mut changed: Vec<(String, String, String, i64)> = Vec::new();
    let (mut retargeted, mut left) = (0usize, 0usize);
    let mut examples: Vec<Value> = Vec::new();
    for d in &docs {
        let (kind, name, content, revision): (String, String, String, i64) =
            (d.get(0), d.get(1), d.get(2), d.get(3));
        // Документ без единой разрешимой цели всё равно проходит осмотр: иначе
        // его нерешённые ссылки не попадут в счёт, и «осталось путём: 0» будет
        // значить «не считали», а не «не осталось».
        let empty = std::collections::HashMap::new();
        let map = by_doc.get(&(kind.clone(), name.clone())).unwrap_or(&empty);
        let mut out = String::with_capacity(content.len());
        let mut last = 0usize;
        let mut touched = 0usize;
        for m in LINK.captures_iter(&content) {
            let whole = m.get(0).expect("совпадение целиком");
            let raw = &m[2];
            // Якорь остаётся якорем: он адресует раздел внутри документа и к
            // файловой эпохе отношения не имеет.
            let (target, anchor) = raw.split_once('#').map(|(t, a)| (t, Some(a))).unwrap_or((raw, None));
            let Some(entity) = map.get(target) else {
                left += 1;
                if examples.len() < 8 {
                    examples.push(json!({ "entity": format!("{kind} {name}"), "target": raw,
                                          "why": "цель не разрешилась в документ" }));
                }
                continue;
            };
            out.push_str(&content[last..whole.start()]);
            let label = &m[1];
            match anchor {
                Some(a) => out.push_str(&format!("[{label}]({entity}#{a})")),
                None => out.push_str(&format!("[{label}]({entity})")),
            }
            last = whole.end();
            touched += 1;
            retargeted += 1;
        }
        if touched > 0 {
            out.push_str(&content[last..]);
            changed.push((kind, name, out, revision));
        }
    }

    let mut written = 0usize;
    let mut conflicts: Vec<String> = Vec::new();
    if !dry {
        let now = now_ms();
        for (kind, name, text, revision) in &changed {
            match crate::store::put(pool, project, kind, name, text, "links-retarget", Some(*revision), now).await {
                Ok(v) if v.get("status").and_then(|s| s.as_str()) == Some("written") => written += 1,
                Ok(v) => conflicts.push(format!("{kind} {name}: {}",
                                                v.get("status").and_then(|s| s.as_str()).unwrap_or("?"))),
                Err(e) => conflicts.push(format!("{kind} {name}: {e}")),
            }
        }
    }
    Ok(json!({
        "dry": dry, "documents": changed.len(), "retargeted": retargeted,
        "written": written, "conflicts": conflicts.len(),
        "conflictEntities": conflicts.iter().take(5).collect::<Vec<_>>(),
        "leftAsPath": left, "examples": examples,
        "why": "цель ссылки становится сущностью; путь файла уходит из текста набора"
    }))
}

pub async fn rewrite_links(
    pool: &Pool,
    kinds: &crate::kinds::Kinds,
    project: &str,
    dry: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Имя → вид: только из предметных таблиц. Ничего не выводится из формы
    // имени — вид берётся оттуда, где сущность объявлена.
    let mut known: std::collections::HashMap<String, String> = Default::default();
    for (sql, kind) in [
        ("SELECT id FROM project_requirements WHERE project_id=$1", "requirement"),
        ("SELECT id FROM project_checks WHERE project_id=$1", "check"),
        ("SELECT id FROM project_needs WHERE project_id=$1", "need"),
        ("SELECT id FROM project_stories WHERE project_id=$1", "story"),
        ("SELECT id FROM project_screens WHERE project_id=$1", "screen"),
        ("SELECT id FROM project_decisions WHERE project_id=$1", "decision"),
        ("SELECT id FROM project_questions WHERE project_id=$1", "question"),
        ("SELECT id FROM project_plan_tasks WHERE project_id=$1 AND kind<>'red'", "task"),
        ("SELECT id FROM project_plan_tasks WHERE project_id=$1 AND kind='red'", "red-task"),
        ("SELECT id FROM project_features WHERE project_id=$1", "feature"),
        ("SELECT id FROM project_terms WHERE project_id=$1", "term"),
    ] {
        for r in client.query(sql, &[&project]).await? {
            known.entry(r.get::<_, String>(0)).or_insert_with(|| kind.to_owned());
        }
    }

    static NAME: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"^\s*`?([A-Z]{1,4}-[A-Z0-9]+(?:-\d+[a-z]?)?|R-[MV]\d+-T[0-9a-z]+|[MV]\d+-T[0-9a-z]+|Q-\d+|ST-\d+|ADR-\d+)`?")
            .expect("образец имени в ярлыке")
    });

    let docs = client
        .query(
            "SELECT entity_kind, entity_name, content, revision FROM project_documents
              WHERE project_id = $1 ORDER BY entity_kind, entity_name",
            &[&project],
        )
        .await?;

    let (mut rewritten, mut unnamed, mut unknown_name) = (0usize, 0usize, 0usize);
    // Переписанное складывается под ИМЕНЕМ сущности: писать его потом будет тот
    // же писатель, что и человека, а он адреса не принимает.
    let mut changed: Vec<(String, String, String, i64)> = Vec::new();
    let mut examples: Vec<Value> = Vec::new();
    for d in &docs {
        let (own_kind, own_name): (String, String) = (d.get(0), d.get(1));
        let (content, revision): (String, i64) = (d.get(2), d.get(3));
        let mut out = String::with_capacity(content.len());
        let mut last = 0usize;
        let mut touched = 0usize;
        for m in LINK.captures_iter(&content) {
            let whole = m.get(0).expect("совпадение целиком");
            let label = m[1].to_owned();
            // Правило здесь одно: ЯРЛЫК несёт имя сущности. Второе — «ярлык есть
            // имя файла» — снято вместе с путём: имена файлов набор больше не
            // держит, а то, ради чего оно было, делает `links-retarget`, идущий
            // от разобранной ЦЕЛИ, а не от вида ярлыка.
            let Some(name) = NAME.captures(&label).map(|c| c[1].to_owned()) else {
                unnamed += 1;
                if examples.len() < 8 {
                    examples.push(json!({ "why": "ярлык имени не несёт", "label": label }));
                }
                continue;
            };
            if false {
                unnamed += 1;
                continue;
            }
            let Some(kind) = known.get(&name) else {
                unknown_name += 1;
                if examples.len() < 8 {
                    examples.push(json!({ "why": "имя ярлыка не разрешается в сущность", "label": label }));
                }
                continue;
            };
            out.push_str(&content[last..whole.start()]);
            out.push_str(&format!("[{label}]({kind}:{name})"));
            last = whole.end();
            touched += 1;
            rewritten += 1;
        }
        if touched > 0 {
            out.push_str(&content[last..]);
            changed.push((own_kind, own_name, out, revision));
        }
    }

    // Запись — только по явной просьбе, и каждая через тот же путь, что у
    // человека: с ожидаемой ревизией. Разошлась ревизия — документ пропущен и
    // назван, а не переписан поверх чужой правки.
    let mut written = 0usize;
    let mut conflicts: Vec<String> = Vec::new();
    if !dry {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for (kind, name, text, revision) in &changed {
            let who = if name.is_empty() { kind.clone() } else { format!("{kind} {name}") };
            match crate::store::put(pool, project, kind, name, text, "links-rewrite", Some(*revision), now).await {
                Ok(v) if v.get("status").and_then(|s| s.as_str()) == Some("written") => written += 1,
                Ok(v) => conflicts.push(format!("{who}: {}", v.get("status").and_then(|s| s.as_str()).unwrap_or("?"))),
                Err(e) => conflicts.push(format!("{who}: {e}")),
            }
        }
    }

    Ok(json!({
        "dry": dry,
        "written": written,
        "conflicts": conflicts.len(),
        "conflictEntities": conflicts.iter().take(5).collect::<Vec<_>>(),
        "rewritten": rewritten,
        "documents": changed.len(),
        "labelWithoutName": unnamed,
        "nameNotResolved": unknown_name,
        "examples": examples,
        "why": "замена идёт от ярлыка; ссылка без имени в ярлыке не трогается и названа",
        "pending": if dry {
            changed.iter().map(|(k, n, _, r)| json!({
                "entity": if n.is_empty() { k.clone() } else { format!("{k} {n}") }, "revision": r
            })).collect::<Vec<_>>()
        } else { vec![] },
    }))
}

/// Объявить пункт гейта.
///
/// Гейты — таблица ОБЪЯВЛЕННОГО, её не пересобирает ни один проход; до сих пор
/// её наполняла команда донора, и потому у `G5` не было ни одного пункта, а
/// «проект закончен» оставалось мнением. Дверь та же, что у всего остального:
/// ручка, а не прямой запрос к базе.
#[allow(clippy::too_many_arguments)]
pub async fn set_gate_item(
    pool: &Pool,
    project: &str,
    phase: &str,
    id: &str,
    title: &str,
    kind: &str,
    query: Option<&str>,
    owner: Option<&str>,
    probe: Option<&str>,
    why: &str,
    drop_it: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Снятие пункта — той же ручкой. Без него пункт, оказавшийся неверным,
    // снимался бы только запросом в базу мимо сервера; замеры снятого пункта
    // уходят вместе с ним, иначе гейт продолжал бы считать его непройденным.
    if drop_it {
        let gone = client
            .execute("DELETE FROM gate_item WHERE phase = $1 AND id = $2", &[&phase, &id])
            .await?;
        client
            .execute("DELETE FROM project_gates WHERE phase = $1 AND id = $2", &[&phase, &id])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                          "phase": phase, "id": id }));
    }
    // ПРОБА ОБЯЗАНА БЫТЬ ЗАПРОСОМ. Дверь принимала прозу — «убрать колонку»,
    // «назвать сценарий», «завести задачу», — и такой пункт не роняли ни разу:
    // самотест отвечал «проба не исполнилась: syntax error at or near "убрать"».
    // На myack так стояли СЕМЬДЕСЯТ СЕМЬ проб из ста десяти, и зелёное у них не
    // значило ничего — ровно то, против чего этот харнес и написан.
    //
    // Проверяется подготовкой запроса, а не исполнением: подготовка ловит и
    // разбор, и несуществующую таблицу, и не пишет ни строки.
    let mut probe_runs: Option<bool> = None;
    if let Some(p) = probe {
        let text = p.trim();
        if !text.is_empty() {
            probe_runs = Some(true);
            if let Err(e) = client.prepare(text).await {
                return Ok(json!({
                    "status": "probe_not_a_query",
                    "why": format!(
                        "проба не разбирается как запрос, и таким пунктом нельзя уронить правило: {}. \
                         Проба — это ЗАПРОС, подсаживающий нарушение, а не описание того, что надо сделать.",
                        db_says(&e)
                    ),
                    "probe": text,
                }));
            }
        }
    }
    // Не переданное берётся у существующей строки: объявить пробу, не повторяя
    // запрос, — обычное дело, а вставляемая строка проверяется целиком, и
    // `query IS NULL` при `kind='query'` не проходит по правилу таблицы.
    // Заголовок не переданный — берётся у существующей строки: правка запроса
    // не должна требовать повторять текст, а пустой заголовок сделал бы пункт
    // безымянным в глазах человека.
    let was = client
        .query(
            "SELECT query, owner, probe, item FROM gate_item WHERE phase = $1 AND id = $2",
            &[&phase, &id],
        )
        .await?;
    let had: (Option<String>, Option<String>, String, String) = match was.first() {
        Some(r) => (r.get(0), r.get(1), r.get(2), r.get(3)),
        None => (None, None, String::new(), String::new()),
    };
    let title = if title.trim().is_empty() { had.3.clone() } else { title.to_owned() };
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless",
                          "why": "пункт гейта без имени не заводится: имя адресует пункт, заголовок его объясняет" }));
    }
    let query = query.map(|q| q.to_owned()).or(had.0);
    let owner = owner.map(|o| o.to_owned()).or(had.1);
    let probe = probe.map(|p| p.to_owned()).unwrap_or(had.2);
    let n = client
        .execute(
            "INSERT INTO gate_item (phase, id, item, kind, query, owner, probe, probe_ok)
             VALUES ($1, $2, $7, $3, $4, $5, $6, $8)
             ON CONFLICT (phase, id) WHERE id <> ''
               DO UPDATE SET item = EXCLUDED.item, kind = EXCLUDED.kind, query = EXCLUDED.query,
                             owner = EXCLUDED.owner, probe = EXCLUDED.probe,
                             probe_ok = EXCLUDED.probe_ok",
            &[&phase, &id, &kind, &query, &owner, &probe, &title, &probe_runs],
        )
        .await?;
    if !why.is_empty() {
        client
            .execute("UPDATE gate_item SET why = $3 WHERE phase = $1 AND id = $2",
                     &[&phase, &id, &why])
            .await?;
    }
    Ok(json!({ "phase": phase, "id": id, "item": title, "kind": kind, "written": n, "why": why }))
}

/// Виды, чьи пункты готовности переносятся. Список назван здесь один раз и
/// используется и переносом, и отказом: иначе «пунктов нет» и «спрашивать
/// нечем» перестанут различаться.
pub const READINESS_OWNERS: &[&str] =
    &["task", "red-task", "question", "acceptance", "test-plan", "document-plan"];

/// Вычислить состояние пункта по объявленному способу.
///
/// Пункт без способа остаётся `unknown` и `done` не становится ни от какого
/// сопоставления слов.
pub async fn readiness_computed(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
) -> Result<Value, tokio_postgres::Error> {
    if !READINESS_OWNERS.contains(&kind) {
        // Чтение документа не заменяет проекции его строк. Пустой список здесь
        // сказал бы «пунктов нет», тогда как их никто не считал.
        return Ok(json!({
            "owner": { "kind": kind, "id": id },
            "unprojected": true,
            "why": format!("о строках вида {kind} спросить нечем: пункты готовности у него не проецируются"),
        }));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT ord, text, declared, method_kind, method FROM readiness_item
              WHERE project_id = $1 AND owner_kind = $2 AND owner_id = $3 ORDER BY ord",
            &[&project, &kind, &id],
        )
        .await?;
    let mut items = Vec::new();
    for r in &rows {
        let method_kind: String = r.get(3);
        let method: String = r.get(4);
        let declared: Option<bool> = r.get(2);
        let v = execute_method(&client, project, &method_kind, &method).await;
        let computed = match v.state {
            "passed" => Some(true),
            "failed" => Some(false),
            _ => None,
        };
        items.push(json!({
            "ord": r.get::<_, i32>(0), "text": r.get::<_, String>(1),
            "declared": declared, "methodKind": method_kind,
            "computed": computed,
            "state": match computed {
                Some(true) => "done",
                Some(false) => "failed",
                None => "unknown",
            },
            // Самое опасное: галочка стоит, а проверка говорит обратное.
            "declaredButNot": declared == Some(true) && computed == Some(false),
        }));
    }
    let computable = items.iter().filter(|i| i["state"] != "unknown").count();
    Ok(json!({
        "owner": { "kind": kind, "id": id },
        "items": items,
        "total": items.len(),
        "computable": computable,
        "unmeasurable": items.len() - computable,
        "declaredButNot": items.iter().filter(|i| i["declaredButNot"] == json!(true)).count(),
    }))
}

/// Вердикт способа проверки. Один на всех, кто спрашивает.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub state: &'static str,
    pub violations: usize,
    pub detail: Vec<String>,
    pub why: String,
}

impl Verdict {
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "state": self.state, "violations": self.violations, "detail": self.detail });
        if !self.why.is_empty() {
            v["why"] = json!(self.why);
        }
        v
    }
}

/// Имя сущности из строки нарушения: первое слово до пробела.
///
/// Запросы гейтов складывают строку вида «`US-DET-01` — заголовок» или
/// «`US-FEED-04` → `SCR-FEED-02`». Первое слово — тот, о ком нарушение, и
/// исключение объявляется на него. Разбирать глубже нечего: если запросу
/// понадобится другая единица, он обязан поставить её первой.
pub fn violator(detail: &str) -> String {
    detail
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| c == '`' || c == '"')
        .to_owned()
}

/// **Единственное место, которое исполняет способ проверки.**
///
/// Пункт гейта, пункт готовности и ступень лестницы — одна форма: предмет,
/// способ, вердикт. Мы построили её дважды; третья реализация разошлась бы с
/// первыми двумя молча — ровно как разошлись две таблицы связей
/// задача→требование, пока одну не удалили. Поэтому исполнитель один, а
/// вызывающих трое.
///
/// `command` сервер не выполняет и выполнять не должен: у него нет ни
/// репозитория, ни оболочки. Команду прогоняет тот, у кого они есть, и подаёт
/// итог — как состояния задач.
pub async fn execute_method(
    client: &deadpool_postgres::Client,
    project: &str,
    method_kind: &str,
    method: &str,
) -> Verdict {
    execute_method_upto(client, project, method_kind, method, 5).await
}

/// То же, но со своим числом примеров.
///
/// Пятёрка хороша там, где ответ читают мельком: подсказка следующего шага,
/// строка готовности. Гейту мало: его пункт открывают затем, чтобы починить, а
/// чинить по пяти именам из тридцати восьми нельзя. Замер гейта хранится, и
/// хранить в нём двести имён вместо пяти ничего не стоит.
pub async fn execute_method_upto(
    client: &deadpool_postgres::Client,
    project: &str,
    method_kind: &str,
    method: &str,
    limit: usize,
) -> Verdict {
    match method_kind {
        "query" if !method.trim().is_empty() => match client.query(method, &[&project]).await {
            Ok(found) => Verdict {
                state: if found.is_empty() { "passed" } else { "failed" },
                violations: found.len(),
                // Пятёрка примеров УПОРЯДОЧЕНА, и это не украшение. Объявленный
                // запрос своего порядка не обязан иметь, а Postgres отдаёт
                // строки в том, в каком они легли; после любой перезаписи
                // таблицы тот же набор называет других пятерых. Два одинаковых
                // прогона расходились примерами — и по такому расхождению
                // нельзя отличить «набор изменился» от «строки переехали».
                detail: {
                    let mut d: Vec<String> =
                        found.iter().map(|r| r.try_get::<_, String>(0).unwrap_or_default()).collect();
                    d.sort();
                    d.truncate(limit);
                    d
                },
                why: String::new(),
            },
            Err(e) => Verdict {
                state: "unknown",
                violations: 0,
                detail: vec![],
                why: format!("запрос не выполнился: {e}"),
            },
        },
        "query" => Verdict {
            state: "unknown",
            violations: 0,
            detail: vec![],
            why: "у способа вида «запрос» запроса нет".into(),
        },
        "command" => Verdict {
            state: "unknown",
            violations: 0,
            detail: vec![method.to_owned()],
            why: "команду выполняет харнес: у сервера нет ни репозитория, ни оболочки".into(),
        },
        "signed" => Verdict {
            state: "unsigned",
            violations: 0,
            detail: vec![],
            why: "подписи нет: не закрыто, а не пройдено молча".into(),
        },
        _ => Verdict {
            state: "unknown",
            violations: 0,
            detail: vec![],
            why: "способ не объявлен: ответить нечем, и это не «выполнено»".into(),
        },
    }
}

/// Диспетчер: первая невыполненная ступень вместе с владельцем.
///
/// Три списка, а не один. Пройденное, пропущенное **с причиной** и
/// неотвечаемое — разные вещи; слипшись, они дают ту самую зелень, которая
/// ничего не мерит.
async fn compute_next_step(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
    record: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let steps = client
        .query(
            "SELECT ord, question, method_kind, method, when_query, when_why, owner_kind, owner, touches
               FROM harness_process_step
              WHERE set_name = $1 AND process = $2 ORDER BY ord",
            &[&set_name, &process],
        )
        .await?;
    if steps.is_empty() {
        return Ok(json!({
            "process": process,
            "at": Value::Null,
            "why": format!("процесса {process} в наборе {set_name} нет ни одной ступенью: отвечать нечем"),
        }));
    }

    // Фаза набора открыта, пока не пройдены все ступени стороны `corpus`.
    // Репозиторную ступень при открытой фазе диспетчер не выдаёт: правило
    // проекта — сперва набор, потом репозиторий.
    let total_steps = steps.len();
    let mut passed = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut unanswerable = Vec::new();
    let mut at: Option<Value> = None;
    let mut corpus_open = false;
    let mut journal: Vec<(i32, String, String)> = Vec::new();

    for row in &steps {
        let ord: i32 = row.get(0);
        let question: String = row.get(1);
        let method_kind: String = row.get(2);
        let method: String = row.get(3);
        let when_query: String = row.get(4);
        let when_why: String = row.get(5);
        let owner_kind: String = row.get(6);
        let owner: String = row.get(7);
        let touches: String = row.get(8);

        // Условие ступени проверяется первым: пропущенная ступень не «пройдена».
        if !when_query.trim().is_empty() {
            let in_play = match client.query(when_query.as_str(), &[&project]).await {
                Ok(found) => !found.is_empty(),
                Err(_) => true,
            };
            if !in_play {
                skipped.push(json!({ "ord": ord, "why": when_why }));
                journal.push((ord, "skipped".into(), when_why.clone()));
                continue;
            }
        }

        let verdict = execute_method(&client, project, &method_kind, &method).await;
        journal.push((ord, verdict.state.to_owned(), verdict.detail.join(" · ")));

        match verdict.state {
            "passed" => {
                passed.push(ord);
                continue;
            }
            "unknown" | "unsigned" | "stale" => {
                unanswerable.push(json!({ "ord": ord, "why": verdict.why, "question": question }));
                if touches == "corpus" {
                    corpus_open = true;
                }
            }
            _ => {
                if touches == "corpus" {
                    corpus_open = true;
                }
            }
        }

        if at.is_none() {
            // Репозиторная ступень при открытой фазе набора не выдаётся, и
            // причина называется: иначе отказ читается как «нечего делать».
            if touches == "repository" && corpus_open {
                at = Some(json!({
                    "ord": ord, "question": question, "state": "held",
                    "ownerKind": owner_kind, "owner": owner, "touches": touches,
                    "why": "ступень пишет в репозиторий, а фаза набора ещё открыта",
                }));
            } else {
                at = Some(json!({
                    "ord": ord, "question": question, "state": verdict.state,
                    "ownerKind": owner_kind, "owner": owner, "touches": touches,
                    "why": verdict.why, "violations": verdict.violations, "detail": verdict.detail,
                }));
            }
        }
    }

    if record {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for (ord, state, detail) in &journal {
            client
                .execute(
                    "INSERT INTO process_run (project_id, process, at, ord, state, detail)
                     VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
                    &[&project, &process, &now, ord, state, detail],
                )
                .await?;
        }
    }

    Ok(json!({
        "process": process,
        "at": at,
        "passed": passed,
        "skipped": skipped,
        "unanswerable": unanswerable,
        "corpusPhaseOpen": corpus_open,
        // Номер ступени — НЕ мера пройденного, и читать его так значит ошибиться.
        // «Где мы» отвечает на «что чинить первым»: лестница останавливается на
        // самой ранней невыполненной. Проект с чистым набором и ненаписанным
        // кодом стоит на девятой, проект с написанным кодом и одной корпусной
        // придиркой — на шестой, и второй при этом дальше по продукту.
        //
        // Поэтому рядом стоит счёт: сколько ступеней выполнено из скольких.
        // Два числа отвечают на два разных вопроса, и ни одно не выдаёт себя за
        // другое.
        "progress": json!({
            "met": passed.len(),
            "of": total_steps,
            "skipped": skipped.len(),
            "why": "номер ступени — самая ранняя невыполненная, а не мера пройденного",
        }),
    }))
}

/// Ступень, которая скачет: один и тот же номер отвечал по-разному.
/// Измерить положение на лестнице и СОХРАНИТЬ измеренное.
///
/// Зовётся не на вопрос, а на изменение набора — тем же работником, что меряет
/// гейты, и строго после них: ступени 4, 7 и 10 читают состояние пунктов гейта,
/// и посчитанные до — прочли бы прошлый круг.
pub async fn measure_process(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
) -> Result<Value, tokio_postgres::Error> {
    let answer = compute_next_step(pool, project, set_name, process, true).await?;
    let at = answer.get("at").cloned().unwrap_or(Value::Null);
    let text = |key: &str| -> String {
        at.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned()
    };
    let ord: Option<i32> = at.get("ord").and_then(|v| v.as_i64()).map(|n| n as i32);
    let passed = answer["passed"].as_array().map(|a| a.len()).unwrap_or(0) as i32;
    let skipped = answer["skipped"].as_array().map(|a| a.len()).unwrap_or(0) as i32;
    let unanswerable = answer["unanswerable"].as_array().map(|a| a.len()).unwrap_or(0) as i32;
    let open = answer["corpusPhaseOpen"].as_bool().unwrap_or(true);
    let now = now_ms();
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute(
            "INSERT INTO process_position (project_id, process, at_ord, at_state, at_question,
                        at_owner, at_owner_kind, at_touches, passed, skipped, unanswerable,
                        corpus_phase_open, result, checked_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
             ON CONFLICT (project_id, process) DO UPDATE SET
               at_ord = EXCLUDED.at_ord, at_state = EXCLUDED.at_state,
               at_question = EXCLUDED.at_question, at_owner = EXCLUDED.at_owner,
               at_owner_kind = EXCLUDED.at_owner_kind, at_touches = EXCLUDED.at_touches,
               passed = EXCLUDED.passed, skipped = EXCLUDED.skipped,
               unanswerable = EXCLUDED.unanswerable, corpus_phase_open = EXCLUDED.corpus_phase_open,
               result = EXCLUDED.result, checked_at = EXCLUDED.checked_at",
            &[&project, &process, &ord, &text("state"), &text("question"), &text("owner"),
              &text("ownerKind"), &text("touches"), &passed, &skipped, &unanswerable,
              &open, &answer, &now],
        )
        .await?;
    Ok(json!({ "at": ord, "passed": passed, "skipped": skipped, "at_": now }))
}

/// Где мы на лестнице — из сохранённого.
///
/// Ответ несёт время замера и признак `stale`: набор мог измениться секунду
/// назад, и работник ещё считает. Агент, попавший в этот промежуток, обязан
/// узнать об этом словом, а не получить прошлое положение как нынешнее.
pub async fn next_step(pool: &Pool, project: &str, process: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let row = client
        .query_opt(
            "SELECT p.result, p.checked_at,
                    coalesce((SELECT d.dirty_at > coalesce(p.checked_at, 0) FROM gate_dirty d
                               WHERE d.project_id = p.project_id), false)
               FROM process_position p WHERE p.project_id = $1 AND p.process = $2",
            &[&project, &process],
        )
        .await?;
    let Some(row) = row else {
        return Ok(json!({
            "process": process, "at": Value::Null, "checkedAt": Value::Null, "stale": true,
            "why": "положение ни разу не считали: пересчёт с заведения процесса не запускался",
        }));
    };
    let mut answer: Value = row.get::<_, Option<Value>>(0).unwrap_or(json!({}));
    if let Some(o) = answer.as_object_mut() {
        o.insert("checkedAt".into(), json!(row.get::<_, Option<i64>>(1)));
        o.insert("stale".into(), json!(row.get::<_, bool>(2)));
    }
    Ok(answer)
}

pub async fn process_history(pool: &Pool, project: &str, process: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT ord, count(DISTINCT state) AS states, count(*) AS runs,
                    string_agg(DISTINCT state, ' · ' ORDER BY state) AS seen
               FROM process_run WHERE project_id = $1 AND process = $2
              GROUP BY ord ORDER BY ord",
            &[&project, &process],
        )
        .await?;
    Ok(json!({
        "steps": rows.iter().map(|r| json!({
            "ord": r.get::<_, i32>(0),
            "distinctStates": r.get::<_, i64>(1),
            "runs": r.get::<_, i64>(2),
            "seen": r.get::<_, Option<String>>(3),
            // Скачущая ступень — та, что отвечала по-разному на одних данных.
            "flapping": r.get::<_, i64>(1) > 1,
        })).collect::<Vec<_>>(),
    }))
}

/// Плитки прогресса: три числа, никогда одно.
pub async fn progress(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT tile, done, open, unknown FROM corpus_progress
              WHERE project_id = $1 OR project_id = '__ladder__' ORDER BY tile",
            &[&project],
        )
        .await?;
    Ok(json!({
        "tiles": rows.iter().map(|r| {
            let done: i64 = r.get(1);
            let open: i64 = r.get(2);
            let unknown: i64 = r.get(3);
            let answerable = done + open;
            json!({
                "tile": r.get::<_, String>(0),
                "done": done, "open": open, "unknown": unknown,
                // Процент — от ОТВЕЧАЕМОГО. Нет отвечаемого — нет процента:
                // «0 %» здесь было бы враньём той же породы, что «100 %».
                // Округление, а не усечение: 99 из 227 — это 44 %, и показать
                // 43 значит соврать в свою пользу на целый процент.
                "percent": if answerable > 0 { json!((done * 200 + answerable) / (answerable * 2)) } else { Value::Null },
                "says": if answerable > 0 {
                    format!("{} из {} отвечаемых", done, answerable)
                } else {
                    "пока не измеряется".to_owned()
                },
            })
        }).collect::<Vec<_>>(),
    }))
}

/// Конвейер задачи: статус достигается ФАКТОМ, а не пометкой.
///
/// Каждый статус считается независимо; «текущий» — самый дальний из подряд
/// достигнутых. Разрыв в середине не проглатывается: задача, имплементированная
/// без предполёта, — это работа, прошедшая мимо проверки, и её надо уметь
/// найти, а не сгладить.
pub async fn task_status(pool: &Pool, project: &str, task: Option<&str>) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let statuses = client
        .query(
            "SELECT ord, name, title, fact, terminal, source, why FROM kind_status WHERE kind = 'task' ORDER BY ord",
            &[],
        )
        .await?;
    // Источник проверяется один раз на статус, а не на каждую задачу.
    let mut recorded = std::collections::HashMap::new();
    for s in &statuses {
        let name: String = s.get(1);
        let source: String = s.get(5);
        let ok = if source.trim().is_empty() {
            true
        } else {
            client.query(source.as_str(), &[&project]).await.map(|r| !r.is_empty()).unwrap_or(false)
        };
        recorded.insert(name, ok);
    }
    let tasks = client
        .query(
            "SELECT id FROM project_plan_tasks WHERE project_id = $1 AND ($2 = '' OR id = $2) ORDER BY id",
            &[&project, &task.unwrap_or("")],
        )
        .await?;
    if tasks.is_empty() {
        return Ok(json!({ "task": task, "known": false, "why": "такой задачи в наборе нет" }));
    }

    let mut out = Vec::new();
    for t in &tasks {
        let id: String = t.get(0);
        let mut reached = Vec::new();
        let mut current: Option<String> = None;
        let mut broken = false;
        let mut unknown = Vec::new();
        for s in &statuses {
            let name: String = s.get(1);
            let fact: String = s.get(3);
            let why: String = s.get(6);
            let state = if fact.trim().is_empty() || !recorded.get(&name).copied().unwrap_or(true) {
                // Факта нет — статус НЕИЗВЕСТЕН. Не «не достигнут»: это разные
                // утверждения, и второе врёт в пользу «не начата». Причина, если
                // она объявлена, идёт рядом: «отложено» и «забыли» — разное.
                unknown.push(if why.is_empty() {
                    json!({ "status": name.clone(), "why": if fact.trim().is_empty() {
                        "факта нет: статус ничем не достигается" } else {
                        "факт объявлен, но его никто не подаёт" } })
                } else {
                    json!({ "status": name.clone(), "why": why })
                });
                "unknown".to_owned()
            } else {
                match client.query(fact.as_str(), &[&project, &id]).await {
                    Ok(found) if !found.is_empty() => "reached".to_owned(),
                    Ok(_) => "not-reached".to_owned(),
                    Err(e) => {
                        unknown.push(json!({ "status": name.clone(), "why": format!("запрос факта не выполнился: {e}") }));
                        "unknown".to_owned()
                    }
                }
            };
            if state == "reached" {
                reached.push(name.clone());
                if !broken {
                    current = Some(name.clone());
                }
            } else if state == "not-reached" {
                broken = true;
            }
        }
        // Достигнутое после разрыва — аномалия: дальний статус есть, ближний нет.
        let anomaly = reached.len() > current.iter().count() && broken;
        out.push(json!({
            "task": id, "current": current, "reached": reached,
            "unknown": unknown, "anomaly": anomaly,
        }));
    }
    if task.is_some() {
        return Ok(out.into_iter().next().unwrap_or(Value::Null));
    }
    Ok(json!({ "tasks": out, "count": out.len() }))
}

/// Задачи, прошедшие мимо середины конвейера.
pub async fn status_anomaly(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let all = task_status(pool, project, None).await?;
    let list: Vec<Value> = all["tasks"]
        .as_array()
        .map(|a| a.iter().filter(|t| t["anomaly"] == json!(true)).cloned().collect())
        .unwrap_or_default();
    Ok(json!({ "count": list.len(), "anomalies": list }))
}

/// Плитка конвейера: сколько задач на каждом статусе, и сколько неизвестно.
pub async fn task_pipeline(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let statuses = client
        .query("SELECT ord, name, title, fact, source FROM kind_status WHERE kind = 'task' ORDER BY ord", &[])
        .await?;
    let total: i64 = client
        .query_one("SELECT count(*) FROM project_plan_tasks WHERE project_id = $1", &[&project])
        .await?
        .get(0);
    let mut tiles = Vec::new();
    for s in &statuses {
        let name: String = s.get(1);
        let fact: String = s.get(3);
        let source: String = s.get(4);
        let recorded = source.trim().is_empty()
            || client.query(source.as_str(), &[&project]).await.map(|r| !r.is_empty()).unwrap_or(false);
        if fact.trim().is_empty() || !recorded {
            // Статус, чей факт не пишется, идёт в `unknown`, а не в «сделано».
            // Сегодня та же неизвестность выглядит как «не начата» — в этом и
            // была цена.
            tiles.push(json!({ "status": name, "title": s.get::<_, String>(2),
                               "reached": Value::Null, "unknown": total,
                               "why": "факт этого статуса никто не записывает" }));
            continue;
        }
        let n: i64 = client
            .query_one(
                &format!("SELECT count(*) FROM project_plan_tasks t WHERE t.project_id = $1 AND EXISTS ({})",
                         fact.replace("$2", "t.id")),
                &[&project],
            )
            .await
            .map(|r| r.get(0))
            .unwrap_or(0);
        tiles.push(json!({ "status": name, "title": s.get::<_, String>(2),
                           "reached": n, "unknown": 0 }));
    }
    Ok(json!({ "total": total, "pipeline": tiles }))
}

/// Волны: что можно вести одновременно.
///
/// Волна — глубина задачи в графе зависимостей: первая волна ничего не ждёт,
/// вторая ждёт только первую. Считается здесь, а не в браузере, по той же
/// причине, что и всё выводимое: вывод не хранится и не пересчитывается дважды
/// разными руками.
///
/// Зависимость от **этапа** раскрывается в рёбра только при счёте — в базе она
/// остаётся связью с этапом, иначе протухнет, когда в этап добавят задачу.
pub async fn waves(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let tasks = client
        .query(
            "SELECT t.id, t.milestone_id, t.title, t.kind, t.state, t.ord,
                    coalesce(r.checks, 0) AS checks,
                    (SELECT count(*) FROM task_requirement q
                      WHERE q.project_id = t.project_id AND q.task_id = t.id) AS requirements,
                    (SELECT v.verdict FROM preflight_verdict v
                      WHERE v.project_id = t.project_id AND v.task_id = t.id
                      ORDER BY v.at DESC LIMIT 1) AS preflight
               FROM project_plan_tasks t
               LEFT JOIN red_task r ON r.project_id = t.project_id AND r.id = t.id
              WHERE t.project_id = $1
              ORDER BY t.milestone_id, t.ord, t.id",
            &[&project],
        )
        .await?;
    let edges = client
        .query(
            "SELECT task_id, depends_on FROM project_plan_task_deps WHERE project_id = $1
             UNION ALL
             SELECT md.task_id, t.id FROM task_milestone_dep md
               JOIN project_plan_tasks t
                 ON t.project_id = md.project_id AND t.milestone_id = md.milestone_id
              WHERE md.project_id = $1 AND t.id <> md.task_id",
            &[&project],
        )
        .await?;

    let ids: Vec<String> = tasks.iter().map(|r| r.get::<_, String>(0)).collect();
    let index: std::collections::HashMap<&str, usize> =
        ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let mut depends: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    for e in &edges {
        let (from, to): (String, String) = (e.get(0), e.get(1));
        if let (Some(&a), Some(&b)) = (index.get(from.as_str()), index.get(to.as_str())) {
            depends[a].push(b);
        }
    }

    // Глубина считается послаблением: волна задачи на единицу больше самой
    // дальней из тех, что она ждёт. Круг в графе не углубляет — иначе счёт не
    // сошёлся бы вовсе, а задачи в круге и правда не начать ни одну.
    let mut wave = vec![1usize; ids.len()];
    for _ in 0..ids.len().min(64) {
        let mut moved = false;
        for i in 0..ids.len() {
            let deepest = depends[i].iter().map(|&j| wave[j]).max().unwrap_or(0);
            if deepest + 1 > wave[i] {
                wave[i] = deepest + 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let mut cards = Vec::new();
    for (i, r) in tasks.iter().enumerate() {
        let kind: String = r.get(3);
        cards.push(json!({
            "id": ids[i], "milestone": r.get::<_, String>(1), "title": r.get::<_, String>(2),
            "kind": kind, "state": r.get::<_, String>(4),
            "checks": r.get::<_, i32>(6), "requirements": r.get::<_, i64>(7),
            "preflight": r.get::<_, Option<String>>(8),
            "wave": wave[i],
            "waits": depends[i].iter().filter(|&&j| tasks[j].get::<_, String>(4) != "closed").count(),
        }));
    }
    let open_red = cards
        .iter()
        .filter(|c| c["kind"] == "red" && c["state"] != "closed")
        .count();
    Ok(json!({
        "cards": cards,
        "total": cards.len(),
        // Барьер красной фазы: весь трек проверок предшествует всему коду.
        "openRed": open_red,
    }))
}

/// Фазы с полной картиной: документы, гейт и задачи — каждой своим счётом.
///
/// До сих пор в плане были только вехи кода, и трек проверок в картину не
/// входил. Теперь фаза знает все три свои части, и **ни одна не складывается с
/// другой**: документы считаются по плану, гейт вычисляется своими запросами,
/// задачи — по состоянию из истории.
async fn compute_phases(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT id, ord, title, gate, plan_level, task_kind FROM phase ORDER BY ord",
            &[],
        )
        .await?;
    let gates = gate(pool, project, None).await?;
    let mut out = Vec::new();
    for r in &rows {
        let id: String = r.get(0);
        let gate_id: String = r.get(3);
        let level: String = r.get(4);
        let kind: String = r.get(5);

        // Документы фазы — из плана: есть · объявлено отсутствующим · не объявлено.
        let docs = client
            .query(
                // «Этот файл» считается написанным. `self` носит одна строка —
                // сама опись, и она существует по факту того, что её читают.
                // Прежде она попадала в знаменатель и не попадала в числитель, и
                // рамка выглядела написанной наполовину при полном комплекте.
                "SELECT count(*) FILTER (WHERE claim IN ('present','self')),
                        count(*) FILTER (WHERE claim = 'absent'),
                        count(*)
                   FROM project_document_plan
                  WHERE project_id = $1 AND level = ANY (string_to_array($2, '|'))",
                &[&project, &level],
            )
            .await?;
        let (present, absent, all): (i64, i64, i64) = docs
            .first()
            .map(|d| (d.get(0), d.get(1), d.get(2)))
            .unwrap_or((0, 0, 0));

        // Задачи фазы — по виду. Красные принадлежат Ф3, код — Ф4: так говорят
        // сами документы задач, а не наша догадка.
        let tasks = client
            .query(
                "SELECT count(*) FILTER (WHERE state = 'closed'), count(*)
                   FROM project_plan_tasks WHERE project_id = $1 AND ($2 = '' OR kind = $2)
                    AND ($2 <> '')",
                &[&project, &kind],
            )
            .await?;
        let (closed, total): (i64, i64) = tasks.first().map(|t| (t.get(0), t.get(1))).unwrap_or((0, 0));

        let g = gates["gates"]
            .as_array()
            .and_then(|a| a.iter().find(|g| g["gate"] == gate_id.as_str()))
            .cloned();

        out.push(json!({
            "phase": id, "title": r.get::<_, String>(2),
            "gate": if gate_id.is_empty() { Value::Null } else { json!(gate_id) },
            "gateState": g.as_ref().map(|g| g["computed"].clone()).unwrap_or(Value::Null),
            "documents": if level.is_empty() { Value::Null } else {
                json!({ "present": present, "absent": absent, "declared": all })
            },
            "tasks": if kind.is_empty() { Value::Null } else {
                json!({ "closed": closed, "open": total - closed, "total": total })
            },
        }));
    }
    Ok(json!({ "phases": out }))
}
/// Кому принадлежит документ и все ли виды заселены.
///
/// Вопрос «какие документы не достаются ни одним видом и какие достаются
/// двумя» отвечает теперь КЛЮЧ, а не обход. Пока документ опознавался файлом,
/// вид у него выводился раскладкой при каждом запросе, и обе беды были
/// возможны: путь, не попавший ни под один корень, и путь, попавший под два.
/// Вид и имя записаны в самой строке, а первичный ключ запрещает второй такой
/// паре существовать — поэтому оба числа структурно нулевые, и это сказано
/// вслух, а не показано нулём без объяснения.
///
/// Остаётся то, что ключом не проверяется: виды, объявленные раскладкой и
/// незаселённые, и виды, которых раскладка не объявляет вовсе.
/// Измерить фазы и СОХРАНИТЬ измеренное — числами в своих колонках.
///
/// Считается после гейтов: состояние гейта фазы берётся у них. Пустое число
/// документов или задач значит «фаза их не объявляет», и оно остаётся пустым:
/// ноль сказал бы «объявила, и нет ни одной».
pub async fn measure_phases(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let computed = compute_phases(pool, project).await?;
    let empty = Vec::new();
    let list = computed["phases"].as_array().unwrap_or(&empty);
    let now = now_ms();
    let client = pool.get().await.expect("пул отдал соединение");
    client
        .execute("DELETE FROM phase_state WHERE project_id = $1", &[&project])
        .await?;
    for (ord, p) in list.iter().enumerate() {
        let num = |path: &str, key: &str| -> Option<i32> {
            p[path][key].as_i64().map(|n| n as i32)
        };
        client
            .execute(
                "INSERT INTO phase_state (project_id, phase, ord, title, gate, gate_state,
                        documents_present, documents_absent, documents_declared,
                        tasks_closed, tasks_open, tasks_total, checked_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                &[&project,
                  &p["phase"].as_str().unwrap_or("").to_owned(),
                  &(ord as i32),
                  &p["title"].as_str().unwrap_or("").to_owned(),
                  &p["gate"].as_str().unwrap_or("").to_owned(),
                  &p["gateState"].as_str().unwrap_or("").to_owned(),
                  &num("documents", "present"), &num("documents", "absent"),
                  &num("documents", "declared"),
                  &num("tasks", "closed"), &num("tasks", "open"), &num("tasks", "total"),
                  &now],
            )
            .await?;
    }
    Ok(json!({ "phases": list.len(), "at": now }))
}

/// Фазы — из сохранённого.
pub async fn phases(pool: &Pool, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT phase, title, gate, gate_state, documents_present, documents_absent,
                    documents_declared, tasks_closed, tasks_open, tasks_total, checked_at
               FROM phase_state WHERE project_id = $1 ORDER BY ord",
            &[&project],
        )
        .await?;
    if rows.is_empty() {
        return Ok(json!({ "phases": [], "checkedAt": Value::Null,
                          "why": "фазы ни разу не считали: пересчёт не запускался" }));
    }
    let stale: bool = client
        .query_one(
            "SELECT coalesce((SELECT d.dirty_at > coalesce((SELECT min(checked_at) FROM phase_state
                                WHERE project_id = $1), 0)
                                FROM gate_dirty d WHERE d.project_id = $1), false)",
            &[&project],
        )
        .await?
        .get(0);
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let gate: String = r.get(2);
            let state: String = r.get(3);
            let opt = |i: usize| -> Value {
                r.get::<_, Option<i32>>(i).map(|n| json!(n)).unwrap_or(Value::Null)
            };
            json!({
                "phase": r.get::<_, String>(0),
                "title": r.get::<_, String>(1),
                "gate": if gate.is_empty() { Value::Null } else { json!(gate) },
                "gateState": if state.is_empty() { Value::Null } else { json!(state) },
                "documents": if r.get::<_, Option<i32>>(6).is_none() { Value::Null } else {
                    json!({ "present": opt(4), "absent": opt(5), "declared": opt(6) })
                },
                "tasks": if r.get::<_, Option<i32>>(9).is_none() { Value::Null } else {
                    json!({ "closed": opt(7), "open": opt(8), "total": opt(9) })
                },
            })
        })
        .collect();
    Ok(json!({ "phases": out, "checkedAt": rows[0].get::<_, Option<i64>>(10), "stale": stale }))
}

pub async fn coverage(pool: &Pool, kinds: &crate::kinds::Kinds, project: &str) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT entity_kind, count(*) FROM project_documents
              WHERE project_id = $1 GROUP BY 1 ORDER BY 1",
            &[&project],
        )
        .await?;
    let mut held: std::collections::BTreeMap<String, i64> = Default::default();
    for r in &rows {
        held.insert(r.get(0), r.get(1));
    }
    let undeclared: Vec<Value> = held
        .iter()
        .filter(|(k, _)| kinds.get(k).is_none())
        .map(|(k, n)| json!({ "kind": k, "documents": n }))
        .collect();
    let empty: Vec<&String> = kinds
        .0
        .iter()
        .filter(|(name, k)| !k.is_inner() && !held.contains_key(*name))
        .map(|(name, _)| name)
        .collect();
    let nameless = client
        .query_one(
            "SELECT count(*) FROM project_documents WHERE project_id = $1 AND entity_kind = ''",
            &[&project],
        )
        .await?
        .get::<_, i64>(0);
    Ok(json!({
        "documents": held.values().sum::<i64>(),
        "kinds": held.iter().map(|(k, n)| json!({ "kind": k, "documents": n })).collect::<Vec<_>>(),
        "nameless": nameless,
        "undeclaredKinds": undeclared,
        "emptyKinds": empty,
        "doubleClaimed": 0,
        "why": "вид и имя записаны в документе, а ключ запрещает второе такое имя: ничьих и двойных быть неоткуда"
    }))
}
pub async fn history(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    limit: i64,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT revision, bytes, content_hash, written_at, written_by
               FROM project_document_revisions
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
              ORDER BY revision DESC LIMIT $4",
            &[&project, &kind, &name, &limit],
        )
        .await?;
    Ok(json!({
        "count": rows.len(),
        "revisions": rows.iter().map(|r| json!({
            "revision": r.get::<_, i64>(0),
            "bytes": r.get::<_, i32>(1),
            "hash": r.get::<_, String>(2),
            "at": r.get::<_, i64>(3),
            "by": r.get::<_, String>(4),
        })).collect::<Vec<_>>(),
    }))
}

/// Текст сущности, каким он был на названной правке.
pub async fn at_revision(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    revision: i64,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT content, bytes, written_at, written_by FROM project_document_revisions
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 AND revision = $4",
            &[&project, &kind, &name, &revision],
        )
        .await?;
    match rows.first() {
        Some(r) => Ok(json!({
            "revision": revision, "content": r.get::<_, String>(0),
            "bytes": r.get::<_, i32>(1), "at": r.get::<_, i64>(2), "by": r.get::<_, String>(3),
        })),
        None => Ok(json!({ "revision": revision, "known": false,
                           "why": "такой правки у этой сущности нет" })),
    }
}

/// Блоки документа — так, как их разобрал сервер.
///
/// Интерфейс до сих пор получал текст простынёй и разбирал его **заново** своим
/// markdown. Разбор при этом уже лежит в базе: 50860 блоков и 25991 ячейка,
/// посчитанные один раз при записи. Два разбора одного документа расходятся
/// молча — этим уже обжигались на границе раздела.
///
/// Таблица отдаётся ЯЧЕЙКАМИ, а не строкой разметки: колонки в ней уже
/// разделены — с учётом экранированной черты и черты внутри кодовой вставки,
/// на которых наивное деление ошибается.
pub async fn blocks(
    pool: &Pool,
    project: &str,
    entity_kind: &str,
    entity_name: &str,
    anchor: Option<&str>,
    own: bool,
) -> Result<Value, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");

    // Раздел ограничивает выборку своими блоками; без него — весь документ.
    // `own` обрывает раздел на первом вложенном заголовке: оглавление грузит
    // подразделы по отдельности, и без обрыва текст пришёл бы дважды.
    let (from, to) = match anchor {
        Some(a) if !a.is_empty() => {
            let rows = client
                .query(
                    "SELECT first_block,
                            CASE WHEN $5 THEN
                              COALESCE((SELECT min(n.first_block) - 1
                                          FROM project_document_sections n
                                         WHERE n.project_id = s.project_id
                                           AND n.entity_kind = s.entity_kind AND n.entity_name = s.entity_name
                                           AND n.first_block > s.first_block
                                           AND n.first_block <= s.last_block), s.last_block)
                            ELSE last_block END
                       FROM project_document_sections s
                      WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 AND anchor = $4",
                    &[&project, &entity_kind, &entity_name, &a, &own],
                )
                .await?;
            match rows.first() {
                Some(r) => (r.get::<_, i32>(0), r.get::<_, i32>(1)),
                None => return Ok(json!({ "known": false, "why": "нет такого раздела" })),
            }
        }
        _ => (-1, i32::MAX),
    };

    let rows = client
        .query(
            "SELECT ord, kind, level, raw FROM project_document_blocks
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                AND ord >= $4 AND ord <= $5
              ORDER BY ord",
            &[&project, &entity_kind, &entity_name, &from, &to],
        )
        .await?;
    let cells = client
        .query(
            "SELECT block_ord, row_ord, col, value, raw FROM project_document_cells
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                AND block_ord >= $4 AND block_ord <= $5
              ORDER BY block_ord, row_ord, col",
            &[&project, &entity_kind, &entity_name, &from, &to],
        )
        .await?;

    let mut by_block: std::collections::BTreeMap<i32, Vec<(i32, i32, String, String)>> = Default::default();
    for c in &cells {
        by_block
            .entry(c.get(0))
            .or_default()
            .push((c.get(1), c.get(2), c.get(3), c.get(4)));
    }

    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let ord: i32 = r.get(0);
            let kind: String = r.get(1);
            let mut block = json!({
                "ord": ord, "kind": kind, "level": r.get::<_, Option<i32>>(2),
                "raw": r.get::<_, String>(3),
            });
            if let Some(cells) = by_block.get(&ord) {
                let rows_count = cells.iter().map(|c| c.0).max().unwrap_or(0) + 1;
                let table: Vec<Vec<Value>> = (0..rows_count)
                    .map(|row| {
                        let mut line: Vec<(i32, &String, &String)> = cells
                            .iter()
                            .filter(|c| c.0 == row)
                            .map(|c| (c.1, &c.2, &c.3))
                            .collect();
                        line.sort_by_key(|c| c.0);
                        line.into_iter().map(|(_, value, raw)| json!({ "value": value, "raw": raw })).collect()
                    })
                    .collect();
                block["cells"] = json!(table);
            }
            block
        })
        .collect();
    Ok(json!({ "count": out.len(), "blocks": out }))
}
