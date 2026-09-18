//! Проекции, которых у донора нет.
//!
//! Донор разбирает документ на блоки, разделы, ячейки и поля — это его работа, и
//! трогать её нельзя. Отсюда и граница: здесь из **структуры** считаются те
//! таблицы, которых в доноре не заведено. Каждая — по правилу, названному вслух;
//! правило, не записанное рядом, назавтра разойдётся с набором незаметно.

use crate::db::Says;
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


/// Цепочка фаз, вердикт гейта и фаза задачи — СВОЕЙ пачкой, а не в общей.
///
/// Форма этих трёх видов ещё меняется, а `CREATE OR REPLACE` отказывает, как
/// только у вида меняется набор колонок: снимать приходится. Но `DROP VIEW`
/// берёт ACCESS EXCLUSIVE, и снятие внутри общей пачки — это ход, которым новый
/// экземпляр встаёт в очередь за чужим `next-task`, читающим `phase_open`, и
/// держит за собой всю схему. При сине-зелёной выкатке — зависший запуск.
///
/// Своя пачка снимает ровно это: ожидание замка касается трёх видов и ничего
/// больше. Условия на форму здесь НЕТ НАРОЧНО — условие пришлось бы править
/// вместе с каждой колонкой, а забытое оно роняет запуск. Пересоздание дёшево и
/// верно всегда.
const PHASE_VIEWS: &str = r#"
DROP VIEW IF EXISTS task_phase;
DROP VIEW IF EXISTS phase_open;
DROP VIEW IF EXISTS gate_state;

-- ВЕРДИКТ ГЕЙТА ЦЕЛИКОМ — одной записью, а не пересказом у каждого читателя.
--
-- Слово в слово то же правило, что у ручки `gate`: красен хоть один пункт —
-- `failed`; есть непосчитанный или протухший — `open`; иначе `passed`. Пересказ
-- расходился уже дважды: барьер фаз и ручка `gate` отвечали о том же гейте
-- по-разному.
--
-- Гейт БЕЗ ЕДИНОЙ СТРОКИ замера сюда не попадает вовсе, и читатель обязан
-- считать его непройденным: «не мерили» — это не «пройден». Пустота, принятая
-- за зелёное, снимает барьер там, где его ни разу не проверяли.
--
-- СЧИТАЮТСЯ ОБЪЯВЛЕННЫЕ ПУНКТЫ, А НЕ ЗАМЕРЫ. Правило то же и для одного пункта:
-- объявленный, но ни разу не меренный пункт — это «не мерили», а не «пройден».
-- Пока считались строки замера, заведённый пункт до первого круга не считался
-- вовсе, и гейт над ним отвечал `passed`: прибор изменили, числа остались от
-- прежнего, и выглядели они настоящими.
CREATE VIEW gate_state AS
  SELECT p.project_id, i.phase AS gate,
         count(*) FILTER (WHERE g.result->>'computed' = 'failed') AS failed,
         count(*) FILTER (WHERE g.result IS NULL
                             OR g.result->>'computed' IN ('unknown','stale')) AS open,
         CASE
           WHEN count(*) FILTER (WHERE g.result->>'computed' = 'failed') > 0 THEN 'failed'
           WHEN count(*) FILTER (WHERE g.result IS NULL
                                    OR g.result->>'computed' IN ('unknown','stale')) > 0 THEN 'open'
           ELSE 'passed'
         END AS computed
    FROM (SELECT DISTINCT project_id FROM project_gates) p
    CROSS JOIN gate_item i
    LEFT JOIN project_gates g
      ON g.project_id = p.project_id AND g.phase = i.phase AND g.id = i.id
   GROUP BY 1, 2;

-- ОТКРЫТА ЛИ ФАЗА — одним местом на всех, кто про это спрашивает.
--
-- Спрашивают трое, и порознь: `next-task` («можно ли выдать эту задачу»),
-- ступени лестницы («чьи задачи считать») и пункты гейтов («не закрыли ли
-- задачу раньше её гейта»). Три копии условия разошлись бы на первой же правке
-- цепочки фаз — и разошлись бы молча.
--
-- Фаза открыта, когда КАЖДЫЙ предшествующий гейт пройден. Свой гейт при этом не
-- смотрится: Ф3 открыта и с красным G3 — красен он ровно потому, что в фазе
-- идёт работа.
--
-- ПУСТОЙ ГЕЙТ У ПРЕДШЕСТВЕННИКА ДЕРЖИТ, а не пропускает. Соблазн написать
-- `p2.gate <> ''` велик и неверен: он превращает «фаза гейта не объявила» в
-- «проходи», и одна забытая при заведении привязка снимала бы барьер всей
-- оставшейся цепочки молча. Это ровно та пустота, которую вид задач зовёт «не
-- объявлено», и звать её здесь иначе значило бы держать два правила на одну
-- пустоту. Хвостовой фазе это ничего не стоит: предшественником она не бывает.
--
-- Гейт, о котором замеров нет, держит по той же причине: `coalesce` называет
-- его `open`, а не пропускает молча.
--
-- НАБОР ЗДЕСЬ — НЕ СТРОКА В `projects`. Вид стоял `FROM projects`, и примерка,
-- копирующая набор под именем, которого среди проектов нет, получала ноль фаз:
-- все семьдесят восемь закрытых задач читались как «вид задачи не отображён ни
-- на одну фазу», и примерка на ПУСТОЙ правке показывала семьдесят семь находок,
-- которых нет. Набор — это то, о чём есть замеры; строка в `projects` говорит
-- лишь, что набор показывают человеку и тянет сборщик.
CREATE VIEW phase_open AS
  SELECT н.project_id, ph.id AS phase, ph.ord, ph.gate, ph.task_kind,
         NOT EXISTS (SELECT 1 FROM phase p2
                      WHERE p2.ord < ph.ord
                        AND coalesce((SELECT s.computed FROM gate_state s
                                       WHERE s.project_id = н.project_id AND s.gate = p2.gate),
                                     'open') <> 'passed') AS open
    FROM (SELECT id AS project_id FROM projects
           UNION
          SELECT DISTINCT project_id FROM project_gates WHERE project_id <> '') н
    CROSS JOIN phase ph;

-- Задача и её фаза. Отображение объявлено записью — `phase.task_kind`, дверь
-- `phase-set`, — а не прозой плана.
--
-- `phase` пусто — вид задачи НЕ ОТОБРАЖЁН ни на одну фазу, и это «не
-- объявлено», а не «можно всё»: `open` тогда тоже пусто, и всякий читатель
-- обязан отличить пустоту от разрешения.
CREATE VIEW task_phase AS
  SELECT t.project_id, t.id AS task_id, t.kind, t.state,
         o.phase, o.ord AS phase_ord, o.gate, o.open
    FROM project_plan_tasks t
    LEFT JOIN phase_open o
      ON o.project_id = t.project_id AND o.task_kind <> '' AND o.task_kind = t.kind;
"#;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS project_checks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, area TEXT NOT NULL,
    requirement_id TEXT, spec TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_db_tables(
    project_id TEXT NOT NULL, name TEXT NOT NULL, migration TEXT NOT NULL DEFAULT '',
    migration_file TEXT NOT NULL DEFAULT '', columns TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, name)
  );

CREATE TABLE IF NOT EXISTS project_decision_alternatives(
    project_id TEXT NOT NULL, decision_id TEXT NOT NULL, ord INTEGER NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL,
    PRIMARY KEY (project_id, decision_id, ord)
  );

CREATE TABLE IF NOT EXISTS project_decision_links(
    project_id TEXT NOT NULL, decision_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN
      ('refines','related','relates-to-decision','supersedes','closes','amends-article')),
    target TEXT NOT NULL,
    PRIMARY KEY (project_id, decision_id, kind, target)
  );

CREATE TABLE IF NOT EXISTS project_decisions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('accepted','superseded','template')),
    status_text TEXT NOT NULL DEFAULT '', date TEXT NOT NULL DEFAULT '',
    deciders TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_document_links(
    project_id TEXT NOT NULL, path TEXT NOT NULL, block_ord INTEGER NOT NULL, ord INTEGER NOT NULL,
    label TEXT NOT NULL, target_path TEXT NOT NULL, target_anchor TEXT NOT NULL,
    PRIMARY KEY (project_id, path, block_ord, ord)
  );

CREATE TABLE IF NOT EXISTS project_document_plan(
    project_id TEXT NOT NULL, name TEXT NOT NULL, level TEXT NOT NULL DEFAULT '',
    contains TEXT NOT NULL DEFAULT '', state_text TEXT NOT NULL DEFAULT '',
    claim TEXT NOT NULL CHECK (claim IN ('present','absent','self','unknown')),
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, name)
  );

CREATE TABLE IF NOT EXISTS project_document_plan_counts(
    project_id TEXT NOT NULL, name TEXT NOT NULL, subject TEXT NOT NULL, claimed INTEGER NOT NULL,
    PRIMARY KEY (project_id, name, subject)
  );

CREATE TABLE IF NOT EXISTS project_document_revisions(
    id           BIGSERIAL PRIMARY KEY,
    project_id   TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    content      TEXT   NOT NULL,
    content_hash TEXT   NOT NULL,
    bytes        INTEGER NOT NULL,
    revision     BIGINT NOT NULL,
    written_at   BIGINT NOT NULL,
    written_by   TEXT   NOT NULL,
    UNIQUE (project_id, path, revision)
  );

CREATE TABLE IF NOT EXISTS project_documents(
    project_id   TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    content      TEXT   NOT NULL,
    content_hash TEXT   NOT NULL,
    bytes        INTEGER NOT NULL,
    revision     BIGINT NOT NULL,
    updated_at   BIGINT NOT NULL,
    updated_by   TEXT   NOT NULL,
    PRIMARY KEY (project_id, path)
  );

CREATE TABLE IF NOT EXISTS project_feature_stories(
    project_id TEXT NOT NULL, feature_id TEXT NOT NULL, story_id TEXT NOT NULL,
    PRIMARY KEY (project_id, feature_id, story_id)
  );

CREATE TABLE IF NOT EXISTS project_gates(
    project_id TEXT NOT NULL, phase TEXT NOT NULL, item TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('query','command','signed')),
    query TEXT, owner TEXT,
    state TEXT NOT NULL DEFAULT 'unknown' CHECK (state IN ('unknown','passed','failed','refused')),
    violations INTEGER NOT NULL DEFAULT 0,
    detail TEXT NOT NULL DEFAULT '',
    checked_at BIGINT,
    signed_by TEXT,
    signed_at BIGINT,
    article INTEGER,
    PRIMARY KEY (project_id, phase, item),
    CHECK ((kind = 'query') = (query IS NOT NULL)),
    CHECK ((kind = 'signed') = (owner IS NOT NULL))
  );

CREATE TABLE IF NOT EXISTS project_needs(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    text TEXT NOT NULL DEFAULT '', sides TEXT NOT NULL DEFAULT '',
    sources TEXT NOT NULL DEFAULT '', theme TEXT NOT NULL DEFAULT '',
    priority TEXT NOT NULL CHECK (priority IN ('must','should','later','unknown')),
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_plan_versions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_plan_milestones(
    project_id TEXT NOT NULL, id TEXT NOT NULL, version_id TEXT NOT NULL,
    ord INTEGER NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, version_id) REFERENCES project_plan_versions(project_id, id) ON DELETE CASCADE
  );

CREATE TABLE IF NOT EXISTS project_plan_tasks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, milestone_id TEXT NOT NULL,
    ord INTEGER NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL, size TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('not_started','claimed','closed')),
    kind TEXT NOT NULL DEFAULT 'dev' CHECK (kind IN ('dev','test')),
    closing_commit TEXT,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, milestone_id) REFERENCES project_plan_milestones(project_id, id) ON DELETE CASCADE
  );

CREATE TABLE IF NOT EXISTS project_plan_task_deps(
    project_id TEXT NOT NULL, task_id TEXT NOT NULL, depends_on TEXT NOT NULL,
    PRIMARY KEY (project_id, task_id, depends_on),
    CHECK (task_id <> depends_on),
    FOREIGN KEY (project_id, task_id) REFERENCES project_plan_tasks(project_id, id) ON DELETE CASCADE
  );

CREATE TABLE IF NOT EXISTS project_questions(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('open','decided','closed')),
    state_text TEXT NOT NULL DEFAULT '', gate TEXT NOT NULL DEFAULT '',
    opened_at TEXT NOT NULL DEFAULT '', closed_at TEXT NOT NULL DEFAULT '',
    has_answer BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_requirements(
    project_id TEXT NOT NULL, id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('FR','NFR')),
    area TEXT NOT NULL, text TEXT NOT NULL, path TEXT NOT NULL,
    satisfied BOOLEAN NOT NULL,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_risks(
    project_id TEXT NOT NULL, id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('open','accepted','closed')),
    impact TEXT NOT NULL DEFAULT '', probability TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL DEFAULT '', trigger_sign TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '', settled_by TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_runs_log(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '', milestone TEXT NOT NULL DEFAULT '',
    is_milestone BOOLEAN NOT NULL DEFAULT FALSE,
    left_open BOOLEAN NOT NULL DEFAULT FALSE, sections INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, path)
  );

CREATE TABLE IF NOT EXISTS project_screen_references(
    project_id TEXT NOT NULL, source TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('story','task')), screen_id TEXT NOT NULL,
    PRIMARY KEY (project_id, source, screen_id)
  );

CREATE TABLE IF NOT EXISTS project_screens(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    area TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_stories(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    area TEXT NOT NULL DEFAULT '', persona TEXT NOT NULL DEFAULT '',
    phase TEXT NOT NULL DEFAULT '', feature TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_story_requirements(
    project_id TEXT NOT NULL, story_id TEXT NOT NULL, requirement_id TEXT NOT NULL,
    PRIMARY KEY (project_id, story_id, requirement_id)
  );

CREATE TABLE IF NOT EXISTS project_terms(
    project_id TEXT NOT NULL, id TEXT NOT NULL, term TEXT NOT NULL,
    meaning TEXT NOT NULL DEFAULT '', area TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
  );

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

-- Объявленный способ СТУПЕНИ живёт отдельно от самой ступени — ровно по тому
-- же образцу, что способ пункта готовности. Причина та же и уже измеренная:
-- способ, лежащий в строке ступени, стирается всяким проходом, который эту
-- строку трогает, и объявленное исчезает молча. Семь способов так и пропали.
CREATE TABLE IF NOT EXISTS harness_process_method (
  set_name text NOT NULL, process text NOT NULL, ord integer NOT NULL,
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','unknown')),
  method text NOT NULL DEFAULT '', declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (set_name, process, ord));

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
  PRIMARY KEY (project_id, phase));

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
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','unknown')),
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
    CHECK (method_kind IN ('query','command','unknown')),
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
  method_kind text NOT NULL CHECK (method_kind IN ('query','command','unknown')),
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

-- Вердикт предполёта знает ревизию, на которой получен: «предполёт устарел» —
-- сравнение ревизий, а не память.
-- Открытое рабочее дерево — факт репозитория, а не набора: его подаёт харнес.
CREATE TABLE IF NOT EXISTS task_worktree (
  project_id text NOT NULL, task_id text NOT NULL,
  branch text NOT NULL DEFAULT '', since bigint NOT NULL,
  PRIMARY KEY (project_id, task_id));

-- СДЕЛАННОЕ НЕ В СВОЙ ЧЕРЁД — ЭТО ДОЛГ, И ДОЛГ ЗАПИСЫВАЕТСЯ.
--
-- Задача, закрытая при закрытой фазе, нарушила порядок: гейт, который эту фазу
-- открывает, тогда не был пройден, и то, на что работа опиралась, ещё не стояло.
-- Границы фаз двигать нельзя — двигается способ выйти: делать то, что фазой
-- ниже, а сделанное раньше срока ПЕРЕДЕЛЫВАТЬ.
--
-- Считать это на лету нельзя: правило видит долг, пока фаза закрыта, и теряет
-- его в тот самый миг, когда гейт зеленеет, — то есть ровно тогда, когда
-- переделывать становится можно. Поэтому долг замечается и лежит.
--
-- Гасится он одним: задача закрыта ЗАНОВО, коммитом позже того, которым долг
-- замечен. Не словом, не побегом и не позеленевшим гейтом.
CREATE TABLE IF NOT EXISTS task_redo (
  project_id text NOT NULL,
  task_id    text NOT NULL,
  noticed_at bigint NOT NULL,
  phase      text NOT NULL DEFAULT '',
  gate       text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, task_id)
);

-- Закрытие судится ОДИН РАЗ — первым замером, начатым после того, как план его
-- увидел, и отмечается своим коммитом. Судить каждой пересборкой значило судить
-- давнее закрытие по нынешней фазе: гейт, покрасневший после, ставил долг
-- задачам, закрытым в свой черёд. А судить пересборкой — по замеру, сделанному
-- ДО закрытия.
--
-- Отметка — своей таблицей, а не колонкой `task_state`: подача полная и снимает
-- строки задач, которых в ней нет. Задача, выпавшая из одной подачи, теряла бы
-- отметку и судилась заново по нынешней фазе.
CREATE TABLE IF NOT EXISTS task_closing_judged (
  project_id     text NOT NULL,
  task_id        text NOT NULL,
  closing_commit text NOT NULL,
  PRIMARY KEY (project_id, task_id, closing_commit)
);

CREATE TABLE IF NOT EXISTS task_plan (
  project_id text NOT NULL,
  task_id text NOT NULL,
  at bigint NOT NULL,
  task_revision bigint NOT NULL,
  body text NOT NULL DEFAULT '',
  declared_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, task_id, at));

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

-- НАД ЧЕМ ПУНКТ МЕРЯЕТ. Правило, у которого предмет ИСЧЕЗ, отчитывалось
-- тишиной: строк нет — нарушений нет — пункт зелен и пропал из перечня
-- непройденных. Счёт при этом улучшался, и гейт зеленел от того, что мерить
-- стало нечего.
--
-- Ровно тот класс, против которого в наборе написано «пустой список вместо
-- ответа врёт», — и он сработал на самом харнесе: сто пятьдесят четыре нарушения
-- ушли молча, когда из плана пропали красные задачи.
-- ПРОГОН ВЕРСИИ. Прогоны раскладывались по имени: задачные и этапные, а всё
-- прочее уходило в `_ => continue` БЕЗ ЕДИНОГО СЛОВА. Документ `run` с именем
-- `v1` не появлялся в наборе, ошибки не было, счётчик не менялся — тот же род
-- молчания, который вычищен из подачи фактов.
-- ИСТОЧНИК И ЕГО КОПИИ. Нормативный перечень живёт в четырёх местах, и
-- равенство держит рука: три копии честно помечены производными, но пометка —
-- ПРОЗА. Двадцать первая настройка потребует пяти правок, и красной станет ноль.
--
-- Дверь одна на два решения владельца: там сверяется пара чисел, здесь — тело
-- названной функции (запрет восьмой копии помощника). Разница только в способе
-- сравнения, и он объявляется полем, а не второй дверью.
CREATE TABLE IF NOT EXISTS derived_copy (
  project_id text NOT NULL,
  name text NOT NULL,
  source_kind text NOT NULL,
  source_name text NOT NULL,
  copy_kind text NOT NULL,
  copy_name text NOT NULL,
  compare text NOT NULL CHECK (compare IN ('count', 'body')),
  -- ОБРАЗЕЦ У КАЖДОЙ СТОРОНЫ СВОЙ. Источник и копия говорят одно и то же
  -- РАЗНЫМИ словами — «`ADR-0020` называет девять» против «гейтятся девять», — и
  -- один образец на обоих берёт из одного нужное, из другого соседнее слово.
  source_pattern text NOT NULL DEFAULT '',
  pattern text NOT NULL,
  why text NOT NULL DEFAULT '',
  decided_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, name, copy_kind, copy_name));

-- ДОМА ПРАВИЛУ КОДА И УТВЕРЖДЕНИЮ. У `tot-ade` под видом `check` лежали три
-- разные вещи: 56 критериев приёмки, 141 ИМЯ ПРАВИЛА В КОДЕ (11 из 12 найдены
-- в `crates/`) и 30 утверждений с пространством имён.
--
-- Пока дома нет, чинить извлечение нельзя: заработавшая пересборка снесла бы
-- 171 запись как «устаревшую проекцию» — и это была бы потеря, а не уборка.
CREATE TABLE IF NOT EXISTS project_lint_rules (
    project_id  text NOT NULL,
    id          text NOT NULL,
    area        text NOT NULL DEFAULT '',
    entity_kind text NOT NULL DEFAULT '',
    entity_name text NOT NULL DEFAULT '',
    section_ord integer,
    origin      text NOT NULL DEFAULT 'projected',
    PRIMARY KEY (project_id, id)
);

-- РАССУЖДЕНИЕ КАК СУЩНОСТЬ. В пяти документах `myack` 81 килобайт прозы не
-- держала ни одна колонка: разделы, которые ничего не объявляют, а объясняют —
-- «Инварианты и их держатели», «MON — мониторы», «Открытые вопросы». Померено
-- дверью `document-coverage`: из 41 раздела `srs` 14 только текст, 13 смешанных.
--
-- Своего имени у рассуждения нет, и потому сущностью оно стать не могло. Но
-- адрес есть: ЯКОРЬ раздела — он выведен из заголовка, уникален внутри
-- документа (7394 из 7394) и переживает правки выше по тексту, в отличие от
-- номера блока.
--
-- Тело хранится колонкой: рассуждение — это и есть его текст, и вынимать из
-- него «суть» значило бы пересказывать, а пересказ протухает.
-- Имя рассуждения — ЕГО АДРЕС: документ и якорь, склеенные решёткой
-- (`srs#3-20-invarianty`). Якорь уникален внутри документа, но не в наборе, а
-- отметки и связи ключуются парой «род, имя» — значит имя обязано быть
-- полным адресом, иначе рассуждения двух документов слились бы в одно.
CREATE TABLE IF NOT EXISTS project_rationale (
    project_id  text NOT NULL,
    id          text NOT NULL,
    entity_kind text NOT NULL,
    entity_name text NOT NULL,
    anchor      text NOT NULL,
    section_ord integer,
    title       text NOT NULL DEFAULT '',
    body        text NOT NULL DEFAULT '',
    origin      text NOT NULL DEFAULT 'projected',
    PRIMARY KEY (project_id, id)
);

-- ЧЕМ ДОКАЗАНО ТРЕБОВАНИЕ — одной таблицей на все роды доказательства.
--
-- Правило «у каждого требования есть проверка» знало ровно три источника, и
-- все три — про тест-кейс. А у `tot-ade` 122 требования доказываются ПРАВИЛОМ
-- КОДА: линт не пробует случай, он отказывается собирать нарушение. Считать
-- это «непокрытым» значит требовать сценарий там, где стоит запрет.
--
-- Годится ли род доказательством — ОБЪЯВЛЯЕТСЯ (`kind-proves`), а не решается
-- здесь: это суждение о том, чем в проекте принято доказывать.
CREATE TABLE IF NOT EXISTS project_requirement_proof (
    project_id     text NOT NULL,
    requirement_id text NOT NULL,
    proof_kind     text NOT NULL,
    proof_id       text NOT NULL,
    origin         text NOT NULL DEFAULT 'projected',
    PRIMARY KEY (project_id, requirement_id, proof_kind, proof_id)
);

CREATE TABLE IF NOT EXISTS project_assertions (
    project_id  text NOT NULL,
    id          text NOT NULL,
    area        text NOT NULL DEFAULT '',
    entity_kind text NOT NULL DEFAULT '',
    entity_name text NOT NULL DEFAULT '',
    section_ord integer,
    origin      text NOT NULL DEFAULT 'projected',
    PRIMARY KEY (project_id, id)
);

-- КОГДА СТРОКА СУЩНОСТИ МЕНЯЛАСЬ. Прежде это искалось по ревизиям: самая
-- ранняя запись документа, где текст стоит дословно. Стоило 5,5 секунды при
-- каждом замере гейта, требовало «различимого» текста и потому выбрасывало
-- истории с короткими заголовками.
--
-- Помнить проще, чем восстанавливать. Пересборка сверяет отпечаток текста с
-- прошлым: совпал — дата держится, разошёлся — ставится новая. Никакого
-- поиска по истории, и любой род сущности годится без разбора, различим его
-- текст или нет.
CREATE TABLE IF NOT EXISTS entity_stamp (
    project_id text   NOT NULL,
    kind       text   NOT NULL,
    id         text   NOT NULL,
    text_hash  text   NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL,
    PRIMARY KEY (project_id, kind, id)
);

CREATE TABLE IF NOT EXISTS entity_confirm (
    project_id text   NOT NULL,
    kind       text   NOT NULL,
    id         text   NOT NULL,
    at         bigint NOT NULL,
    by_whom    text   NOT NULL DEFAULT '',
    why        text   NOT NULL,
    cause      text   NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, kind, id, at)
);

-- ОЧЕРЕДЬ РЕШЕНИЙ ВЛАДЕЛЬЦА. Заявка сессии на изменение харнеса и просьба
-- подтвердить коммит лежат в одной очереди: человек открывает пульт с одним
-- вопросом — «что от меня ждут», — и два места для ответа означали бы, что
-- половина ожиданий не видна.
CREATE TABLE IF NOT EXISTS owner_ask (
  id          bigserial PRIMARY KEY,
  project_id  text   NOT NULL DEFAULT '',
  kind        text   NOT NULL,
  title       text   NOT NULL,
  body        text   NOT NULL DEFAULT '',
  asked_by    text   NOT NULL DEFAULT '',
  at          bigint NOT NULL,
  run_id      text   NOT NULL DEFAULT '',
  state       text   NOT NULL DEFAULT 'open',
  why         text   NOT NULL DEFAULT '',
  decided_by  text   NOT NULL DEFAULT '',
  decided_at  bigint
);

-- Прогон задачи агентом: одна строка на попытку. Таблица досталась от прежнего
-- харнеса вместе со своим словарём состояний и запретом двух живых прогонов
-- одной задачи; объявлена она здесь, чтобы у неё был один хозяин, и в точности
-- такой, какая лежит в базе.
CREATE TABLE IF NOT EXISTS project_task_runs (
  id           text    NOT NULL PRIMARY KEY,
  project_id   text    NOT NULL,
  task_id      text    NOT NULL,
  agent_id     text    NOT NULL DEFAULT '',
  state        text    NOT NULL DEFAULT 'running',
  attempt      integer NOT NULL DEFAULT 1,
  run_id       text,
  session_id   text,
  note         text    NOT NULL DEFAULT '',
  created_at   bigint  NOT NULL,
  updated_at   bigint  NOT NULL,
  finished_at  bigint
);

-- Переход прогона из состояния в состояние: чем кончилась попытка и по чьей
-- воле. Таблица тоже от прежнего харнеса.
CREATE TABLE IF NOT EXISTS project_task_run_events (
  id          bigserial PRIMARY KEY,
  task_run_id text   NOT NULL REFERENCES project_task_runs (id) ON DELETE CASCADE,
  from_state  text,
  to_state    text   NOT NULL,
  reason      text   NOT NULL DEFAULT '',
  actor       text   NOT NULL,
  at          bigint NOT NULL
);

-- ЧТО ПРОГОН УСПЕЛ: шаги, вопросы, отказы. Переход состояния лежит рядом, в
-- своей таблице, и о работе не говорит ничего: состояние сообщает, что прогон
-- жив, и молчит о том, дошёл ли он хоть до чего-нибудь.
CREATE TABLE IF NOT EXISTS task_run_event (
  id          bigserial PRIMARY KEY,
  project_id  text   NOT NULL,
  run_id      text   NOT NULL,
  at          bigint NOT NULL,
  kind        text   NOT NULL,
  text        text   NOT NULL
);

-- Слово человека прогону и ответ прогона. Доставленное помечается, иначе
-- агент читал бы одно и то же на каждом круге.
CREATE TABLE IF NOT EXISTS task_run_message (
  id           bigserial PRIMARY KEY,
  project_id   text   NOT NULL,
  run_id       text   NOT NULL,
  at           bigint NOT NULL,
  side         text   NOT NULL,
  text         text   NOT NULL,
  delivered_at bigint
);

CREATE TABLE IF NOT EXISTS chat_message (
  id           bigserial PRIMARY KEY,
  project_id   text   NOT NULL,
  thread_id    text   NOT NULL,
  at           bigint NOT NULL,
  side         text   NOT NULL,
  text         text   NOT NULL,
  delivered_at bigint
);

-- ПЕРЕЧНИ ЭТАПА. Документ этапа несёт четыре списка — «Требования (6)»,
-- «Истории (6)», «Экраны (10)», «Проверки (27)» — и сам говорит рядом: «все
-- ссылки списками, потому что этап проверяется по ним». Дом был только у
-- первого: 372 имени проверок и 213 имён историй жили одним лишь разбором,
-- таблицы `milestone × check` не существовало вовсе.
--
-- Одна таблица, «что» — колонка. Три новые (`milestone_checks`,
-- `milestone_stories`, `milestone_screens`) разошлись бы между собой, а пар
-- «вид → вид» в наборе замерено 132: по таблице на пару набор не удержит.
--
-- `project_milestone_requirements` НЕ переносится сюда намеренно: её читают
-- правила, и у неё свой инвариант — требование принадлежит ровно одному
-- этапу. Переезд ради стройности стоил бы переписывания работающих правил.
CREATE TABLE IF NOT EXISTS project_milestone_links (
    project_id   text NOT NULL,
    milestone_id text NOT NULL,
    kind         text NOT NULL,
    target       text NOT NULL,
    origin       text NOT NULL DEFAULT 'projected',
    PRIMARY KEY (project_id, milestone_id, kind, target)
);

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
  kind text NOT NULL CHECK (kind IN ('query','command','manual','unknown')),
  query text,
  owner text,
  probe text NOT NULL DEFAULT '',
  why text NOT NULL DEFAULT '',
  PRIMARY KEY (phase, item));

CREATE TABLE IF NOT EXISTS gate_head (
  phase text PRIMARY KEY,
  title text NOT NULL DEFAULT '');

CREATE TABLE IF NOT EXISTS phase (
  id text PRIMARY KEY,
  ord integer NOT NULL,
  title text NOT NULL,
  gate text NOT NULL DEFAULT '',
  plan_level text NOT NULL DEFAULT '',
  task_kind text NOT NULL DEFAULT '');

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

-- СЛЕД ПЕРЕГРУЗКИ ПЕРЕЖИВАЕТ ПЕРЕГРУЗКУ. Отказы «занято» и взятия второго
-- соединения при живом первом жили счётчиком в памяти и строкой в журнале: от
-- часа простоя 2026-09-17 не осталось ничего, что можно прочесть дверью и
-- сравнить с другим днём. Сборщик кладёт сюда накопленное и обнуляет счёт.
CREATE TABLE IF NOT EXISTS server_strain (
  at bigint NOT NULL,
  busy bigint NOT NULL DEFAULT 0,
  nested bigint NOT NULL DEFAULT 0);

-- ─── Таблицы, которых сервер не заводил ──────────────────────────────────────
--
-- ДВАДЦАТЬ СЕМЬ ТАБЛИЦ СЕРВЕР ТОЛЬКО ПРАВИЛ, а заводил их когда-то донор —
-- среди них `project_documents`, `project_gates`, `project_plan_tasks`. Жили
-- они с тех пор и существовали лишь потому, что однажды их кто-то создал.
-- Значит, нового экземпляра харнеса не существовало: на чистой базе первый же
-- `ALTER` несуществующей таблицы ронял всю схему, и проверить её было негде —
-- ни в CI, ни на стенде. Класс ошибок «запрос зовёт снятую колонку» ловился
-- поэтому только наборами и только на живом.
--
-- Определения взяты у донора (`legacy-qm/src/projects/*.ts`) КАК ЕСТЬ, вместе с
-- колонкой `path` и старыми ограничениями. Это не небрежность: ниже по этой же
-- пачке лежат переезды, которые `path` снимают, ограничения переписывают и
-- добавляют нынешние колонки. Повторённая история приводит чистую базу ровно к
-- тому, чем живая стала за год; «чистое» определение разошлось бы с переездами
-- молча — и разошлось бы в ту же сторону, в какую уже разошёлся сам донор:
-- у `project_gates` он объявляет колонки, снятые этой весной.
CREATE TABLE IF NOT EXISTS project_article_references(
    project_id TEXT NOT NULL, path TEXT NOT NULL, number INTEGER NOT NULL,
    PRIMARY KEY (project_id, path, number)
  );

CREATE TABLE IF NOT EXISTS project_document_blocks(
    project_id TEXT NOT NULL, path TEXT NOT NULL, ord INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('heading','prose','table','code','list','html','blank')),
    level INTEGER, raw TEXT NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  );

CREATE TABLE IF NOT EXISTS project_document_cells(
    project_id TEXT NOT NULL, path TEXT NOT NULL, block_ord INTEGER NOT NULL,
    row_ord INTEGER NOT NULL, col INTEGER NOT NULL, raw TEXT NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY (project_id, path, block_ord, row_ord, col)
  );

CREATE TABLE IF NOT EXISTS project_document_fields(
    project_id TEXT NOT NULL, path TEXT NOT NULL, section_ord INTEGER NOT NULL, ord INTEGER NOT NULL,
    name TEXT NOT NULL, shape TEXT NOT NULL CHECK (shape IN ('row','bullet')),
    value_raw TEXT NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  );

CREATE TABLE IF NOT EXISTS project_document_sections(
    project_id TEXT NOT NULL, path TEXT NOT NULL, ord INTEGER NOT NULL,
    level INTEGER NOT NULL, title TEXT NOT NULL, anchor TEXT NOT NULL,
    parent_ord INTEGER, first_block INTEGER NOT NULL, last_block INTEGER NOT NULL,
    PRIMARY KEY (project_id, path, ord)
  );

CREATE TABLE IF NOT EXISTS project_features(
    project_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL, path TEXT NOT NULL,
    stories INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, id)
  );

CREATE TABLE IF NOT EXISTS project_gate_signatures(
    project_id TEXT NOT NULL, phase TEXT NOT NULL, item TEXT NOT NULL,
    path TEXT NOT NULL, content_hash TEXT NOT NULL,
    PRIMARY KEY (project_id, phase, item, path)
  );

CREATE TABLE IF NOT EXISTS project_plan_status(
    project_id TEXT NOT NULL, task_id TEXT NOT NULL, milestone TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('not_started','claimed','closed')),
    commit_hash TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, task_id)
  );

CREATE TABLE IF NOT EXISTS project_requirement_needs(
    project_id TEXT NOT NULL, requirement_id TEXT NOT NULL, need_id TEXT NOT NULL,
    PRIMARY KEY (project_id, requirement_id, need_id)
  );

CREATE TABLE IF NOT EXISTS project_traceability_claims(
    project_id TEXT NOT NULL, area TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
    requirements INTEGER NOT NULL DEFAULT 0, described INTEGER NOT NULL DEFAULT 0,
    in_contract INTEGER, in_code TEXT NOT NULL DEFAULT '', path TEXT NOT NULL,
    PRIMARY KEY (project_id, area)
  );

CREATE TABLE IF NOT EXISTS project_articles(
    project_id TEXT NOT NULL, number INTEGER NOT NULL,
    title TEXT NOT NULL, path TEXT NOT NULL, anchor TEXT NOT NULL, body TEXT NOT NULL,
    PRIMARY KEY (project_id, number)
  );

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

-- Чем уронить ступень. То же, что `probe` у пункта гейта: запрос, подсаживающий
-- нарушение в откатываемой транзакции. Ступень без пробы не «прошла самотест» —
-- про неё просто не сказано, чем её ронять, и это разные ответы.
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS probe text NOT NULL DEFAULT '';
-- Команда, которой ЕДИНИЦА РАБОТЫ этой ступени видна. `{name}` подставляется
-- первым словом находки. Пусто — команды нет, и это видно: `next-step` не
-- выдумывает её за набор.
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS work_run text NOT NULL DEFAULT '';
-- МЕТКА ПЕРЕЕХАЛА ВМЕСТЕ С ЗАПИСЯМИ. Метка подстановки — не имя в коде, а
-- слово внутри строки, которая ЛЕЖИТ В БАЗЕ: сменить её в коде и не тронуть
-- записанное значит молча перестать подставлять имя во всех объявленных
-- ступенях. Перенос идёт здесь и один раз: после него `{имя}` в колонке нет.
UPDATE harness_process_step SET work_run = replace(work_run, '{имя}', '{name}')
 WHERE work_run LIKE '%{имя}%';
-- ВИД ЕДИНИЦЫ РАБОТЫ СТУПЕНИ: документ, вопрос, гейт, задача. Без него ответ
-- `next-step` неисполним без догадки — `name` и `run` есть, а что это за имя,
-- вызывающий угадывал. И второе, важнее: `next-task` должен знать, с какой
-- ступени лестница начинает выдавать задачи, а знать это было неоткуда.
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS unit text NOT NULL DEFAULT '';
-- НАД ЧЕМ ступень меряет. Ступень «в плане документов не осталось ненаписанных»
-- проходится, когда плана нет ВОВСЕ: пустой перечень отвечает «нарушений нет», и
-- лестница объявляет пройденным то, чего не смотрела. Предмет объявляется, а не
-- подразумевается: пусто — «нечем мерить», и это третье состояние.
-- ЧЕЙ ПРЕДМЕТ СПОРА. У пункта гейта было две двери: отменить весь пункт с
-- причиной и объявить побег одной сущности. Ни одна не выражает того, что бывает
-- чаще всего: правило верное, набор в порядке, НЕ ЧИТАЕТ СЕРВЕР. Отменить пункт
-- — похоронить настоящую находку вместе со слепотой; оставить как есть — считать
-- проект неготовым по причине, которая от проекта не зависит.
--
-- Находка при этом ОСТАЁТСЯ КРАСНОЙ. Это не побег: счёт разделяется, а не
-- уменьшается, и «одиннадцать нарушений» перестаёт быть одним числом.
--
-- `fixed_by` обязателен: признак без указания, чем это чинится, — жалоба, а не
-- запись.
CREATE TABLE IF NOT EXISTS finding_blame (
  project_id text NOT NULL,
  rule text NOT NULL,
  entity_id text NOT NULL,
  blame text NOT NULL CHECK (blame IN ('harness', 'corpus')),
  fixed_by text NOT NULL,
  why text NOT NULL DEFAULT '',
  decided_by text NOT NULL DEFAULT '',
  PRIMARY KEY (project_id, rule, entity_id));

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

-- ХРАПОВИК: объявленный потолок долга, который может только опускаться.
--
-- Гейт, краснеющий в день, когда его завели, назавтра выключают. Долг, который
-- уже есть, не должен никого держать; РОСТ долга — должен. Потолок называет,
-- сколько находок сегодня терпимо, и почему.
--
-- Красным становится и счёт НИЖЕ потолка: долг погасили, а потолок не опустили —
-- значит завтра он молча вырастет обратно, и никто не заметит.
-- КОГДА ПЕРЕСБОРКА ПОСЛЕДНИЙ РАЗ УДАЛАСЬ, и чем упала, если упала.
--
-- Гейт читает СОХРАНЁННЫЙ замер. Пересборка, упавшая на полпути, оставляет
-- проекции недособранными, а гейт продолжает отдавать прежние числа — уверенно
-- и неверно. Так был потерян час на тридцать одну ложную находку: `reproject`
-- ответил «duplicate key», а гейт этого не знал.
--
-- Одна строка на проект: важно последнее состояние, а не журнал.
CREATE TABLE IF NOT EXISTS reproject_state (
  project_id text PRIMARY KEY,
  at bigint NOT NULL,
  ok boolean NOT NULL,
  why text NOT NULL DEFAULT '');

-- БЕСЕДА — не прогон. Прогон делает задачу и кончается; беседа думает вслух над
-- набором: спросить по ходу, разобрать документ, проверить замысел. Общая
-- таблица сделала бы «идёт» бессмысленным: у беседы нет ни задачи, ни попытки.
CREATE TABLE IF NOT EXISTS chat_thread (
  id          text   NOT NULL PRIMARY KEY,
  project_id  text   NOT NULL,
  title       text   NOT NULL DEFAULT '',
  session_id  text   NOT NULL DEFAULT '',
  state       text   NOT NULL DEFAULT 'open',
  created_at  bigint NOT NULL,
  updated_at  bigint NOT NULL
);

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






































































































































































































-- Сама функция — `RENAME_IN_COLUMNS`: её заводит `ensure` следом за этой пачкой.














CREATE UNIQUE INDEX IF NOT EXISTS project_task_runs_one_active ON project_task_runs (project_id, task_id)
  WHERE state <> ALL (ARRAY['done', 'failed', 'cancelled']);








































































-- Летопись сущности. Две породы записей, и они не смешиваются:
--
--   `declared` — то, что документ говорит о себе сам (журнал внутри вопроса).
--     Выводится из текста и пересобирается вместе с ним;
ALTER TABLE entity_event ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT 'declared';
DO $$ BEGIN
  ALTER TABLE entity_event DROP CONSTRAINT entity_event_pkey;
  ALTER TABLE entity_event ADD PRIMARY KEY (project_id, entity_kind, entity_id, source, ord);
EXCEPTION WHEN others THEN NULL; END $$;

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
-- ПРОЕКТНОЙ КОПИИ ЦЕПОЧКИ ФАЗ ЗДЕСЬ БОЛЬШЕ НЕТ, и это не упущение.
--
-- `project_phase` объявляла то же, что общая `phase`, и однажды её засеяла. С
-- тех пор читатели ходили в `phase`, а единственная дверь писала в
-- `project_phase`: объявленное дверью не читал никто, и привязка гейта к фазе
-- пропадала молча. Одна запись, одна дверь — `phase` и `phase-set`.
DROP TABLE IF EXISTS project_phase;
ALTER TABLE kind_status ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT '';
-- Почему факта нет. Пустой статус без причины читается как «забыли»; с
-- причиной — как решение, которое кто-то принял и записал.
ALTER TABLE kind_status ADD COLUMN IF NOT EXISTS why text NOT NULL DEFAULT '';
-- Переживает ли факт ступени её прохождение. У большинства ступеней факт
-- накопительный: трейлер закрытия остаётся в истории навсегда. У «в работе»
-- факт — текущее состояние: ветку заводят и сносят, и её отсутствие СЕГОДНЯ
-- ничего не говорит о том, была ли задача в работе ВЧЕРА. Доска отличает одно
-- от другого: пропуском считается только непройденная накопительная ступень.
ALTER TABLE kind_status ADD COLUMN IF NOT EXISTS durable boolean NOT NULL DEFAULT true;

-- Подача факта: кто и когда подал. Без этой записи пустая таблица деревьев
-- значит и «никто не работает», и «никто ни разу не подавал» — а это разные
-- ответы, и второй обязан быть «неизвестно».
-- Наблюдение датчика о РЕПОЗИТОРИИ: таблицы миграций, операции контракта,
-- крейты. Сервер репозитория не видит и видеть не будет; харнес видит и
-- **молчит о выводах** — он подаёт факт, судит сервер.
-- Расхождение порождённого файла с тем, что считает сервер. Пишется сборкой,
-- читается гейтом: правило гейта — запрос, и вычисление на Rust должно оставить
-- ему след в базе, а не проситься исключением в исполнитель.
-- Родов у пункта четыре, а было три.
--
--   `manual` — проиграно человеком: онбординг на чистой установке — это
--     ДЕЙСТВИЕ с исходом, а не утверждение о тексте. Назвав его подписью, мы
--     получали гейт, которому будто нужны две подписи;
--   `unknown` — пункт плана, машинного способа у которого пока нет. Он обязан
--     существовать: отсутствие пункта нельзя ни показать, ни посчитать.
--
-- Стережёт их `gate_item`: род объявляется там, и там же стоит ограничение. У
-- замера своего рода больше нет — он был копией.
DO $$ BEGIN
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_kind_check;
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_check;
  ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_check1;
EXCEPTION WHEN others THEN NULL; END $$;

-- Ответ вопроса — ПОЛЕ, а не догадка по заголовку раздела. Рядом флаг для
-- быстрого поиска и сортировки: отвечен · искали и не нашли · не сказано.
-- Третье состояние обязательно: «искали, набор молчит» зовёт владельца, а
-- «не сказано» — автора вопроса, и это разные работы.
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS answer text NOT NULL DEFAULT '';
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS answer_state text NOT NULL DEFAULT 'unsaid';
CREATE INDEX IF NOT EXISTS project_questions_by_answer
  ON project_questions(project_id, answer_state);

-- ПОДПИСИ ГЕЙТА БОЛЬШЕ НЕТ. Гейт автоматический: он закрыт, когда выполнены
-- его условия, и человеку нечего добавить к машинному замеру. Подпись только
-- откладывала закрытие — `G2` стоял с двадцатью пятью зелёными пунктами и
-- ждал росчерка, — а однажды заставила меня написать правило на понятие,
-- которого в наборе нет.
--
-- Таблицы `gate_signature` и `gate_signature_doc` НЕ СНОСЯТСЯ из базы: в них
-- лежат две записи с формулировками владельца о принятом — свидетельство о
-- дне, а не механизм. Снятие DDL их не трогает; решать их судьбу владельцу.

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
DELETE FROM term_retired WHERE retired_by = '' AND declared_in = 'docs-lint RETIRED + Article 12';







-- КАК ИСПОЛНИТЕЛЬ СОБИРАЕТСЯ ДЕЛАТЬ — записью, и записью РАНЬШЕ правки.
--
-- «Скажи, как будешь делать, прежде чем напишешь строку» стоит в скилле задачи
-- отдельной ступенью и не доказывается ничем: прогон, где её пропустили,
-- выглядит точно как прогон, где её прошли. Из семи дисциплин исполнения
-- меряются две — «красное прежде зелёного» и «у закрытой задачи назван
-- коммит», — и ровно эти две в наборе не нарушаются. Общее у них одно: обе
-- проверяемы ПОРЯДКОМ, а не наличием.
--
-- Поэтому у плана своё время. «Подумал сначала» становится тем же, чем стало
-- «написал проверку сначала»: сравнением двух отметок, а не обещанием.
--
-- `task_revision` — та же привязка, что у предполёта: план, написанный до
-- переписывания задачи, планом для новой задачи не является.
-- ОБЪЯВЛЕННОЕ ПЕРЕЖИВАЕТ ПЕРЕСБОРКУ — у этих трёх не переживало.
--
-- Пересборка сносит свою проекцию целиком и пишет заново. У проекций, где
-- объявленное соседствует с выведенным, снос ограничен `origin='projected'` —
-- у `project_checks`, `project_requirements`, `project_questions` так и есть. У
-- этих трёх колонки не было вовсе: `decision-link-add`, `feature-story-add` и
-- `story-requirement-add` отвечали «записано», и ближайшая пересборка стирала
-- записанное без звука. Дверь, чей итог живёт до следующего прогона, — это
-- дверь, которой нет.
ALTER TABLE project_decision_links ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';

-- С КАКОГО ДНЯ ПУНКТ СУДИТ. Ноль — «судит всё», и это умолчание: почти всякое
-- правило меряет состояние, а состояние возраста не имеет — раздела в документе
-- либо нет, либо он есть, и когда правило завелось, к делу не относится.
--
-- Иначе у правил, меряющих ПОРЯДОК. «План записан до закрытия» судит не
-- состояние, а последовательность двух событий, и событие, случившееся раньше
-- самого правила, оно судить не может: правила тогда не было. У `tot-ade` так
-- набралось 78 находок, из которых ни одна правила не нарушала — все 78 задач
-- закрыты прежде, чем харнес узнал слово «план». Закрыть их набору нечем:
-- дописать план задним числом — подделка порядка, ради которого правило и
-- заведено; переоткрыть 78 задач — сказать, что ни одна не доведена, при 84
-- закрывающих трейлерах в истории; 78 побегов — отмена правила чужими руками.
--
-- Граница применения — свойство ПРИБОРА, а не набора: держать её решением в
-- каждом наборе значило бы писать один довод столько раз, сколько наборов.
-- Поэтому она стоит у пункта и объявляется вместе с ним.
ALTER TABLE gate_item ADD COLUMN IF NOT EXISTS since bigint NOT NULL DEFAULT 0;

-- КОГДА ФАКТ СЛУЧИЛСЯ — ЭТО НЕ «КОГДА ХАРНЕС ЕГО УВИДЕЛ».
--
-- `seen_at` — про наблюдение, и раньше принятия харнеса его не бывает по
-- определению: набор, проработавший год и подключённый вчера, показывает всю
-- свою историю вчерашним днём. Судить по такой отметке ПОРЯДОК нельзя — ни одно
-- правило порядка не отличит сделанного до себя от сделанного после.
--
-- Время закрытия знает коммит, а коммиты знает тот, у кого репозиторий. Он же
-- их и подаёт. Ноль — «не сказано», и правило тогда падает обратно на `seen_at`:
-- подающий старой сборки не должен ломаться молча.
ALTER TABLE task_state ADD COLUMN IF NOT EXISTS closed_at bigint NOT NULL DEFAULT 0;
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'task_state' AND column_name = 'judged_commit') THEN
    INSERT INTO task_closing_judged (project_id, task_id, closing_commit)
      SELECT project_id, task_id, judged_commit FROM task_state WHERE judged_commit IS NOT NULL
      ON CONFLICT DO NOTHING;
    ALTER TABLE task_state DROP COLUMN judged_commit;
  END IF;
END $$;
ALTER TABLE project_feature_stories ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE project_story_requirements ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
-- И ещё две того же рода: у `project_screen_references` есть дверь
-- `screen-reference-add`, у `task_requirement` — `task-requirement-add`, и обе
-- проекции стирались целиком.
ALTER TABLE project_screen_references ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';
ALTER TABLE task_requirement ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'projected';

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
-- Часть проверок живёт ЗАПРОСАМИ в самой базе: статус вида несёт `fact`,
-- заявленное число несёт `counts`. Это данные, а не код, и переписать их обязан
-- тот же переезд — иначе они молча начнут отвечать «запрос не выполнился», а
-- гейт покажет `unknown` там, где раньше мерил.
--
-- Переписывание пунктов гейта отсюда снято вместе с копией правила в замере:
-- те строки правили `project_gates`, а меряется `gate_item`, и поправленной
-- копии не читал никто.
--
-- Проверяется это самотестом: `gate-selftest` роняет каждый запросный пункт
-- своей пробой, и пункт, чей запрос сломан, там сразу виден.
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


-- СЛОВАРЬ В ДВА СЛОЯ. `scheme_term` не имел колонки проекта, и `scheme-term-set`
-- выглядел проектной дверью: зовётся из репозитория, отвечает про проект, стоит
-- в одном ряду с `requirement-add`. Проект, поправивший СВОЁ слово, молча менял
-- разбор у всех остальных — проверено: слово, объявленное из одного набора,
-- пришло в ответе соседнего.
--
-- Цена была не в неудобстве: правку, закрывавшую 77 нарушений, пришлось
-- отменить ровно потому, что она ложилась на чужой набор.
--
-- Пустой `project_id` — ОБЩИЙ слой. Раздавать сто семнадцать слов по владельцам
-- не нужно: они и есть общее объявление, а проект переопределяет ту роль, где
-- его форма своя.
-- СВЕЖЕСТЬ ФАКТА. Пункты спрашивали «подавал ли КОГДА-НИБУДЬ» — `NOT EXISTS
-- (SELECT 1 FROM fact_push …)`, — и датчик, подававший однажды и переставший,
-- держал свой пункт зелёным на замороженных данных сколько угодно долго.
--
-- Это разворот доктрины ровно там, где она сформулирована. «Не подавал — значит
-- неизвестно, а не зелёно» защищает от датчика, который не подавал НИ РАЗУ, и
-- ничего не говорит про тот, что подавал и умолк. Второй опаснее: у него есть
-- данные, и они выглядят измеренными.
--
-- Срок объявляется набором (`sensor.stale_after_ms`). Не объявлен — свежесть не
-- судится: выдумывать срок за набор значило бы краснеть по своему усмотрению.
-- ЧЕМ ИМЕННО плох род факта: не подавал ни разу или подавал и протух. Это
-- разные беды и чинятся разным — завести кормильца либо вернуть умолкшего, — а
-- пункт называл первую в обоих случаях. Слово, называющее не тот случай, шлёт
-- читателя не туда.
CREATE OR REPLACE FUNCTION fact_gap(p text, f text) RETURNS text AS $г$
  SELECT CASE WHEN EXISTS (SELECT 1 FROM fact_push WHERE project_id = p AND fact = f)
              THEN 'подавал и ПРОТУХ'
              ELSE 'ни разу не подавал' END
$г$ LANGUAGE sql STABLE;

CREATE OR REPLACE FUNCTION fact_fresh(p text, f text) RETURNS boolean AS $ф$
  SELECT CASE
    WHEN NOT EXISTS (SELECT 1 FROM fact_push WHERE project_id = p AND fact = f) THEN false
    WHEN (SELECT s.stale_after_ms FROM sensor s WHERE s.project_id = p AND s.fact = f) IS NULL THEN true
    ELSE (SELECT max(fp.at) FROM fact_push fp WHERE fp.project_id = p AND fp.fact = f)
         > (extract(epoch from now()) * 1000)::bigint
           - (SELECT s.stale_after_ms FROM sensor s WHERE s.project_id = p AND s.fact = f)
  END
$ф$ LANGUAGE sql STABLE;
-- ФАЙЛОВАЯ РАСКЛАДКА СНЯТА. `at`, `under`, `not-under`, `file` — след переезда
-- из каталога в базу: сущность живёт строкой, а не файлом. Читал их код в трёх
-- местах, и все три спрашивали не адрес, а форму: «много ли документов», «есть ли
-- перечень имён». Форма сущности это знает сама — `shape` и `single`, — а
-- «есть каталог» в точности совпадало с «документный и не одиночный»: 21 из 21,
-- ни одного исключения.
--
-- Ключ, который ничего не значит, врёт читающему: по нему пойдут искать файл.
UPDATE kind_layout SET spec = spec - 'at' - 'under' - 'not-under' - 'file'
 WHERE spec ?| array['at', 'under', 'not-under', 'file'];
ALTER TABLE scheme_term ADD COLUMN IF NOT EXISTS project_id text NOT NULL DEFAULT '';
ALTER TABLE scheme_term DROP CONSTRAINT IF EXISTS scheme_term_pkey;
ALTER TABLE scheme_term ADD PRIMARY KEY (project_id, role, value);

-- Слова, которыми говорит ЭТОТ набор: своё, если роль объявлена, иначе общее.
-- Переопределение идёт РОЛЬЮ ЦЕЛИКОМ, а не отдельным словом: список из двух
-- источников — это список, которого не писал никто.
CREATE OR REPLACE FUNCTION scheme(p text)
RETURNS TABLE(role text, value text, ord integer, why text) AS $ф$
  SELECT t.role, t.value, t.ord, t.why FROM scheme_term t
   WHERE t.project_id = p
      OR (t.project_id = '' AND NOT EXISTS (
            SELECT 1 FROM scheme_term o WHERE o.project_id = p AND o.role = t.role))
$ф$ LANGUAGE sql STABLE;
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS subject_query text NOT NULL DEFAULT '';
ALTER TABLE harness_process_step ADD COLUMN IF NOT EXISTS subject_why text NOT NULL DEFAULT '';

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

ALTER TABLE project_decisions DROP COLUMN IF EXISTS waiver;
DROP TABLE IF EXISTS column_server_filled, field_column_alias, rule_exception, rule_ceiling, gate_item_waiver;

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
-- Ребро без происхождения пересборка не смела снять: оно могло быть объявлено
-- дверью. И ребро, однажды выведенное из «Зависит от», жило вечно — имя
-- убирали из документа, а задача продолжала ждать.
--
-- Прежние рёбра ложатся ОБЪЯВЛЕННЫМИ: отличить их нечем, а снятое по ошибке
-- объявление делает задачу готовой раньше срока. Первая пересборка переметит
-- выведенными те, что называет документ.
ALTER TABLE project_plan_task_deps ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';
ALTER TABLE project_plan_task_deps ALTER COLUMN origin SET DEFAULT 'projected';

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

ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS what text NOT NULL DEFAULT '';
ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS blocked_by text NOT NULL DEFAULT '';
-- Закрытие вехи — дата, а не абзац. Веха `M0` закрылась до того, как завели
-- дисциплину задач-документов, и потому не имеет ни одной: ступень «у каждого
-- этапа есть задачи» спрашивала с неё разбивку через полгода после закрытия.
ALTER TABLE project_plan_milestones ADD COLUMN IF NOT EXISTS closed text NOT NULL DEFAULT '';

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
ALTER TABLE gate_item DROP COLUMN IF EXISTS probe_ok;
ALTER TABLE derived_copy ADD COLUMN IF NOT EXISTS source_pattern text NOT NULL DEFAULT '';
ALTER TABLE project_runs_log ADD COLUMN IF NOT EXISTS is_version boolean NOT NULL DEFAULT false;
ALTER TABLE gate_item ADD COLUMN IF NOT EXISTS subject_query text NOT NULL DEFAULT '';
ALTER TABLE gate_item ADD COLUMN IF NOT EXISTS subject_why text NOT NULL DEFAULT '';
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS probe_ok boolean;
ALTER TABLE project_gates ADD COLUMN IF NOT EXISTS id text NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS project_gates_by_id ON project_gates (project_id, phase, id);
-- ЧИСТКИ СИРОТ ПРИ СТАРТЕ ЗДЕСЬ БОЛЬШЕ НЕТ, и это не упущение.
--
-- Она сверяла замер с объявлением ПО ЗАГОЛОВКУ — ключом, который перестал быть
-- ключом, когда им стало имя. Переписанный заголовок делал живой замер сиротой
-- в её глазах, и она сносила его до того, как пункт успевал померяться заново.
-- Ту же работу делает теперь `measure_gates`: по имени, в пределах проекта и
-- каждым кругом, а не раз в перезапуск.
ALTER TABLE project_gates DROP CONSTRAINT IF EXISTS project_gates_pkey;
ALTER TABLE project_gates ADD PRIMARY KEY (project_id, phase, id);
ALTER TABLE project_sensor_spec ADD COLUMN IF NOT EXISTS skip_re text NOT NULL DEFAULT '';
-- Места, где правило не действует, перечнем образцов пути через пробел.
-- Шкала размеров живёт в одном файле, и запрещать ей называть размеры значит
-- запрещать шкале быть шкалой.
ALTER TABLE project_sensor_spec ADD COLUMN IF NOT EXISTS allow text NOT NULL DEFAULT '';



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
-- Пометка записи и полный путь листа: «+» обещает, что файла ещё нет, «!» и
-- «-» — что он есть. Сверить обещание с деревом без них нечем.
ALTER TABLE project_task_tree_leaf ADD COLUMN IF NOT EXISTS op text NOT NULL DEFAULT '';
ALTER TABLE project_task_tree_leaf ADD COLUMN IF NOT EXISTS path text NOT NULL DEFAULT '';
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
ALTER TABLE project_named_id ADD COLUMN IF NOT EXISTS heads_row boolean NOT NULL DEFAULT false;
-- ГДЕ ИМЕННО документ назвал имя. Связь была на уровне документа: `ui-spec`
-- называет `SCR-SHELL-01`, а ui-spec — 40 КБ и 26 секций. Спросить у экрана,
-- какая секция его задаёт, было нечем. Секция — ord блока-заголовка, тот же,
-- каким она лежит в `project_document_sections`.
ALTER TABLE project_named_id ADD COLUMN IF NOT EXISTS section_ord integer;

-- МЕСТО СТРОКИ В ЕЁ ДОКУМЕНТЕ. Строка сущности называла документ-источник, но
-- не место в нём: 900 строк в семи таблицах, адреса нет ни у одной. Пока места
-- нет, собрать документ обратно из таблицы нельзя — неизвестен ни порядок, ни
-- к какому разделу строка относится, а это и есть условие разворота «таблица
-- источник, документ — сборка».
ALTER TABLE project_terms ADD COLUMN IF NOT EXISTS section_ord integer;
ALTER TABLE project_needs ADD COLUMN IF NOT EXISTS section_ord integer;
ALTER TABLE project_risks ADD COLUMN IF NOT EXISTS section_ord integer;
ALTER TABLE project_db_tables ADD COLUMN IF NOT EXISTS section_ord integer;
ALTER TABLE project_requirements ADD COLUMN IF NOT EXISTS section_ord integer;
ALTER TABLE project_checks ADD COLUMN IF NOT EXISTS section_ord integer;

-- ССЫЛКА ИЗ ТЕКСТА ТРЕБОВАНИЯ ложится в `project_requirement_sources` — она
-- уже есть, той же формы `(requirement_id, kind, target)`, и дверь к ней тоже
-- есть: `requirement-source-add` со словарём `decision · article · screen ·
-- requirement`. Пустовала она потому, что заполнять её было некому.
--
-- Я завёл было отдельную таблицу и снял: две таблицы одной формы разошлись бы
-- при первой же правке, а правил, читающих обе, не написал бы никто.
--
-- `origin` отделяет извлечённое от объявленного: без него пересборка стирала
-- бы объявленное дверью.
ALTER TABLE project_requirement_sources ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'declared';

-- ДАТЫ ВОПРОСА — КОЛОНКАМИ. `opened_at` и `closed_at` были текстом, взятым из
-- прозы, и у 290 закрытых вопросов из 373 дата закрытия — прочерк: её
-- потеряли при переносе из реестра («закрыт (перенесён из реестра)»).
--
-- Пока даты нет, правило «закрытый вопрос переоткрывается, если связь
-- обновлена после него» применить не к чему: сравнивать не с чем.
--
-- Даты берутся из истории самого документа — первая ревизия и последняя, — и
-- ложатся колонками сущности, а не выводятся при каждом чтении.
-- ТЕЛО ЭКРАНА КОЛОНКОЙ. Экран четвёртый по зависимости — на него ссылаются 23
-- рода документов, 1290 связей, — и он выпадал из каскада: `purpose` пуст у
-- всех 69, а `title` слишком короток, чтобы найтись в ревизии осмысленно.
--
-- Описание при этом ЕСТЬ у всех 69: секция своего документа, названная именем
-- экрана, тело в среднем 2104 знака. Считать её видом при каждом чтении дорого
-- — кладётся колонкой.
ALTER TABLE project_screens ADD COLUMN IF NOT EXISTS spec text NOT NULL DEFAULT '';

-- ПЕРЕИМЕНОВАНИЕ ИМЕНИ ВНУТРИ КОЛОНОК. Дверь правила документы и `id` вида, а
-- имя живёт ещё в ссылочных и прозаических колонках: `measured_by` требования
-- называет проверки строкой, `check_requirements.check_id` — ссылкой. У
-- `tot-ade` 227 проверок воссоздавались каждой пересборкой из `measured_by`,
-- потому что там стояли старые имена.
--
-- Колонки НЕ ПЕРЕЧИСЛЕНЫ здесь и не угаданы: функция обходит текстовые колонки
-- проектных таблиц и правит те, где имя ДЕЙСТВИТЕЛЬНО стоит. Перечень в коде
-- разошёлся бы со схемой при первой новой таблице.
--
-- `project_documents.content` исключён намеренно: его правит сама дверь, и
-- правит с ревизией. Дважды переписать — потерять след.
--
-- Границы слова обязательны: без них `Q-1` попал бы внутрь `Q-15`.
-- Снос перед созданием обязателен: `CREATE OR REPLACE` не меняет ТИП ВОЗВРАТА,
-- и сервер, у которого функция уже есть в прежней форме, не поднялся бы вовсе
-- — со словом «db error», как это и случилось при первой правке.
DROP FUNCTION IF EXISTS rename_in_columns(text, text, text);
-- ПРИМЕРКА: набор копируется целиком, правка кладётся в копию, гейт меряется по
-- ней, копия снимается.
--
-- Харнес силён в «правильно ли сейчас» и нем в «будет ли правильно, если я
-- сделаю так». Двери, умеющей примерить, у него двенадцать из ста шестидесяти
-- одной, и агент правит вслепую: цену узнаёт после. Отсюда и реестр вопросов,
-- работающий переполнением, — спросить дешевле, чем проверить.
--
-- Одной транзакцией это не делается: пересборка берёт пул сама и открывает свои
-- транзакции, снаружи их не объять. Зато копия набора — делается, и весь
-- существующий код работает по ней без единой правки: он и так принимает имя
-- проекта доводом.
--
-- Имя копии не значится среди проектов: сборщик её не видит, а чистка сирот
-- подберёт, если прогон оборвётся на полпути. Самоисцеление тут не украшение —
-- примерка идёт десятки секунд.
--
-- Колонки с `nextval` не копируются: их значения принадлежат своей
-- последовательности, и перенос столкнул бы копию с подлинником на первичном
-- ключе.
CREATE OR REPLACE FUNCTION project_copy(откуда text, куда text) RETURNS bigint AS $$
DECLARE t record; n bigint := 0; m bigint;
BEGIN
  -- ПУСТОЕ ИМЯ ПРОЕКТА — НЕ ПРОЕКТ, А ОБЩИЙ СЛОЙ СЛОВАРЯ.
  --
  -- `scheme_term` с `project_id = ''` — это словарь, общий на все наборы, и
  -- `scheme($1)` читает его как основание. Он уже погиб однажды: `orphans-purge`
  -- снёс 118 его строк, потому что `''` не значится среди проектов, и вернуть
  -- их удалось лишь из ночного дампа. Здесь тот же обход по всем таблицам с
  -- `project_id`, только адресный — и без этой проверки одна опечатка в доводе
  -- повторила бы ту потерю.
  IF coalesce(откуда, '') = '' OR coalesce(куда, '') = '' THEN
    RAISE EXCEPTION 'имя набора пусто: `''''` — это общий слой словаря, а не набор';
  END IF;
  IF откуда = куда THEN
    RAISE EXCEPTION 'копировать набор в себя же нечем: имена совпали (%)', откуда;
  END IF;
  -- ПОРЯДОК ВСТАВКИ — ПО ВНЕШНИМ КЛЮЧАМ, А НЕ ПО АЛФАВИТУ.
  --
  -- `project_document_blocks` ссылается на `project_documents`, а по имени идёт
  -- раньше: обход в алфавитном порядке клал ребёнка прежде родителя и падал на
  -- ключе, не скопировав ничего. Глубина считается от таблиц, ни на кого не
  -- ссылающихся; `уровень < 10` — не глубина схемы (она три), а обрыв на случай
  -- кольца: кольцо в ключах свалило бы примерку бесконечным обходом.
  FOR t IN
    WITH RECURSIVE ссылка AS (
      SELECT DISTINCT c.conrelid AS дитя, c.confrelid AS предок
        FROM pg_constraint c
       WHERE c.contype = 'f' AND c.conrelid <> c.confrelid
         AND c.connamespace = 'public'::regnamespace
    ), глубина AS (
      SELECT k.oid AS таблица, 0 AS уровень
        FROM pg_class k JOIN pg_namespace n ON n.oid = k.relnamespace
       WHERE k.relkind = 'r' AND n.nspname = 'public'
      UNION ALL
      SELECT s.дитя, г.уровень + 1
        FROM глубина г JOIN ссылка s ON s.предок = г.таблица
       WHERE г.уровень < 10
    ), порядок AS (
      SELECT (таблица::regclass)::text AS имя, max(уровень) AS уровень
        FROM глубина GROUP BY 1
    )
    SELECT c.table_name AS имя,
           string_agg(quote_ident(c.column_name), ', ' ORDER BY c.ordinal_position) AS колонки,
           string_agg(CASE WHEN c.column_name = 'project_id' THEN quote_literal(куда)
                           ELSE quote_ident(c.column_name) END, ', ' ORDER BY c.ordinal_position) AS отбор
      FROM information_schema.columns c
      JOIN information_schema.tables tb
        ON tb.table_schema = c.table_schema AND tb.table_name = c.table_name
       AND tb.table_type = 'BASE TABLE'
      LEFT JOIN порядок п ON п.имя = c.table_name
     WHERE c.table_schema = 'public'
       AND coalesce(c.column_default, '') NOT LIKE 'nextval%'
     GROUP BY c.table_name, п.уровень
    HAVING bool_or(c.column_name = 'project_id')
       AND NOT EXISTS (
         SELECT 1 FROM pg_index i
          WHERE i.indrelid = ('public.' || quote_ident(c.table_name))::regclass AND i.indisunique
            AND NOT EXISTS (
              SELECT 1 FROM pg_attribute a
               WHERE a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
                 AND (a.attname = 'project_id'
                      OR EXISTS (SELECT 1 FROM pg_attrdef d
                                  WHERE d.adrelid = a.attrelid AND d.adnum = a.attnum
                                    AND pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval%'))))
     ORDER BY coalesce(п.уровень, 0), c.table_name
  LOOP
    EXECUTE format('INSERT INTO %I (%s) SELECT %s FROM %I WHERE project_id = $1',
                   t.имя, t.колонки, t.отбор, t.имя)
      USING откуда;
    GET DIAGNOSTICS m = ROW_COUNT; n := n + m;
  END LOOP;
  RETURN n;
END $$ LANGUAGE plpgsql;

-- Снять набор целиком. Не `orphans-purge`: тот берёт ВСЕХ, кого нет среди
-- проектов, и снёс бы чужую примерку, идущую рядом.
CREATE OR REPLACE FUNCTION project_forget(чей text) RETURNS bigint AS $$
DECLARE t record; n bigint := 0; m bigint;
BEGIN
  IF coalesce(чей, '') = '' THEN
    RAISE EXCEPTION 'имя набора пусто: `''''` — это общий слой словаря, а не набор';
  END IF;
  FOR t IN
    SELECT tb.table_name AS имя
      FROM information_schema.tables tb
      JOIN information_schema.columns c
        ON c.table_schema = tb.table_schema AND c.table_name = tb.table_name
       AND c.column_name = 'project_id'
     WHERE tb.table_schema = 'public' AND tb.table_type = 'BASE TABLE'
  LOOP
    EXECUTE format('DELETE FROM %I WHERE project_id = $1', t.имя) USING чей;
    GET DIAGNOSTICS m = ROW_COUNT; n := n + m;
  END LOOP;
  RETURN n;
END $$ LANGUAGE plpgsql;

-- Снимается трёхдоводная: у неё не было слова о склейке, и склейку она делала
-- всегда — молча и удалением.
DROP FUNCTION IF EXISTS rename_in_columns(text, text, text);
ALTER TABLE entity_stamp ADD COLUMN IF NOT EXISTS body_version integer NOT NULL DEFAULT 1;
ALTER TABLE entity_stamp ADD COLUMN IF NOT EXISTS confirmed_at bigint NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS owner_ask_open ON owner_ask (state, at DESC);
ALTER TABLE owner_ask ADD COLUMN IF NOT EXISTS delivered_at bigint;
-- Вопрос набора, отданный владельцу: его имя. Пометка «решает владелец» без
-- записи в очереди прятала бы вопрос — ступень 5 его пропускает, а владелец не
-- видит.
ALTER TABLE owner_ask ADD COLUMN IF NOT EXISTS question_id text NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS task_run_event_by_run ON task_run_event (project_id, run_id, at DESC);
CREATE INDEX IF NOT EXISTS task_run_message_by_run ON task_run_message (project_id, run_id, at);


CREATE INDEX IF NOT EXISTS chat_thread_by_project ON chat_thread (project_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS chat_message_by_thread ON chat_message (project_id, thread_id, at);

ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS created_at bigint;
ALTER TABLE project_questions ADD COLUMN IF NOT EXISTS updated_at bigint;

-- РОЛЬ связи: определяет документ сущность или только называет её.
--
-- Связь знала «ui-spec называет SCR-SHELL-01», но не знала, задаёт он экран
-- или ссылается на него. На вопрос «где определено» приходил двадцать один
-- ответ вместо одного.
--
-- Роль НЕ колонка: колонка разошлась бы с истиной при первой пересборке, а
-- источник у сущности уже записан — `entity_kind`/`entity_name` её
-- собственной строки. Вид соединяет и не может устареть.
--
-- Словарь проверен замером, и первая его редакция ВРАЛА: `dangles` набирал
-- 830 при четырёх настоящих, потому что 824 имени раскрыты из диапазона
-- («TC-STP-01…14», где существует не каждый). Слово, называющее законное
-- состояние дефектом, дороже отсутствия слова: по нему чинят несломанное.
-- Порядок обязателен: верхние виды СТОЯТ НА нижних, и снос нижнего без
-- верхнего роняет сервер на старте. Так и вышло: вид, заведённый рукой мимо
-- DDL, пережил первую пересборку и убил вторую, а ошибка пришла словом
-- «db error» — тем самым, которое сегодня уже стоило трёх попыток.
DROP VIEW IF EXISTS question_live;
DROP VIEW IF EXISTS entity_live;
DROP VIEW IF EXISTS entity_link;
DROP VIEW IF EXISTS entity_row;
DROP VIEW IF EXISTS named_id_role;
CREATE OR REPLACE VIEW named_id_role AS
SELECT n.project_id, n.entity_kind, n.entity_name, n.said_id, n.caveated,
       n.from_range, n.heads_row, n.section_ord,
       src.kind AS source_kind, src.name AS source_name,
       CASE
         WHEN src.kind IS NOT NULL AND src.kind = n.entity_kind
                                  AND src.name = n.entity_name THEN 'defines'
         WHEN src.kind IS NOT NULL                             THEN 'mentions'
         WHEN n.caveated                                       THEN 'foretells'
         WHEN n.from_range                                     THEN 'in-range'
         ELSE                                                       'dangles'
       END AS role
  FROM project_named_id n
  LEFT JOIN LATERAL (
        SELECT r.entity_kind AS kind, r.entity_name AS name FROM project_requirements r
         WHERE r.project_id = n.project_id AND r.id = n.said_id
        UNION ALL
        SELECT c.entity_kind, c.entity_name FROM project_checks c
         WHERE c.project_id = n.project_id AND c.id = n.said_id
        UNION ALL
        SELECT s.entity_kind, s.entity_name FROM project_screens s
         WHERE s.project_id = n.project_id AND s.id = n.said_id
        UNION ALL
        SELECT d.entity_kind, d.entity_name FROM project_needs d
         WHERE d.project_id = n.project_id AND d.id = n.said_id
        UNION ALL
        SELECT t.entity_kind, t.entity_name FROM project_stories t
         WHERE t.project_id = n.project_id AND t.id = n.said_id
        LIMIT 1
  ) src ON true;

-- ЗАПИСЬ СУЩНОСТИ ЦЕЛИКОМ. Одно требование — одна запись; отпечаток берётся с
-- неё всей, а не с одного поля: правка области, приоритета или признака
-- «удовлетворено» — такая же правка требования, как правка его текста.
--
-- Прежде здесь стоял отбор «видов с различимым текстом», и он выбрасывал
-- историю с коротким заголовком и экран без описания. Отбор больше не нужен.
--
-- `origin` исключён: он про то, ОТКУДА запись, а не что в ней.
CREATE OR REPLACE VIEW entity_row AS
       SELECT project_id, 'requirement' AS kind, id, entity_kind, entity_name, section_ord,
              (to_jsonb(r.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'section_ord')::text AS body, r.origin FROM project_requirements r
 UNION ALL SELECT project_id, 'check', id, entity_kind, entity_name, section_ord,
              (to_jsonb(c.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'section_ord')::text, c.origin FROM project_checks c
 UNION ALL SELECT project_id, 'need', id, entity_kind, entity_name, section_ord,
              (to_jsonb(n.*) - 'project_id' - 'entity_kind' - 'entity_name' - 'section_ord')::text, 'projected'::text FROM project_needs n
 UNION ALL SELECT project_id, 'decision', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(d.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name')::text, d.origin FROM project_decisions d
 UNION ALL SELECT project_id, 'screen', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(s.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'out_of_version')::text, s.origin FROM project_screens s
 UNION ALL SELECT project_id, 'story', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(t.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name')::text, t.origin FROM project_stories t
 UNION ALL SELECT project_id, 'task', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(p.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'ord' - 'number' - 'preflight' - 'preflight_at' - 'preflight_revision' - 'preflight_findings' - 'preflight_fresh')::text, p.origin FROM project_plan_tasks p
 UNION ALL SELECT project_id, 'milestone', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(m.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'ord')::text, m.origin FROM project_plan_milestones m
 UNION ALL SELECT project_id, 'question', id, entity_kind, entity_name, NULL::integer,
              (to_jsonb(q.*) - 'project_id' - 'origin' - 'created_at' - 'updated_at' - 'entity_kind' - 'entity_name')::text, q.origin
         FROM project_questions q
 -- Рассуждение — тоже запись: у него есть тело, адрес и дата правки, и оно
 -- переоткрывается, когда меняется то, что оно объясняет.
 UNION ALL SELECT project_id, 'rationale', id, entity_kind, entity_name, section_ord,
              (to_jsonb(a.*) - 'project_id' - 'origin' - 'entity_kind' - 'entity_name' - 'section_ord')::text, a.origin FROM project_rationale a;

-- НАПРАВЛЕННАЯ СВЯЗЬ: КТО НА КОМ СТОИТ. Раньше каскад шёл по совместному
-- упоминанию — «названо в той же секции», — и это оказалось не зависимостью, а
-- СОСЕДСТВОМ: правка `FR-ORG-01` переоткрывала `FR-ORG-02…09` только потому,
-- что они стоят в том же разделе `srs`. Того, что обновился документ, о самой
-- записи не говорит ничего.
--
-- Здесь собраны настоящие связи — те, что уже лежат таблицами и у которых есть
-- сторона: задача СТОИТ НА требовании, проверка СТОИТ НА требовании, этап
-- СТОИТ НА своих задачах и перечнях. Правка правого переоткрывает левое, и
-- никогда наоборот.
CREATE OR REPLACE VIEW entity_link AS
       SELECT project_id, 'task' AS from_kind, task_id AS from_id,
              'requirement' AS to_kind, requirement_id AS to_id FROM task_requirement
 UNION ALL SELECT project_id, 'check', check_id, 'requirement', requirement_id
         FROM project_check_requirements
 UNION ALL SELECT project_id, 'milestone', milestone_id, 'requirement', requirement_id
         FROM project_milestone_requirements
 UNION ALL SELECT l.project_id, 'milestone', l.milestone_id, l.kind, coalesce(dm.id, l.target)
         FROM project_milestone_links l
         LEFT JOIN project_decisions dm ON dm.project_id = l.project_id AND l.kind = 'decision'
               AND dm.number = substring(l.target from '^ADR-0*([0-9]+)$')::int
 UNION ALL SELECT project_id, 'story', story_id, 'requirement', requirement_id
         FROM project_story_requirements
 UNION ALL SELECT project_id, 'requirement', requirement_id, 'need', need_id
         FROM project_requirement_needs
 UNION ALL SELECT rs.project_id, 'requirement', rs.requirement_id, rs.kind, coalesce(dr.id, rs.target)
         FROM project_requirement_sources rs
         LEFT JOIN project_decisions dr ON dr.project_id = rs.project_id AND rs.kind = 'decision'
               AND dr.number = substring(rs.target from '^ADR-0*([0-9]+)$')::int
 -- Этап стоит на своих задачах, красная задача — на родительской. Оба ребра
 -- лежали колонками `milestone_id` и `parent_task_id` и в связи не попадали,
 -- отчего цепочка обрывалась на первом же шаге: правка требования доходила до
 -- задачи и дальше не шла.
 UNION ALL SELECT project_id, 'milestone', milestone_id, 'task', id
         FROM project_plan_tasks WHERE milestone_id <> '' AND kind <> 'red'
 UNION ALL SELECT project_id, 'red-task', id, 'task', parent_task_id
         FROM project_plan_tasks WHERE kind = 'red' AND parent_task_id <> ''
 -- Вопрос стоит на том, что называет, и таблицы под это нет. Брать упоминание
 -- здесь МОЖНО — в отличие от `srs`: документ вопроса это ОДИН вопрос, и
 -- соседства по разделу тут не бывает. Ровно поэтому у требования упоминание
 -- не годится, а у вопроса годится.
 UNION ALL SELECT DISTINCT n.project_id, 'question', n.entity_name, s.kind, n.said_id
         FROM project_named_id n
         JOIN entity_stamp s ON s.project_id = n.project_id AND s.id = n.said_id
        WHERE n.entity_kind = 'question' AND n.entity_name <> '' AND NOT n.caveated;

-- ЖИВОЕ СОСТОЯНИЕ. Сущность остаётся закрытой, только пока ничто из
-- названного ею не изменилось после неё. Состояние СЛЕДУЕТ из связей.
--
-- Виды берутся из объявления `reopens`, а не перечислены здесь: решение и
-- прогон объявлены непереоткрываемыми, и вид о них молчит сам.
--
-- Связи берутся НАПРАВЛЕННЫЕ, из `entity_link`: «стоит на», а не «названо
-- рядом». Соседство по разделу зависимостью не является.
CREATE OR REPLACE VIEW entity_live AS
WITH RECURSIVE прямо AS (
       -- Первый шаг: то, на чём запись стоит, изменилось ПОСЛЕ неё.
       SELECT st.project_id, st.kind, st.id, l.to_id AS cause, l.to_kind AS cause_kind,
              s2.updated_at AS cause_at, 0 AS depth
         FROM entity_stamp st
         JOIN kind_layout k ON k.name = st.kind AND (k.spec->>'reopens')::boolean IS TRUE
         JOIN entity_link l
           ON l.project_id = st.project_id AND l.from_kind = st.kind AND l.from_id = st.id
         JOIN entity_stamp s2
           ON s2.project_id = l.project_id AND s2.kind = l.to_kind AND s2.id = l.to_id
        WHERE s2.updated_at > greatest(st.updated_at, st.confirmed_at)
),
цепь AS (
       SELECT * FROM прямо
        UNION
       -- Дальше: кто стоит на переоткрытом, переоткрыт и сам. Глубина ограничена
       -- шестью: связи образуют не дерево, а граф, и без предела обход по кругу
       -- не кончится. Шесть — длина самой длинной цепочки набора плюс запас.
       SELECT l.project_id, l.from_kind, l.from_id, ц.cause, ц.cause_kind, ц.cause_at, ц.depth + 1
         FROM цепь ц
         JOIN entity_link l
           ON l.project_id = ц.project_id AND l.to_kind = ц.kind AND l.to_id = ц.id
         JOIN kind_layout k ON k.name = l.from_kind AND (k.spec->>'reopens')::boolean IS TRUE
         JOIN entity_stamp sf
           ON sf.project_id = l.project_id AND sf.kind = l.from_kind AND sf.id = l.from_id
        WHERE ц.depth < 6 AND ц.cause_at > greatest(sf.updated_at, sf.confirmed_at)
)
SELECT e.project_id, e.kind, e.id, st.updated_at, st.created_at,
       ц.cause AS stale_link, ц.cause_at AS link_changed, ц.depth,
       CASE WHEN ц.cause IS NOT NULL THEN 'reopened' ELSE 'current' END AS live_state,
       CASE WHEN ц.cause IS NULL THEN ''
            WHEN ц.depth = 0
              THEN 'связь ' || ц.cause || ' обновлена '
                   || to_char(to_timestamp(ц.cause_at/1000),'YYYY-MM-DD')
                   || ', а сама запись стояла с '
                   || to_char(to_timestamp(st.updated_at/1000),'YYYY-MM-DD')
            ELSE 'переоткрыто по цепочке: ' || ц.cause || ' обновлена '
                 || to_char(to_timestamp(ц.cause_at/1000),'YYYY-MM-DD')
                 || ', через ' || ц.depth || ' связь' END AS why
  FROM entity_row e
  JOIN kind_layout k ON k.name = e.kind AND (k.spec->>'reopens')::boolean IS TRUE
  JOIN entity_stamp st
    ON st.project_id = e.project_id AND st.kind = e.kind AND st.id = e.id
  LEFT JOIN LATERAL (SELECT * FROM цепь c
                      WHERE c.project_id = e.project_id AND c.kind = e.kind AND c.id = e.id
                      ORDER BY c.depth, c.cause_at DESC LIMIT 1) ц ON true;

-- Вопрос — частный случай общего вида: к живому состоянию добавлено
-- объявленное состояние («открыт», «закрыт», «решён»), потому что вопрос
-- бывает открытым и без всяких правок связей.
CREATE OR REPLACE VIEW question_live AS
SELECT q.project_id, q.id, q.number, q.title, q.state, q.created_at, q.updated_at,
       l.stale_link, l.link_changed,
       CASE WHEN q.state = 'open'          THEN 'open'
            WHEN l.live_state = 'reopened' THEN 'reopened'
            ELSE q.state END AS live_state,
       coalesce(l.why, '') AS why
  FROM project_questions q
  LEFT JOIN entity_live l
    ON l.project_id = q.project_id AND l.kind = 'question' AND l.id = q.id;
-- Заявленное число и объявленный предмет связываются ТОЖДЕСТВОМ ДОКУМЕНТА, а не
-- совпадением строк: «constitution:» и «constitution.md» — одно и то же, и
-- строковое равенство их не сводило ни разу.
ALTER TABLE project_document_plan_counts ADD COLUMN IF NOT EXISTS planned_kind text NOT NULL DEFAULT '';
ALTER TABLE project_document_plan_counts ADD COLUMN IF NOT EXISTS planned_name text NOT NULL DEFAULT '';
ALTER TABLE claim_subject ADD COLUMN IF NOT EXISTS entity_kind text NOT NULL DEFAULT '';
ALTER TABLE claim_subject ADD COLUMN IF NOT EXISTS entity_name text NOT NULL DEFAULT '';




-- Своё у справки — откуда снята; чужое — что о себе говорил исходный документ.
-- Эти пять повторяются у десятков справок и потому колонки, а не проза в шапке.
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS written text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS updated text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS tags text NOT NULL DEFAULT '';
ALTER TABLE project_reference_source ADD COLUMN IF NOT EXISTS role text NOT NULL DEFAULT '';

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

INSERT INTO gate_head (phase, title)
SELECT DISTINCT ON (phase) phase, title FROM gate ORDER BY phase, project_id
ON CONFLICT (phase) DO NOTHING;

-- Вид задач принадлежит ОДНОЙ фазе, место в цепочке — тоже одной. Две фазы,
-- назвавшие `dev` своим, сделали бы вопрос «в какой фазе эта задача»
-- двусмысленным; два одинаковых `ord` — вопрос «какая раньше»: `p2.ord < ph.ord`
-- ложно в обе стороны, и барьер между такими фазами исчезает молча.
--
-- Оба индекса заводятся ОТДЕЛЬНО ОТ ПАКЕТА. Схема накатывается одним
-- `batch_execute`, то есть одной транзакцией: уникальный индекс, споткнувшийся о
-- уже стоящую пару одинаковых значений, откатил бы ВСЮ схему и сервер не
-- поднялся бы. Ограничение здесь желательно, но не ценой запуска.
DO $$ BEGIN
  CREATE UNIQUE INDEX IF NOT EXISTS phase_by_task_kind ON phase (task_kind) WHERE task_kind <> '';
EXCEPTION WHEN others THEN NULL; END $$;
DO $$ BEGIN
  CREATE UNIQUE INDEX IF NOT EXISTS phase_by_ord ON phase (ord);
EXCEPTION WHEN others THEN NULL; END $$;

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
CREATE INDEX IF NOT EXISTS server_strain_at ON server_strain (at DESC);

-- ЗАМЕР БОЛЬШЕ НЕ ДЕРЖИТ КОПИЮ ПРАВИЛА. Запрос, проба, довод, подписант,
-- заголовок и род лежали и в объявлении, и в каждой строке замера — по числу
-- наборов. Копию писал только круг замера, и пробу он не писал вовсе: её
-- колонку заполняли переезды схемы, а дверь отдавала пустое у всех ста
-- шестидесяти пунктов. Заголовок и род при этом отставали на круг: правка была
-- не видна ни в двери, ни на доске до следующего пересчёта.
--
-- Два описания одного правила расходятся молча; описание теперь одно —
-- `gate_item`, — и читают его все, кому нужно правило, а не замер.
ALTER TABLE project_gates DROP COLUMN IF EXISTS query;
ALTER TABLE project_gates DROP COLUMN IF EXISTS probe;
ALTER TABLE project_gates DROP COLUMN IF EXISTS why;
ALTER TABLE project_gates DROP COLUMN IF EXISTS owner;
ALTER TABLE project_gates DROP COLUMN IF EXISTS item;
ALTER TABLE project_gates DROP COLUMN IF EXISTS kind;
ALTER TABLE project_gates DROP COLUMN IF EXISTS article;

-- Статья конституции держится за пункт таблицей `project_article_gates` — и
-- держалась всегда. Колонка `article` у пункта и у замера осталась от прежнего
-- устройства: её не читал и не писал никто, а объявление в репозитории её и не
-- знает. Колонка, которую нельзя ни прочесть, ни объявить, — обещание связи,
-- которой нет.
ALTER TABLE gate_item DROP COLUMN IF EXISTS article;

-- КЛЮЧ ПУНКТА — ЕГО ИМЯ, а не заголовок. Первичным ключом стояла пара (гейт,
-- заголовок), а имя держал ЧАСТИЧНЫЙ уникальный указатель `WHERE id <> ''`:
-- запись «по имени» приходилось оговаривать этим же условием, иначе Postgres
-- отказывал — «нет ограничения, подходящего к ON CONFLICT». Условие это давно
-- пустое: имя есть у каждого пункта, а объявление без имени раскладка не
-- принимает вовсе. Заодно уходит случайное правило «заголовки в гейте
-- различны»: заголовок — слова для человека, и одинаковыми им быть можно.
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_constraint c
               JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
              WHERE c.conname = 'gate_item_pkey' AND a.attname = 'item') THEN
    -- Безымянная строка ключом не становится: две такие в одной фазе уронили бы
    -- перевод ключа, а с ним и всю схему — сервер не поднялся бы больше никогда,
    -- и лечилось бы это только руками в базе. Объявить такой пункт нельзя, и
    -- раскладка сняла бы его следующей же строкой.
    DELETE FROM gate_item WHERE id = '';
    ALTER TABLE gate_item DROP CONSTRAINT gate_item_pkey;
    ALTER TABLE gate_item ADD PRIMARY KEY (phase, id);
  END IF;
  DROP INDEX IF EXISTS gate_item_by_id;
END $$;"#;

/// Переименование внутри колонок. Отдельной пачкой, а не в `DDL`: её же заводит
/// тест, и функция в нём та самая, что в базе.
///
/// Имя приходит ГОТОВЫМ ОБРАЗЦОМ (`name_in_text`), а новое — готовой заменой
/// (`replacement_of`). Прежде образец собирался здесь же — `\m` имя `\M`, без
/// экранирования: `/`, `.` и `-` считались границей слова, и переименование
/// `20-surface` переписывало заодно `20-surface/configure` — имя ДРУГОГО
/// документа, молча.
const RENAME_IN_COLUMNS: &str = r#"
DROP FUNCTION IF EXISTS rename_in_columns(text, text, text, boolean);
CREATE OR REPLACE FUNCTION rename_matching_in_columns(п text, образец text, стало text, слить boolean DEFAULT false)
RETURNS TABLE(таблица text, колонка text, строк bigint, снято bigint) AS $$
DECLARE c record; n bigint; d bigint; места tid[]; место tid;
BEGIN
  FOR c IN
    SELECT k.table_name AS t, k.column_name AS col
      FROM information_schema.columns k
      JOIN information_schema.columns pid
        ON pid.table_schema = k.table_schema AND pid.table_name = k.table_name
       AND pid.column_name = 'project_id'
     WHERE k.table_schema = 'public' AND k.data_type = 'text'
       AND k.table_name LIKE 'project\_%'
       AND k.column_name <> 'project_id'
       -- Содержание документа правит сама дверь, и правит с ревизией; дважды
       -- переписать — потерять след.
       AND NOT (k.table_name = 'project_documents' AND k.column_name = 'content')
       -- ИСТОРИЮ НЕ ПРАВИТЬ. `project_document_revisions.content` — запись о
       -- том, что документ говорил ТОГДА, и обход хотел переписать 479 таких
       -- записей. Это подделка прошлого: старое имя в старой ревизии — правда,
       -- а не опечатка.
       AND k.table_name <> 'project_document_revisions'
       -- ЧТО ПЕРЕЕДЕТ САМО — НЕ ТРОГАТЬ. Пять колонок разбора ссылаются на
       -- `project_documents.entity_name` связью `ON UPDATE CASCADE`: родитель
       -- переименуется, дети переедут за ним. Обход же шёл по
       -- `information_schema` в её собственном порядке и брал ребёнка РАНЬШЕ
       -- родителя — тогда строка ячейки показывала на имя, которого ещё нет, и
       -- вся правка падала `project_document_cells_document_fk`. Воспроизводилось
       -- на любом виде с таблицей: `task`, `run`, `story`.
       AND NOT EXISTS (
             SELECT 1 FROM information_schema.referential_constraints rc
               JOIN information_schema.key_column_usage u
                 ON u.constraint_name = rc.constraint_name
                AND u.constraint_schema = rc.constraint_schema
              WHERE rc.update_rule = 'CASCADE'
                AND u.table_schema = 'public'
                AND u.table_name = k.table_name AND u.column_name = k.column_name)
     ORDER BY k.table_name, k.column_name
  LOOP
    d := 0;
    BEGIN
      EXECUTE format(
        'UPDATE %I SET %I = regexp_replace(%I, $1, $2, ''g'')
          WHERE project_id = $3 AND %I ~ $1', c.t, c.col, c.col, c.col)
        USING образец, стало, п;
      GET DIAGNOSTICS n = ROW_COUNT;
    EXCEPTION WHEN unique_violation THEN
      -- ДВОЙНИК. Прежде здесь стояло удаление ВСЕХ строк со старым именем —
      -- без спроса и без слова о склейке. Пять строк ссылались на старое имя,
      -- одна из них сталкивалась с существующей — и удалялись все пять вместо
      -- «четыре переименовать, одну слить». Итог писался как `строк=0`, то есть
      -- дверь отвечала «ничего не переименовано» там, где потеряла запись.
      IF NOT слить THEN
        RAISE EXCEPTION
          'имя «%» в %.% уже занято: переименование склеило бы две записи в одну',
          стало, c.t, c.col USING ERRCODE = 'unique_violation',
          HINT = 'если это одна и та же сущность, заведённая дважды, — скажите merge';
      END IF;
      -- Со словом о склейке — построчно: переименовывается всё, что может, и
      -- снимается только то, что вправду столкнулось.
      n := 0;
      EXECUTE format('SELECT array_agg(ctid) FROM %I WHERE project_id = $1 AND %I ~ $2',
                     c.t, c.col)
        INTO места USING п, образец;
      FOREACH место IN ARRAY coalesce(места, ARRAY[]::tid[]) LOOP
        BEGIN
          EXECUTE format(
            'UPDATE %I SET %I = regexp_replace(%I, $1, $2, ''g'') WHERE ctid = $3',
            c.t, c.col, c.col)
            USING образец, стало, место;
          n := n + 1;
        EXCEPTION WHEN unique_violation THEN
          EXECUTE format('DELETE FROM %I WHERE ctid = $1', c.t) USING место;
          d := d + 1;
        END;
      END LOOP;
    END;
    IF n > 0 OR d > 0 THEN
      таблица := c.t; колонка := c.col; строк := n; снято := d; RETURN NEXT;
    END IF;
  END LOOP;
END $$ LANGUAGE plpgsql;
"#;

/// Завести таблицы, если их ещё нет. Зовётся один раз при старте.
///
/// Не при каждой пересборке: `CREATE TABLE IF NOT EXISTS` печатает NOTICE, а
/// печатать что-либо в разговоре по stdio нельзя.
pub async fn ensure(pool: &Pool) -> Result<(), crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    client.batch_execute(DDL).await?;
    client.batch_execute(RENAME_IN_COLUMNS).await?;
    // Представления заводятся после таблиц: они их читают.
    client.batch_execute(VIEWS).await?;
    // Своей пачкой — чтобы ожидание замка на снятии не держало за собой схему.
    Ok(client.batch_execute(PHASE_VIEWS).await?)
}

/// Проекции, которые обязаны быть готовы ДО донорской пересборки.
///
/// Донорские проекции читают `task_requirement`; посчитанная после них, она
/// накормила бы их данными прошлого круга — расхождение на один шаг, невидимое
/// глазом и оттого худшее.
pub async fn rebuild_before(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
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
    // Объявленное дверью `task-requirement-add` переживает пересборку: своё
    // выводится из поля «Требования», чужое объявлено и выводиться неоткуда.
    tx.execute("DELETE FROM task_requirement WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
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

/// Снять переделанное: задача закрыта заново, коммитом позже замеченного долга.
///
/// Больше долг не уходит ничем. Задача, выпавшая из плана, своего долга не
/// теряет: читают долг только через план, и выпавшая его не показывает, а
/// вернувшись — показывает прежний. Иначе снять задачу и вернуть её было бы
/// способом погасить долг без переделки.
pub(crate) async fn clear_redone(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
) -> Result<(), crate::db::Fail> {
    client
        .execute(
            "DELETE FROM task_redo r
              WHERE r.project_id = $1
                AND EXISTS (SELECT 1 FROM task_state ts
                             WHERE ts.project_id = r.project_id AND ts.task_id = r.task_id
                               AND ts.state = 'closed' AND ts.closed_at > r.noticed_at)",
            &[&project],
        )
        .await?;
    Ok(())
}

/// Закрытия, какими их видит план перед замером: задача и закрывающий коммит.
///
/// Закрытие опознаётся коммитом, и другого опознания у него нет: подача без
/// коммита не принимается (`closings_without_commit`), объявить задачу
/// закрытой дверью нельзя (`declare_task`).
pub(crate) async fn closings_to_judge(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
) -> Result<Vec<(String, String)>, crate::db::Fail> {
    let rows = client
        .query(
            "SELECT id, coalesce(closing_commit, '') FROM project_plan_tasks
              WHERE project_id = $1 AND state = 'closed'",
            &[&project],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Суд над закрытиями, взятыми до замера, — по фазе, какой её оставил замер.
///
/// Закрытая при закрытой фазе задача получает долг. Судится и отмечается одно и
/// то же закрытие: тот коммит, что был взят, если история всё ещё держит его
/// закрытым, суд его ещё не отметил, а вид задачи отображён на фазу.
/// Сменилось за время замера — судит следующий замер; отметил другой замер —
/// второго суда нет; фазы у вида нет — судить не по чему.
pub(crate) async fn judge_closings(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    pending: &[(String, String)],
) -> Result<u64, crate::db::Fail> {
    if pending.is_empty() {
        return Ok(0);
    }
    let (tasks, commits): (Vec<String>, Vec<String>) = pending.iter().cloned().unzip();
    client
        .execute(
            "WITH j AS (
               SELECT tp.project_id, tp.task_id, tp.open, coalesce(tp.phase, '') AS phase,
                      coalesce(tp.gate, '') AS gate, p.closing_commit, ts.closed_at, ts.seen_at
                 FROM unnest($2::text[], $3::text[]) AS p(task_id, closing_commit)
                 JOIN task_phase tp ON tp.project_id = $1 AND tp.task_id = p.task_id
                 JOIN task_state ts ON ts.project_id = tp.project_id AND ts.task_id = tp.task_id
                                   AND ts.state = 'closed' AND ts.closing_commit = p.closing_commit
                WHERE tp.open IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM task_closing_judged d
                                   WHERE d.project_id = tp.project_id AND d.task_id = tp.task_id
                                     AND d.closing_commit = p.closing_commit)
             ), mark AS (
               INSERT INTO task_closing_judged (project_id, task_id, closing_commit)
               SELECT project_id, task_id, closing_commit FROM j
               ON CONFLICT DO NOTHING
             )
             INSERT INTO task_redo (project_id, task_id, noticed_at, phase, gate)
             SELECT project_id, task_id, coalesce(nullif(closed_at, 0), seen_at), phase, gate
               FROM j WHERE open IS FALSE
             ON CONFLICT (project_id, task_id) DO NOTHING",
            &[&project, &tasks, &commits],
        )
        .await.map_err(Into::into)
}

/// Замер, вокруг которого судятся закрытия: взятые ДО него — по фазе, какой он
/// её оставил. Порядок живёт здесь одним местом, а не у каждого, кто меряет.
///
/// Соединение берётся на каждый запрос и сразу отдаётся: держать его, пока замер
/// берёт своё, значит занимать два слота на замер — а при восьми одновременных
/// замерах это тупик, уже однажды снятый в пересборке.
pub(crate) async fn judging_closings<T, F, Fut>(
    pool: &Pool,
    project: &str,
    measure: F,
) -> Result<T, crate::db::Fail>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::db::Fail>>,
{
    let pending = closings_to_judge(&*crate::db::conn(pool).await?, project).await?;
    let out = measure().await?;
    judge_closings(&*crate::db::conn(pool).await?, project, &pending).await?;
    Ok(out)
}

/// Закрытия без коммита в подаче. Закрытие опознаётся коммитом: без него суд
/// порядка не отличит новое закрытие от прежнего, и такая подача не принимается.
pub(crate) fn closings_without_commit(states: &[(String, String, String, i64)]) -> Vec<String> {
    states
        .iter()
        .filter(|(_, state, commit, _)| state == "closed" && commit.trim().is_empty())
        .map(|(id, _, _, _)| id.clone())
        .collect()
}

/// Пересобрать проекции этого сервера. Возвращает счёт по каждой.
pub async fn rebuild(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    // Отладка: какой из запросов упал, видно по порядку в логе.
    // Пересборка идёт ОДНОЙ транзакцией.
    //
    // Каждая проекция считается как «удалить и записать заново». Без транзакции
    // между этими двумя шагами есть окно, в котором таблица пуста, — и читатель,
    // попавший в него, видит не «пересчитывается», а «ничего нет». Интерфейс на
    // этом уже показал 20 неизвестных вместо 1758 и выглядел правдой.
    let mut client = crate::db::conn(pool).await?;
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
                    -- Имя — ПЕРВОЕ имя задачи в поле, а не всё значение:
                    -- «`M1-T7` — критерий выхода вехи» давало родителя с
                    -- пояснением, связь с зеркалом рвалась, и ни один гейт
                    -- этого не видел. Не нашлось имени — значение как есть.
                    coalesce(max(CASE WHEN f.name ILIKE 'Родительская%' OR f.name = 'Пара'
                                      THEN coalesce(substring(f.value from '[MmVv][0-9]+-[Tt][0-9A-Za-z]+(?:-[0-9A-Za-z]+)*'), f.value) END), ''),
                    coalesce(max(CASE WHEN f.name ILIKE 'Этап родителя%' OR f.name = 'Веха'
                                      THEN coalesce(substring(f.value from '^[[:space:]`]*([A-Za-z]+[0-9]+)(?![A-Za-z0-9-])'), f.value) END), ''),
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
    //
    // СНОС — ЗДЕСЬ ЖЕ, а не в чужом проходе. `NOT EXISTS` делал вставку
    // однократной: строка, однажды легшая в план, больше не обновлялась НИКОГДА.
    // Снятый документ красной задачи уходил из `red_task`, а его строка в плане
    // оставалась призраком — её считали пункты гейта и на неё ссылались
    // переписанные ссылки. Исправленная «Пара» или «Веха» не доезжала по той же
    // причине: заголовок и веха замирали на первом значении.
    //
    // Снос вплотную к вставке, в одной транзакции: окна, в которое план виден
    // без трека проверок, нет — и призраков нет.
    tx.execute(
        "DELETE FROM project_plan_tasks WHERE project_id = $1 AND entity_kind = 'red-task'",
        &[&project],
    )
    .await?;
    let red_in_plan = tx
        .execute(
            "INSERT INTO project_plan_tasks
                (project_id, id, milestone_id, ord, title, entity_kind, entity_name, size, kind, state, closing_commit)
             SELECT r.project_id, r.id, r.milestone, 0,
                    r.id || ' · проверки для ' || r.parent_task,
                    'red-task', r.id, '', 'red', 'not_started', ''
               FROM red_task r
              WHERE r.project_id = $1
                -- Красная задача, не назвавшая существующей вехи, роняла ВЕСЬ
                -- пересчёт о внешний ключ: набор tot объявляет её вехой ноль
                -- раз из семидесяти семи. Пропуск здесь виден в числе строк, а
                -- падение не видно ничем, кроме отказа всей ручки.
                AND EXISTS (SELECT 1 FROM project_plan_milestones ms
                             WHERE ms.project_id = r.project_id AND ms.id = r.milestone)
             ON CONFLICT (project_id, id) DO UPDATE SET milestone_id = EXCLUDED.milestone_id,
               ord = EXCLUDED.ord, title = EXCLUDED.title, entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, size = EXCLUDED.size, kind = EXCLUDED.kind,
               state = EXCLUDED.state, closing_commit = EXCLUDED.closing_commit, origin = 'projected'
             WHERE project_plan_tasks.origin = 'declared'",
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

    // Долг «закрыта не в свой черёд» здесь только СНИМАЕТСЯ. Ставится он замером
    // (`judge_closings`): судить закрытие по гейту, измеренному до него, значило
    // бы судить по прошлому.
    clear_redone(&tx, project).await?;

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

    // ── Над чем считано заявленное число ─────────────────────────────────────
    // Без этого сверка кричит волком: `172` считано по ФАЙЛАМ решений, а таблица
    // держит записи; `272` — по функциональным, а таблица держит FR и NFR
    // вместе. Два ложных крика утопят единственный настоящий.
    tx.execute("DELETE FROM claim_subject WHERE project_id = $1", &[&project]).await?;
    let subjects = tx
        .execute(
            "INSERT INTO claim_subject (project_id, name, subject, counts, note, entity_kind) VALUES
               ($1, 'constitution.md', 'articles',
                'SELECT count(*) FROM project_articles WHERE project_id = $1', '', 'constitution'),
               ($1, 'strs.md', 'needs',
                'SELECT count(*) FROM project_needs WHERE project_id = $1', '', 'strs'),
               ($1, 'srs.md', 'requirements',
                'SELECT count(*) FROM project_requirements WHERE project_id = $1 AND id NOT LIKE $$NFR-%$$',
                'заявлено про функциональные; таблица держит FR и NFR вместе', 'srs'),
               ($1, 'srs.md', 'nfr',
                'SELECT count(*) FROM project_requirements WHERE project_id = $1 AND id LIKE $$NFR-%$$', '', 'srs'),
               ($1, 'ui-spec.md', 'screens',
                'SELECT count(*) FROM project_screens WHERE project_id = $1', '', 'ui-spec'),
               ($1, 'adr/', 'decisionFiles',
                'SELECT count(*) FROM project_documents WHERE project_id = $1
                   AND entity_kind IN ($$decision$$, $$decision-template$$)',
                'считано по файлам вместе с шаблоном; записей решений на одну меньше', 'decision'),
               -- «Подсистемы» считаются по РАЗНЫМ множествам у разных
               -- документов: у требований — свои области, у проверок — свои, и
               -- совпадение их чисел ничего не доказывает. Предмет объявлен
               -- каждому свой.
               ($1, 'srs.md', 'areas',
                'SELECT count(DISTINCT area) FROM project_requirements WHERE project_id = $1 AND area <> $$$$',
                '', 'srs'),
               ($1, 'test-cases.md', 'areas',
                'SELECT count(DISTINCT area) FROM project_checks WHERE project_id = $1 AND area <> $$$$',
                '', 'test-cases'),
               ($1, 'test-cases.md', 'checks',
                'SELECT count(*) FROM project_checks WHERE project_id = $1', '', 'test-cases')
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
             source = 'SELECT 1 FROM fact_push WHERE project_id = $1 AND fact = ''worktree''',
             durable = false
           WHERE kind = 'task' AND name = 'в работе' AND fact = ''"#,
        &[],
    )
    .await?;
    // Набор, заведённый до колонки `durable`, уже имеет факт — правило выше
    // его не трогает. Свойство ступени проставляется отдельно и однократно.
    //
    // Ненакопительных ступеней две, и обе такие ПО ЗАМЫСЛУ.
    // «в работе» — ветка, заведённая сейчас: её сносят, и ступень гаснет.
    // «проанализирована» — вердикт предполёта на ТЕКУЩЕЙ ревизии: правка
    // задачи гасит её нарочно, в этом весь каскад. Считать такую ступень
    // пропущенной — значит звать «ступенью через голову» то, что на деле
    // «устарело после правки»; первое зовёт разбираться, второе — перепройти
    // предполёт, и очередь предполёта об этом уже говорит.
    tx.execute(
        "UPDATE kind_status SET durable = false
          WHERE kind = 'task' AND name IN ('в работе', 'проанализирована') AND durable",
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

    let retired = tx
        .query_one("SELECT count(*) FROM term_retired WHERE project_id = $1", &[&project])
        .await?
        .get::<_, i64>(0);

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
    // Соединение отпускается ДО связей: они берут своё и свой замок, и два
    // сразу на одну сборку — тот самый способ запереть пул, что чинится здесь.
    drop(client);
    // Связи считаются ПОСЛЕДНИМИ и здесь, а не в пересборке: красные задачи
    // кладёт эта сборка, и до неё их в плане нет. Связь, посчитанная раньше,
    // легла бы на пустоту и показала бы ноль как «сошлось».
    let relations = crate::reproject::relations::project(pool, project).await?;

    Ok(json!({
        "relations": relations,
        "entity_event": events,
        "norm_version": versions,
        "measurement": measurements,
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
pub(crate) async fn next_task(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
                    tp.phase, tp.gate, tp.open,
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
                               WHERE c.project_id = $1 AND r.task_id = o.id), '') AS checks,
                    tp.phase_ord
               FROM open o
               LEFT JOIN task_phase tp ON tp.project_id = $1 AND tp.task_id = o.id
              WHERE o.id NOT IN (SELECT task_id FROM blocked_by_task)
                AND o.id NOT IN (SELECT task_id FROM blocked_by_milestone)
              ORDER BY (tp.open IS NOT TRUE), o.milestone_id, o.ord
              LIMIT 1",
            &[&project],
        )
        .await?;

    let Some(r) = rows.first() else {
        // ПЕРЕДЕЛКА — ТОЖЕ РАБОТА, и когда другой не осталось, она и есть ответ.
        //
        // Задача, закрытая при закрытой фазе, опиралась на то, чего ещё не
        // стояло. Пока фаза закрыта, брать её нельзя — тот же барьер; открылась
        // — её надо переделать, и сказать об этом больше некому: в плане она
        // числится закрытой и в перечень незакрытых не попадает никогда.
        let debt = client
            .query(
                "SELECT r.task_id, r.phase, r.gate, t.title, tp.phase_ord
                   FROM task_redo r
                   JOIN task_phase tp ON tp.project_id = r.project_id AND tp.task_id = r.task_id
                   LEFT JOIN project_plan_tasks t
                     ON t.project_id = r.project_id AND t.id = r.task_id
                  WHERE r.project_id = $1 AND tp.open IS TRUE
                  ORDER BY t.milestone_id, t.ord, r.task_id",
                &[&project],
            )
            .await?;
        if let Some(first_one) = debt.first() {
            if let Some(holds_rows) = ladder_holds(&*client, project, first_one.get(4)).await? {
                return Ok(json!({ "task": null, "candidate": first_one.get::<_, String>(0), "redo": true,
                                  "why": holds_rows["why"], "ladder": holds_rows }));
            }
            return Ok(json!({
                "task": {
                    "id": first_one.get::<_, String>(0),
                    "title": first_one.get::<_, Option<String>>(3),
                    "phase": first_one.get::<_, String>(1),
                    "gate": first_one.get::<_, String>(2),
                    "redo": true,
                },
                "why": format!(
                    "незакрытых задач нет, но {} закрыты не в свой черёд: их фаза тогда не была \
                     открыта, и то, на что работа опиралась, ещё не стояло. Фаза открыта сейчас — \
                     переделать их можно и нужно. Долг гасится НОВЫМ закрывающим коммитом, а не \
                     словом", debt.len()),
                "redo": debt.iter().map(|r| json!({
                    "id": r.get::<_, String>(0), "phase": r.get::<_, String>(1),
                    "gate": r.get::<_, String>(2) })).collect::<Vec<_>>(),
            }));
        }
        return Ok(json!({ "task": null, "why": "незакрытых задач с закрытыми зависимостями нет" }));
    };
    let id: String = r.get(0);
    let phase: Option<String> = r.get(5);
    let gate: Option<String> = r.get(6);
    let phase_ord: Option<i32> = r.get(11);

    // БАРЬЕР ФАЗ. Прежде здесь стоял барьер красной фазы: «есть незакрытые
    // красные — задачи не выдаём». Он мерил не то. На `myack` все 82 красные
    // задачи закрыты, G3 при этом красен, и барьер пропускал `M0-T12` — задачу
    // Ф4, фазы, которую G3 ещё не открыл. Заодно он не выдавал и сами красные
    // задачи: пока хоть одна открыта, ответом было `null` на любую.
    //
    // Мерится теперь фаза, а не вид: задача выдаётся, когда её фаза открыта, то
    // есть когда пройден каждый предшествующий гейт. Ф3 при красном G3 открыта —
    // красен он оттого, что в ней идёт работа.
    //
    // Отказ НАЗЫВАЕТ, чем держится: фазу, её гейт, состояние гейта и его красные
    // пункты поимённо. Отказ без имён отправляет искать причину руками.
    if r.get::<_, Option<bool>>(7) != Some(true) {
        let Some(phase) = phase else {
            // «Цепочки фаз нет вовсе» и «цепочка есть, а этот вид ей не назван» —
            // разное, и чинится разным. Один ответ на оба отправлял бы
            // объявлять вид там, где объявлять его ещё некуда.
            let phases: i64 = client.query_one("SELECT count(*) FROM phase", &[]).await?.get(0);
            return Ok(json!({
                "task": null,
                "candidate": id,
                "why": if phases == 0 {
                    "цепочка фаз не объявлена ни одной фазой: порядок работ сказать нечем, \
                     и это не «можно всё». Фаза объявляется дверью `phase-set`: \
                     `phase`, `ord`, `title`, `gate`, `taskKind`".to_owned()
                } else {
                    format!("вид задачи «{}» не отображён ни на одну фазу: отображение НЕ ОБЪЯВЛЕНО, \
                             и это не «можно всё». Объявляется дверью `phase-set` доводом `taskKind`",
                            r.get::<_, Option<String>>(3).unwrap_or_default())
                },
            }));
        };
        // Держит первый из предшествующих гейтов, который не пройден: именно он
        // откроет фазу, и именно его пункты — работа, которую надо взять вместо
        // этой задачи.
        let held = client
            .query_opt(
                "SELECT CASE WHEN p2.gate = '' THEN 'гейт фазы ' || p2.id || ' не объявлен'
                             ELSE p2.gate END,
                        (SELECT string_agg(g.id, ' · ' ORDER BY g.id) FROM project_gates g
                          WHERE g.project_id = $1 AND g.phase = p2.gate AND g.state = 'failed')
                   FROM phase p2
                  WHERE p2.ord < (SELECT ord FROM phase WHERE id = $2)
                    AND coalesce((SELECT s.computed FROM gate_state s
                                   WHERE s.project_id = $1 AND s.gate = p2.gate), 'open') <> 'passed'
                  ORDER BY p2.ord LIMIT 1",
                &[&project, &phase],
            )
            .await?;
        let holder: Option<String> = held.as_ref().map(|h| h.get(0));
        let named: Option<String> = held.as_ref().and_then(|h| h.get(1));
        // ПРОТУХШИЙ ДАТЧИК — НЕ НАХОДКА, а непрогнанный `mh sense`, и барьер
        // обязан говорить это сам. Ручка `gate` такую оговорку несёт давно:
        // пункт, читающий несвежий род фактов, красен не потому, что что-то
        // нашлось, а потому что мерить сейчас нечем. Пока это красило доску,
        // цена была в потерянном часе; теперь оно ПРИДЕРЖИВАЕТ РАБОТУ, и отказ
        // без оговорки отправляет искать находку, которой нет.
        let stale: Vec<String> = client
            .query(
                "SELECT s.fact FROM sensor s
                  WHERE s.project_id = $1 AND NOT fact_fresh($1, s.fact) ORDER BY s.fact",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect();
        return Ok(json!({
            "task": null,
            "candidate": id,
            "why": format!("задача принадлежит фазе {phase}, а фаза не открыта: {} ещё не пройден",
                           holder.clone().unwrap_or_else(|| "предшествующий гейт".into())),
            "phase": {
                "phase": phase,
                "gate": gate,
                "heldBy": holder,
                // «Красен» и «не мерян» — разное, и складывать их нельзя: пустой
                // список красных пунктов при непройденном гейте значит, что его
                // ещё не мерили.
                "gateState": held.as_ref().map(|_| if named.is_some() { "failed" } else { "open" }),
                "redItems": named,
                "staleSensors": if stale.is_empty() { Value::Null } else {
                    json!({ "facts": stale,
                            "why": "эти роды фактов не свежи. Пункты, читающие их, красны НЕ ПО \
                                    НАХОДКЕ, а потому что мерить сейчас нечем, — и барьер держит \
                                    работу за них. Прогоните `mh sense`: может статься, держать \
                                    нечего" })
                },
            },
            // ОТКАЗ ОБЯЗАН НАЗЫВАТЬ РАБОТУ, А НЕ ТОЛЬКО ПРЕПЯТСТВИЕ.
            //
            // Барьер отвечал `task: null` и перечислял красные пункты — и на
            // этом обрывался. У `myack` задачи ложатся только в Ф3 и Ф4, обе
            // закрыты, а работа Ф0–Ф2 задачей не бывает вовсе: открытой задачи
            // нет ни одной, и спрашивающий упирался в тупик. Границы фаз при
            // этом двигать нельзя — двигается способ выйти: делать то, что
            // фазой НИЖЕ, а сделанное не в свой черёд переделывать.
            //
            // Что делать фазой ниже, лестница знает и без нас — она это и
            // считает. Держать здесь второй ответ на тот же вопрос значило бы
            // завести два порядка работ, расходящихся молча.
            "instead": step_instead(&*client, project).await?,
        }));
    }
    if let Some(holds_rows) = ladder_holds(&*client, project, phase_ord).await? {
        return Ok(json!({ "task": null, "candidate": id, "why": holds_rows["why"], "ladder": holds_rows }));
    }
    // ДОЛГ ВИДЕН СРАЗУ, А НЕ В КОНЦЕ ОЧЕРЕДИ.
    //
    // Предъявлять переделки только когда незакрытых задач не осталось значит
    // молчать о них ровно столько, сколько идёт работа: у `myack` это 99
    // переделок за 122 задачами. Агент прошёл бы весь черёд и лишь потом узнал,
    // что девяносто девять шагов надо повторить. Очередь они при этом не
    // перехватывают — задача выдаётся та же; долг едет рядом числом и именами.
    let debt = client
        .query(
            "SELECT r.task_id FROM task_redo r
               JOIN task_phase tp ON tp.project_id = r.project_id AND tp.task_id = r.task_id
               LEFT JOIN project_plan_tasks t ON t.project_id = r.project_id AND t.id = r.task_id
              WHERE r.project_id = $1 AND tp.open IS TRUE ORDER BY t.milestone_id, t.ord, r.task_id",
            &[&project],
        )
        .await?;
    let mut answer = json!({
        "task": {
            "id": id,
            "milestone": r.get::<_, String>(1),
            "title": r.get::<_, String>(2),
            "kind": r.get::<_, Option<String>>(3),
            "size": r.get::<_, Option<String>>(4),
            "phase": phase,
            "requirements": r.get::<_, i64>(8),
            "requirementText": r.get::<_, String>(9),
            "checks": r.get::<_, String>(10),
        }
    });
    if !debt.is_empty() {
        answer["redo"] = json!({
            "count": debt.len(),
            "tasks": debt.iter().take(20).map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
            "why": "эти задачи закрыты не в свой черёд: их фаза тогда не была открыта, и то, на \
                    что работа опиралась, ещё не стояло. Фаза открыта сейчас — переделать их \
                    можно. Долг гасится НОВЫМ закрывающим коммитом, а не словом",
        });
    }
    Ok(answer)
}

/// Почему задача не берётся: перечень того, чего она ждёт.
pub(crate) async fn task_blockers(pool: &Pool, project: &str, task: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // ПУСТОЙ СПИСОК ВМЕСТО ОТВЕТА ВРЁТ. Ручку звали без задачи, и она отвечала
    // `{"task": "", "waitsForMilestones": [], "waitsForTasks": []}` — неотличимо
    // от «ничто не блокирует». Тот же приём, что у `kinds-due` («у
    // неспроецированных — не ноль, а пусто»), здесь применён не был.
    if task.trim().is_empty() {
        return Ok(json!({
            "task": Value::Null,
            "why": "задача не названа, и это НЕ «ничто не блокирует»: спрашивать не о чем. \
                    Текущую задачу даёт `next-task`, перечень — `task-list`.",
        }));
    }
    let known: i64 = client
        .query_one(
            "SELECT count(*) FROM project_plan_tasks WHERE project_id = $1 AND id = $2",
            &[&project, &task],
        )
        .await?
        .get(0);
    if known == 0 {
        return Ok(json!({
            "task": task,
            "why": format!("задачи «{task}» в наборе нет: пустой перечень здесь значил бы, \
                            что она ничем не блокирована, а её просто не существует"),
        }));
    }
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
    // ФАЗА — ТОЖЕ БЛОКИРОВКА, и спрашивают о ней здесь.
    //
    // Барьер фаз живёт в `next-task`, а задачу исполнителю называет человек — и
    // зовут его мимо барьера. Тогда дверь «почему задача не берётся» отвечала
    // «ничто не держит» о задаче фазы, которую гейт ещё не открыл: два ответа об
    // одном, и берут тот, что короче.
    let phase = client
        .query_opt(
            "SELECT tp.phase, tp.gate, tp.open,
                    (SELECT CASE WHEN p2.gate = '' THEN 'гейт фазы ' || p2.id || ' не объявлен'
                                 ELSE p2.gate END
                       FROM phase p2
                      WHERE p2.ord < tp.phase_ord
                        AND coalesce((SELECT s.computed FROM gate_state s
                                       WHERE s.project_id = $1 AND s.gate = p2.gate), 'open') <> 'passed'
                      ORDER BY p2.ord LIMIT 1)
               FROM task_phase tp WHERE tp.project_id = $1 AND tp.task_id = $2",
            &[&project, &task],
        )
        .await?;
    let phase_open = phase.as_ref().and_then(|r| r.get::<_, Option<bool>>(2));
    let held_by: Option<String> = phase.as_ref().and_then(|r| r.get(3));
    let free = tasks.is_empty() && milestones.is_empty() && phase_open == Some(true);
    Ok(json!({
        "task": task,
        "phase": match &phase {
            Some(r) => json!({
                "phase": r.get::<_, Option<String>>(0),
                "gate": r.get::<_, Option<String>>(1),
                "open": phase_open,
                "opensWith": held_by,
                "why": match phase_open {
                    Some(true) => "фаза задачи открыта".to_owned(),
                    Some(false) => format!("фаза не открыта: её открывает {}",
                                           held_by.clone().unwrap_or_else(|| "предшествующий гейт".into())),
                    None => "вид задачи не отображён ни на одну фазу: отображение не объявлено,                              и это не «можно всё»".to_owned(),
                },
            }),
            None => Value::Null,
        },
        "waitsForTasks": tasks.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
        "waitsForMilestones": milestones.iter().map(|r| json!({
            "milestone": r.get::<_, String>(0),
            "said": r.get::<_, String>(1),
            "openTasks": r.get::<_, i64>(2),
        })).collect::<Vec<_>>(),
        // Пустота СКАЗАНА СЛОВОМ: два пустых списка и «ничто не держит» — разное
        // только для того, кто знает, что задача существует и была спрошена.
        "why": if free { "ничто не держит: все зависимости закрыты и фаза задачи открыта" } else { "" },
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
pub(crate) async fn measure_gates(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let mut conn = crate::db::conn(pool).await?;
    // ВЕСЬ КРУГ — ОДНОЙ ТРАНЗАКЦИЕЙ, и это не про скорость.
    //
    // Строки писались по одной, и всякий, кто читал гейт в это время, складывал
    // вердикт из пунктов двух разных кругов — состояния, которого не было ни в
    // один момент. Пока замер кормил только доску, цена была косметической;
    // барьер фаз решает по нему, выдавать ли работу, и полукруг выдал бы задачу
    // фазы, гейт которой ещё не дописан. Читатель видит либо прошлый круг
    // целиком, либо этот целиком.
    //
    // Внутри круга порядок при этом сохраняется: пункт, читающий чужие замеры —
    // а таковы оба пункта про порядок фаз, и `corpus` меряется последним, —
    // видит свежие числа этого круга, а не прошлого.
    let client = conn.transaction().await?;
    // Объявление — общее, замер — проектный. Пункты берутся из `gate_item`, и
    // проект, у которого их ещё не было, получает все сразу: гейт отвечает на
    // вопрос «можно ли идти дальше», и ответ не должен зависеть от того, кто как
    // завёл проверки у себя.
    let rows = client
        .query("SELECT phase, kind, query, why, id, subject_query, subject_why, since
                  FROM gate_item ORDER BY phase, id", &[])
        .await?;
    let now = now_ms();
    let mut measured = 0usize;
    let mut failed = 0usize;
    let mut unwritten: Vec<Value> = Vec::new();
    for r in &rows {
        let phase: String = r.get(0);
        let kind: String = r.get(1);
        let query: Option<String> = r.get(2);
        // Отмена и отметка адресуются ИМЕНЕМ, а не заголовком: заголовок
        // переписывают, и прежде всякая правка формулировки роняла отмену.
        let id: String = r.get(4);
        // Неприменимый пункт НЕ ИСПОЛНЯЕТСЯ. Исполнить и прощать значило бы
        // считать нарушением то, чего в этом проекте не существует: `.sqlx` у
        // проекта без sqlx не «не снята» — её тут не бывает.
        // ОБЪЯВЛЕННЫЙ ЗАПРОС ИСПОЛНЯЕТСЯ ПОД ТОЧКОЙ ВОЗВРАТА, и без неё круг в
        // транзакции был бы хуже круга без транзакции.
        //
        // Запрос пункта пишет не харнес, а набор, и сломанный среди них —
        // случай рядовой: ради него и заведены ветка «запрос не выполнился» и
        // самотест. В транзакции же первая же ошибка переводит её в состояние
        // aborted, и ВСЁ следующее отказывает: ветка ловит ошибку и идёт
        // дальше, а ближайшая запись замера роняет круг целиком. Сто двадцать
        // восемь замеров теряются из-за одного кривого запроса, и каждый
        // следующий пункт при этом врёт на себя — «запрос не выполнился»
        // вместо «сосед отравил транзакцию».
        //
        // Точка возврата снимается ВСЕГДА, а не по ошибке: замер обязан только
        // читать, и откат ему нечего терять. Запись замера идёт уже после неё.
        client.batch_execute("SAVEPOINT замер").await?;
        // ПРЕДМЕТ СПРАШИВАЕТСЯ ПЕРВЫМ. Пустой предмет — установленное отсутствие
        // сущностей пункта, и пункт пройден. Поэтому предмет обязан быть
        // непустым, пока отсутствие не установлено: датчик не свеж, сборка не
        // прогонялась. Есть сущности — пункт мерится, и красное только чинится.
        let subject: String = r.get(5);
        let subject_rows = if subject.trim().is_empty() {
            Ok(1)
        } else {
            client.query(subject.as_str(), &[&project]).await.map(|rows| rows.len())
        };
        let entry = match subject_rows {
            Ok(0) => {
                let w: String = r.get(6);
                json!({ "computed": "passed", "violations": 0, "detail": [],
                        "why": if w.trim().is_empty() {
                            "сущностей пункта в проекте нет: проверять нечего".to_owned()
                        } else { w } })
            }
            Ok(_) => measure_item(&client, project, &kind, query.as_deref(), r).await?,
            // Запрос предмета, который не исполнился, — не «предмет пуст» и не
            // повод мерить: транзакция после ошибки прервана до точки возврата.
            Err(e) => json!({ "computed": "unknown",
                              "why": format!("запрос предмета не исполнился: {}", e.says()) }),
        };
        // Откат ВСЕГДА: замер обязан только читать, и терять ему нечего. Заодно
        // он отменяет то, что объявленный запрос успел написать: дверь проверяет
        // запрос подготовкой, а `INSERT` подготавливается не хуже `SELECT`.
        client.batch_execute("ROLLBACK TO SAVEPOINT замер; RELEASE SAVEPOINT замер").await?;
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
        // было. ОБЪЯВЛЕНИЕ РЯДОМ НЕ КЛАДЁТСЯ: копия отставала от `gate_item`, а
        // читал её только ответ двери — и отдавал пустую пробу, потому что её
        // копию не переписывал никто. Запросы ступеней берут отсюда состояние,
        // а правило — из объявления.
        // ЗАПИСЬ — ПОД СВОЕЙ ТОЧКОЙ ВОЗВРАТА, и это не та же точка, что у замера.
        //
        // Строку замера таблица отвергала по своим ограничениям — четыре пункта
        // из сорока однажды не мерились вовсе, — и стоило это четырёх пунктов;
        // в транзакции без этой точки стоило бы всего круга. Ограничений тех
        // больше нет вместе с колонками объявления, но отказ записи бывает и
        // другой, а круг обязан пережить его тем же способом.
        client.batch_execute("SAVEPOINT запись").await?;
        let written = client
            .execute(
                "INSERT INTO project_gates (project_id, phase, state, violations, detail,
                                            result, checked_at, id)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 ON CONFLICT (project_id, phase, id) DO UPDATE SET
                   state = EXCLUDED.state, violations = EXCLUDED.violations,
                   detail = EXCLUDED.detail, result = EXCLUDED.result,
                   checked_at = EXCLUDED.checked_at",
                &[&project, &phase, &flat, &violations, &detail, &entry, &now, &id],
            )
            .await;
        match written {
            Ok(_) => {
                client.batch_execute("RELEASE SAVEPOINT запись").await?;
                measured += 1;
            }
            // Пункт, чью строку не принимает таблица, остаётся НЕПОМЕРЕННЫМ и
            // назван: прежняя строка его замера не трогается, а круг идёт дальше.
            Err(e) => {
                client.batch_execute("ROLLBACK TO SAVEPOINT запись; RELEASE SAVEPOINT запись").await?;
                if flat == "failed" {
                    failed -= 1;
                }
                unwritten.push(json!({ "phase": phase, "id": id, "why": e.says() }));
            }
        }
    }
    // СИРОТЫ СНИМАЮТСЯ ТЕМ ЖЕ КРУГОМ.
    //
    // Перечень пунктов читается в начале круга; снятие пункта дверью в середине
    // круга этот перечень не меняет, и замер снятого записывался заново — а
    // убирала его только чистка при старте. Строка жила до перезапуска,
    // считалась в состояние гейта и держала фазу закрытой правилом, которого
    // не найти.
    //
    // Это БЕЗОПАСНОЕ направление чистки, и различие тут не словесное. Та, что
    // однажды ела объявления, снимала ОБЪЯВЛЕНИЯ по отсутствию замеров — и
    // стирала только что заведённый пункт, у которого замера ещё не было. Эта
    // снимает ЗАМЕРЫ по отсутствию объявления и не может задеть ничего, кроме
    // строки, чьего правила больше нет.
    // ПОРЯДОК КЛЮЧА — И ЗДЕСЬ. Круг выше берёт строки `ORDER BY phase, id`, а эта
    // чистка шла в порядке кучи и целилась ровно в то, чего круг не трогал: у
    // сироты нет объявления, значит её замок берётся последним и не по ключу.
    // Этого хватает на цикл — сосед держит строку, которую хочет эта, и ждёт ту,
    // что она уже держит.
    let orphans = client
        .execute(
            "DELETE FROM project_gates
              WHERE (phase, id) IN (SELECT g.phase, g.id FROM project_gates g
                                     WHERE g.project_id = $1
                                       AND NOT EXISTS (SELECT 1 FROM gate_item i
                                                        WHERE i.phase = g.phase AND i.id = g.id)
                                     ORDER BY g.phase, g.id FOR UPDATE)
                AND project_id = $1",
            &[&project],
        )
        .await?;
    client.commit().await?;
    Ok(json!({ "measured": measured, "failed": failed, "at": now,
               "orphansPurged": orphans,
               "unwritten": unwritten.len(), "unwrittenItems": unwritten }))
}

/// Один пункт: чем меряют — тем и меряется.
async fn measure_item(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    query: Option<&str>,
    r: &tokio_postgres::Row,
) -> Result<Value, crate::db::Fail> {
    let kind = kind.to_owned();
        let why_col: String = r.try_get("why").unwrap_or_default();
        let entry = if kind == "unknown" || kind == "manual" {
            json!({ "computed": "unknown",
                    "why": if why_col.is_empty() {
                        if kind == "manual" { "проиграно ли — не записано" } else { "требуется, но машинного способа нет" }
                    } else { why_col.as_str() } })
        } else if kind == "query" {
            {
                // Запрос гейта исполняется ОБЩИМ исполнителем — тем же, что у
                // пункта готовности и ступени лестницы. Свой здесь мерил бы не
                // то, что обещано пунктом, и разошёлся бы молча.
                let sql = query;
                {
                    let since: i64 = r.try_get("since").unwrap_or(0);
                    let v = execute_method_upto(client, project, "query", sql.unwrap_or(""), 200, since).await;
                    // `why` — про то, ПОЧЕМУ запрос не выполнился; это про
                    // ЭТОТ круг замера и потому здесь. Чем пункт меряет
                    // (`means`) — объявление, и его отдаёт дверь из `gate_item`:
                    // снимок объявления в замере отставал на круг.
                    json!({
                        "computed": v.state, "violations": v.violations, "detail": v.detail,
                        "why": v.why,
                    })
                }
            }
        } else {
            // ПОДПИСИ ГЕЙТА БОЛЬШЕ НЕТ. Гейт автоматический: он закрыт, когда
            // выполнены все его условия, и подпись человека под этим ничего не
            // добавляла — только откладывала. Ветка остаётся отказом, а не
            // тишиной: род пункта, которого машина не знает, — дефект объявления.
            json!({
                "computed": "unknown",
                "why": format!("род пункта «{kind}» машине не известен: мерить им нечем"),
            })
        };
        Ok(entry)
}

pub(crate) async fn gate(pool: &Pool, project: &str, phase: Option<&str>) -> Result<Value, crate::db::Fail> {
    let mut conn = crate::db::conn(pool).await?;
    // ВЕРДИКТ И ЕГО ОСНОВАНИЕ ЧИТАЮТСЯ ОДНИМ СНИМКОМ. Числа гейта берутся у
    // `gate_state`, а пункты, из которых они сложены, — у `project_gates`; это
    // два запроса, и с тех пор как замер пишется одной транзакцией, они ложатся
    // по разные стороны её фиксации. Тогда ручка напечатала бы «красных 3» над
    // перечнем, в котором красного нет ни одного. Пока счёт считался по самому
    // перечню, такое было невозможно; вернуть эту невозможность стоит одной
    // транзакции на чтение.
    let client = conn.transaction().await?;
    // ПЕРЕСБОРКА, УПАВШАЯ НА ПОЛПУТИ, оставляет проекции недособранными. Замер
    // по ним посчитан, лежит и читается как настоящий: гейт отвечал числами,
    // которые не значили ничего, и час ушёл на тридцать одну ложную находку.
    //
    // Числа при этом НЕ ПРЯЧУТСЯ: спрятать их значило бы потерять и то, что
    // всё-таки посчиталось. Они отдаются вместе со словом о том, что доверять
    // им нельзя, — и это слово стоит первым.
    let stale = last_reproject(&client, project).await.filter(|(ok, _, _)| !*ok);
    // ПРОТУХШИЙ ДАТЧИК — НЕ НАХОДКА, А НЕПРОГНАННЫЙ `mh sense`. Первый запуск
    // после смены бинарника даёт пригоршню краснот «датчик подавал и ПРОТУХ», и
    // человек, не знающий этого, идёт их чинить. Само поведение верное — но
    // сказать, чем оно лечится, обязан тот, кто его показывает.
    let stale_sensors: Vec<String> = client
        .query(
            "SELECT s.fact FROM sensor s
              WHERE s.project_id = $1 AND NOT fact_fresh($1, s.fact) ORDER BY s.fact",
            &[&project],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    // ЧИТАЕТСЯ сохранённое, а не считается заново. Замер делает `measure_gates`,
    // и делает его при изменении набора; страница показывает результат и время,
    // когда он получен. Непосчитанный пункт называется непосчитанным.
    let rows = client
        .query(
           // Правило отдаётся ЦЕЛИКОМ и ИЗ ОБЪЯВЛЕНИЯ, а не из замера. Прежде
           // ответ нёс только замер, и починить пункт, не имея прямого доступа
           // к базе, было нельзя: имена восстанавливались сопоставлением
           // заголовков вручную, а запросов не видел никто.
           //
           // Потом замер завёл СВОЮ копию правила, и копия отстала: пробу он не
           // переписывал ни разу — её колонку заполняли только переезды схемы, —
           // и `mh call gate` отдавал пустую пробу у всех ста шестидесяти
           // пунктов, говоря рядом `probeRuns: true`. Прочитанное вполовину
           // хуже непрочитанного: оно выглядит полным. Предмет и граница `since`
           // не отдавались вовсе, а именно они решают, когда пункт говорит
           // «судить нечем».
           // СОЕДИНЕНИЕ ПОЛНОЕ, и это не про аккуратность. Объявление без замера
           // — «ещё не мерили», и оно обязано быть видно: иначе заведённый пункт
           // не существует до первого круга. Замер без объявления — сирота, и он
           // обязан быть виден ТЕМ ЖЕ ответом: его считает `gate_state` и им
           // держится барьер фаз, а внутренним соединением он пропадал из ответа
           // молча — дверь говорила «у гейта нет ни одного пункта» там, где
           // барьер называл его непройденным.
            "SELECT coalesce(i.phase, g.phase), coalesce(i.id, g.id), i.item, i.kind,
                    g.result, g.checked_at, i.why, i.query, i.probe, i.owner, g.probe_ok,
                    i.subject_query, i.subject_why, i.since
               FROM gate_item i
               FULL JOIN project_gates g
                 ON g.phase = i.phase AND g.id = i.id AND g.project_id = $1
              WHERE (i.id IS NOT NULL OR g.project_id = $1)
                AND ($2 = '' OR coalesce(i.phase, g.phase) = $2)
              ORDER BY 1, 2",
               &[&project, &phase.unwrap_or("")],
        )
        .await?;

    // Признаки «чей предмет спора» — ОДНИМ запросом на весь гейт, а не по
    // запросу на пункт: пунктов сто одиннадцать.
    let blame_of: std::collections::HashMap<String, Vec<(String, String, String)>> = {
        let rows = client
            .query(
                "SELECT rule, entity_id, blame, fixed_by FROM finding_blame WHERE project_id = $1",
                &[&project],
            )
            .await?;
        let mut m: std::collections::HashMap<String, Vec<(String, String, String)>> =
            std::collections::HashMap::new();
        for r in &rows {
            m.entry(r.get::<_, String>(0)).or_default().push((
                r.get::<_, String>(1),
                r.get::<_, String>(2),
                r.get::<_, String>(3),
            ));
        }
        m
    };

    // Сам гейт: формулировка из плана и нужна ли ему подпись.
    let heads = client
        .query("SELECT g.phase, g.title FROM gate_head g", &[])
        .await?;
    let head_of: std::collections::HashMap<String, String> =
        heads.iter().map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))).collect();

    let mut gates: std::collections::BTreeMap<String, Vec<Value>> = std::collections::BTreeMap::new();
    let mut checked_at: std::collections::BTreeMap<String, Option<i64>> = std::collections::BTreeMap::new();
    for r in &rows {
        let phase: String = r.get(0);
        let rule: String = r.get(1);
        let title: Option<String> = r.get(2);
        let stored: Option<Value> = r.get(4);
        let at: Option<i64> = r.get(5);
        // Пустой замер — не «пройден» и не «провален». Он значит, что пункт ещё
        // ни разу не мерили, и сказать это надо словом.
        let mut entry = stored.unwrap_or_else(|| {
            json!({ "computed": "unknown",
                    "why": "ещё не мерили: пересчёт с заведения пункта не запускался" })
        });

        // ПРАВИЛО — ИЗ ОБЪЯВЛЕНИЯ, ЦЕЛИКОМ И ПОВЕРХ ЗАМЕРА. Заголовок, род и
        // довод лежали ещё и в снимке замера, и снимок этот делался в прошлый
        // круг: правка рода или довода не была видна ни в двери, ни на доске до
        // следующего пересчёта — то самое молчаливое расхождение двух описаний,
        // только спрятанное внутрь `result`.
        if let Some(m) = entry.as_object_mut() {
            m.insert("id".into(), json!(rule));
            m.insert("phase".into(), json!(phase));
            match title {
                Some(title) => {
                    m.insert("item".into(), json!(title));
                    m.insert("kind".into(), json!(r.get::<_, Option<String>>(3).unwrap_or_default()));
                    m.insert("means".into(), json!(r.get::<_, Option<String>>(6).unwrap_or_default()));
                    m.insert("query".into(), json!(r.get::<_, Option<String>>(7).unwrap_or_default()));
                    m.insert("probe".into(), json!(r.get::<_, Option<String>>(8).unwrap_or_default()));
                    m.insert("owner".into(), json!(r.get::<_, Option<String>>(9).unwrap_or_default()));
                    m.insert("subject".into(), json!(r.get::<_, Option<String>>(11).unwrap_or_default()));
                    m.insert("subjectWhy".into(), json!(r.get::<_, Option<String>>(12).unwrap_or_default()));
                    m.insert("since".into(), json!(r.get::<_, Option<i64>>(13).unwrap_or(0)));
                }
                // СИРОТА НАЗЫВАЕТСЯ ВСЛУХ. Замер, чьё правило снято, продолжает
                // считаться состоянием гейта и держать барьер фаз; молча спрятав
                // его, дверь сказала бы «пунктов нет» о том, чем барьер держит.
                None => {
                    m.insert("item".into(), json!(rule.clone()));
                    m.insert("kind".into(), json!("unknown"));
                    m.insert("orphan".into(), json!(true));
                    m.insert("means".into(), json!(
                        "замер есть, а правила нет: пункт снят, а его замер остался и \
                         продолжает считаться состоянием гейта"));
                }
            }
            // ВЕРДИКТ ОДНИМ СЛОВОМ. Состояние у пункта было — `computed`, — но
            // рядом лежало `violations`, и всякий подсчёт вида «красный, если
            // нарушений больше нуля» клал `unknown` в зелёные: у него
            // `violations` равно `null`. Пункт при этом честен и называет
            // причину словами; правило «незнание называется словом» исполнялось
            // ВНУТРИ пункта и терялось на выходе.
            //
            // Три значения и никаких больше: сложить «неизвестно» с зелёным
            // теперь можно только нарочно.
            m.insert(
                "verdict".into(),
                json!(match m.get("computed").and_then(|v| v.as_str()).unwrap_or("") {
                    "passed" => "green",
                    "failed" => "red",
                    _ => "unknown",
                }),
            );
            // «Пробу не проверяли» и «проба не роняет» — разное, и оба не
            // «зелёное»: пункт, который ни разу не уронили, никто не проверял.
            m.insert("probeRuns".into(), json!(r.get::<_, Option<bool>>(10)));
            // ЧЕЙ ПРЕДМЕТ СПОРА. Находка остаётся красной, но счёт разделён:
            // «одиннадцать нарушений» смешивало своё с чужим, и по одному числу
            // нельзя было решить, работа это набора или слепота сервера.
            if let Some(b) = blame_of.get(&rule) {
                m.insert("blamed".into(), json!(b.len()));
                m.insert(
                    "blamedOn".into(),
                    json!(b.iter().map(|(e, bl, fx)| json!({
                        "entityId": e, "blame": bl, "fixedBy": fx
                    })).collect::<Vec<_>>()),
                );
                let mine = (m.get("violations").and_then(|n| n.as_i64()).unwrap_or(0) as usize)
                    .saturating_sub(b.iter().filter(|(_, bl, _)| bl == "harness").count());
                m.insert("violationsOurs".into(), json!(mine));
            }
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

    // Состояние гейта ВЫВОДИТСЯ ЦЕЛИКОМ: все проверки прошли — гейт закрыт.
    // Прежде здесь стояло «и подпись стоит и не снята», и это противоречило
    // самому замыслу: гейт автоматический, условия машинные, и человеку нечего
    // добавить к тому, что уже померено.
    //
    // ВЫВОДИТСЯ ОНО ВИДОМ `gate_state`, а не пересчётом здесь. Пересчёт был
    // вторым описанием одного правила, и разошлось оно дважды. Двух описаний
    // больше нет — есть одно, и обе стороны читают его.
    let verdicts: std::collections::HashMap<String, (String, i64, i64)> = client
        .query(
            "SELECT gate, computed, failed, open FROM gate_state WHERE project_id = $1",
            &[&project],
        )
        .await?
        .iter()
        .map(|r| (r.get::<_, String>(0),
                  (r.get::<_, String>(1), r.get::<_, i64>(2), r.get::<_, i64>(3))))
        .collect();
    let mut out = Vec::new();
    for (phase, items) in gates {
        let (computed, failed, unknown) = verdicts
            .get(&phase)
            .map(|(c, f, u)| (c.as_str(), *f as usize, *u as usize))
            .unwrap_or(("open", 0, items.len()));
        let checked = checked_at.get(&phase).copied().flatten();
        out.push(json!({
            "gate": phase,
            "title": head_of.get(&phase).cloned().unwrap_or_default(),
            "computed": computed,
            "items": items,
            "failedItems": failed,
            "openItems": unknown,
            "checkedAt": checked,
        }));
    }
    // Пригоршня краснот «датчик подавал и ПРОТУХ» — не находки, а непрогнанный
    // `mh sense`. Числа отдаются как есть, но чем это лечится, сказано рядом.
    let sensors_note = if stale_sensors.is_empty() {
        Value::Null
    } else {
        json!({
            "facts": stale_sensors,
            "why": "эти роды фактов не свежи, и пункты, читающие их, красны НЕ ПО НАХОДКЕ, \
                    а потому что мерить сейчас нечем. Прогоните `mh sense` — после него \
                    останутся только настоящие.",
        })
    };
    client.commit().await?;
    match stale {
        Some((_, why, at)) => Ok(json!({
            "gates": out,
            // Слово стоит ПЕРВЫМ и не прячет числа: спрятать их значило бы
            // потерять и то, что всё-таки посчиталось.
            "stale": true,
            "why": format!(
                "последняя пересборка не удалась ({why}); показанные числа посчитаны по \
                 недособранным проекциям и могут значить не то. Прогоните `reproject` и `rebuild`."
            ),
            "staleAt": at,
            "staleSensors": sensors_note,
        })),
        None => Ok(json!({ "gates": out, "staleSensors": sensors_note })),
    }
}

/// Сверка заявленного числа с фактом — но сперва сказав, что именно считается.
///
/// Без оговорки сверка кричит волком дважды и топит единственный настоящий крик.
pub(crate) async fn claims(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT c.name, c.subject, c.claimed, s.counts, s.note
               FROM project_document_plan_counts c
               LEFT JOIN claim_subject s
                 ON s.project_id = c.project_id AND s.subject = c.subject
                AND s.entity_kind = c.planned_kind AND s.entity_name = c.planned_name
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
            Some(sql) => match client.query_one(sql, &[&project]).await {
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


/// Требования задачи — с объявленным отсутствием как ответом, а не пустотой.
pub(crate) async fn requirements_of(pool: &Pool, project: &str, task: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn tasks_of_story(pool: &Pool, project: &str, story: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
    }))
}

/// Очередь предполёта: чего ещё не смотрели или смотрели до правки.
pub(crate) async fn preflight_queue(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
                    coalesce(r.ready, false) AS ready, t.entity_kind
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
                "kind": r.get::<_, String>(5),
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
pub(crate) async fn push_task_state(
    pool: &Pool,
    project: &str,
    states: &[(String, String, String, i64)],
    seen_at: i64,
) -> Result<Value, crate::db::Fail> {
    let bare = closings_without_commit(states);
    if !bare.is_empty() {
        return Ok(json!({ "status": "closed_without_commit", "tasks": bare,
            "why": "закрытие приходит закрывающим трейлером и опознаётся его коммитом: без коммита \
                    нельзя отличить новое закрытие от прежнего. Ничего не записано" }));
    }
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    // Подача полная, а не добавочная: задача, исчезнувшая из подачи, потеряла
    // трейлер, и держать её прежнее состояние значило бы помнить отменённое.
    // Уходят поэтому ТОЛЬКО отсутствующие в подаче, а не все подряд.
    //
    // «КОГДА УВИДЕЛИ» — ЭТО ПРО ФАКТ, А НЕ ПРО ПОДАЧУ.
    //
    // Снос-и-вставка переписывали `seen_at` текущим временем каждой подаче, а
    // подают на каждом прогоне. Отметка значила «когда подали в последний раз»,
    // то есть всегда «только что», и правило `plan-before-commit` сравнивало
    // время плана с нею: любой план оказывался записан «до закрытия». Плечо
    // порядка — то самое, ради которого правило и заведено, — не работало
    // вовсе; красным его держало одно лишь «плана нет вовсе».
    //
    // Отметка двигается теперь при СМЕНЕ состояния. Это всё ещё не время
    // коммита — его знает только тот, у кого репозиторий, — но это правда о
    // факте, а не о разговоре.
    let ids: Vec<String> = states.iter().map(|(t, _, _, _)| t.clone()).collect();
    tx.execute("DELETE FROM task_state WHERE project_id = $1 AND task_id <> ALL($2)",
               &[&project, &ids]).await?;
    let mut written = 0;
    for (task, state, commit, closed_at) in states {
        written += tx
            .execute(
                "INSERT INTO task_state (project_id, task_id, state, closing_commit, seen_at,
                                         closed_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (project_id, task_id) DO UPDATE
                   SET state = EXCLUDED.state, closing_commit = EXCLUDED.closing_commit,
                       closed_at = EXCLUDED.closed_at,
                       seen_at = CASE WHEN task_state.state IS DISTINCT FROM EXCLUDED.state
                                      THEN EXCLUDED.seen_at ELSE task_state.seen_at END",
                &[&project, task, state, commit, &seen_at, closed_at],
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
pub(crate) async fn state_disagreements(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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

/// Поля способа, как их принимает дверь `method-set`.
pub(crate) struct Method<'a> {
    pub kind: &'a str,
    pub id: &'a str,
    pub ord: i32,
    pub method_kind: &'a str,
    pub method: &'a str,
    pub declared_by: &'a str,
    pub drop: bool,
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
pub(crate) async fn set_method(pool: &Pool, project: &str, fields: Method<'_>) -> Result<Value, crate::db::Fail> {
    let Method { kind, id, ord, method_kind, method, declared_by, drop } = fields;
    let client = crate::db::conn(pool).await?;
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
pub(crate) fn now_ms() -> i64 {
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
pub(crate) async fn record_edit(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
    revision: i64,
    actor: &str,
    event: &str,
) -> Result<u64, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    client
        .execute(
            "INSERT INTO entity_event (project_id, entity_kind, entity_id, ord, at, event, actor, source)
             VALUES ($1, $2, $3, $4, current_date, $5, $6, 'edit')
             ON CONFLICT (project_id, entity_kind, entity_id, source, ord)
               DO UPDATE SET event = EXCLUDED.event, actor = EXCLUDED.actor",
            &[&project, &kind, &id, &(revision as i32), &event, &actor],
        )
        .await.map_err(Into::into)
}

/// Приём наблюдений о репозитории. Подача полная в пределах вида факта.
///
/// Датчик подаёт то, что видит: «такие таблицы есть в миграциях», «в контракте
/// столько операций». Ни одного вывода: сошлось ли это с набором — вопрос к
/// серверу, у которого лежит и то и другое.
pub(crate) async fn push_code_facts(
    pool: &Pool,
    project: &str,
    kind: &str,
    facts: &[(String, String)],
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
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
pub(crate) async fn push_skills(
    pool: &Pool,
    set_name: &str,
    skills: &[(String, String, String)],
    actor: &str,
    dry: bool,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
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
    for name in was.keys() {
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
/// Записать план задачи — то, как исполнитель собирается её делать.
///
/// Отметка времени ставит СЕРВЕР, а не подающий: «план написан до правки»
/// доказывается порядком, и порядок, названный тем же, кто его нарушает, ничего
/// не доказывает. По той же причине правка задачи обесценивает план — он
/// привязан к правке, которую читал.
pub(crate) async fn push_task_plan(
    pool: &Pool,
    project: &str,
    task: &str,
    body: &str,
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if body.trim().is_empty() {
        return Ok(json!({ "status": "empty",
                          "why": "план без текста — это отметка о том, что думали, а не то,                                   что придумали. Записывать нечего" }));
    }
    let row = client
        .query_opt(
            "SELECT d.revision, t.state FROM project_plan_tasks t
               JOIN project_documents d
                 ON d.project_id = t.project_id AND d.entity_kind = t.entity_kind
                AND d.entity_name = t.entity_name
              WHERE t.project_id = $1 AND t.id = $2",
            &[&project, &task],
        )
        .await?;
    let Some(row) = row else {
        return Ok(json!({ "status": "not_found", "task": task,
                          "why": format!("задачи «{task}» в наборе нет: план не к чему привязать") }));
    };
    let revision: i64 = row.get(0);
    let state: String = row.get(1);
    // ПЛАН ПОСЛЕ ЗАКРЫТИЯ — не план, а пересказ сделанного. Записывается, но
    // назван своим словом: правило порядка всё равно его не зачтёт, и узнать об
    // этом лучше здесь, чем на гейте.
    let now = now_ms();
    client
        .execute(
            "INSERT INTO task_plan (project_id, task_id, at, task_revision, body, declared_by)
             VALUES ($1,$2,$3,$4,$5,$6)",
            &[&project, &task, &now, &revision, &body, &actor],
        )
        .await?;
    Ok(json!({ "task": task, "at": now, "taskRevision": revision, "by": actor,
               "why": if state == "closed" {
                   "задача уже закрыта: это пересказ сделанного, а не план. Правило порядка                     его не зачтёт"
               } else { "" } }))
}

pub(crate) async fn push_preflight(
    pool: &Pool,
    project: &str,
    verdicts: &[(String, i64, i64, String, i32, String)],
    // Стирать ли ЧУЖОЕ. Подача была полной, а частичной формы не было вовсе — и
    // первый же, кто предполётил не весь план сразу, стирал остальное: тринадцать
    // свежих вердиктов сносили сто восемь прежних. Источник, откуда прежние
    // приехали, снесён, и восстановить было бы неоткуда.
    //
    // Теперь по умолчанию подача СЛИВАЕТСЯ: добавляет и заменяет по имени
    // задачи. Полное стирание осталось — но его надо сказать вслух.
    replace_all: bool,
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    let before = tx
        .query_one("SELECT count(*) FROM preflight_verdict WHERE project_id = $1", &[&project])
        .await?
        .get::<_, i64>(0);
    if replace_all {
        tx.execute("DELETE FROM preflight_verdict WHERE project_id = $1", &[&project]).await?;
    } else {
        // Снимается ровно то, что подаётся заново: остальное чужое и остаётся.
        let names: Vec<String> = verdicts.iter().map(|v| v.0.clone()).collect();
        tx.execute(
            "DELETE FROM preflight_verdict WHERE project_id = $1 AND task_id = ANY($2)",
            &[&project, &names],
        )
        .await?;
    }
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
    // Итог считается ДО фиксации, той же транзакцией: после неё соединение уже
    // отдано, и вопрос к нему — вопрос в никуда.
    let now_total = tx
        .query_one("SELECT count(*) FROM preflight_verdict WHERE project_id = $1", &[&project])
        .await?
        .get::<_, i64>(0);
    tx.commit().await?;
    Ok(json!({ "was": before, "written": written, "now": now_total,
               "kept": now_total - written as i64,
               "mode": if replace_all { "полная замена: чужие вердикты сняты" }
                       else { "слияние: подано своё, чужое на месте" },
               "refused": refused.len(), "refusedTasks": refused,
               "staleOnArrival": stale.len(), "stale": stale }))
}

/// Приём открытых рабочих деревьев — статус «в работе».
///
/// Единственный статус, который меняется В ТЕЧЕНИЕ волны, и ради него в
/// интерфейс и смотрят. Факт знает только тот, у кого есть репозиторий:
/// `git worktree list`. Подача полная — закрытое дерево исчезает тем, что не
/// пришло.
pub(crate) async fn push_worktrees(
    pool: &Pool,
    project: &str,
    open: &[(String, String, i64)],
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
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
pub(crate) async fn question_holders(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;

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
pub(crate) async fn summary(pool: &Pool, project: &str, kind: &str) -> Result<Value, Miss> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn links_of(pool: &Pool, project: &str, kind: &str, id: &str) -> Result<Value, Miss> {
    let client = crate::db::conn(pool).await?;
    links_at(&*client, project, kind, id).await
}

pub(crate) async fn links_at(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    id: &str,
) -> Result<Value, Miss> {
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
        // ЭКРАН И ЗАДАЧА отвечали «вид в базу не спроецирован», а `summary
        // kind=screen` печатал их связи колонками — два ответа об одном
        // предмете. Отказ был неправдой: связи в базе есть, их не спрашивала
        // эта ручка.
        "screen" => vec![
            ("requirements", "SELECT s.requirement_id, coalesce(left(r.text, 90), ''), 'requirement'
                                FROM project_screen_requirements s
                                LEFT JOIN project_requirements r ON r.project_id = s.project_id AND r.id = s.requirement_id
                               WHERE s.project_id = $1 AND s.screen_id = $2 ORDER BY s.requirement_id"),
            ("references", "SELECT r.source, r.source_kind, 'reference'
                              FROM project_screen_references r
                             WHERE r.project_id = $1 AND r.screen_id = $2 ORDER BY r.source"),
        ],
        "task" => vec![
            ("requirements", "SELECT r.requirement_id, coalesce(left(q.text, 90), ''), 'requirement'
                                FROM task_requirement r
                                LEFT JOIN project_requirements q ON q.project_id = r.project_id AND q.id = r.requirement_id
                               WHERE r.project_id = $1 AND r.task_id = $2 ORDER BY r.requirement_id"),
            ("checks", "SELECT c.check_id, c.said_as, 'check' FROM project_task_check c
                         WHERE c.project_id = $1 AND c.task_id = $2 ORDER BY c.check_id"),
            // Ребро, объявленное дверью, документ не называет: без пометки его
            // искали в «Зависит от» и не находили.
            ("dependsOn", "SELECT d.depends_on, coalesce(left(t.title, 90), '')
                                  || CASE WHEN d.origin = 'declared' THEN ' · объявлено дверью task-dep-add' ELSE '' END, 'task'
                             FROM project_plan_task_deps d
                             LEFT JOIN project_plan_tasks t ON t.project_id = d.project_id AND t.id = d.depends_on
                            WHERE d.project_id = $1 AND d.task_id = $2 ORDER BY d.depends_on"),
            ("blocks", "SELECT d.task_id, coalesce(left(t.title, 90), '')
                               || CASE WHEN d.origin = 'declared' THEN ' · объявлено дверью task-dep-add' ELSE '' END, 'task'
                          FROM project_plan_task_deps d
                          LEFT JOIN project_plan_tasks t ON t.project_id = d.project_id AND t.id = d.task_id
                         WHERE d.project_id = $1 AND d.depends_on = $2 ORDER BY d.task_id"),
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
pub(crate) async fn set_author(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    author: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn authors(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn edge_admits(pool: &Pool, principal: &str) -> Result<bool, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let declared: i64 = client
        .query_one("SELECT count(*) FROM edge_principal", &[])
        .await
        ?
        .get(0);
    let allowed = if declared == 0 {
        true
    } else {
        client
            .query_one(
                "SELECT count(*) FROM edge_principal WHERE principal = $1",
                &[&principal],
            )
            .await
            ?
            .get::<_, i64>(0)
            > 0
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
    Ok(allowed)
}

/// Объявить, что этому человеку можно войти. Пустая пометка снимает объявление.
pub(crate) async fn allow_principal(
    pool: &Pool,
    principal: &str,
    note: Option<&str>,
    drop: bool,
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn principals(pool: &Pool) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn freeze_version(
    pool: &Pool,
    project: &str,
    version: &str,
    actor: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn version_delta(pool: &Pool, project: &str, version: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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

/// Поля помощника, как их принимает дверь `agent-set`.
pub(crate) struct Agent<'a> {
    pub set_name: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub body: &'a str,
    pub tools: Option<&'a str>,
    pub model: Option<&'a str>,
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
pub(crate) async fn set_agent(pool: &Pool, fields: Agent<'_>, actor: &str, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Agent { set_name, name, description, body, tools, model } = fields;
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "субагент без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля умения, как их принимает дверь `skill-set`.
pub(crate) struct Skill<'a> {
    pub set_name: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub body: &'a str,
    pub allowed_tools: Option<&'a str>,
    pub disable_model_invocation: Option<bool>,
}

pub(crate) async fn set_skill(pool: &Pool, fields: Skill<'_>, actor: &str, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Skill { set_name, name, description, body, allowed_tools, disable_model_invocation } = fields;
    let client = crate::db::conn(pool).await?;
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
        && description.is_none_or(|d| d == old_description)
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
pub(crate) async fn skills_with_paths(pool: &Pool, set_name: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn set_screen_area(
    pool: &Pool,
    project: &str,
    screen: &str,
    area: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn check_generated(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let computed = order(pool, project).await?;
    let want = computed["content"].as_str().unwrap_or("");
    let client = crate::db::conn(pool).await?;
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
    since: i64,
) -> Result<Vec<String>, String> {
    match if sql.contains("$2") {
        tx.query(sql, &[&project, &since]).await
    } else {
        tx.query(sql, &[&project]).await
    } {
        Ok(rows) => {
            let mut out: Vec<String> =
                rows.iter().map(|r| r.try_get::<_, String>(0).unwrap_or_default()).collect();
            out.sort();
            Ok(out)
        }
        Err(e) => Err(e.says()),
    }
}

async fn subject_planted(
    tx: &deadpool_postgres::Transaction<'_>,
    subject: &str,
    project: &str,
) -> Result<(), String> {
    if subject.trim().is_empty() {
        return Ok(());
    }
    match tx.query(subject, &[&project]).await {
        Err(e) => Err(format!("запрос предмета не исполнился после подсадки: {}", e.says())),
        Ok(rows) if rows.is_empty() => Err("предмет пуст после подсадки: сущности под нарушением проба \
                                            не завела, и замер пройдёт по отсутствию"
            .to_owned()),
        Ok(_) => Ok(()),
    }
}

/// Ошибка базы словами, а не «db error».
///
/// Снять ступень и сдвинуть номера следом идущих.
///
/// Обратная сторона `add_step`, и с той же заботой: номер ступени — ссылка на
/// неё в способе, умении и журнале прогонов, и все они едут вместе. Способ и
/// проба снятой ступени уходят вместе с ней: оставленные, они однажды
/// достанутся чужой ступени, въехавшей на освободившийся номер.
pub(crate) async fn remove_step(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
    ord: i32,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
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

/// Поля записи прогона, как их принимает дверь `run-record-add`.
pub(crate) struct RunRecord<'a> {
    pub id: &'a str,
    pub task: &'a str,
    pub milestone: &'a str,
    pub title: &'a str,
    pub commits: &'a str,
    pub dates: &'a str,
    pub review: &'a str,
    pub appeared: &'a str,
    pub left_open: &'a str,
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
pub(crate) async fn declare_run_record(pool: &Pool, project: &str, fields: RunRecord<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let RunRecord { id, task, milestone, title, commits, dates, review, appeared, left_open } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "прогон без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_decision_link(
    pool: &Pool, project: &str, decision: &str, kind: &str, target: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if decision.trim().is_empty() || target.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без решения или без цели не объявляется" }));
    }
    if !matches!(kind, "closes" | "supersedes" | "refines" | "touches" | "relates") {
        return Ok(json!({ "status": "bad_kind",
                          "why": "связь бывает: closes · supersedes · refines · touches · relates" }));
    }
    let client = crate::db::conn(pool).await?;
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
        "INSERT INTO project_decision_links (project_id, decision_id, kind, target, origin)
         VALUES ($1,$2,$3,$4,'declared') ON CONFLICT DO NOTHING",
        &[&project, &decision, &kind, &target]).await?;
    Ok(json!({ "status": "declared", "decision": decision, "kind": kind, "target": target }))
}

/// Поля правила рамки, как их принимает дверь `frame-rule-add`.
pub(crate) struct FrameRule<'a> {
    pub kind: &'a str,
    pub number: i32,
    pub title: &'a str,
    pub body: &'a str,
    pub held_by: &'a str,
}

pub(crate) async fn declare_frame_rule(pool: &Pool, project: &str, fields: FrameRule<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let FrameRule { kind, number, title, body, held_by } = fields;
    if title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "без формулировки не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля строки процесса, как их принимает дверь `process-row-add`.
pub(crate) struct ProcessRow<'a> {
    pub kind: &'a str,
    pub a: &'a str,
    pub b: &'a str,
    pub c: &'a str,
    pub d: &'a str,
    pub ord: i32,
}
pub(crate) async fn declare_process_row(pool: &Pool, project: &str, fields: ProcessRow<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let ProcessRow { kind, a, b, c, d, ord } = fields;
    let client = crate::db::conn(pool).await?;
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

/// Поля подробности этапа, как их принимает дверь `milestone-detail-add`.
pub(crate) struct MilestoneDetail<'a> {
    pub milestone: &'a str,
    pub what: &'a str,
    pub blocked_by: &'a str,
    pub requirement: &'a str,
    pub gate: &'a str,
    pub closed: &'a str,
}
pub(crate) async fn declare_milestone_detail(pool: &Pool, project: &str, fields: MilestoneDetail<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let MilestoneDetail { milestone, what, blocked_by, requirement, gate, closed } = fields;
    if milestone.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без этапа не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
        // Снятие — той же ручкой, что и объявление. Без него объявленное
        // убирается только запросом мимо сервера, и сервер перестаёт быть
        // единственной дверью — а значит, снятое где-то останется.
        if drop_it {
            // СНИМАЕТСЯ ТЕМ ЖЕ КЛЮЧОМ, КАКИМ ОБЪЯВЛЯЛОСЬ. Прежде снятие читало
            // `what` — поле, которое при объявлении значит совсем другое, «что
            // делается». Объявив `requirement=FR-11`, снять его тем же доводом
            // было нельзя: дверь отвечала `not_found` про строку, которая есть.
            // Узнать ключ можно было только открыв исходник.
            //
            // `what` принимается и дальше: им снимали, пока это был единственный
            // способ, и ломать записанное незачем.
            let req = if requirement.trim().is_empty() { what } else { requirement };
            let g = if gate.trim().is_empty() { what } else { gate };
            let mut gone = 0u64;
            if !req.trim().is_empty() {
                gone += client
                    .execute("DELETE FROM project_milestone_requirements WHERE project_id = $1 AND milestone_id = $2 AND requirement_id = $3", &[&project, &milestone, &req])
                    .await?;
            }
            if !g.trim().is_empty() {
                gone += client
                    .execute("DELETE FROM project_milestone_gates WHERE project_id = $1 AND milestone_id = $2 AND gate = $3", &[&project, &milestone, &g])
                    .await?;
            }
            return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                              "by": if requirement.trim().is_empty() && gate.trim().is_empty() { "what" } else { "requirement/gate" } }));
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

/// Поля подробности экрана, как их принимает дверь `screen-detail-add`.
pub(crate) struct ScreenDetail<'a> {
    pub screen: &'a str,
    pub purpose: &'a str,
    pub opens_when: &'a str,
    pub empty_and_broken: &'a str,
    pub requirement: &'a str,
}

pub(crate) async fn declare_screen_detail(pool: &Pool, project: &str, fields: ScreenDetail<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let ScreenDetail { screen, purpose, opens_when, empty_and_broken, requirement } = fields;
    if screen.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без экрана не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_story_detail(
    pool: &Pool, project: &str, story: &str, screen: &str, persona: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if story.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "подробность без истории не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Объявить, что история несёт требование.
///
/// Правило `story-has-requirement` читает `project_story_requirements`, а
/// заполнить эту таблицу через сервер было нечем: `story-add` знает имя,
/// заголовок и область, `story-detail-add` — персону и экран, двери про
/// требование не было ни одной. У `tot-ade` все семнадцать историй несут
/// строку «**Требования:** …», связь есть в документе и нет в колонках, и
/// закрыть пункт правдой было невозможно — только отменой.
///
/// Обе стороны ПРОВЕРЯЮТСЯ: связь на несуществующее — подписанная пустота,
/// и читается она как знание.
pub(crate) async fn declare_story_requirement(
    pool: &Pool, project: &str, story: &str, requirement: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if story.trim().is_empty() || requirement.trim().is_empty() {
        return Ok(json!({ "status": "empty",
            "why": "связь объявляется двумя сторонами: история и требование" }));
    }
    if drop_it {
        let gone = client
            .execute(
                "DELETE FROM project_story_requirements
                  WHERE project_id = $1 AND story_id = $2 AND requirement_id = $3",
                &[&project, &story, &requirement],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                          "story": story, "requirement": requirement }));
    }
    if client
        .query_opt("SELECT 1 FROM project_stories WHERE project_id = $1 AND id = $2",
                   &[&project, &story])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_story", "story": story,
            "why": "истории с таким именем в наборе нет: связь описывала бы то, чего нет" }));
    }
    if client
        .query_opt("SELECT 1 FROM project_requirements WHERE project_id = $1 AND id = $2",
                   &[&project, &requirement])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_requirement", "requirement": requirement,
            "why": "требования с таким именем в наборе нет" }));
    }
    let n = client
        .execute(
            "INSERT INTO project_story_requirements (project_id, story_id, requirement_id, origin)
             VALUES ($1,$2,$3,'declared') ON CONFLICT DO NOTHING",
            &[&project, &story, &requirement],
        )
        .await?;
    Ok(json!({ "status": if n > 0 { "declared" } else { "already" },
               "story": story, "requirement": requirement }))
}

/// Объявить, что фича несёт историю.
///
/// Правило `feature-matches-stories` читает `project_feature_stories`. У
/// `tot-ade` эту связь не говорит НИ ОДНА сторона: обход пятидесяти фич не
/// нашёл ни одного имени истории, обратной строки «**Фича:**» нет ни в одной
/// из семнадцати. Там пункт красен по делу — и закрыть его было нечем.
pub(crate) async fn declare_feature_story(
    pool: &Pool, project: &str, feature: &str, story: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if feature.trim().is_empty() || story.trim().is_empty() {
        return Ok(json!({ "status": "empty",
            "why": "связь объявляется двумя сторонами: фича и история" }));
    }
    if drop_it {
        let gone = client
            .execute(
                "DELETE FROM project_feature_stories
                  WHERE project_id = $1 AND feature_id = $2 AND story_id = $3",
                &[&project, &feature, &story],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" },
                          "feature": feature, "story": story }));
    }
    if client
        .query_opt("SELECT 1 FROM project_features WHERE project_id = $1 AND id = $2",
                   &[&project, &feature])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_feature", "feature": feature,
            "why": "фичи с таким именем в наборе нет" }));
    }
    if client
        .query_opt("SELECT 1 FROM project_stories WHERE project_id = $1 AND id = $2",
                   &[&project, &story])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_story", "story": story,
            "why": "истории с таким именем в наборе нет" }));
    }
    let n = client
        .execute(
            "INSERT INTO project_feature_stories (project_id, feature_id, story_id, origin)
             VALUES ($1,$2,$3,'declared') ON CONFLICT DO NOTHING",
            &[&project, &feature, &story],
        )
        .await?;
    Ok(json!({ "status": if n > 0 { "declared" } else { "already" },
               "feature": feature, "story": story }))
}

pub(crate) async fn declare_feature_link(
    pool: &Pool, project: &str, feature: &str, requirement: &str, article: i32,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if feature.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без фичи не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля критерия приёмки, как их принимает дверь `acceptance-add`.
pub(crate) struct Acceptance<'a> {
    pub id: &'a str,
    pub story: &'a str,
    pub number: i32,
    pub title: &'a str,
    pub preconditions: &'a str,
    pub steps: &'a str,
    pub observed: &'a str,
    pub fails_when: &'a str,
}

pub(crate) async fn declare_acceptance(pool: &Pool, project: &str, fields: Acceptance<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Acceptance { id, story, number, title, preconditions, steps, observed, fails_when } = fields;
    if id.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "сценарий без имени или без названия не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля цели, как их принимает дверь `goal-add`.
pub(crate) struct Goal<'a> {
    pub id: &'a str,
    pub number: i32,
    pub level: &'a str,
    pub title: &'a str,
    pub measured_by: &'a str,
    pub checked_when: &'a str,
    pub fails_when: &'a str,
    pub state_now: &'a str,
}

pub(crate) async fn declare_goal(pool: &Pool, project: &str, fields: Goal<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Goal { id, number, level, title, measured_by, checked_when, fails_when, state_now } = fields;
    if id.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "цель без имени или без формулировки не объявляется" }));
    }
    // Цель без способа измерить — намерение, и так и называется.
    let state = if measured_by.trim().is_empty() { "намерение" } else { "цель" };
    let client = crate::db::conn(pool).await?;
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

/// Поля риска, как их принимает дверь `risk-add`.
pub(crate) struct Risk<'a> {
    pub id: &'a str,
    pub number: i32,
    pub title: &'a str,
    pub state: &'a str,
    pub mitigation: &'a str,
    pub trigger: &'a str,
    pub owner: &'a str,
    pub source: &'a str,
}

pub(crate) async fn declare_risk(pool: &Pool, project: &str, fields: Risk<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Risk { id, number, title, state, mitigation, trigger, owner, source } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "риск без имени не объявляется" }));
    }
    let state = if matches!(state, "open" | "accepted" | "closed") { state } else { "open" };
    let client = crate::db::conn(pool).await?;
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

/// Поля вопроса, как их принимает дверь `question-add`.
pub(crate) struct Question<'a> {
    pub id: &'a str,
    pub number: i32,
    pub title: &'a str,
    pub state: &'a str,
    pub answer: &'a str,
    pub closed_by: &'a str,
    pub owner: bool,
}
pub(crate) async fn declare_question(pool: &Pool, project: &str, fields: Question<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Question { id, number, title, state, answer, closed_by, owner } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "вопрос без имени не объявляется" }));
    }
    // Снятие — той же ручкой. Без него вопрос, заведённый по ошибке (занятым
    // номером, чужим именем), снимался только запросом в базу мимо сервера — и
    // так уже вышло дважды за один разбор.
    if drop_it {
        let client = crate::db::conn(pool).await?;
        let gone = client
            .execute("DELETE FROM project_questions WHERE project_id = $1 AND id = $2",
                     &[&project, &id])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let state = if matches!(state, "open" | "decided" | "closed") { state } else { "open" };
    let client = crate::db::conn(pool).await?;
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
          &(if !answer.trim().is_empty() { "answered" } else if owner { "owner" } else { "unsaid" }),
          &!answer.trim().is_empty()]).await?;
    // СВЯЗЬ «ЧЕМ ЗАКРЫТ» — ЧАСТЬ ОТВЕТА, а не побочное действие. Вставка стояла
    // под `.ok()`: связь молча не писалась, а дверь всё равно отвечала
    // «declared». Спросивший получал слово о том, чего не произошло.
    let mut closed = json!(closed_by);
    if !closed_by.is_empty() {
        match client.execute(
            "INSERT INTO project_decision_links (project_id, decision_id, kind, target, origin)
             VALUES ($1,$2,'closes',$3,'declared') ON CONFLICT DO NOTHING",
            &[&project, &closed_by, &id]).await
        {
            Ok(_) => {}
            Err(e) => closed = json!({ "asked": closed_by, "written": false,
                                       "why": e.says() }),
        }
    }
    Ok(json!({ "status": "declared", "id": id, "state": state, "closedBy": closed }))
}

pub(crate) async fn declare_task_requirement(
    pool: &Pool, project: &str, task: &str, requirement: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if task.trim().is_empty() || requirement.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без задачи или без требования не объявляется" }));
    }
    // Снятие той же дверью: связь, оставшаяся без задачи, — сирота, и находит
    // её только тот, кто знал, что она была.
    if drop_it {
        let client = crate::db::conn(pool).await?;
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
    let client = crate::db::conn(pool).await?;
    client.execute(
        "INSERT INTO task_requirement (project_id, task_id, requirement_id, origin)
         VALUES ($1,$2,$3,'declared')
         ON CONFLICT DO NOTHING", &[&project, &task, &requirement]).await?;
    Ok(json!({ "status": "declared", "task": task, "requirement": requirement }))
}

pub(crate) async fn declare_screen_reference(
    pool: &Pool, project: &str, source: &str, source_kind: &str, screen: &str, drop: bool,
) -> Result<Value, crate::db::Fail> {
    if source.trim().is_empty() || screen.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "ссылка без источника или без экрана не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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
        "INSERT INTO project_screen_references (project_id, source, source_kind, screen_id, origin)
         VALUES ($1,$2,$3,$4,'declared') ON CONFLICT DO NOTHING",
        &[&project, &source, &source_kind, &screen]).await?;
    Ok(json!({ "status": "declared", "source": source, "screen": screen }))
}

pub(crate) async fn declare_alternative(
    pool: &Pool, project: &str, decision: &str, ord: i32, title: &str, body: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if decision.trim().is_empty() || title.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "вариант без решения или без названия не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_version(
    pool: &Pool, project: &str, id: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "выпуск без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_milestone(
    pool: &Pool, project: &str, id: &str, version: &str, ord: i32, title: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "этап без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_task(pool: &Pool, project: &str, fields: Task<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Task { id, milestone, ord, title, kind, state, size } = fields;
    // Снятие идёт той же дверью, и только объявленное: спроецированная задача
    // уходит со своим документом, а не отдельной рукой.
    if drop_it {
        if id.trim().is_empty() {
            return Ok(json!({ "status": "nameless", "why": "снимать задачу без имени нечего" }));
        }
        let client = crate::db::conn(pool).await?;
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
    // Взятие и закрытие приходят трейлером, а не словом двери: объявленное
    // закрытие не опознать коммитом, и суд порядка обошёл бы его. Незнание
    // называется «не начата».
    if matches!(state, "claimed" | "closed") {
        return Ok(json!({ "status": "state_from_history", "id": id, "state": state,
            "why": "взятие и закрытие задачи приходят закрывающим трейлером (`Task: <id> closed`) \
                    и подачей `task-state-push`, а не объявлением" }));
    }
    let client = crate::db::conn(pool).await?;
    // Объявление поверх документа роняло пересборку всего набора: строка
    // красной задачи ложилась в план второй раз. Документ полнее объявления, и
    // пересборка всё равно перепишет строку по нему.
    if let Some(row) = client
        .query_opt(
            "SELECT entity_kind FROM project_documents
              WHERE project_id = $1 AND entity_kind IN ('task', 'red-task') AND lower(entity_name) = lower($2)
              LIMIT 1",
            &[&project, &id],
        )
        .await?
    {
        let kind: String = row.get(0);
        return Ok(json!({ "status": "has_document", "id": id,
            "why": format!("у задачи есть документ вида {kind}: документ полнее объявления, и план строится по нему — объявлять не нужно") }));
    }
    client.execute(
        "INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state,
                                         closing_commit, kind, entity_kind, entity_name, origin)
         VALUES ($1,$2,$3,$4,$5,$6,'not_started','',$7,'task',$2,'declared')
         ON CONFLICT (project_id, id) DO UPDATE SET milestone_id = EXCLUDED.milestone_id,
           ord = EXCLUDED.ord, title = EXCLUDED.title, size = EXCLUDED.size,
           kind = EXCLUDED.kind, origin = 'declared'",
        &[&project, &id, &milestone, &ord, &title, &size, &kind]).await?;
    Ok(json!({ "status": "declared", "id": id }))
}

/// Поля решения, как их принимает дверь `decision-add`.
pub(crate) struct Decision<'a> {
    pub id: &'a str,
    pub number: i32,
    pub title: &'a str,
    pub status: &'a str,
    pub status_text: &'a str,
    pub date: &'a str,
    pub deciders: &'a str,
    pub context: &'a str,
    pub decision: &'a str,
    pub consequences: &'a str,
}

pub(crate) async fn declare_decision(pool: &Pool, project: &str, fields: Decision<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Decision { id, number, title, status, status_text, date, deciders, context, decision, consequences } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "решение без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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
                        status, status_text, date, deciders, context, decision, consequences, origin)
             VALUES ($1,$2,$3,$4,'decision',$2,$5,$6,$7,$8,$9,$10,$11,'declared')
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
pub(crate) async fn declare_story(
    pool: &Pool, project: &str, id: &str, title: &str, area: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "история без имени не объявляется" }));
    }
    // Снятие той же дверью: без него объявленное убирается только запросом
    // мимо сервера — второй дверью, о которой сервер не знает.
    if drop_it {
        let client = crate::db::conn(pool).await?;
        let gone = client
            .execute(
                "DELETE FROM project_stories WHERE project_id = $1 AND id = $2 AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn declare_screen(
    pool: &Pool, project: &str, id: &str, title: &str, area: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "экран без имени не объявляется" }));
    }
    // Снятие той же дверью.
    if drop_it {
        let client = crate::db::conn(pool).await?;
        let gone = client
            .execute(
                "DELETE FROM project_screens WHERE project_id = $1 AND id = $2 AND origin = 'declared'",
                &[&project, &id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "id": id }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_article(
    pool: &Pool, project: &str, number: i32, title: &str, body: &str, anchor: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if title.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "статья без заголовка не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля задачи, как их принимает дверь `task-add`.
pub(crate) struct Task<'a> {
    pub id: &'a str,
    pub milestone: &'a str,
    pub ord: i32,
    pub title: &'a str,
    pub kind: &'a str,
    pub state: &'a str,
    pub size: &'a str,
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
pub(crate) async fn declare_task_dep(
    pool: &Pool, project: &str, task: &str, depends_on: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if task.trim().is_empty() || depends_on.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "зависимость без задачи или без цели не объявляется" }));
    }
    // Снятие идёт той же дверью. Без него объявленное ребро убирается только
    // запросом мимо сервера — второй дверью, о которой сервер не знает, и
    // разойдутся они молча.
    if drop_it {
        let client = crate::db::conn(pool).await?;
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
    let client = crate::db::conn(pool).await?;
    // Обе стороны обязаны существовать: ребро в несуществующую задачу тихо
    // выпадает из порядка и делает волну шире, чем она есть.
    let known = client
        .query("SELECT id, entity_kind FROM project_plan_tasks WHERE project_id = $1 AND id = ANY($2)",
               &[&project, &vec![task.to_owned(), depends_on.to_owned()]]).await?;
    if known.len() < 2 {
        return Ok(json!({ "status": "unknown_task",
                          "why": format!("нет задачи: {task} либо {depends_on}") }));
    }
    // Строки красных задач каждая пересборка сносит и кладёт заново, и ребро
    // уходит с ними каскадом: дверь отвечала «объявлено» про то, что не
    // доживало до следующей записи.
    if known.iter().any(|r| r.get::<_, String>(0) == task && r.get::<_, String>(1) == "red-task") {
        return Ok(json!({ "status": "red_task",
            "why": format!("{task} — красная задача: её строка пересобирается из документа, и объявленное ребро уйдёт с первой же пересборкой. Зависимость от обычной задачи пишется полем «Зависит от» её документа; зависимость от другой красной задачи поле не выражает") }));
    }
    client.execute(
        "INSERT INTO project_plan_task_deps (project_id, task_id, depends_on, origin) VALUES ($1,$2,$3,'declared')
         ON CONFLICT (project_id, task_id, depends_on) DO UPDATE SET origin = 'declared'",
        &[&project, &task, &depends_on]).await?;
    Ok(json!({ "status": "declared", "task": task, "dependsOn": depends_on }))
}

pub(crate) async fn declare_release_artifact(
    pool: &Pool, project: &str, name: &str, what: &str, installed_to: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "артефакт без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля строки заморозки, как их принимает дверь `freeze-row-add`.
pub(crate) struct FreezeRow<'a> {
    pub version: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
    pub hash: &'a str,
}

pub(crate) async fn declare_freeze_row(pool: &Pool, project: &str, fields: FreezeRow<'_>, actor: &str, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let FreezeRow { version, kind, name, hash } = fields;
    if version.trim().is_empty() || kind.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "строка слепка без выпуска или без вида не вносится" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля разбора, как их принимает дверь `postmortem-add`.
pub(crate) struct Postmortem<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub timeline: &'a str,
    pub root_cause: &'a str,
    pub lesson: &'a str,
}

pub(crate) async fn declare_postmortem(pool: &Pool, project: &str, fields: Postmortem<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Postmortem { id, title, summary, timeline, root_cause, lesson } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "разбор без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля знака оформления, как их принимает дверь `token-add`.
pub(crate) struct Token<'a> {
    pub name: &'a str,
    pub dark: &'a str,
    pub light: &'a str,
    pub purpose: &'a str,
    pub section: &'a str,
}

pub(crate) async fn declare_token(pool: &Pool, project: &str, fields: Token<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Token { name, dark, light, purpose, section } = fields;
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "токен без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля источника ссылки, как их принимает дверь `reference-source-add`.
pub(crate) struct ReferenceSource<'a> {
    pub name: &'a str,
    pub source: &'a str,
    pub note: &'a str,
    pub taken: &'a str,
    pub sha: &'a str,
    pub ref_type: &'a str,
    pub from_project: &'a str,
    pub repo: &'a str,
    pub written: &'a str,
    pub updated: &'a str,
    pub status: &'a str,
    pub tags: &'a str,
    pub role: &'a str,
}
pub(crate) async fn declare_reference_source(pool: &Pool, project: &str, fields: ReferenceSource<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let ReferenceSource { name, source, note, taken, sha, ref_type, from_project, repo, written, updated, status, tags, role } = fields;
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "происхождение без документа не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля алгоритма, как их принимает дверь `algorithm-add`.
pub(crate) struct Algorithm<'a> {
    pub id: &'a str,
    pub story: &'a str,
    pub title: &'a str,
    pub preconditions: &'a str,
    pub flow: &'a str,
    pub failure: &'a str,
    pub not_covered: &'a str,
    pub link_kind: &'a str,
    pub link_target: &'a str,
}

pub(crate) async fn declare_algorithm(pool: &Pool, project: &str, fields: Algorithm<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Algorithm { id, story, title, preconditions, flow, failure, not_covered, link_kind, link_target } = fields;
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "алгоритм без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_stand_row(
    pool: &Pool, project: &str, section: &str, name: &str, value: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "строка стенда без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля спецификации датчика, как их принимает дверь `sensor-spec-add`.
pub(crate) struct SensorSpec<'a> {
    pub fact: &'a str,
    pub reads: &'a str,
    pub extract: &'a str,
    pub note: &'a str,
    pub how: &'a str,
    pub skip: &'a str,
    pub allow: &'a str,
}

/// Объявить датчик: где искать, чем вынимать, как назвать факт.
pub(crate) async fn declare_sensor_spec(pool: &Pool, project: &str, fields: SensorSpec<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let SensorSpec { fact, reads, extract, note, how, skip, allow } = fields;
    if fact.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "датчик без имени факта не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
    if drop_it {
        let gone = client.execute("DELETE FROM project_sensor_spec WHERE project_id = $1 AND fact = $2",
                                  &[&project, &fact]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "fact": fact }));
    }
    // Незнакомый род прежде молча становился `extract`: датчик объявляли одним,
    // он снимал другое и говорил «снято». Отказ называет допустимые роды.
    const KINDS: [&str; 17] = ["extract", "files", "secret-fields", "declared-paths", "lines", "retired-terms",
                              "domain-vs-check", "contract-vs-schema", "contract-ops", "contract-body", "declared-lines", "contract-marks", "contract-head", "frozen-tree",
                              "task-trailers", "holder-stub", "file-matches"];
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

/// Объявлено дверью и не написано документом.
///
/// `question-list` отдавал имена, по которым `get` отказывает: сущность
/// объявлена дверью, документа у неё нет. Двенадцать таких у myack, и увидеть
/// расхождение можно было только вычитанием одного перечня из другого — то есть
/// зная заранее, что оно бывает. `sweep` их не видит: он чистит разбор
/// документов, а не объявления.
///
/// Различает их колонка `origin`. С тех пор как документ ПОГЛОЩАЕТ объявление
/// того же имени, `declared` значит ровно одно: текста, из которого это
/// выводится, нет.
///
/// ВНУТРЕННИЕ виды сюда не идут: у проверки и требования своего документа не
/// бывает, и «не написано» у них — не пробел, а устройство.
/// Побеги вместе с ответом «дыра ещё есть?».
///
/// Прежде `holeGone` спрашивал: «является ли `entity_id` историей, у которой
/// есть требования». Для правила «история без требования» это верно; для
/// остальных пятнадцати семейств вопрос бессмысленный — у побега
/// `column-input`/`absences.synced_at` спрашивалось, не стала ли колонка
/// историей. Ответ всегда `false`, и `toRetire` был структурно ноль: 207 побегов
/// из 211 получали ответ на чужой вопрос.
///
/// Побег с причиной, которая перестала быть правдой, — та же дыра, только её
/// никто не перечитает: причина написана, выглядит убедительно, и ровно поэтому
/// её не проверяют.
///
/// Считается ТЕМ ЖЕ ЗАПРОСОМ, что и сам пункт. Запрос исключения ОТСЕИВАЕТ, и
/// потому побеги правила снимаются в транзакции, запрос исполняется, транзакция
/// откатывается: вернулась находка — дыра на месте.
///
/// Правило без запроса отвечает `null`, а не `false`: «не знаем» и «дыра есть» —
/// разное, и путать их значит скрывать устаревшие побеги под видом действующих.
/// Объявить, что поле контракта и колонка схемы — одно и то же под разными
/// именами.
///
/// Причина обязательна: пара без неё неотличима от опечатки, и снять её потом
/// будет нечем — ровно та беда, от которой заведена сама запись.
/// Объявить, что колонку заполняет сервер, и назвать ЧЕМ.
/// Объявленные держатели инварианта: требование и путь.
///
/// Нужны датчику: судить, настоящий ли держатель, можно только прочитав файл, а
/// файл есть у клиента, не у сервера.
/// Объявленные числа против измеренных: показать, а по слову — записать.
///
/// Харнес СВЕРЯЕТ каждое объявленное число пятью пунктами, а двери, которая эти
/// числа ПИШЕТ, не было ни одной. Заведение одного требования стоило правки
/// больше тридцати чисел в шести документах, и каждое из них харнес уже знал: он
/// их и посчитал, чтобы сравнить.
///
/// Проверку это не обесценивает: она и сегодня краснеет на устаревшем числе.
/// Дверь снимает переписывание руками того, что уже посчитано.
///
/// Пишется ТОЛЬКО чистый счёт — число, стоящее перед именем вида. Колонка «в
/// коде (домен)» меряется прогоном, а не набором, и такая дверь обязана её не
/// знать: записать туда посчитанное значило бы объявить сделанным то, чего нет.
pub(crate) async fn counts_sync(
    pool: &Pool,
    project: &str,
    // Область: вид и имя документа. Пусто — все. Запись без области правит
    // столько документов, сколько нашлось, и это не всегда то, чего хотят.
    only_kind: &str,
    only_name: &str,
    apply: bool,
    author: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // Тот же образец и тот же счёт, что у пункта `kind-count-matches`: второй
    // источник правды разошёлся бы с первым.
    let rows = client
        .query(
            r#"WITH факт(вид, n) AS (
                 SELECT 'FR', count(*) FROM project_requirements WHERE project_id = $1 AND kind = 'FR'
                 UNION ALL SELECT 'NFR', count(*) FROM project_requirements WHERE project_id = $1 AND kind = 'NFR'
                 UNION ALL SELECT 'TC', count(*) FROM project_checks WHERE project_id = $1
                 UNION ALL SELECT 'ST', count(*) FROM project_needs WHERE project_id = $1
                 UNION ALL SELECT 'US', count(*) FROM project_stories WHERE project_id = $1
                 UNION ALL SELECT 'SCR', count(*) FROM project_screens WHERE project_id = $1),
             образец AS (SELECT '([0-9]{2,4})\s+(?:(?:' ||
                 coalesce((SELECT string_agg(c.value, '|') FROM scheme($1) c WHERE c.role = 'word.counts'),
                          'НЕТ-ТАКОГО-СЛОВА') ||
                 ')\s+)?`?(FR|NFR|TC|ST|US|SCR)(?:-nn|-NN)?`?(?![A-Za-z0-9-])' AS re),
             строка AS (SELECT d.entity_kind AS ok, d.entity_name AS oi, l.line
                          FROM project_documents d
                          CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') AS l(line)
                         WHERE d.project_id = $1)
             SELECT s.ok, s.oi, x[1]::int, x[2], s.line, к.n
               FROM строка s
               CROSS JOIN LATERAL regexp_matches(regexp_replace(s.line, '«[^»]*»', ' ', 'g'),
                                                 (SELECT re FROM образец), 'g') AS x
               JOIN факт к ON к.вид = x[2]
              WHERE x[1]::int <> к.n
                AND s.line !~ '[~≈]?[0-9]+\s*[–—-]\s*[0-9]+'
                AND s.line !~ '\[0-9\]'
                -- ТЕ ЖЕ ОГОВОРКИ, ЧТО У ПУНКТА. Второй набор условий разошёлся бы
                -- с первым, и дверь писала бы туда, куда правило не смотрит: в
                -- летопись, в текст вопроса, в строку с историческим числом.
                AND NOT EXISTS (SELECT 1 FROM scheme($1) c WHERE c.role = 'word.caveat'
                                 AND lower(s.line) LIKE '%' || lower(c.value) || '%')
                AND NOT EXISTS (SELECT 1 FROM scheme($1) q WHERE q.role = 'word.count-prefix'
                                 AND lower(s.line) LIKE '%' || lower(q.value) || '%')
                AND ($2 = '' OR s.ok = $2)
                AND ($3 = '' OR s.oi = $3)
              ORDER BY 1, 2, 3"#,
            &[&project, &only_kind, &only_name],
        )
        .await?;
    let mut items = Vec::new();
    for r in &rows {
        items.push(json!({
            "kind": r.get::<_, String>(0),
            "name": r.get::<_, String>(1),
            "said": r.get::<_, i32>(2),
            "subject": r.get::<_, String>(3),
            "fact": r.get::<_, i64>(5),
            "line": r.get::<_, String>(4).trim().chars().take(120).collect::<String>(),
        }));
    }
    if !apply {
        return Ok(json!({
            "drift": items.len(), "counts": items,
            "means": "показано, а не записано. Записать — `counts-sync apply=true`: \
                      пишется только чистый счёт, число перед именем вида.",
        }));
    }
    // Запись идёт ДОКУМЕНТОМ ЦЕЛИКОМ и через ту же дорогу, что у человека: сервер
    // — единственная дверь и для себя тоже.
    let mut changed = Vec::new();
    for r in &rows {
        let (kind, name): (String, String) = (r.get(0), r.get(1));
        let said: i32 = r.get(2);
        let fact: i64 = r.get(5);
        let line: String = r.get(4);
        let Some(row) = client
            .query_opt(
                "SELECT content, revision FROM project_documents
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                &[&project, &kind, &name],
            )
            .await?
        else {
            continue;
        };
        let content: String = row.get(0);
        // Заменяется число ИМЕННО В ЭТОЙ СТРОКЕ, и только оно: строка целиком —
        // единственный надёжный якорь, а слепая замена по всему документу задела
        // бы одинаковые числа о другом.
        let fixed_line = line.replacen(&said.to_string(), &fact.to_string(), 1);
        if fixed_line == line || !content.contains(&line) {
            continue;
        }
        let next = content.replacen(&line, &fixed_line, 1);
        client
            .execute(
                "UPDATE project_documents SET content = $4, revision = revision + 1,
                        updated_by = $5
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                &[&project, &kind, &name, &next, &author],
            )
            .await?;
        changed.push(json!({ "kind": kind, "name": name, "subject": r.get::<_, String>(3),
                             "from": said, "to": fact }));
    }
    Ok(json!({
        "changed": changed.len(), "counts": changed,
        "means": "записано измеренное. Пересборку зовите отдельно: числа — текст документа, \
                  и проекции его ещё не читали.",
    }))
}

/// Объявить вид обязательным для проекта — или снять это.
///
/// Объявление ОБЩЕЕ: минимальный набор документов один для всех, как гейты,
/// фазы и лестница.
pub(crate) async fn set_kind_required(
    pool: &Pool, kind: &str, required: bool, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "вид без имени не объявляется" }));
    }
    if required && why.trim().is_empty() {
        return Ok(json!({ "status": "no_why", "why":
            "не сказано, ПОЧЕМУ вид обязателен. Обязательность без довода нечем оспорить, \
             и снять её потом будет не за что." }));
    }
    let n = client
        .execute(
            "UPDATE kind_layout SET spec = spec || jsonb_build_object('required', $2::boolean,
                                                                      'required-why', $3::text)
              WHERE name = $1",
            &[&kind, &required, &why],
        )
        .await?;
    Ok(json!({ "status": if n > 0 { "declared" } else { "no_kind" }, "kind": kind,
               "required": required,
               "means": "минимальный набор объявлен один раз и общий для всех проектов" }))
}

/// Объявить, ЧЕМ вид становится в наборе.
///
/// Словарь: `done` — у вида своя предметная таблица; `container` — документ
/// держит сущности ДРУГИХ видов, и они лежат в названных таблицах; `due` —
/// таблица положена и не разложена; `prose` — рассуждение, предметом не
/// становится; `render`; `provenance`.
///
/// Слова `container` не было, и шестнадцать видов стояли «не объявлено» —
/// среди них `srs`, из которого набор берёт 509 требований и 227 проверок
/// КАЖДЫМ замером. Дыра была в словаре, а не в машине: сказать правду о них
/// было нечем, и ручка звала дефектом плана то, что работает.
///
/// Для `container` названные таблицы ПРОВЕРЯЮТСЯ по связи `entity_kind`.
/// Вид, объявленный держателем того, чего в таблицах нет, — это пустота,
/// только подписанная; она читается как знание. Дверь отказывает.
pub(crate) async fn set_kind_projection(
    pool: &Pool, kind: &str, projection: &str, holds: &[String],
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "вид без имени не объявляется" }));
    }
    const GLOSSARY: [&str; 6] = ["done", "container", "due", "prose", "render", "provenance"];
    if !GLOSSARY.contains(&projection) {
        return Ok(json!({ "status": "unknown_word", "word": projection,
            "vocabulary": GLOSSARY,
            "why": "проекция объявляется словом из словаря: новое слово не поймут ни ручки, \
                    ни правила, и вид останется невидимым при живой записи" }));
    }
    if client
        .query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_kind", "kind": kind,
            "why": "вид не объявлен в раскладке: сперва заводится вид, потом его проекция" }));
    }

    // Держатель без содержимого — отказ. Проверяется КАЖДАЯ названная таблица.
    let mut holds_rows = Vec::new();
    // `done` и `container` — оба УТВЕРЖДЕНИЯ ПРО ТАБЛИЦЫ, и проверяются одинаково.
    // Проверять только контейнер значило бы верить `done` на слово: `red-task`
    // стоял `due` при живой таблице, и заметить это было нечем.
    let about_table = matches!(projection, "container" | "done");
    if about_table {
        if holds.is_empty() {
            return Ok(json!({ "status": "holds_nothing", "kind": kind,
                "projection": projection,
                "why": "слово утверждает, что вид ложится в таблицы. В какие — не сказано, \
                        и проверить объявление нечем" }));
        }
        for table in holds {
            // Имя таблицы в запрос подставляется, поэтому берётся только то,
            // что схема подтвердила: чужого имени сюда не попадёт.
            if client
                .query_opt(
                    "SELECT 1 FROM information_schema.columns
                      WHERE table_schema = 'public' AND table_name = $1
                        AND column_name = 'entity_kind'",
                    &[table],
                )
                .await?
                .is_none()
            {
                return Ok(json!({ "status": "no_such_holder", "table": table,
                    "why": "такой таблицы нет, либо она не связана с документом колонкой \
                            `entity_kind`: связь держится записью, а не прозой" }));
            }
            let mut n: i64 = client
                .query_one(
                    &format!("SELECT count(*) FROM {table} WHERE entity_kind = $1"),
                    &[&kind],
                )
                .await?
                .get(0);
            // ВНУТРЕННИЙ ВИД ПОМЕЧЕН НЕ СОБОЙ. Записи требования лежат в
            // `project_requirements`, но `entity_kind` у них — `srs`: это
            // контейнер, из которого их вынули. Проверять их именем вида
            // значит требовать невозможного, и дверь отказывала `requirement`
            // при 306 живых записях.
            //
            // Второй способ так же строг: образец имени, объявленный самим
            // видом. Пустой перечень им не пройдёт.
            if n == 0 {
                if let Some(pattern) = client
                    .query_opt("SELECT spec->>'id' FROM kind_layout WHERE name = $1", &[&kind])
                    .await?
                    .and_then(|r| r.get::<_, Option<String>>(0))
                {
                    let exists: bool = client
                        .query_one(
                            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                                             WHERE table_schema='public' AND table_name=$1
                                               AND column_name='id')",
                            &[table],
                        )
                        .await?
                        .get(0);
                    if exists {
                        n = client
                            .query_one(&format!("SELECT count(*) FROM {table} WHERE id ~ $1"), &[&pattern])
                            .await?
                            .get(0);
                    }
                }
            }
            if n == 0 {
                return Ok(json!({ "status": "holds_nothing_here", "kind": kind, "table": table,
                    "why": "ни одной строки этого вида в названной таблице: объявление \
                            описывает то, чего набор не делает" }));
            }
            holds_rows.push(json!({ "table": table, "rows": n }));
        }
    } else if !holds.is_empty() {
        return Ok(json!({ "status": "holds_without_container", "kind": kind,
            "projection": projection,
            "why": "таблицы называют только `done` и `container`: прочие слова \
                    как раз и говорят, что предметом вид не становится" }));
    }

    let list = json!(holds);
    client
        .execute(
            "UPDATE kind_layout
                SET spec = spec || jsonb_build_object('projection', $2::text, 'holds', $3::jsonb)
              WHERE name = $1",
            &[&kind, &projection, &list],
        )
        .await?;
    Ok(json!({ "status": "declared", "kind": kind, "projection": projection,
               "holds": holds_rows,
               "means": "объявление ОБЩЕЕ: чем вид становится — одно для всех проектов" }))
}

/// Ребро графа связей: откуда, куда — и сколько раз с примерами.
type EdgeSeen = std::collections::HashMap<(String, String, String, String), (i64, Vec<String>)>;

/// Дерево связанного: от одного документа — всё, что с ним связано, по записям.
///
/// Замысел набора: сущности лежат по своим таблицам, а документы из них
/// собираются. `srs` — не текст про требования, это 306 требований. Значит от
/// любого документа можно дойти до всех связанных, и ходить надо по записям.
///
/// Ребро строится так: документ А называет имя, у имени есть определяющий
/// документ Б — значит А зависит от Б. Роль здесь обязательна: без неё Б был
/// бы любым из двадцати говорящих, а не тем одним, где имя написано.
///
/// Замер на `myack`: от `srs` первый уровень — `strs` через 77 потребностей и
/// `test-cases` через 19 проверок; второй — сорок документов.
pub(crate) async fn tree(
    pool: &Pool, project: &str, kind: &str, name: &str, depth: i32,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // Глубина ограничена: на четвёртом шаге обход накрывает почти весь набор, и
    // ответ «связано всё» не отвечает ни на один вопрос.
    let depth = depth.clamp(1, 4);
    if client
        .query_opt(
            "SELECT 1 FROM project_documents WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_document", "kind": kind, "name": name,
            "why": "такого документа в наборе нет: дерево строится от записи, а не от имени" }));
    }
    let rows = client
        .query(
            "WITH RECURSIVE шаг AS (
                 SELECT $2::text AS kind, $3::text AS name, 0 AS depth,
                        ARRAY[$2 || '/' || $3] AS путь,
                        ''::text AS parent_kind, ''::text AS parent_name
                 UNION ALL
                 SELECT b.entity_kind, b.entity_name, s.depth + 1,
                        s.путь || (b.entity_kind || '/' || b.entity_name),
                        s.kind, s.name
                   FROM шаг s
                   JOIN named_id_role a
                     ON a.project_id = $1 AND a.entity_kind = s.kind AND a.entity_name = s.name
                   JOIN named_id_role b
                     ON b.project_id = $1 AND b.said_id = a.said_id AND b.role = 'defines'
                  WHERE s.depth < $4
                    AND NOT ((b.entity_kind || '/' || b.entity_name) = ANY(s.путь)))
             SELECT kind, name, min(depth) AS depth,
                    (array_agg(parent_kind ORDER BY depth))[1] AS parent_kind,
                    (array_agg(parent_name ORDER BY depth))[1] AS parent_name
               FROM шаг GROUP BY kind, name ORDER BY min(depth), kind, name",
            &[&project, &kind, &name, &depth],
        )
        .await?;

    // Чем ребро держится: имена, из-за которых родитель дошёл до ребёнка.
    // Без них ответ «srs связан с strs» ничего не даёт: связь надо открыть.
    let through = client
        .query(
            "SELECT a.entity_kind, a.entity_name, b.entity_kind, b.entity_name,
                    count(DISTINCT a.said_id)::bigint, (array_agg(DISTINCT a.said_id))[1:3]
               FROM named_id_role a
               JOIN named_id_role b
                 ON b.project_id = a.project_id AND b.said_id = a.said_id AND b.role = 'defines'
              WHERE a.project_id = $1
                AND (b.entity_kind, b.entity_name) IS DISTINCT FROM (a.entity_kind, a.entity_name)
              GROUP BY 1, 2, 3, 4",
            &[&project],
        )
        .await?;
    let mut edge: EdgeSeen = std::collections::HashMap::new();
    for r in &through {
        edge.insert(
            (r.get(0), r.get(1), r.get(2), r.get(3)),
            (r.get(4), r.get::<_, Vec<String>>(5)),
        );
    }

    // Сборка в дерево: у каждого узла записан родитель, которым до него дошли
    // ПЕРВЫМ — самым коротким путём. Документ, до которого ведёт двадцать
    // дорог, стоит в ответе один раз.
    let mut children: std::collections::HashMap<(String, String), Vec<Value>> =
        std::collections::HashMap::new();
    let mut nodes = 0usize;
    for r in rows.iter().rev() {
        let (k, n): (String, String) = (r.get(0), r.get(1));
        let d: i32 = r.get(2);
        if d == 0 {
            continue;
        }
        nodes += 1;
        let (pk, pn): (String, String) = (r.get(3), r.get(4));
        let (how_many, examples) = edge
            .get(&(pk.clone(), pn.clone(), k.clone(), n.clone()))
            .cloned()
            .unwrap_or((0, Vec::new()));
        let own = children.remove(&(k.clone(), n.clone())).unwrap_or_default();
        let mut node = serde_json::Map::new();
        node.insert("kind".into(), json!(k));
        node.insert("name".into(), json!(n));
        node.insert("depth".into(), json!(d));
        node.insert("via".into(), json!(how_many));
        node.insert("names".into(), json!(examples));
        node.insert("by".into(), json!(format!("mh call {k} id={n}")));
        if !own.is_empty() {
            node.insert("children".into(), json!(own));
        }
        children.entry((pk, pn)).or_default().push(Value::Object(node));
    }
    Ok(json!({
        "root": { "kind": kind, "name": name },
        "depth": depth,
        "documents": nodes,
        "tree": children.remove(&(kind.to_owned(), name.to_owned())).unwrap_or_default(),
        "means": "ребро: этот документ называет имя, которое написано в том. \
                  Связь по записям, глубина ограничена четырьмя шагами",
    }))
}

/// Что документа уже живёт в таблицах, а что держит только текст.
///
/// Разворот модели — «таблица источник, документ сборка» — упирается в один
/// вопрос: что потеряется, если текст убрать. Отвечать на него мнением нельзя,
/// поэтому дверь считает по РАЗДЕЛАМ, а не по документу целиком: в одном
/// документе половина разделов воспроизводима, половина — рассуждение, и
/// среднее по нему врёт про обе.
///
/// Блок относится к САМОМУ ВНУТРЕННЕМУ разделу: у родителя `last_block`
/// накрывает и детей, и по нему абзац ребёнка засчитался бы дважды.
pub(crate) async fn document_coverage(
    pool: &Pool, project: &str, kind: &str, name: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if client
        .query_opt(
            "SELECT 1 FROM project_documents WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_document", "kind": kind, "name": name,
            "why": "такого документа в наборе нет: сверять сборку не с чем" }));
    }
    let rows = client
        .query(
            "WITH блок AS (
                 SELECT b.ord, b.kind AS вид, length(b.raw) AS байт,
                        (SELECT max(s.ord) FROM project_document_sections s
                          WHERE s.project_id = b.project_id AND s.entity_kind = b.entity_kind
                            AND s.entity_name = b.entity_name AND s.ord <= b.ord) AS раздел
                   FROM project_document_blocks b
                  WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3
                    AND b.kind <> 'blank'),
             строка AS (
                 SELECT section_ord FROM project_requirements
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                 UNION ALL SELECT section_ord FROM project_checks
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                 UNION ALL SELECT section_ord FROM project_terms
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                 UNION ALL SELECT section_ord FROM project_needs
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                 UNION ALL SELECT section_ord FROM project_risks
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                 -- Рассуждение — ТОЖЕ строка. Прежде раздел, несущий довод и
                 -- ничего не объявляющий, считался «только текст»: правда на тот
                 -- день, когда доводу негде было лежать.
                 UNION ALL SELECT section_ord FROM project_rationale
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3)
             SELECT s.ord, s.title, s.level,
                    coalesce((SELECT count(*) FROM строка r WHERE r.section_ord = s.ord), 0)::bigint AS строк,
                    coalesce((SELECT count(*) FROM блок b WHERE b.раздел = s.ord AND b.вид = 'table'), 0)::bigint,
                    coalesce((SELECT count(*) FROM блок b WHERE b.раздел = s.ord AND b.вид NOT IN ('table','heading')), 0)::bigint,
                    coalesce((SELECT sum(b.байт) FROM блок b WHERE b.раздел = s.ord AND b.вид NOT IN ('table','heading')), 0)::bigint,
                    EXISTS (SELECT 1 FROM project_document_sections c
                             WHERE c.project_id = s.project_id AND c.entity_kind = s.entity_kind
                               AND c.entity_name = s.entity_name AND c.parent_ord = s.ord)
               FROM project_document_sections s
              WHERE s.project_id = $1 AND s.entity_kind = $2 AND s.entity_name = $3
              ORDER BY s.ord",
            &[&project, &kind, &name],
        )
        .await?;

    let (mut at_table, mut only_text, mut mixed, mut empty) = (0, 0, 0, 0);
    let mut prose_bytes: i64 = 0;
    let mut sections = Vec::new();
    for r in &rows {
        let row_count: i64 = r.get(3);
        let prose: i64 = r.get(5);
        let bytes: i64 = r.get(6);
        let parent: bool = r.get(7);
        // Родительский заголовок без своего текста — не потеря и не находка:
        // он вернётся сам, когда соберутся дети.
        let verdict = match (row_count > 0, prose > 0, parent) {
            (true, false, _) => { at_table += 1; "воспроизводится из таблицы" }
            (true, true, _) => { mixed += 1; prose_bytes += bytes; "часть в таблице, часть только текстом" }
            (false, true, _) => { only_text += 1; prose_bytes += bytes; "только текст" }
            (false, false, true) => { empty += 1; "заголовок-родитель: соберётся из детей" }
            (false, false, false) => { empty += 1; "пусто" }
        };
        sections.push(json!({
            "ord": r.get::<_, i32>(0), "title": r.get::<_, String>(1),
            "level": r.get::<_, i32>(2),
            "rows": row_count, "tables": r.get::<_, i64>(4), "prose": prose, "bytes": bytes,
            "verdict": verdict,
        }));
    }
    Ok(json!({
        "kind": kind, "name": name,
        "sections": rows.len(),
        "fromTable": at_table, "textOnly": only_text, "mixed": mixed, "empty": empty,
        "proseBytes": prose_bytes,
        "bySection": sections,
        "means": "«только текст» и «часть только текстом» — это и есть то, что \
                  потеряется, если убрать текст. Считано по разделам: среднее \
                  по документу врёт про обе половины",
    }))
}

/// Объявить, переоткрывается ли вид при правке того, на что он опирается.
///
/// Вопрос, закрытый по требованию, которое потом изменили, закрыт по памяти:
/// его ответ говорил о тексте, которого больше нет. Задача, сделавшая
/// изменённое требование, тоже не сделана. А вот РЕШЕНИЕ так не работает —
/// оно датированная запись о выборе, и не становится непринятым оттого, что
/// требование переписали; его отменяют отдельным решением, а не пересборкой.
///
/// Поэтому «переоткрывается» — объявление, а не догадка машины.
///
/// Проверяется возможность: у вида должна быть предметная таблица с колонкой
/// состояния. Объявить переоткрываемым то, у чего состояния нет, — подписать
/// намерение, которое никогда не исполнится.
pub(crate) async fn set_kind_reopens(
    pool: &Pool, kind: &str, reopens: bool, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "вид без имени не объявляется" }));
    }
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_why", "kind": kind,
            "why": "не сказано, ПОЧЕМУ. Переоткрытие — это отмена чужой работы по расписанию, \
                    и оспорить его потом будет нечем" }));
    }
    let Some(spec) = client
        .query_opt("SELECT spec FROM kind_layout WHERE name = $1", &[&kind])
        .await?
    else {
        return Ok(json!({ "status": "no_kind", "kind": kind,
            "why": "вид не объявлен в раскладке" }));
    };
    // Состояние ищется в таблицах, которые вид объявил своими: `holds` уже
    // назван и проверен дверью проекции — второго перечня заводить не надо.
    let spec: Value = spec.get(0);
    let holds: Vec<String> = spec
        .get("holds")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if reopens {
        if holds.is_empty() {
            return Ok(json!({ "status": "no_tables", "kind": kind,
                "why": "вид не назвал таблиц (`kind-projection ... holds`): искать состояние негде" }));
        }
        let mut where_at = Vec::new();
        for table in &holds {
            let exists: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                                     WHERE table_schema = 'public' AND table_name = $1
                                       AND column_name IN ('state','status','closed','satisfied'))",
                    &[table],
                )
                .await?
                .get(0);
            if exists {
                where_at.push(table.clone());
            }
        }
        if where_at.is_empty() {
            return Ok(json!({ "status": "no_state", "kind": kind, "tables": holds,
                "why": "ни в одной таблице вида нет колонки состояния (`state`, `status`, \
                        `closed`, `satisfied`): переоткрывать нечего" }));
        }
        client
            .execute(
                "UPDATE kind_layout SET spec = spec || jsonb_build_object('reopens', true,
                                                                          'reopens-why', $2::text)
                  WHERE name = $1",
                &[&kind, &why],
            )
            .await?;
        return Ok(json!({ "status": "declared", "kind": kind, "reopens": true,
                          "stateIn": where_at,
                          "means": "объявление ОБЩЕЕ: чем вид становится при правке связей — \
                                    одно для всех проектов" }));
    }
    client
        .execute(
            "UPDATE kind_layout SET spec = spec || jsonb_build_object('reopens', false,
                                                                      'reopens-why', $2::text)
              WHERE name = $1",
            &[&kind, &why],
        )
        .await?;
    Ok(json!({ "status": "declared", "kind": kind, "reopens": false }))
}

/// Таблицы, где записи вида лежат под своими именами: объявленные видом своими
/// (`kind-projection … holds`) и держащие запись колонкой `id` рядом с
/// `project_id`.
///
/// Держатель бывает и чужих строк: у `index` это связи и источники поверхностей,
/// у них `id` нет. Спрашивать их по имени записи значило ронять дверь
/// «column id does not exist» — `entity-rename` падала на всём виде.
async fn own_tables(
    client: &impl deadpool_postgres::GenericClient, kind: &str,
) -> Result<Vec<String>, crate::db::Fail> {
    Ok(client
        .query(
            "SELECT h.t
               FROM kind_layout k
              CROSS JOIN LATERAL jsonb_array_elements_text(
                      CASE WHEN jsonb_typeof(k.spec->'holds') = 'array' THEN k.spec->'holds' ELSE '[]' END
                    ) WITH ORDINALITY AS h(t, n)
              WHERE k.name = $1
                AND EXISTS (SELECT 1 FROM information_schema.columns c
                             WHERE c.table_schema = 'public' AND c.table_name = h.t AND c.column_name = 'id')
                AND EXISTS (SELECT 1 FROM information_schema.columns c
                             WHERE c.table_schema = 'public' AND c.table_name = h.t AND c.column_name = 'project_id')
              ORDER BY h.n",
            &[&kind],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect())
}

/// Переименовать сущность во ВСЁМ наборе.
///
/// Имена приехали переносом, и часть приехала не в том формате: у `tot-ade`
/// 353 сущности зовутся не по общему образцу — `OQ-01` вместо `Q-01`, `UI-01`
/// вместо `FR-UI-01`. Живут они в двух документах, а УПОМИНАЮТСЯ в 427: руками
/// такое не правят, а мимо сервера — тем более.
///
/// Сухой ход по умолчанию, и это та же правка, откаченная, а не её пересказ:
/// показано ровно то, что сделает запись, — колонки, записи и снятое. Ревизия
/// документа сохраняется, как при всякой правке.
///
/// Новое имя проверяется ОБЩИМ образцом вида, а не проектным: переименование
/// ради того и делается, чтобы расхождение ушло.
pub(crate) async fn rename_entity(
    pool: &Pool, project: &str, kind: &str, from: &str, to: &str, apply: bool, merge: bool,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL lock_timeout = '2s'").await?;
    let mut report = renaming(&tx, project, kind, from, to, merge).await?;
    if report["status"] != "renamed" || !apply {
        tx.rollback().await?;
        if report["status"] == "renamed" {
            report["status"] = json!("dry");
            report["means"] = json!("показано ровно то, что сделает запись, и ничего не записано. \
                                     Запись — тем же вызовом с `apply=true`");
        }
        return Ok(report);
    }
    tx.commit().await?;
    drop(client);
    // РАЗБОР ДОКУМЕНТА — ЧАСТЬЮ ПРАВКИ. Дверь правила текст и на этом
    // останавливалась: блоки, секции и ячейки оставались прежними, и вниз по
    // течению не менялось НИЧЕГО — переименовал 71 вопрос, а проекция
    // показывала прежние имена, потому что читала старые ячейки.
    //
    // Правка документа мимо разбора — это правка, которой набор не увидит.
    crate::store::reparse_all(pool, project).await?;
    report["means"] = json!(if report["dropped"].as_i64().unwrap_or(0) > 0 {
        "документы разобраны заново; проекции устарели — позовите `reproject`. \
         ВНИМАНИЕ: снято строк — это склейка, названная словом `merge`"
    } else {
        "документы разобраны заново; проекции устарели — позовите `reproject`"
    });
    Ok(report)
}

async fn renaming(
    tx: &impl deadpool_postgres::GenericClient, project: &str, kind: &str, from: &str, to: &str, merge: bool,
) -> Result<Value, crate::db::Fail> {
    if from.trim().is_empty() || to.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "нужны оба имени: старое и новое" }));
    }
    if from == to {
        return Ok(json!({ "status": "same", "why": "имена совпадают: переименовывать нечего" }));
    }
    if tx.query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&from]).await?.is_some() {
        return Ok(json!({ "status": "kind_word", "from": from,
            "why": "старое имя совпадает с именем вида: правка идёт по всем колонкам набора и \
                    переписала бы вид у всех его записей вместе с именем" }));
    }
    let Some(pattern) = tx
        .query_opt("SELECT spec->>'id' FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .and_then(|r| r.get::<_, Option<String>>(0))
    else {
        return Ok(json!({ "status": "no_pattern", "kind": kind,
            "why": "у вида не объявлен образец имени: сверить новое имя не с чем" }));
    };
    let compare = tx.query_one("SELECT $1 ~ $2, ($1 || '/' || $1) ~ $2", &[&to, &pattern]).await?;
    let (matches, slash_at_name): (bool, bool) =
        (compare.get(0), compare.get::<_, bool>(1) || from.contains('/') || to.contains('/'));
    if !matches {
        return Ok(json!({ "status": "not_by_pattern", "to": to, "pattern": pattern,
            "why": "новое имя не следует общему образцу вида: переименование ради того и \
                    делается, чтобы расхождение ушло, а не переехало" }));
    }
    let own = own_tables(tx, kind).await?;
    // ЗАНЯТОСТЬ — ЭТО ЧУЖАЯ ЗАПИСЬ, а не упоминание в тексте. Проверка по
    // документам отвергала законное доделывание: текст уже переименован
    // половинным ходом, а объявленная запись осталась со старым именем, и
    // дверь отказывала «занято» — самой себе.
    //
    // Запись же — доказательство: две сущности с одним именем не бывают.
    let document_taken = documents_kind(tx, project, kind, to).await?;
    let mut busy: i64 = document_taken;
    for table in &own {
        busy += records_under_name(tx, table, project, to).await?;
    }
    // `merge` — ЯВНОЕ слово о том, что это одна и та же сущность, заведённая
    // дважды: разбором под новым именем и остатком под старым. Без него отказ
    // остаётся, потому что отличить «тот же» от «другой» машине нечем, а
    // молчаливое слияние двух сущностей в одну не заметит никто.
    if busy > 0 && !merge {
        return Ok(json!({ "status": "taken", "to": to, "mentions": busy,
            "why": if document_taken > 0 {
                "документ этого вида под новым именем уже есть. `merge=true` СНИМЕТ документ \
                 под старым именем целиком — его текст, ячейки, блоки и связи — и оставит \
                 документ под новым; сухой ход с `merge=true` покажет снятое"
            } else {
                "запись под новым именем уже есть. Если это ТА ЖЕ сущность, заведённая \
                 дважды, скажите об этом словом: `merge=true` — старая строка будет снята, \
                 и снятое посчитано отдельно"
            } }));
    }

    let pattern_words = name_in_text(from, slash_at_name);
    let replacement = replacement_of(to);
    let affected = tx
        .query(
            "SELECT entity_kind, entity_name,
                    ((length(content) - length(regexp_replace(content, $2, '', 'g')))
                      / greatest(length($3), 1))::bigint AS сколько
               FROM project_documents
              WHERE project_id = $1 AND content ~ $2
              ORDER BY 1, 2",
            &[&project, &pattern_words, &from],
        )
        .await?;
    let mentions: i64 = affected.iter().map(|r| r.get::<_, i64>(2)).sum();
    // ПРЕДМЕТ — И ЗАПИСЬ ТОЖЕ, не только текст. Документы могли быть уже
    // переименованы, а объявленная запись остаться со старым именем: тогда
    // «в документах не нашли» — это не «нечего делать», а ровно половина
    // работы, которую и надо доделать.
    let document_old = documents_kind(tx, project, kind, from).await?;
    let mut records: i64 = 0;
    for table in &own {
        records += records_under_name(tx, table, project, from).await?;
    }
    if affected.is_empty() && document_old == 0 && records == 0 {
        return Ok(json!({ "status": "not_found", "from": from,
            "why": "такого имени нет ни в одном документе и ни в одной записи" }));
    }
    let document_moves = document_old > 0 && document_taken == 0;
    let foreign_history: i64 = tx
        .query_one(
            "SELECT count(*) FROM project_document_revisions
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &to],
        )
        .await?
        .get(0);
    if document_moves && foreign_history > 0 {
        return Ok(json!({ "status": "history_taken", "to": to, "revisions": foreign_history,
            "why": "под новым именем лежит история ревизий документа, которого уже нет: \
                    переезд смешал бы её с историей переименуемого документа" }));
    }

    // Правка сперва, ревизия следом: текущее содержимое уже записано под своим
    // номером, и попытка записать его второй раз падала на уникальности пары
    // «документ, ревизия». Ревизия — это НОВАЯ версия, а не копия старой.
    let documents = tx
        .execute(
            "UPDATE project_documents
                SET content = regexp_replace(content, $2, $3, 'g'),
                    revision = revision + 1,
                    updated_at = $4
              WHERE project_id = $1 AND content ~ $2",
            &[&project, &pattern_words, &replacement, &now_ms()],
        )
        .await?;
    tx.execute(
        "INSERT INTO project_document_revisions
                (project_id, entity_kind, entity_name, content, content_hash, bytes, revision,
                 written_at, written_by)
         SELECT project_id, entity_kind, entity_name, content, md5(content), length(content),
                revision, $2, 'rename'
           FROM project_documents
          WHERE project_id = $1 AND content ~ $3
         ON CONFLICT DO NOTHING",
        &[&project, &now_ms(), &regex_escape(to)],
    )
    .await?;
    // ОБЪЯВЛЕННАЯ ЗАПИСЬ ПЕРЕИМЕНОВЫВАЕТСЯ ТОЖЕ. У `tot-ade` все 203 требования
    // и все вопросы заведены ДВЕРЬЮ, а не выведены из документов: пересборка их
    // не трогает, и правка текста меняла бы документы, оставляя записи со
    // старыми именами. Переименование вышло бы половинным и тихим.
    //
    // Таблицы берутся из объявления вида (`holds`), а не перечислены здесь.
    // Имя внутри КОЛОНОК — ссылочных и прозаических. Обход меряет, где имя
    // действительно стоит, и правит только там; перечень колонок в коде
    // разошёлся бы со схемой.
    let at_columns = tx
        .query("SELECT таблица, колонка, строк, снято FROM rename_matching_in_columns($1, $2, $3, $4)",
               &[&project, &pattern_words, &replacement, &merge])
        .await?;
    // СНЯТОЕ НАЗЫВАЕТСЯ. Колонка `снято` считалась и отбрасывалась: ответ нёс
    // `rows: 0` там, где строку удалили, и удаление под видом переименования
    // оставалось невидимым — ровно то, чего довод у самой функции велит не
    // делать.
    let dropped: i64 = at_columns.iter().map(|r| r.get::<_, i64>(3)).sum();
    let columns: Vec<Value> = at_columns
        .iter()
        .map(|r| json!({ "table": r.get::<_, String>(0), "column": r.get::<_, String>(1),
                         "rows": r.get::<_, i64>(2), "dropped": r.get::<_, i64>(3) }))
        .collect();
    for table in &own {
        tx.execute(
            &format!("UPDATE {table} SET id = $2 WHERE project_id = $1 AND id = $3"),
            &[&project, &to, &from],
        )
        .await?;
    }
    let revisions = if document_moves {
        tx.execute(
            "UPDATE project_document_revisions SET entity_name = $4
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &from, &to],
        )
        .await?
    } else {
        0
    };
    Ok(json!({ "status": "renamed", "from": from, "to": to,
               "documents": documents, "mentions": mentions, "records": records,
               "columns": columns, "dropped": dropped, "revisions": revisions,
               "where": affected.iter().take(8).map(|r| json!({
                   "kind": r.get::<_, String>(0), "name": r.get::<_, String>(1),
                   "times": r.get::<_, i64>(2) })).collect::<Vec<_>>() }))
}

async fn records_under_name(
    client: &impl deadpool_postgres::GenericClient, table: &str, project: &str, name: &str,
) -> Result<i64, crate::db::Fail> {
    Ok(client
        .query_one(&format!("SELECT count(*) FROM {table} WHERE project_id = $1 AND id = $2"), &[&project, &name])
        .await?
        .get(0))
}

/// Документ вида под этим именем — тоже запись: у `index` и `mockup` другой нет.
async fn documents_kind(
    client: &impl deadpool_postgres::GenericClient, project: &str, kind: &str, name: &str,
) -> Result<i64, crate::db::Fail> {
    Ok(client
        .query_one(
            "SELECT count(*) FROM project_documents WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?
        .get(0))
}

/// Образец имени в тексте и в колонках. Граница — не `\m…\M`: имена бывают
/// путями, и `.`, `-` — их буквы, а не края. Точка краем остаётся, если по ту
/// сторону не буква: конец предложения — не расширение файла, `...` — не `v1.`.
/// Косая — буква имени только у вида, чей образец пускает её внутрь имени: у
/// `index` `20-surface/configure` — другое имя, у вопросов `OQ-01/OQ-02` — два.
fn name_in_text(name: &str, slash_in_name: bool) -> String {
    let (before, after) = if slash_in_name { ("/", "|/[[:alnum:]_.-]") } else { ("", "") };
    format!(
        "(?<![[:alnum:]_{before}-])(?<![[:alnum:]]\\.){}(?![[:alnum:]_-]|\\.[[:alnum:]]{after})",
        regex_escape(name)
    )
}

/// Экранирование имени для регулярного выражения: имя приходит снаружи, и
/// точка в нём должна значить точку.
fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if "\\^$.|?*+()[]{}".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

fn replacement_of(name: &str) -> String {
    name.replace('\\', "\\\\")
}

/// Завести вид сущности.
///
/// Трёх дверей — `kind-required`, `kind-projection`, `kind-reopens` — хватало,
/// пока виды не заводились: все 57 приехали переносом. А заводить приходится:
/// у `tot-ade` под одним видом `check` живут три разные вещи — 56 критериев
/// приёмки, 141 имя правила В КОДЕ и 30 утверждений с пространством имён.
/// Свести их к общему образцу `TC-ОБЛАСТЬ-nn` значит порвать связь кода с его
/// же правилами ради формата.
///
/// Образец имени ПРОВЕРЯЕТСЯ: неразбираемое выражение молча не поймает ничего,
/// и вид останется пустым при живых сущностях.
pub(crate) async fn add_kind(
    pool: &Pool, name: &str, shape: &str, id_pattern: &str, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if name.trim().is_empty() || why.trim().is_empty() {
        return Ok(json!({ "status": "empty",
            "why": "нужны имя вида и довод: вид без довода не оспорить" }));
    }
    if !matches!(shape, "document" | "inner") {
        return Ok(json!({ "status": "unknown_shape", "shape": shape,
            "vocabulary": ["document", "inner"],
            "why": "форма вида: `document` — у сущности свой документ, `inner` — она \
                    объявлена строкой внутри чужого" }));
    }
    if client
        .query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&name])
        .await?
        .is_some()
    {
        return Ok(json!({ "status": "exists", "kind": name,
            "why": "вид уже объявлен: правьте его дверями `kind-projection`, \
                    `kind-required`, `kind-reopens`" }));
    }
    // Образец проверяется исполнением: `~` на неразбираемом выражении падает,
    // и лучше отказать сейчас, чем завести вид, который никого не найдёт.
    if !id_pattern.trim().is_empty()
        && client.query_one("SELECT 'проба' ~ $1", &[&id_pattern]).await.is_err()
    {
        return Ok(json!({ "status": "bad_pattern", "pattern": id_pattern,
            "why": "образец имени не разбирается: вид с таким образцом не найдёт ни одной \
                    сущности и промолчит об этом" }));
    }
    let spec = json!({ "shape": shape, "id": id_pattern, "why": why });
    client
        .execute(
            "INSERT INTO kind_layout (name, spec, declared_at, declared_by)
             VALUES ($1, $2, $3, 'kind-add')",
            &[&name, &spec, &now_ms()],
        )
        .await?;
    Ok(json!({ "status": "declared", "kind": name, "shape": shape, "id": id_pattern,
               "means": "вид объявлен ОБЩИМ: он виден всем наборам. Чем он становится — \
                         отдельной дверью `kind-projection`" }))
}

/// Объявить, годится ли род доказательством требования.
///
/// Тест-кейс пробует случай; правило кода отказывается собирать нарушение.
/// Оба доказывают, но по-разному, и что из этого принято в проекте — суждение
/// владельца, а не машины. Прежде правило знало один ответ и зашивало его.
///
/// Проверяется, что такие связи ЕСТЬ: объявить доказательством род, которым
/// ничего не доказано, — подписать намерение вместо факта.
pub(crate) async fn set_kind_proves(
    pool: &Pool, project: &str, kind: &str, proves: bool, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() || why.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "нужны род и довод" }));
    }
    if client
        .query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_kind", "kind": kind, "why": "вид не объявлен" }));
    }
    let links: i64 = client
        .query_one(
            "SELECT count(*) FROM project_requirement_proof
              WHERE project_id = $1 AND proof_kind = $2",
            &[&project, &kind],
        )
        .await?
        .get(0);
    if proves && links == 0 {
        return Ok(json!({ "status": "proves_nothing", "kind": kind,
            "why": "этим родом в наборе не доказано ни одно требование: объявление \
                    описывало бы то, чего нет" }));
    }
    client
        .execute(
            "UPDATE kind_layout SET spec = spec || jsonb_build_object('proves', $2::boolean,
                                                                      'proves-why', $3::text)
              WHERE name = $1",
            &[&kind, &proves, &why],
        )
        .await?;
    Ok(json!({ "status": "declared", "kind": kind, "proves": proves, "links": links,
               "means": "объявление ОБЩЕЕ: чем доказывают требование — одно для всех проектов" }))
}

/// Объявить образец имени вида — или поправить объявленный.
///
/// Образец ставился только при заведении вида, и поправить его было нечем:
/// я завёл `rationale` с образцом `^[a-zа-яё0-9-]+$`, а имя рассуждения —
/// его АДРЕС (`srs#3-20-invarianty`), и решётка с косой чертой под образец не
/// подошли. Вид остался невидимым при 381 живой записи.
///
/// Новый образец ПРОВЕРЯЕТСЯ на живых именах: тот, под который не подходит ни
/// одно имя, не поймает ничего и промолчит об этом.
pub(crate) async fn set_kind_id(
    pool: &Pool, project: &str, kind: &str, pattern: &str, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() || pattern.trim().is_empty() || why.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "нужны вид, образец и довод" }));
    }
    if client
        .query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_kind", "kind": kind, "why": "вид не объявлен" }));
    }
    if client.query_one("SELECT 'проба' ~ $1", &[&pattern]).await.is_err() {
        return Ok(json!({ "status": "bad_pattern", "pattern": pattern,
            "why": "образец не разбирается" }));
    }
    // Сверка на живых именах: таблицы берутся из объявления вида.
    let own = own_tables(&*client, kind).await?;
    let mut matched = 0i64;
    let mut total = 0i64;
    for table in &own {
        let r = client
            .query_one(
                &format!(
                    "SELECT count(*) FILTER (WHERE id ~ $2), count(*) FROM {table} WHERE project_id = $1"
                ),
                &[&project, &pattern],
            )
            .await?;
        matched += r.get::<_, i64>(0);
        total += r.get::<_, i64>(1);
    }
    if total > 0 && matched == 0 {
        return Ok(json!({ "status": "matches_nothing", "pattern": pattern, "names": total,
            "why": "под этот образец не подходит ни одно живое имя вида: он не поймает \
                    ничего и промолчит об этом" }));
    }
    client
        .execute(
            "UPDATE kind_layout SET spec = spec || jsonb_build_object('id', $2::text,
                                                                      'id-why', $3::text)
              WHERE name = $1",
            &[&kind, &pattern, &why],
        )
        .await?;
    Ok(json!({ "status": "declared", "kind": kind, "id": pattern,
               "matched": matched, "of": total }))
}

/// Скелет зависимостей набора: какие роды на каких стоят и сколькими связями.
///
/// `entity_link` держит три тысячи связей, и поодиночке их не окинуть взглядом.
/// Свёрнутые до родов, они умещаются в два десятка рёбер — и это карта того,
/// как устроен проект: задача стоит на требовании, требование на потребности,
/// этап на своих перечнях.
pub(crate) async fn links_graph(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let edges = client
        .query(
            "SELECT from_kind, to_kind, count(*)::bigint, count(DISTINCT from_id)::bigint
               FROM entity_link WHERE project_id = $1
              GROUP BY from_kind, to_kind ORDER BY count(*) DESC",
            &[&project],
        )
        .await?;
    // Узел знает свой счёт и сколько его записей переоткрыто: карта показывает
    // не только устройство, но и где сейчас горит.
    let nodes = client
        .query(
            "SELECT s.kind, count(*)::bigint,
                    count(*) FILTER (WHERE l.live_state = 'reopened')::bigint,
                    coalesce((SELECT (k.spec->>'reopens')::boolean FROM kind_layout k
                               WHERE k.name = s.kind), false)
               FROM entity_stamp s
               LEFT JOIN entity_live l
                 ON l.project_id = s.project_id AND l.kind = s.kind AND l.id = s.id
              WHERE s.project_id = $1
              GROUP BY s.kind ORDER BY count(*) DESC",
            &[&project],
        )
        .await?;
    // ОБЛАСТИ. Семнадцать родов и семнадцать рёбер — ещё перечень; свёрнутые
    // до областей, они дают восемь строк, и в них видна архитектура: план
    // стоит на требованиях, знание на требованиях, доказательство на них же.
    let dom = client
        .query("SELECT name, spec->>'domain' FROM kind_layout WHERE spec->>'domain' IS NOT NULL", &[])
        .await?;
    let area: std::collections::HashMap<String, String> =
        dom.iter().map(|r| (r.get(0), r.get(1))).collect();
    let mut digest: std::collections::BTreeMap<(String, String), (i64, i64)> = Default::default();
    for r in &edges {
        let f = area.get(&r.get::<_, String>(0)).cloned().unwrap_or_default();
        let t = area.get(&r.get::<_, String>(1)).cloned().unwrap_or_default();
        let e = digest.entry((f, t)).or_insert((0, 0));
        e.0 += r.get::<_, i64>(2);
        e.1 += 1;
    }
    Ok(json!({
        "nodes": nodes.iter().map(|r| json!({
            "kind": r.get::<_, String>(0), "count": r.get::<_, i64>(1),
            "reopened": r.get::<_, i64>(2), "reopens": r.get::<_, bool>(3),
            "domain": area.get(&r.get::<_, String>(0)).cloned().unwrap_or_default() })).collect::<Vec<_>>(),
        "edges": edges.iter().map(|r| json!({
            "from": r.get::<_, String>(0), "to": r.get::<_, String>(1),
            "links": r.get::<_, i64>(2), "sources": r.get::<_, i64>(3) })).collect::<Vec<_>>(),
        "domains": digest.iter().map(|((f, t), (n, pairs))| json!({
            "from": f, "to": t, "links": n, "pairs": pairs })).collect::<Vec<_>>(),
        "means": "ребро «А → Б» значит «А стоит на Б»: правка Б переоткрывает А",
    }))
}

/// Что переоткроется, если эту запись изменить.
///
/// Вопрос, на который харнес умел отвечать только задним числом: правку делали,
/// пересобирали и смотрели. Здесь он задаётся ВПЕРЁД — обход идёт по тем же
/// направленным связям и той же глубине, что и каскад, но от предполагаемой
/// правки, а не от случившейся.
pub(crate) async fn impact(
    pool: &Pool, project: &str, kind: &str, id: &str, depth: i32,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let depth = depth.clamp(1, 6);
    if client
        .query_opt(
            "SELECT 1 FROM entity_stamp WHERE project_id = $1 AND kind = $2 AND id = $3",
            &[&project, &kind, &id],
        )
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_entity", "kind": kind, "id": id,
            "why": "такой записи в наборе нет: считать последствия правки не от чего" }));
    }
    let rows = client
        .query(
            "WITH RECURSIVE вверх AS (
                 SELECT l.from_kind AS kind, l.from_id AS id, 1 AS depth
                   FROM entity_link l
                  WHERE l.project_id = $1 AND l.to_kind = $2 AND l.to_id = $3
                 UNION
                 SELECT l.from_kind, l.from_id, в.depth + 1
                   FROM вверх в
                   JOIN entity_link l
                     ON l.project_id = $1 AND l.to_kind = в.kind AND l.to_id = в.id
                  WHERE в.depth < $4)
             SELECT в.kind, в.id, min(в.depth)::int,
                    coalesce((SELECT (k.spec->>'reopens')::boolean FROM kind_layout k
                               WHERE k.name = в.kind), false)
               FROM вверх в GROUP BY в.kind, в.id ORDER BY 3, 1, 2",
            &[&project, &kind, &id, &depth],
        )
        .await?;
    let total = rows.len();
    let reopens = rows.iter().filter(|r| r.get::<_, bool>(3)).count();
    Ok(json!({
        "root": { "kind": kind, "id": id }, "depth": depth,
        "touched": total, "reopens": reopens,
        "items": rows.iter().take(200).map(|r| json!({
            "kind": r.get::<_, String>(0), "id": r.get::<_, String>(1),
            "depth": r.get::<_, i32>(2), "reopens": r.get::<_, bool>(3) })).collect::<Vec<_>>(),
        "means": "перечислено то, что СТОИТ НА этой записи. Переоткроется лишь то, чей род \
                  объявлен переоткрываемым; остальное связано, но состояния не меняет",
    }))
}

/// Пульт: что идёт, что ждёт человека, что поехало.
///
/// Три вопроса, ради которых человек открывает проект, — и все три прежде
/// собирались руками из разных ручек. Работа агентов не отдавалась вовсе:
/// `project_task_runs` знает, кто над чем сидит, и дверей к нему не было ни
/// одной.
pub(crate) async fn console(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;

    // Что идёт ПРЯМО СЕЙЧАС: прогон задачи с агентом и состоянием.
    let runs = client
        .query(
            "SELECT r.task_id, r.agent_id, r.state, r.attempt,
                    coalesce(t.title, ''), coalesce(w.state, ''), coalesce(w.branch, '')
               FROM project_task_runs r
               LEFT JOIN project_plan_tasks t
                 ON t.project_id = r.project_id AND t.id = r.task_id
               LEFT JOIN project_task_workspaces w
                 ON w.project_id = r.project_id AND w.task_run_id = r.id
              WHERE r.project_id = $1 AND r.state <> ALL (ARRAY['done', 'failed', 'cancelled'])
              ORDER BY r.updated_at DESC NULLS LAST
              LIMIT 20",
            &[&project],
        )
        .await?;

    // Что ждёт ЧЕЛОВЕКА: вопрос, отданный владельцу, без человека не сдвинется.
    let asks = client
        .query(
            "SELECT id, title, coalesce(nullif(created_at::text,''), '')
               FROM project_questions
              WHERE project_id = $1 AND state = 'open' AND answer_state = 'owner'
              ORDER BY number DESC LIMIT 40",
            &[&project],
        )
        .await?;

    // Что ПОЕХАЛО: записи, стоявшие на том, что правили после них.
    let moved = client
        .query(
            "SELECT kind, id, coalesce(stale_link, ''), coalesce(why, ''), coalesce(depth, 0)
               FROM entity_live
              WHERE project_id = $1 AND live_state = 'reopened'
              ORDER BY link_changed DESC NULLS LAST LIMIT 40",
            &[&project],
        )
        .await?;
    let moved_by_kind = client
        .query(
            "SELECT kind, count(*)::bigint FROM entity_live
              WHERE project_id = $1 AND live_state = 'reopened'
              GROUP BY kind ORDER BY count(*) DESC",
            &[&project],
        )
        .await?;

    Ok(json!({
        "working": runs.iter().map(|r| json!({
            "task": r.get::<_, String>(0), "agent": r.get::<_, Option<String>>(1),
            "state": r.get::<_, String>(2), "attempt": r.get::<_, i32>(3),
            "title": r.get::<_, String>(4),
            "workspace": r.get::<_, String>(5), "branch": r.get::<_, String>(6),
        })).collect::<Vec<_>>(),
        "asks": asks.iter().map(|r| json!({
            "id": r.get::<_, String>(0), "title": r.get::<_, String>(1),
            "since": r.get::<_, String>(2),
        })).collect::<Vec<_>>(),
        "moved": moved.iter().map(|r| json!({
            "kind": r.get::<_, String>(0), "id": r.get::<_, String>(1),
            "cause": r.get::<_, String>(2), "why": r.get::<_, String>(3),
            "depth": r.get::<_, i32>(4),
        })).collect::<Vec<_>>(),
        "movedByKind": moved_by_kind.iter().map(|r| json!({
            "kind": r.get::<_, String>(0), "count": r.get::<_, i64>(1) })).collect::<Vec<_>>(),
        "means": "три полосы пульта: что идёт · что ждёт человека · что поехало",
    }))
}

/// Заявка в очередь владельца: изменение харнеса либо подтверждение коммита.
pub(crate) async fn add_ask(
    pool: &Pool, project: &str, kind: &str, title: &str, body: &str, run_id: &str, by: &str,
) -> Result<Value, crate::db::Fail> {
    if !["request", "approval", "question"].contains(&kind) {
        return Ok(json!({ "status": "kind_unknown", "kind": kind,
            "why": "заявка бывает трёх родов: request — изменение харнеса, approval — подтверждение коммита \
                    и пуша, question — вопрос, без ответа на который прогон не идёт дальше" }));
    }
    if title.trim().is_empty() {
        return Ok(json!({ "status": "nameless",
            "why": "заявка без строки о том, что нужно, читается как пустое место в очереди" }));
    }
    if kind != "request" && run_id.trim().is_empty() {
        return Ok(json!({ "status": "no_run",
            "why": "подтверждение и вопрос просит прогон: без его имени решение некуда вернуть" }));
    }
    let client = crate::db::conn(pool).await?;
    let row = client
        .query_one(
            "INSERT INTO owner_ask (project_id, kind, title, body, asked_by, at, run_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            &[&project, &kind, &title, &body, &by, &now_ms(), &run_id],
        )
        .await?;
    Ok(json!({ "status": "asked", "id": row.get::<_, i64>(0), "kind": kind, "title": title }))
}

/// Очередь: что ждёт решения. Решённое отдаётся по просьбе — им проверяют, что
/// ответ доехал.
pub(crate) async fn list_asks(pool: &Pool, project: &str, state: &str, limit: i64) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT id, project_id, kind, title, body, asked_by, at, run_id, state, why,
                    decided_by, coalesce(decided_at, 0), question_id
               FROM owner_ask
              WHERE ($1 = '' OR project_id = $1 OR project_id = '')
                AND ($2 = '' OR state = $2)
              ORDER BY at DESC LIMIT $3",
            &[&project, &state, &limit],
        )
        .await?;
    Ok(json!({ "count": rows.len(), "asks": rows.iter().map(|r| json!({
        "id": r.get::<_, i64>(0), "project": r.get::<_, String>(1), "kind": r.get::<_, String>(2),
        "title": r.get::<_, String>(3), "body": r.get::<_, String>(4), "askedBy": r.get::<_, String>(5),
        "at": r.get::<_, i64>(6), "runId": r.get::<_, String>(7), "state": r.get::<_, String>(8),
        "why": r.get::<_, String>(9), "decidedBy": r.get::<_, String>(10),
        "decidedAt": r.get::<_, i64>(11), "questionId": r.get::<_, String>(12) })).collect::<Vec<_>>() }))
}

/// Вопросы, отданные владельцу, — в очереди пульта.
///
/// Решение владельца (#12): вопрос с пометкой «решает владелец» не держит
/// ступень 5 и ОБЯЗАТЕЛЬНО виден владельцу. Пометка ставится двумя путями —
/// ролью `section.owner-decides` у документа вопроса и доводом `owner` у
/// `question-add`, — и сводятся они здесь, в пересборке, через которую идут оба.
///
/// Запись заводится один раз на вопрос: ответ владельца не повторяет вопрос.
/// Вопрос, закрытый в наборе или снятый с владельца, закрывает свою запись.
pub(crate) async fn sync_owner_questions(pool: &Pool, project: &str) -> Result<u64, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let now = now_ms();
    let asked = client
        .execute(
            "INSERT INTO owner_ask (project_id, kind, title, body, asked_by, at, question_id)
             SELECT q.project_id, 'question', q.id || ' · ' || q.title, coalesce(q.state_text, ''), 'набор', $2, q.id
               FROM project_questions q
              WHERE q.project_id = $1 AND q.state = 'open' AND q.answer_state = 'owner'
                AND NOT EXISTS (SELECT 1 FROM owner_ask a WHERE a.project_id = q.project_id AND a.question_id = q.id)",
            &[&project, &now],
        )
        .await?;
    client
        .execute(
            "UPDATE owner_ask a SET state = 'done', why = 'вопрос закрыт в наборе либо снят с владельца',
                    decided_by = 'набор', decided_at = $2
              WHERE a.project_id = $1 AND a.question_id <> '' AND a.state = 'open'
                AND NOT EXISTS (SELECT 1 FROM project_questions q
                                 WHERE q.project_id = a.project_id AND q.id = a.question_id
                                   AND q.state = 'open' AND q.answer_state = 'owner')",
            &[&project, &now],
        )
        .await?;
    Ok(asked)
}

/// Решение по заявке. Довод обязателен: очередь без доводов через неделю
/// неотличима от списка «почему-то отклонено».
pub(crate) async fn decide_ask(
    pool: &Pool, id: i64, state: &str, why: &str, by: &str,
) -> Result<Value, crate::db::Fail> {
    const STATES: [&str; 6] = ["taken", "owner", "declined", "done", "approved", "rejected"];
    if !STATES.contains(&state) {
        return Ok(json!({ "status": "state_unknown", "state": state,
            "why": format!("решение бывает такое: {}", STATES.join(" · ")) }));
    }
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_why", "why": "решение без довода не читается и не спорится" }));
    }
    let client = crate::db::conn(pool).await?;
    let n = client
        .execute(
            "UPDATE owner_ask SET state = $2, why = $3, decided_by = $4, decided_at = $5 WHERE id = $1",
            &[&id, &state, &why, &by, &now_ms()],
        )
        .await?;
    Ok(json!({ "status": if n > 0 { "decided" } else { "not_found" }, "id": id, "state": state }))
}

/// Решения владельца, которых прогон ещё не видел. Помечаются тем же вызовом:
/// иначе прогон брался бы за одно и то же подтверждение каждый круг.
pub(crate) async fn ask_inbox(pool: &Pool, project: &str, run: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "UPDATE owner_ask SET delivered_at = $3
              WHERE project_id = $1 AND run_id = $2 AND state <> 'open' AND delivered_at IS NULL
              RETURNING id, kind, title, state, why",
            &[&project, &run, &now_ms()],
        )
        .await?;
    Ok(json!({ "count": rows.len(), "decided": rows.iter().map(|r| json!({
        "id": r.get::<_, i64>(0), "kind": r.get::<_, String>(1), "title": r.get::<_, String>(2),
        "state": r.get::<_, String>(3), "why": r.get::<_, String>(4) })).collect::<Vec<_>>() }))
}

/// Завести прогон задачи: с него начинается всё, что о нём потом рассказывают.
///
/// ДВУХ ЖИВЫХ ПРОГОНОВ ОДНОЙ ЗАДАЧИ НЕ БЫВАЕТ — так сказано указателем самой
/// таблицы. Второй зов отдаёт тот, что уже идёт, а не заводит соперника.
pub(crate) async fn start_run(
    pool: &Pool, project: &str, task: &str, agent: &str, note: &str,
) -> Result<Value, crate::db::Fail> {
    if task.trim().is_empty() {
        return Ok(json!({ "status": "no_task", "why": "прогон заводится под задачу: без неё он ни о чём" }));
    }
    let client = crate::db::conn(pool).await?;
    let live = client
        .query_opt(
            "SELECT id, state, attempt FROM project_task_runs
              WHERE project_id = $1 AND task_id = $2 AND state <> ALL (ARRAY['done', 'failed', 'cancelled'])",
            &[&project, &task],
        )
        .await?;
    if let Some(r) = live {
        return Ok(json!({ "status": "already_running", "runId": r.get::<_, String>(0),
                          "state": r.get::<_, String>(1), "attempt": r.get::<_, i32>(2), "task": task }));
    }
    let attempt: i32 = client
        .query_one(
            "SELECT coalesce(max(attempt), 0) + 1 FROM project_task_runs WHERE project_id = $1 AND task_id = $2",
            &[&project, &task],
        )
        .await?
        .get(0);
    let row = client
        .query_one(
            "INSERT INTO project_task_runs (id, project_id, task_id, agent_id, state, attempt, note, created_at, updated_at)
             VALUES (gen_random_uuid()::text, $1, $2, $3, 'running', $4, $5, $6, $6) RETURNING id",
            &[&project, &task, &agent, &attempt, &note, &now_ms()],
        )
        .await?;
    let id: String = row.get(0);
    client
        .execute(
            "INSERT INTO project_task_run_events (task_run_id, from_state, to_state, reason, actor, at)
             VALUES ($1, NULL, 'running', $2, $3, $4)",
            &[&id, &note, &agent, &now_ms()],
        )
        .await?;
    Ok(json!({ "status": "running", "runId": id, "task": task, "attempt": attempt }))
}

/// Состояние прогона словом: идёт · ждёт · сделан · сорвался · снят.
///
/// Словарь взят у таблицы, а не придуман заново: её указатель считает живым
/// всё, что не `done`, `failed` и `cancelled`, и своё слово вроде `finished`
/// оставило бы задачу навсегда занятой.
pub(crate) async fn set_run_state(
    pool: &Pool, project: &str, run: &str, state: &str, note: &str, session: &str, by: &str,
) -> Result<Value, crate::db::Fail> {
    const STATES: [&str; 5] = ["running", "waiting", "done", "failed", "cancelled"];
    if !STATES.contains(&state) {
        return Ok(json!({ "status": "state_unknown", "state": state,
            "why": format!("состояние прогона бывает такое: {}", STATES.join(" · ")) }));
    }
    if state != "running" && note.trim().is_empty() {
        return Ok(json!({ "status": "no_note",
            "why": "остановка без причины не отличима от обрыва: скажите, на чём встали" }));
    }
    let client = crate::db::conn(pool).await?;
    let Some(was) = client
        .query_opt("SELECT state FROM project_task_runs WHERE project_id = $1 AND id = $2", &[&project, &run])
        .await?
    else {
        return Ok(json!({ "status": "not_found", "runId": run }));
    };
    let from: String = was.get(0);
    client
        .execute(
            "UPDATE project_task_runs
                SET state = $3, note = $4, updated_at = $5,
                    session_id = CASE WHEN $6 = '' THEN session_id ELSE $6 END,
                    finished_at = CASE WHEN $3 = ANY (ARRAY['done', 'failed', 'cancelled']) THEN $5 ELSE finished_at END
              WHERE project_id = $1 AND id = $2",
            &[&project, &run, &state, &note, &now_ms(), &session],
        )
        .await?;
    client
        .execute(
            "INSERT INTO project_task_run_events (task_run_id, from_state, to_state, reason, actor, at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[&run, &from, &state, &note, &by, &now_ms()],
        )
        .await?;
    Ok(json!({ "status": "written", "runId": run, "was": from, "state": state }))
}

/// Шаг прогона: что он сделал или на чём встал.
pub(crate) async fn add_run_event(
    pool: &Pool, project: &str, run: &str, kind: &str, text: &str,
) -> Result<Value, crate::db::Fail> {
    if run.trim().is_empty() || text.trim().is_empty() {
        return Ok(json!({ "status": "incomplete", "why": "событие называет прогон и то, что случилось" }));
    }
    let client = crate::db::conn(pool).await?;
    client
        .execute(
            "INSERT INTO task_run_event (project_id, run_id, at, kind, text) VALUES ($1, $2, $3, $4, $5)",
            &[&project, &run, &now_ms(), &kind, &text],
        )
        .await?;
    client
        .execute("UPDATE project_task_runs SET updated_at = $3 WHERE project_id = $1 AND id = $2",
                 &[&project, &run, &now_ms()])
        .await?;
    Ok(json!({ "status": "written", "runId": run, "kind": kind }))
}

/// Слово человека прогону — и слово прогона в ответ.
pub(crate) async fn say_to_run(
    pool: &Pool, project: &str, run: &str, side: &str, text: &str,
) -> Result<Value, crate::db::Fail> {
    if !["owner", "agent"].contains(&side) {
        return Ok(json!({ "status": "side_unknown", "side": side, "why": "говорит либо owner, либо agent" }));
    }
    if run.trim().is_empty() || text.trim().is_empty() {
        return Ok(json!({ "status": "incomplete", "why": "сказанное принадлежит прогону и не бывает пустым" }));
    }
    let client = crate::db::conn(pool).await?;
    client
        .execute(
            "INSERT INTO task_run_message (project_id, run_id, at, side, text, delivered_at)
             VALUES ($1, $2, $3::bigint, $4, $5, CASE WHEN $4 = 'agent' THEN $3::bigint END)",
            &[&project, &run, &now_ms(), &side, &text],
        )
        .await?;
    Ok(json!({ "status": "said", "runId": run, "side": side }))
}

/// Что человек сказал прогону и он ещё не прочёл. Прочитанное помечается тем же
/// вызовом: иначе агент отвечал бы на одно и то же каждый круг.
pub(crate) async fn run_inbox(pool: &Pool, project: &str, run: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "UPDATE task_run_message SET delivered_at = $3
              WHERE project_id = $1 AND run_id = $2 AND side = 'owner' AND delivered_at IS NULL
              RETURNING at, text",
            &[&project, &run, &now_ms()],
        )
        .await?;
    Ok(json!({ "count": rows.len(), "said": rows.iter().map(|r| json!({
        "at": r.get::<_, i64>(0), "text": r.get::<_, String>(1) })).collect::<Vec<_>>() }))
}

/// Завести беседу: место, где думают вслух над набором.
pub(crate) async fn start_chat(pool: &Pool, project: &str, title: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let row = client
        .query_one(
            "INSERT INTO chat_thread (id, project_id, title, created_at, updated_at)
             VALUES (gen_random_uuid()::text, $1, $2, $3, $3) RETURNING id",
            &[&project, &title, &now_ms()],
        )
        .await?;
    Ok(json!({ "status": "open", "thread": row.get::<_, String>(0), "title": title }))
}

/// Сказанное в беседе. Сторона названа: `owner` — человек, `agent` — тот, кто
/// отвечает.
pub(crate) async fn say_in_chat(
    pool: &Pool, project: &str, thread: &str, side: &str, text: &str,
) -> Result<Value, crate::db::Fail> {
    if !["owner", "agent"].contains(&side) {
        return Ok(json!({ "status": "side_unknown", "side": side, "why": "говорит либо owner, либо agent" }));
    }
    if thread.trim().is_empty() || text.trim().is_empty() {
        return Ok(json!({ "status": "incomplete", "why": "сказанное принадлежит беседе и не бывает пустым" }));
    }
    let client = crate::db::conn(pool).await?;
    let n = client
        .execute(
            "INSERT INTO chat_message (project_id, thread_id, at, side, text, delivered_at)
             SELECT $1, $2, $3::bigint, $4, $5, CASE WHEN $4 = 'agent' THEN $3::bigint END
              WHERE EXISTS (SELECT 1 FROM chat_thread t WHERE t.project_id = $1 AND t.id = $2)",
            &[&project, &thread, &now_ms(), &side, &text],
        )
        .await?;
    if n == 0 {
        return Ok(json!({ "status": "no_thread", "thread": thread, "why": "беседы с таким именем у набора нет" }));
    }
    client
        .execute("UPDATE chat_thread SET updated_at = $3 WHERE project_id = $1 AND id = $2",
                 &[&project, &thread, &now_ms()])
        .await?;
    Ok(json!({ "status": "said", "thread": thread, "side": side }))
}

/// Что человек сказал беседе и она ещё не прочла; прочитанное помечается.
/// Здесь же беседа называет сессию, которой отвечает: без неё каждый ответ
/// начинался бы с чистого листа.
pub(crate) async fn chat_inbox(
    pool: &Pool, project: &str, thread: &str, session: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if !session.trim().is_empty() {
        client
            .execute("UPDATE chat_thread SET session_id = $3, updated_at = $4 WHERE project_id = $1 AND id = $2",
                     &[&project, &thread, &session, &now_ms()])
            .await?;
    }
    let rows = client
        .query(
            "UPDATE chat_message SET delivered_at = $3
              WHERE project_id = $1 AND thread_id = $2 AND side = 'owner' AND delivered_at IS NULL
              RETURNING at, text",
            &[&project, &thread, &now_ms()],
        )
        .await?;
    let session_id: String = client
        .query_one("SELECT coalesce(session_id, '') FROM chat_thread WHERE project_id = $1 AND id = $2",
                   &[&project, &thread])
        .await
        .map(|r| r.get(0))
        .unwrap_or_default();
    Ok(json!({ "count": rows.len(), "sessionId": session_id, "said": rows.iter().map(|r| json!({
        "at": r.get::<_, i64>(0), "text": r.get::<_, String>(1) })).collect::<Vec<_>>() }))
}

/// Беседы набора, а с именем беседы — её строки.
pub(crate) async fn read_chat(
    pool: &Pool, project: &str, thread: &str, limit: i64,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if thread.trim().is_empty() {
        let rows = client
            .query(
                "SELECT t.id, t.title, t.state, t.updated_at,
                        (SELECT count(*) FROM chat_message m WHERE m.project_id = t.project_id AND m.thread_id = t.id),
                        (SELECT count(*) FROM chat_message m
                          WHERE m.project_id = t.project_id AND m.thread_id = t.id
                            AND m.side = 'owner' AND m.delivered_at IS NULL)
                   FROM chat_thread t WHERE t.project_id = $1
                  ORDER BY t.updated_at DESC LIMIT $2",
                &[&project, &limit],
            )
            .await?;
        return Ok(json!({ "count": rows.len(), "threads": rows.iter().map(|r| json!({
            "thread": r.get::<_, String>(0), "title": r.get::<_, String>(1), "state": r.get::<_, String>(2),
            "at": r.get::<_, i64>(3), "said": r.get::<_, i64>(4), "waiting": r.get::<_, i64>(5) })).collect::<Vec<_>>() }));
    }
    let rows = client
        .query(
            "SELECT at, side, text, delivered_at IS NOT NULL FROM chat_message
              WHERE project_id = $1 AND thread_id = $2 ORDER BY at DESC LIMIT $3",
            &[&project, &thread, &limit],
        )
        .await?;
    Ok(json!({ "thread": thread, "count": rows.len(), "said": rows.iter().rev().map(|r| json!({
        "at": r.get::<_, i64>(0), "side": r.get::<_, String>(1), "text": r.get::<_, String>(2),
        "read": r.get::<_, bool>(3) })).collect::<Vec<_>>() }))
}

/// Прогоны с последними шагами: пульту нужен не список состояний, а рассказ.
pub(crate) async fn list_runs(pool: &Pool, project: &str, limit: i64) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let runs = client
        .query(
            "SELECT r.id, r.task_id, coalesce(t.title, ''), r.state, r.attempt, r.note,
                    r.created_at, coalesce(r.updated_at, r.created_at), coalesce(r.session_id, '')
               FROM project_task_runs r
               LEFT JOIN project_plan_tasks t ON t.project_id = r.project_id AND t.id = r.task_id
              WHERE r.project_id = $1
              ORDER BY coalesce(r.updated_at, r.created_at) DESC LIMIT $2",
            &[&project, &limit],
        )
        .await?;
    let mut out = Vec::new();
    for r in &runs {
        let id: String = r.get(0);
        let events = client
            .query(
                "SELECT at, kind, text FROM task_run_event
                  WHERE project_id = $1 AND run_id = $2 ORDER BY at DESC LIMIT 12",
                &[&project, &id],
            )
            .await?;
        let said = client
            .query(
                "SELECT at, side, text FROM task_run_message
                  WHERE project_id = $1 AND run_id = $2 ORDER BY at DESC LIMIT 12",
                &[&project, &id],
            )
            .await?;
        out.push(json!({
            "runId": id, "task": r.get::<_, String>(1), "title": r.get::<_, String>(2),
            "state": r.get::<_, String>(3), "attempt": r.get::<_, i32>(4), "note": r.get::<_, String>(5),
            "startedAt": r.get::<_, i64>(6), "at": r.get::<_, i64>(7), "sessionId": r.get::<_, String>(8),
            "events": events.iter().map(|e| json!({
                "at": e.get::<_, i64>(0), "kind": e.get::<_, String>(1), "text": e.get::<_, String>(2) })).collect::<Vec<_>>(),
            "said": said.iter().map(|m| json!({
                "at": m.get::<_, i64>(0), "side": m.get::<_, String>(1), "text": m.get::<_, String>(2) })).collect::<Vec<_>>(),
        }));
    }
    Ok(json!({ "count": out.len(), "runs": out }))
}

/// Объявить, к какой области проекта принадлежит вид.
///
/// Список родов — семнадцать строк по счёту записей, и по нему не видно, что
/// требование, потребность и история про ОДНО, а задача и этап про другое.
///
/// Вывести это не из чего: фаза не годится — почти каждый род судится в
/// нескольких (требование в G1, G2, G3, G5 и корпусе), а `in` говорит лишь,
/// в каком документе род живёт. Область — суждение о смысле, и потому она
/// ОБЪЯВЛЯЕТСЯ с доводом, как проекция и переоткрытие.
pub(crate) async fn set_kind_domain(
    pool: &Pool, kind: &str, domain: &str, why: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if kind.trim().is_empty() || domain.trim().is_empty() || why.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "нужны вид, область и довод" }));
    }
    if client
        .query_opt("SELECT 1 FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .is_none()
    {
        return Ok(json!({ "status": "no_kind", "kind": kind, "why": "вид не объявлен" }));
    }
    client
        .execute(
            "UPDATE kind_layout SET spec = spec || jsonb_build_object('domain', $2::text,
                                                                      'domain-why', $3::text)
              WHERE name = $1",
            &[&kind, &domain, &why],
        )
        .await?;
    let beside: Vec<String> = client
        .query(
            "SELECT name FROM kind_layout WHERE spec->>'domain' = $1 ORDER BY name",
            &[&domain],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    Ok(json!({ "status": "declared", "kind": kind, "domain": domain, "with": beside,
               "means": "объявление ОБЩЕЕ: области одни для всех проектов" }))
}

pub(crate) async fn holders(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT requirement_id, path FROM project_requirement_holder
              WHERE project_id = $1 ORDER BY requirement_id, path",
            &[&project],
        )
        .await?;
    Ok(json!({ "holders": rows.iter().map(|r| json!({
        "requirement": r.get::<_, String>(0), "path": r.get::<_, String>(1) })).collect::<Vec<_>>() }))
}

/// Поля выведенной копии, как их принимает дверь `derived-copy-set`.
pub(crate) struct DerivedCopy<'a> {
    pub name: &'a str,
    pub source: &'a str,
    pub copy: &'a str,
    pub compare: &'a str,
    pub pattern: &'a str,
    pub source_pattern: &'a str,
    pub why: &'a str,
    pub decided_by: &'a str,
}

/// Объявить производную копию: вот источник, вот копия, вот чем сверять.
///
/// Причина обязательна: копия без довода неотличима от случайного совпадения, и
/// снять её потом будет не за что.
pub(crate) async fn set_derived_copy(pool: &Pool, project: &str, fields: DerivedCopy<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let DerivedCopy { name, source, copy, compare, pattern, source_pattern, why, decided_by } = fields;
    let split = |v: &str| -> (String, String) {
        match v.split_once(':') {
            Some((k, n)) => (k.trim().to_owned(), n.trim().to_owned()),
            None => (v.trim().to_owned(), String::new()),
        }
    };
    let (sk, sn) = split(source);
    let (ck, cn) = split(copy);
    let client = crate::db::conn(pool).await?;
    if drop_it {
        let gone = client
            .execute("DELETE FROM derived_copy WHERE project_id = $1 AND name = $2 AND copy_kind = $3 AND copy_name = $4",
                     &[&project, &name, &ck, &cn]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
    }
    if name.trim().is_empty() || sk.is_empty() || ck.is_empty() {
        return Ok(json!({ "status": "incomplete",
            "why": "называются трое: величина, источник и копия — каждый видом и именем через двоеточие" }));
    }
    if !matches!(compare, "count" | "body") {
        return Ok(json!({ "status": "unknown_compare",
            "why": "чем сверять: `count` — число, названное образцом; `body` — тело названного куска" }));
    }
    if pattern.trim().is_empty() {
        return Ok(json!({ "status": "no_pattern",
            "why": "не сказано, ЧТО именно сверять: без образца сверка не отличит нужное число от соседнего" }));
    }
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_why",
            "why": "не сказано, почему это копия. Без довода она неотличима от случайного совпадения, \
                    и снять её потом будет не за что." }));
    }
    client.execute(
        "INSERT INTO derived_copy (project_id, name, source_kind, source_name, copy_kind, copy_name,
                                   compare, pattern, source_pattern, why, decided_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         ON CONFLICT (project_id, name, copy_kind, copy_name) DO UPDATE SET
           source_kind = EXCLUDED.source_kind, source_name = EXCLUDED.source_name,
           compare = EXCLUDED.compare, pattern = EXCLUDED.pattern,
           source_pattern = EXCLUDED.source_pattern, why = EXCLUDED.why,
           decided_by = EXCLUDED.decided_by",
        &[&project, &name, &sk, &sn, &ck, &cn, &compare, &pattern,
          &(if source_pattern.trim().is_empty() { pattern } else { source_pattern }),
          &why, &decided_by]).await?;
    Ok(json!({ "status": "declared", "name": name,
               "pair": format!("{sk}:{sn} → {ck}:{cn}"), "compare": compare,
               "means": "равенство держит теперь машина, а не рука" }))
}

pub(crate) async fn declared_unwritten(
    pool: &Pool,
    kinds: &crate::kinds::Kinds,
    project: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let with_origin: std::collections::HashSet<String> = client
        .query(
            "SELECT table_name FROM information_schema.columns
              WHERE column_name = 'origin' AND table_name LIKE 'project_%'",
            &[],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    let mut out = Vec::new();
    let mut total = 0usize;
    let mut inner = Vec::new();
    for (name, kind) in &kinds.0 {
        let Some((table, id_col, _)) = crate::kinds::table_of(name) else { continue };
        if !with_origin.contains(table) {
            continue;
        }
        if kind.is_inner() {
            inner.push(name.clone());
            continue;
        }
        // ПРИЗРАК ОТДАЁТСЯ ЦЕЛИКОМ. Прежде перечень называл имена, и прочесть
        // объявленное было нечем: `get` по такому имени отказывает, а заголовок
        // доезжал только через запрос гейта и обрезанным до шестидесяти знаков.
        // Восстанавливать пришлось замером предмета, потому что самого текста не
        // отдавала ни одна дверь.
        //
        // Объявление — ВСЯ правда о призраке: документа за ним нет.
        let rows = client
            .query(
                &format!("SELECT {id_col}, to_jsonb(t) FROM {table} t
                           WHERE project_id = $1 AND origin = 'declared' ORDER BY 1"),
                &[&project],
            )
            .await?;
        if rows.is_empty() {
            continue;
        }
        let ids: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
        let declared: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut v: Value = r.get(1);
                // Пустые колонки не показываются: у призрака их большинство, и
                // они топят то немногое, что о нём сказано.
                if let Some(o) = v.as_object_mut() {
                    o.retain(|k, val| {
                        k != "project_id" && k != "origin"
                            && !matches!(val, Value::String(s) if s.trim().is_empty())
                            && *val != Value::Null
                    });
                }
                v
            })
            .collect();
        total += ids.len();
        out.push(json!({ "kind": name, "count": ids.len(), "ids": ids, "declared": declared }));
    }
    Ok(json!({
        "declared": total,
        "kinds": out,
        "innerSkipped": inner,
        "means": "объявлено дверью, документа нет: `get` по такому имени откажет, \
                  а перечень вида его покажет. Написать — `document-add`, снять — `<вид>-add drop=true`.",
    }))
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
pub(crate) async fn readiness_gaps(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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


/// Записать исход пересборки: удалась или упала и чем.
///
/// Пишется ВСЕГДА, а не только при удаче: молчание об упавшей пересборке
/// неотличимо от её отсутствия, и гейт продолжает отдавать прежние числа.
pub async fn note_reproject(pool: &Pool, project: &str, ok: bool, why: &str) {
    let client = match crate::db::conn(pool).await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("исход пересборки набора {project} не записан: {}", e.says());
            return;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // Отказ записать отказ — не повод уронить работу: пересборка уже случилась,
    // и её исход важнее, чем запись о нём.
    let _ = client
        .execute(
            "INSERT INTO reproject_state (project_id, at, ok, why) VALUES ($1,$2,$3,$4)
             ON CONFLICT (project_id) DO UPDATE SET at = EXCLUDED.at, ok = EXCLUDED.ok, why = EXCLUDED.why",
            &[&project, &now, &ok, &why],
        )
        .await;
}

/// Что известно о последней пересборке: `None` — не пересобирали ни разу.
pub(crate) async fn last_reproject(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
) -> Option<(bool, String, i64)> {
    let row = client
        .query_opt("SELECT ok, why, at FROM reproject_state WHERE project_id = $1", &[&project])
        .await
        .ok()??;
    Some((row.get(0), row.get(1), row.get(2)))
}


/// Чем снимать факты: перечень объявленных датчиков для клиента.
pub(crate) async fn sensor_specs(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn declare_project(
    pool: &Pool, id: &str, name: &str, repo: &str, actor: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "проект без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn whose_repo(pool: &Pool, path: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn donors_and_guards(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn declare_donor(
    pool: &Pool, project: &str, path: &str, what: &str, frozen_by: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if path.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "донор без пути не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля сторожа, как их принимает дверь `guard-add`.
pub(crate) struct Guard<'a> {
    pub name: &'a str,
    pub enforces: &'a str,
    pub scope: &'a str,
    pub refuses: &'a str,
    pub acts_on: &'a str,
    pub path_re: &'a str,
    pub content_re: &'a str,
    pub command_re: &'a str,
}

/// Сторож: чем правило принуждается ДО действия, а не меряется после.
pub(crate) async fn declare_guard(pool: &Pool, project: &str, fields: Guard<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Guard { name, enforces, scope, refuses, acts_on, path_re, content_re, command_re } = fields;
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "сторож без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_crate(
    pool: &Pool, project: &str, name: &str, does: &str, does_not: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if name.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "крейт без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_protocol_op(
    pool: &Pool, project: &str, op: &str, group: &str, events: &str, requirement: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_article_gate(
    pool: &Pool, project: &str, article: i32, gate: &str, state: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if article <= 0 || gate.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "связь без статьи или без гейта не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_requirement_source(
    pool: &Pool, project: &str, id: &str, kind: &str, target: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() || target.trim().is_empty() {
        return Ok(json!({ "status": "empty", "why": "опора без требования или без цели не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn declare_requirement_scope(
    pool: &Pool, project: &str, id: &str, out_of_version: &str, crosscutting: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if id.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "область без требования не объявляется" }));
    }
    if out_of_version.trim().is_empty() && crosscutting.trim().is_empty() {
        return Ok(json!({ "status": "no_reason",
                          "why": "и «вне выпуска», и «сквозное» объявляются причиной, а не флагом" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля требования, как их принимает дверь `requirement-add`.
pub(crate) struct Requirement<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub area: &'a str,
    pub title: &'a str,
    pub text: &'a str,
    pub measured_by: &'a str,
    pub priority: &'a str,
}

pub(crate) async fn declare_requirement(pool: &Pool, project: &str, fields: Requirement<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Requirement { id, kind, area, title, text, measured_by, priority } = fields;
    // Снятие идёт той же дверью. Без него объявленное требование убирается
    // только запросом мимо сервера — а это уже вторая дверь, о которой сервер
    // не знает, и разойдутся они молча.
    if drop_it {
        if id.trim().is_empty() {
            return Ok(json!({ "status": "nameless", "why": "снимать требование без имени нечего" }));
        }
        let client = crate::db::conn(pool).await?;
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
    let client = crate::db::conn(pool).await?;
    // Способ доказательства, объявленный поверх абзаца «Проверяется», жил до
    // первой пересборки: дверь отвечала `hasCheck: true`, а следующая запись
    // любого документа молча возвращала поле к тексту документа.
    if !measured_by.trim().is_empty() {
        let written_in: Option<String> = client
            .query_opt(
                "SELECT d.entity_kind || ' ' || d.entity_name FROM project_documents d
                  WHERE d.project_id = $1 AND d.content LIKE '%**' || $2 || ' ·%'
                    AND EXISTS (SELECT 1 FROM scheme($1) m WHERE m.role = 'marker.verified-by'
                                 AND d.content LIKE '%' || m.value || '%')
                  ORDER BY 1 LIMIT 1",
                &[&project, &id],
            )
            .await?
            .map(|r| r.get(0));
        if let Some(doc) = written_in {
            return Ok(json!({ "status": "written_in_document", "id": id,
                "why": format!("требование {id} записано блоком в «{doc}», и способ доказательства — его абзац «Проверяется»: пересборка берёт поле оттуда. Правьте документ") }));
        }
    }
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
pub(crate) async fn declare_term(
    pool: &Pool, project: &str, term: &str, meaning: &str, area: &str, drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if term.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "термин без имени не объявляется" }));
    }
    let client = crate::db::conn(pool).await?;
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

pub(crate) async fn retire_requirement(
    pool: &Pool,
    project: &str,
    id: &str,
    why: &str,
    retired_by: &str,
    actor: &str,
    drop: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn declare_sensor(
    pool: &Pool,
    project: &str,
    fact: &str,
    about: &str,
    stale_after_ms: Option<i64>,
    actor: &str,
    drop: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    if drop {
        let n = client
            .execute("DELETE FROM sensor WHERE project_id = $1 AND fact = $2", &[&project, &fact])
            .await?;
        // СНИМАЕТСЯ И ЗАПИСЬ О ПОДАЧЕ. Датчик, подавший факт однажды, оставался
        // в `sensors.undeclared` навсегда: спецификация снята, объявление снято,
        // факты стёрты — а он висит. Убрать его можно было только запросом в
        // базу мимо сервера, то есть никак.
        //
        // Цена: «подаёт, но никем не объявлен» переставало быть находкой — в
        // перечне копился мусор от проб. А проба ровно то, чем выясняют, как
        // датчик именует факт.
        let pushed = client
            .execute("DELETE FROM fact_push WHERE project_id = $1 AND fact = $2", &[&project, &fact])
            .await?;
        let facts = client
            .execute("DELETE FROM code_fact WHERE project_id = $1 AND kind = $2", &[&project, &fact])
            .await?;
        return Ok(json!({ "status": if n + pushed + facts > 0 { "dropped" } else { "not_found" },
                          "fact": fact, "declaration": n, "pushRecord": pushed, "facts": facts,
                          "means": "снято ВСЁ о роде: объявление, запись о подаче и поданные факты" }));
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
/// Натуга сервера: отказы «занято» и вложенные взятия соединения. След живёт
/// месяц; старое убирает сборщик.
///
/// Счёт живёт в памяти минуту, а строки — в базе: перегрузку читают ПОСЛЕ неё,
/// и журнал процесса для этого не годится — он переживает не всякий перезапуск
/// и его нельзя спросить дверью.
pub(crate) async fn strain(pool: &Pool) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT at, busy, nested FROM server_strain ORDER BY at DESC LIMIT 50",
            &[],
        )
        .await?;
    let (busy, nested) = (
        crate::db::BUSY.load(std::sync::atomic::Ordering::Relaxed),
        crate::db::NESTED.load(std::sync::atomic::Ordering::Relaxed),
    );
    Ok(json!({
        "сейчас": { "занято": busy, "вложенных": nested,
                    "означает": "накоплено с прошлой записи; сборщик кладёт это строкой" },
        "было": rows.iter().map(|r| json!({
            "at": r.get::<_, i64>(0), "busy": r.get::<_, i64>(1), "nested": r.get::<_, i64>(2) }))
            .collect::<Vec<_>>(),
        "означает": "`busy` — отказов из-за перегрузки; `nested` — взятий второго соединения \
                     при живом первом: это запирает пул на себе же и должно быть нулём"
    }))
}

pub(crate) async fn sensors(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT s.fact, s.about, s.stale_after_ms, f.at, f.rows, f.actor,
                    f.at IS NOT NULL AND NOT fact_fresh(s.project_id, s.fact)
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
    // Протухание — ТО ЖЕ, что у гейта: `fact_fresh`. Своими часами дверь
    // отвечала «протухших 0» в ту же секунду, когда гейт печатал «ПРОТУХ», и
    // спросить причину красноты было не у кого.
    Ok(json!({
        "sensors": rows.iter().map(|r| {
            let at: Option<i64> = r.get(3);
            json!({
                "fact": r.get::<_, String>(0),
                "about": r.get::<_, String>(1),
                "lastAt": at,
                "staleAfterMs": r.get::<_, Option<i64>>(2),
                "rows": r.get::<_, Option<i32>>(4),
                "by": r.get::<_, Option<String>>(5),
                "silent": at.is_none(),
                "stale": r.get::<_, bool>(6),
            })
        }).collect::<Vec<_>>(),
        // Подающий, которого никто не объявлял, — не ошибка и не порядок: это
        // факт, о котором надо знать, чтобы решить.
        "undeclared": undeclared.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>(),
    }))
}

/// Поля ступени лестницы, как их принимает дверь `step-add`.
pub(crate) struct Step<'a> {
    pub set_name: &'a str,
    pub process: &'a str,
    pub ord: i32,
    pub question: &'a str,
    pub owner_kind: &'a str,
    pub owner: &'a str,
    pub touches: &'a str,
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
pub(crate) async fn add_step(pool: &Pool, project: &str, fields: Step<'_>) -> Result<Value, crate::db::Fail> {
    let Step { set_name, process, ord, question, owner_kind, owner, touches } = fields;
    if !matches!(owner_kind, "skill" | "agent" | "none") {
        return Ok(json!({ "status": "bad_owner_kind", "why": "закрывает ступень скилл, субагент либо человек" }));
    }
    if !matches!(touches, "corpus" | "repository") {
        return Ok(json!({ "status": "bad_touches", "why": "ступень читает набор либо пишет в репозиторий" }));
    }
    if question.trim().is_empty() {
        return Ok(json!({ "status": "nameless", "why": "ступень без условия не заводится" }));
    }
    let mut client = crate::db::conn(pool).await?;
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
pub(crate) async fn set_version_state(
    pool: &Pool,
    project: &str,
    version: &str,
    state: &str,
    actor: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    if state != "open" && state != "closed" {
        return Ok(json!({ "status": "bad_state", "why": "выпуск бывает открыт либо закрыт" }));
    }
    let client = crate::db::conn(pool).await?;
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

/// Поля фазы, как их принимает дверь `phase-set`.
pub(crate) struct Phase<'a> {
    pub phase: &'a str,
    pub ord: Option<i32>,
    pub title: Option<&'a str>,
    pub gate: Option<&'a str>,
    pub plan_level: Option<&'a str>,
    pub task_kind: Option<&'a str>,
}

/// Объявить фазу: место в цепочке, заголовок, гейт и вид её задач.
///
/// ВИД ЗАДАЧ — ЗАПИСЬ, А НЕ ПРОЗА. «Красная задача принадлежит Ф3, задача кода —
/// Ф4» жило абзацем в плане проекта, и оттого барьер фаз не мог его прочесть:
/// `next-task` предлагал задачу Ф4 при красном G3, а ступени звали владельца
/// отправлять то, что отправлять нельзя. Объявленное здесь читают все трое
/// через `task_phase`.
///
/// Проект, не назвавший отображение, получает пустоту — «не объявлено», — и
/// читатель обязан звать её словом, а не разрешением.
pub(crate) async fn set_phase(pool: &Pool, fields: Phase<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Phase { phase, ord, title, gate, plan_level, task_kind } = fields;
    let client = crate::db::conn(pool).await?;
    if drop_it {
        let gone = client.execute("DELETE FROM phase WHERE id = $1", &[&phase]).await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "phase": phase }));
    }
    // Заводится фаза ЦЕЛИКОМ: без места в цепочке «раньше» и «позже» не
    // существует, а на них держится вся открытость фаз.
    let known: i64 = client
        .query_one("SELECT count(*) FROM phase WHERE id = $1", &[&phase])
        .await?
        .get(0);
    if known == 0 && (ord.is_none() || title.is_none()) {
        return Ok(json!({ "status": "not_found", "phase": phase,
                          "why": "фазы с таким именем нет, а завести её без `ord` и `title` нельзя: \
                                  без места в цепочке не считается ни «раньше», ни «позже»" }));
    }
    // ГЕЙТ ПРОВЕРЯЕТСЯ ПО ИМЕНИ. Опечатка в нём — не описка, а вечный затвор:
    // гейта с таким именем нет, замеров у него не будет никогда, и всякая
    // последующая фаза окажется закрыта навсегда отказом, называющим гейт,
    // которого не найти.
    //
    // СПРАШИВАЕТСЯ `gate_item`, а не `gate_head`. Заголовок гейта пишет
    // единственный посев при старте, из проектной таблицы, — на новой установке
    // он пуст, и проверка по нему отказывала бы всякому непустому имени. Выход
    // был бы только один и нигде не названный: пересобрать документ, объявляющий
    // гейты, и перезапустить процесс. Пункты же приходят объявлением репозитория
    // при выкладке, и «у гейта есть хоть один объявленный пункт» — ровно то
    // условие, при котором замеры вообще могут появиться. Заголовок принимается
    // тоже: объявленный гейт без пунктов — намерение, и оно падает закрытым.
    if let Some(g) = gate.filter(|g| !g.is_empty()) {
        let heard: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM gate_item WHERE phase = $1)
                     OR EXISTS (SELECT 1 FROM gate_head WHERE phase = $1)",
                &[&g],
            )
            .await?
            .get(0);
        if !heard {
            return Ok(json!({ "status": "not_found", "phase": phase, "gate": g,
                              "why": format!("гейта «{g}» в наборе нет ни одним пунктом и ни одним \
                                              заголовком. Привязать фазу к несуществующему гейту значит \
                                              закрыть все последующие навсегда: замеров у него не будет \
                                              никогда. Пункты гейта объявляются в репозитории харнеса, \
                                              каталог `instrument/gate`") }));
        }
    }
    // ПУСТОЙ ГЕЙТ У ФАЗЫ, ЗА КОТОРОЙ ЕСТЬ ДРУГИЕ, — сказан вслух. Читатель
    // считает такую фазу непройденной и держит на ней всю оставшуюся цепочку;
    // молчание здесь означало бы, что забытая при заведении привязка
    // останавливает проект, и никто не сказал об этом ни слова.
    let tail = match (gate, ord) {
        (Some(""), _) | (None, _) if known == 0 => client
            .query_one("SELECT count(*) FROM phase WHERE ord > coalesce($1, 0)", &[&ord])
            .await?
            .get::<_, i64>(0),
        (Some(""), _) => client
            .query_one("SELECT count(*) FROM phase WHERE ord > (SELECT ord FROM phase WHERE id = $1)",
                       &[&phase])
            .await?
            .get::<_, i64>(0),
        _ => 0,
    };
    // ЗАНЯТОЕ МЕСТО И ЗАНЯТЫЙ ВИД называются словом, а не отказом Postgres.
    //
    // Уникальность держат индексы, но заводятся они с проглоченной ошибкой:
    // база, где пара уже стоит, осталась бы вовсе без индекса, и тогда эта
    // проверка — единственная. Она же превращает `duplicate key value violates
    // unique constraint` в имя фазы, которая место занимает.
    let taken = |what: &str, val: &str| {
        format!("{what} уже за фазой «{val}»: два одинаковых делают вопрос о порядке \
                 двусмысленным, и барьер между такими фазами исчезает молча. \
                 Сперва освободите место у неё")
    };
    if let Some(o) = ord {
        if let Some(row) = client
            .query_opt("SELECT id FROM phase WHERE ord = $1 AND id <> $2", &[&o, &phase])
            .await?
        {
            return Ok(json!({ "status": "taken", "phase": phase, "ord": o,
                              "why": taken("место в цепочке", &row.get::<_, String>(0)) }));
        }
    }
    if let Some(k) = task_kind.filter(|k| !k.is_empty()) {
        if let Some(row) = client
            .query_opt("SELECT id FROM phase WHERE task_kind = $1 AND id <> $2", &[&k, &phase])
            .await?
        {
            return Ok(json!({ "status": "taken", "phase": phase, "taskKind": k,
                              "why": taken(&format!("вид задач «{k}»"), &row.get::<_, String>(0)) }));
        }
    }
    client
        .execute(
            "INSERT INTO phase (id, ord, title, gate, plan_level, task_kind)
             VALUES ($1, coalesce($2, 0), coalesce($3, ''), coalesce($4, ''),
                     coalesce($5, ''), coalesce($6, ''))
             ON CONFLICT (id) DO UPDATE SET
               ord = coalesce($2, phase.ord), title = coalesce($3, phase.title),
               gate = coalesce($4, phase.gate), plan_level = coalesce($5, phase.plan_level),
               task_kind = coalesce($6, phase.task_kind)",
            &[&phase, &ord, &title, &gate, &plan_level, &task_kind],
        )
        .await?;
    let r = client
        .query_one(
            "SELECT ord, title, gate, plan_level, task_kind FROM phase WHERE id = $1",
            &[&phase],
        )
        .await?;
    Ok(json!({ "phase": phase, "ord": r.get::<_, i32>(0), "title": r.get::<_, String>(1),
               "gate": r.get::<_, String>(2), "planLevel": r.get::<_, String>(3),
               "taskKind": r.get::<_, String>(4),
               "why": if tail > 0 && r.get::<_, String>(2).is_empty() {
                   format!("гейт не объявлен, а за этой фазой стоят ещё {tail}: проходить нечего, \
                            и читатель держит на ней всю оставшуюся цепочку. Это не «можно всё»")
               } else { String::new() } }))
}

/// Объявить, когда ступень вообще в игре.
pub(crate) async fn set_step_when(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    when_query: &str,
    when_why: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn set_step_probe(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    probe: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn step_selftest(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
    under: &str,
) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let steps = client
        .query(
            // НАБОР В ОТБОРЕ, и это не украшение: ключ лестницы — тройка
            // `(set_name, process, ord)`, и всякий её читатель, кроме этого,
            // набор называет. Второй набор с одноимённым процессом отдал бы
            // сюда СВОИ ступени, прогнанные по чужому проекту, и номера в
            // отчёте задвоились бы.
            "SELECT ord, question, method_kind, method, probe, subject_query FROM harness_process_step
              WHERE set_name = $1 AND process = $2 ORDER BY ord",
            &[&set_name, &process],
        )
        .await?;

    let (mut alive, mut broken, mut undeclared) = (Vec::new(), Vec::new(), Vec::new());
    for r in &steps {
        let ord: i32 = r.get(0);
        let question: String = r.get(1);
        let method_kind: String = r.get(2);
        let method: String = r.get(3);
        let probe: String = r.get(4);
        let subject: String = r.get(5);
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
        force_gates(&tx, project, under).await?;
        let saw = match answer_of(&tx, &method, project, 0).await {
            Err(e) => Err(format!("запрос ступени не исполнился: {e}")),
            Ok(before) => match tx.execute(probe.as_str(), &[&project]).await {
                Err(e) => Err(format!("проба не исполнилась: {}", e.says())),
                // ЧИСЛО СТРОК ДО ПОДСАДКИ — В ОТЧЁТ. Дважды за сессию самотест
                // назвал сломанным пункт, который через минуту оказался цел, и
                // отличить «проба неверна» от «подсаживать было не во что»
                // отчёт не давал: причину искали руками и не нашли.
                Ok(0) => Err(format!(
                    "проба ничего не подсадила; запрос пункта до пробы вернул {} строк",
                    before.len())),
                Ok(_) => match subject_planted(&tx, &subject, project).await {
                    Err(why) => Err(why),
                    Ok(()) => match answer_of(&tx, &method, project, 0).await {
                        Ok(after) => Ok((before, after)),
                        Err(e) => Err(format!("запрос ступени не исполнился после подсадки: {e}")),
                    },
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
        "under": if under.is_empty() { "как есть" } else { under },
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
pub(crate) async fn set_step_question(
    pool: &Pool,
    set_name: &str,
    process: &str,
    ord: i32,
    question: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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

/// Виды единицы работы ступени. Перечень закрыт: слово вне него дверь не примет.
const UNITS: [&str; 8] = ["document", "link", "version", "question", "gate", "sensor", "milestone", "task"];

/// Поля способа ступени, как их принимает дверь `step-method-set`.
pub(crate) struct StepMethod<'a> {
    pub set_name: &'a str,
    pub process: &'a str,
    pub ord: i32,
    pub method_kind: &'a str,
    pub method: &'a str,
    /// Команда, которой видна единица работы этой ступени. `None` — не трогать
    /// уже объявленную.
    pub run: Option<&'a str>,
    /// Вид единицы работы ступени. `None` — не трогать объявленный.
    pub unit: Option<&'a str>,
    /// Запрос предмета ступени и слово о пустом предмете. `None` — не трогать.
    pub subject: Option<&'a str>,
    pub subject_why: Option<&'a str>,
    pub declared_by: &'a str,
    pub drop: bool,
}

pub(crate) async fn set_step_method(pool: &Pool, fields: StepMethod<'_>) -> Result<Value, crate::db::Fail> {
    let StepMethod { set_name, process, ord, method_kind, method, run, unit, subject,
                        subject_why, declared_by, drop } = fields;
    let client = crate::db::conn(pool).await?;
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
    // НЕ ПЕРЕДАННОЕ БЕРЁТСЯ У СТРОКИ. Дверь писала `method` всегда, и вызов,
    // объявлявший одну лишь команду, СТИРАЛ запрос ступени: ступень оставалась
    // объявленной, отвечала «мерить нечем» и молчала об этом. Проверено на себе
    // дважды за один проход — на шестой ступени и на одиннадцатой.
    // Слово вида сверяется с перечнем: опечатка `tasks` вместо `task` молча сняла
    // бы барьер лестницы — задачная ступень перестала бы находиться.
    let unit: Option<&str> = unit.map(str::trim).filter(|u| !u.is_empty());
    if let Some(u) = unit {
        if !UNITS.contains(&u) {
            return Ok(json!({ "status": "unknown_unit", "unit": u,
                              "why": format!("вида единицы «{u}» не бывает; бывают: {}", UNITS.join(" · ")) }));
        }
    }
    let method: Option<&str> = if method.trim().is_empty() { None } else { Some(method) };
    client
        .execute(
            "INSERT INTO harness_process_method (set_name, process, ord, method_kind, method, declared_by)
             VALUES ($1,$2,$3,$4,coalesce($5,''),$6)
             ON CONFLICT (set_name, process, ord) DO UPDATE SET method_kind = EXCLUDED.method_kind,
               method = coalesce($5, harness_process_method.method),
               declared_by = EXCLUDED.declared_by",
            &[&set_name, &process, &ord, &method_kind, &method, &declared_by],
        )
        .await?;
    // Метка имени приводится к объявленной: описания у наборов разъезжаются с
    // кодом, и поданная по старой памяти `{имя}` не подставилась бы молча.
    let run = run.map(|command| command.replace("{имя}", WORK_RUN_NAME));
    let run = run.as_deref();
    // Команда, не переданная, БЕРЁТСЯ У СТРОКИ: правка запроса не должна стирать
    // уже объявленную команду — так же, как правка запроса пункта гейта не
    // стирает его пробу.
    let n = client
        .execute(
            "UPDATE harness_process_step SET method_kind = $4,
                    method = coalesce($5, method),
                    work_run = coalesce($6, work_run),
                    subject_query = coalesce($7, subject_query),
                    subject_why = coalesce($8, subject_why),
                    unit = coalesce($9, unit)
              WHERE set_name = $1 AND process = $2 AND ord = $3",
            &[&set_name, &process, &ord, &method_kind, &method, &run, &subject, &subject_why, &unit],
        )
        .await?;
    Ok(json!({ "updated": n, "methodKind": method_kind, "declaredBy": declared_by,
               "survivesRebuild": true }))
}

/// Сдвинуть ВСЕ гейты проекта в одно состояние — внутри откатываемой пробы.
///
/// Проба, живая только в сегодняшнем состоянии, не доказывает ничего. Мои же
/// пробы ступеней 9 и 10 переоткрывали закрытую задачу ОТКРЫТОЙ фазы: пока
/// открытая фаза с задачами была, самотест говорил «живы», а стоило гейту
/// покраснеть — обе назвались сломанными. Живой считается та, что роняет своё
/// правило и при всех зелёных гейтах, и при всех красных; разошлись ответы —
/// проба зависит от состояния, которого сама не форсирует.
async fn force_gates(
    tx: &deadpool_postgres::Transaction<'_>,
    project: &str,
    under: &str,
) -> Result<(), crate::db::Fail> {
    // ТРОГАЕТСЯ РОВНО ТО, ЧТО МЕНЯЕТ ВЕРДИКТ, а не весь замер.
    //
    // Первая редакция красила все строки проекта — сто тридцать одну, и делала
    // это на КАЖДЫЙ пункт. Сто тридцать полных обходов с блокировками наперегонки
    // со сборщиком, который в это же время меряет соседний проект: прогон
    // отказывал «база не ответила», и отказ не называл причины. Зелёным гейт
    // делает отсутствие непройденных строк, красным — одна красная; больше
    // ничего трогать не нужно.
    // СТРОКИ БЕРУТСЯ В ТОМ ЖЕ ПОРЯДКЕ, ЧТО И КРУГОМ ЗАМЕРА, — `phase, id`.
    //
    // Это не украшение, а условие, без которого два писателя в `project_gates`
    // встают в тупик. Пока круг замера писал по строке за раз, инверсия порядка
    // разруливалась сама: замок отпускался сразу. Круг стал одной транзакцией и
    // держит свои замки до фиксации — и первый же писатель, берущий те же строки
    // в порядке планировщика, получает `deadlock detected`. Поймано ровно так:
    // отказ «база не ответила: db error» ничего не говорил, пока дверь не начала
    // называть причину.
    //
    // `FOR UPDATE` в подзапросе и задаёт порядок: без него `ORDER BY` в UPDATE
    // не выразить вовсе.
    // Отбор без `FOR UPDATE`: предзахват ниже уже держит каждую строку проекта, и
    // повторять порядок внутри правки незачем.
    let (sql, order) = match under {
        "green" => (
            "UPDATE project_gates
                SET state = 'passed',
                    result = jsonb_set(coalesce(result, '{}'::jsonb), '{computed}', '\"passed\"')
              WHERE project_id = $1 AND state <> 'passed'",
            true,
        ),
        // Красным гейт делает ОДНА красная строка: трогать весь замер незачем.
        "red" => (
            "UPDATE project_gates g
                SET state = 'failed',
                    result = jsonb_set(coalesce(result, '{}'::jsonb), '{computed}', '\"failed\"')
              WHERE g.project_id = $1
                AND g.id = (SELECT min(x.id) FROM project_gates x
                             WHERE x.project_id = $1 AND x.phase = g.phase)",
            true,
        ),
        _ => ("", false),
    };
    if order {
        // ЗАМКИ — НА ВСЕ СТРОКИ ПРОЕКТА И В ПОРЯДКЕ КЛЮЧА, правка — по-прежнему на
        // немногие. Сузив правку, я сузил и захват, и встретился со сборщиком,
        // идущим тем же ключом, в тупике: `deadlock detected` в пробе
        // `phase-loop`.
        //
        // Что это ЗАКРЫВАЕТ: наблюдавшуюся инверсию по `project_gates` этого
        // проекта. Чего оно НЕ ДАЁТ — полного порядка, и обещать его тут нельзя.
        // Проба пункта — данные: её пишет оператор дверью и исполняет эта же
        // транзакция следом, по каким угодно таблицам и в каком угодно порядке.
        // Сверх того `gate_item` общий на все наборы, а предзахват — по одному
        // проекту. Полным порядок делает только замок вокруг КАЖДОЙ транзакции,
        // пишущей гейт, а не выборка внутри одной.
        tx.execute(
            "SELECT 1 FROM project_gates WHERE project_id = $1 ORDER BY phase, id FOR UPDATE",
            &[&project],
        )
        .await?;
        tx.execute(sql, &[&project]).await?;
    }
    Ok(())
}

pub(crate) async fn gate_selftest(pool: &Pool, project: &str, under: &str) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    // Замка проекта здесь нет по той же причине, что и у круга пересчёта, — см.
    // довод в `watch.rs`. Пока аренды со сроком нет, «сломан» у самотеста
    // означает либо настоящую беду пробы, либо то, что под ней шёл пересчёт;
    // различить их прогон не умеет, и второй прогон подряд — единственное, чем
    // сегодня отличают одно от другого.
    let items = client
        .query(
            "SELECT phase, id, query, probe, since, subject_query FROM gate_item
              WHERE kind = 'query' ORDER BY phase, item",
            &[],
        )
        .await?;

    let (mut alive, mut broken, mut undeclared) = (Vec::new(), Vec::new(), Vec::new());
    // Третья корзина: правило, чей род факта не свеж, судить отказывается, и
    // уронить его подсадкой нельзя — реагировать нечему.
    let mut stale: Vec<Value> = Vec::new();
    // Самотест ЗАПИСЫВАЕТ приговор пробе: «ни разу не роняли» должен видеть
    // всякий, кто смотрит гейт, а не только тот, кто позвал самотест.
    let mut verdict: Vec<(String, String, Option<bool>)> = Vec::new();
    for r in &items {
        let (phase, item): (String, String) = (r.get(0), r.get(1));
        let query: Option<String> = r.get(2);
        let probe: String = r.get(3);
        // Проба судится границей САМОГО ПУНКТА: подсаженное нарушение случается
        // сейчас, то есть позже границы, — и правило обязано его увидеть. Дать
        // сюда ноль значило бы проверять не то правило, что работает.
        let since: i64 = r.try_get("since").unwrap_or(0);
        let subject: String = r.get(5);
        let sql = query.unwrap_or_default();
        if probe.trim().is_empty() {
            verdict.push((phase.clone(), item.clone(), None));
            undeclared.push(json!({ "phase": phase, "item": item,
                                    "why": "проба не объявлена: чем ронять этот пункт — не сказано" }));
            continue;
        }
        // Подсадка живёт внутри транзакции и умирает вместе с ней: набор после
        // самотеста обязан остаться тем же, чем был.
        let tx = client.transaction().await?;
        force_gates(&tx, project, under).await?;
        // Запрос исполняется ДВАЖДЫ, до подсадки и после, и живым считается
        // пункт, у которого число выросло.
        //
        // Прежде смотрели только «вернул ли запрос строки после подсадки», и на
        // четырнадцати уже красных пунктах это доказывало ровно ничего: они
        // вернули бы строки и с пустой пробой. Самотест отвечал «35 из 35
        // живы», а проверено было двадцать одно.
        let saw = match answer_of(&tx, &sql, project, since).await {
            Err(e) => Err(format!("запрос пункта не исполнился: {e}")),
            Ok(before) => match tx.execute(probe.as_str(), &[&project]).await {
                Err(e) => Err(format!("проба не исполнилась: {}", e.says())),
                // ЧИСЛО СТРОК ДО ПОДСАДКИ — В ОТЧЁТ. Дважды за сессию самотест
                // назвал сломанным пункт, который через минуту оказался цел, и
                // отличить «проба неверна» от «подсаживать было не во что»
                // отчёт не давал: причину искали руками и не нашли.
                Ok(0) => Err(format!(
                    "проба ничего не подсадила; запрос пункта до пробы вернул {} строк",
                    before.len())),
                Ok(_) => match subject_planted(&tx, &subject, project).await {
                    Err(why) => Err(why),
                    Ok(()) => answer_of(&tx, &sql, project, since)
                        .await
                        .map(|after| (before, after))
                        .map_err(|e| format!("запрос пункта не исполнился после подсадки: {e}")),
                },
            },
        };
        tx.rollback().await?;
        match saw {
            Ok((was, became)) if became != was => {
                verdict.push((phase.clone(), item.clone(), Some(true)));
                alive.push(json!({ "phase": phase, "item": item,
                    "was": was.len(), "became": became.len() }));
            }
            Ok((was, _)) => {
                // ПРОТУХШИЙ ДАТЧИК — ТРЕТЬЕ, а не «сломан». Правило, чей род
                // факта не свеж, честно отказывается судить: оно отвечает одной
                // строкой об этом и на подсадку не реагирует — реагировать
                // нечему. Звать это сломанным значит валить в одну кучу «пробу
                // не проверяли» и «проверять сейчас нечем», а чинится это разным:
                // первое — пробой, второе — кормильцем.
                let mut tx_done = false;
                let stale_fact = {
                    static F: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                        regex::Regex::new(r"fact_fresh\(\$1, '([a-z-]+)'\)").expect("образец рода")
                    });
                    let mut found: Option<String> = None;
                    for c in F.captures_iter(&sql) {
                        let f = c[1].to_owned();
                        let fresh: bool = client
                            .query_one("SELECT fact_fresh($1, $2)", &[&project, &f])
                            .await
                            .map(|r| r.get::<_, Option<bool>>(0).unwrap_or(true))
                            .unwrap_or(true);
                        if !fresh {
                            found = Some(f);
                            break;
                        }
                    }
                    found
                };
                if let Some(f) = stale_fact {
                    // НЕ СУДИЛИ — значит `null`, а не «проба проверена». Прежде
                    // сюда шло `true`, и пункт, чей датчик протух насовсем,
                    // навсегда показывал зелёный приговор пробе, которую никто
                    // ни разу не ронял. Колонка `null` допускает, и «не судили»
                    // у неё уже есть.
                    verdict.push((phase.clone(), item.clone(), None));
                    stale.push(json!({ "phase": phase, "item": item, "fact": f,
                        "why": format!("датчик «{f}» не свеж: правило отказывается судить, и уронить \
                                        его подсадкой нельзя — реагировать нечему. Это не «не роняли», \
                                        а «сейчас нечем мерить»") }));
                    tx_done = true;
                }
                if !tx_done {
                verdict.push((phase.clone(), item.clone(), Some(false)));
                broken.push(json!({ "phase": phase, "item": item,
                    "why": format!("на подсаженном нарушении ответ пункта не изменился: те же {} строк, слово в слово", was.len()) }));
                }
            }
            Err(why) => {
                verdict.push((phase.clone(), item.clone(), Some(false)));
                broken.push(json!({ "phase": phase, "item": item, "why": why }));
            }
        }
    }

    // Приговор записывается: «ни разу не роняли» должен видеть всякий, кто
    // смотрит гейт, а не только тот, кто позвал самотест.
    //
    // НО ТОЛЬКО ПРОГОН «КАК ЕСТЬ». Прогоны при крашеных гейтах — диагностика: они
    // отвечают на вопрос «не зависит ли проба от состояния, которого сама не
    // форсирует», и состояние это выдумано. Записывая их, колонка становилась
    // последним-кто-успел: пункт, сломанный как есть и под зелёными, но живой под
    // красными, сохранялся как проверенный — приговор, обратный тому, что сказал
    // отчёт. Требование «жив в обоих состояниях» держит скрипт `selftest.sh`,
    // который роняет прогон; колонка же говорит об одном — о настоящем
    // состоянии набора.
    let persist = under.is_empty();
    if persist {
        for (phase, id, ok) in &verdict {
            // ЗАМЕР ПРИНАДЛЕЖИТ ПРОЕКТУ, и без его имени приговор растекался по
            // всем: `project_gates` ключуется тройкой, а писали сюда парой.
            // Самотест проекта A перекрашивал одноимённый пункт у B — пробой,
            // которую у B никто не запускал, и по данным, которых у B нет.
            client
                .execute(
                    "UPDATE project_gates SET probe_ok = $3
                      WHERE project_id = $4 AND phase = $1 AND id = $2",
                    &[phase, id, ok, &project],
                )
                .await?;
        }
    }

    Ok(json!({
        "under": if under.is_empty() { "как есть" } else { under },
        "persisted": persist,
        "queryItems": items.len(),
        "alive": alive.len(), "aliveItems": alive,
        "broken": broken.len(), "brokenItems": broken,
        "stale": stale.len(), "staleItems": stale,
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
pub(crate) async fn order(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn retarget_links(
    pool: &Pool,
    project: &str,
    dry: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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

    // Соединение отпускается ПЕРЕД записью: каждая правка берёт своё, и держать
    // при этом читающее значит брать два разом на каждый документ.
    drop(client);
    let mut written = 0usize;
    let mut conflicts: Vec<String> = Vec::new();
    if !dry {
        let now = now_ms();
        for (kind, name, text, revision) in &changed {
            match crate::store::put(pool, project, crate::store::Document { kind, name, content: text }, "links-retarget", Some(*revision), now).await {
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

pub(crate) async fn rewrite_links(
    pool: &Pool,
    project: &str,
    dry: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
    // То же, что у `links-retarget`: читающее соединение до записи не нужно.
    drop(client);
    if !dry {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for (kind, name, text, revision) in &changed {
            let who = if name.is_empty() { kind.clone() } else { format!("{kind} {name}") };
            match crate::store::put(pool, project, crate::store::Document { kind, name, content: text }, "links-rewrite", Some(*revision), now).await {
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

/// Виды, чьи пункты готовности переносятся. Список назван здесь один раз и
/// используется и переносом, и отказом: иначе «пунктов нет» и «спрашивать
/// нечем» перестанут различаться.
pub(crate) const READINESS_OWNERS: &[&str] =
    &["task", "red-task", "question", "acceptance", "test-plan", "document-plan"];

/// Вычислить состояние пункта по объявленному способу.
///
/// Пункт без способа остаётся `unknown` и `done` не становится ни от какого
/// сопоставления слов.
pub(crate) async fn readiness_computed(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
) -> Result<Value, crate::db::Fail> {
    if !READINESS_OWNERS.contains(&kind) {
        // Чтение документа не заменяет проекции его строк. Пустой список здесь
        // сказал бы «пунктов нет», тогда как их никто не считал.
        return Ok(json!({
            "owner": { "kind": kind, "id": id },
            "unprojected": true,
            "why": format!("о строках вида {kind} спросить нечем: пункты готовности у него не проецируются"),
        }));
    }
    let client = crate::db::conn(pool).await?;
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
        let v = execute_method(&*client, project, &method_kind, &method).await;
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
pub(crate) struct Verdict {
    pub state: &'static str,
    pub violations: usize,
    pub detail: Vec<String>,
    pub why: String,
}


/// Роли словаря: что спрашивают проекции, что объявлено и что молчит.
///
/// Молчащая роль не «нашла ноль» — она ВЫКЛЮЧАЕТ правило, и снаружи это
/// неотличимо от зелёного. Узнать, чего не хватает, можно было только чтением
/// исходника; один набор объявил так семь ролей за день, каждую — после того,
/// как нашёл её в коде.
pub(crate) async fn scheme_roles(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query("SELECT role, value, coalesce(nullif(project_id,''),'') AS чей
                  FROM scheme_term ORDER BY role, ord, value", &[])
        .await?;
    let mut own: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut shared: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for r in &rows {
        let (role, value, whose): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        if whose == project {
            own.entry(role).or_default().push(value);
        } else if whose.is_empty() {
            shared.entry(role).or_default().push(value);
        }
    }
    // РОЛИ, КОТОРЫЕ СПРАШИВАЕТ SQL ПУНКТОВ, — из самих запросов, а не списком.
    //
    // Половина спрашивающих — не Rust: четырнадцать пунктов гейта читают словарь
    // через `scheme($1) WHERE role = '…'`, и статического перечня для них быть
    // не должно — пункты заводят дверью. Роль `term.foreign-home` стоила
    // шестнадцати находок, `register.questions` — «в реестре строк 0»,
    // `word.manifest-key` — зависимости, которой нет; о каждой узнавали чтением
    // SQL.
    //
    // Отсюда же берётся и «чего стоит»: пункт, который её спрашивает, назван
    // поимённо — точнее любого описания, которое можно придумать заранее.
    let from_rules = client
        .query(
            // Роль спрашивают двумя формами — `role = 'x'` и `role IN ('x','y')`, —
            // и вторая несёт их несколько. Брать только первую значило бы снова
            // потерять роль: так `term.foreign-home` не попала в реестр, а её
            // молчание стоило шестнадцати находок.
            "SELECT m[1] AS роль,
                    string_agg(DISTINCT phase || ' · ' || id, ', ') AS пункты
               FROM gate_item g
               CROSS JOIN LATERAL regexp_matches(g.query, '''([a-z]+\\.[a-z.-]+)''', 'g') m
              WHERE g.kind = 'query' AND g.query LIKE '%scheme(%'
                AND g.query ~ ('role\\s*(=|IN)')
              GROUP BY 1 ORDER BY 1",
            &[],
        )
        .await?;
    let mut asked: Vec<(String, String, String)> = crate::scheme::ROLES
        .iter()
        .map(|(r, how, than)| ((*r).to_owned(), (*how).to_owned(), (*than).to_owned()))
        .collect();
    for r in &from_rules {
        let (role_name, items): (String, String) = (r.get(0), r.get(1));
        if !asked.iter().any(|(name_said, _, _)| *name_said == role_name) {
            asked.push((role_name, "all".to_owned(), format!("её спрашивает пункт гейта: {items}")));
        }
    }
    asked.sort();

    // ОБРАЗЕЦ, НЕ ПОЙМАВШИЙ НИ ОДНОГО ЖИВОГО ИМЕНИ, — находка, а не настройка.
    //
    // Умолчание `id.requirement` было `FR-[A-Z]+-[0-9]+` против живых `FR-122` и
    // `NFR-07`: совпало с нулём имён и промолчало, а раздел держателей
    // инварианта не работал трое суток. Дверь `kind-id-set` этот приём знает —
    // «сверяется на живых именах»; здесь он тот же, только для роли.
    let mut not_catch: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (role_name, _, _) in &asked {
        let Some(kind_name) = role_name.strip_prefix("id.") else { continue };
        let patterns: Vec<String> = own
            .get(role_name.as_str())
            .or_else(|| shared.get(role_name.as_str()))
            .cloned()
            .unwrap_or_default();
        if patterns.is_empty() {
            continue;
        }
        let (mut matched, mut total) = (0i64, 0i64);
        for table in own_tables(&*client, kind_name).await? {
            let r = client
                .query_one(
                    &format!(
                        "SELECT count(*) FILTER (WHERE id ~ ANY($2)), count(*)
                           FROM {table} WHERE project_id = $1"
                    ),
                    &[&project, &patterns],
                )
                .await?;
            matched += r.get::<_, i64>(0);
            total += r.get::<_, i64>(1);
        }
        // Ноль живых имён — не находка: ловить нечего, и это другое.
        if total > 0 && matched == 0 {
            not_catch.insert(role_name.clone(), total);
        }
    }

    let mut stay_silent = 0usize;
    let mut ambiguous = 0usize;
    let mut blind = 0usize;
    let out: Vec<Value> = asked
        .iter()
        .map(|(role, how_asked, than_pays)| {
            let role = role.as_str();
            let how_asked = how_asked.as_str();
            let s = own.get(role);
            let o = shared.get(role);
            let words = s.or(o);
            let how_many = words.map(|v| v.len()).unwrap_or(0);
            // Ноль слов — беда всегда. Два слова — беда ТОЛЬКО у роли, которую
            // спрашивают одним: `one` про такую честно отвечает `None`, и
            // правило выключается. Роль-список двумя значениями не ломается, и
            // звать это двусмысленностью значит поднимать тревогу на здоровом.
            let state = match (how_many, how_asked) {
                (0, _) => { stay_silent += 1; "молчит" }
                (n, "one") if n > 1 => { ambiguous += 1; "двусмысленна" }
                _ if not_catch.contains_key(role) => { blind += 1; "не ловит имён" }
                _ => "считает",
            };
            json!({
                "role": role, "state": state, "asked": how_asked,
                "liveNames": not_catch.get(role).map(|n| json!(n)).unwrap_or(Value::Null),
                "words": words.cloned().unwrap_or_default(),
                "from": if s.is_some() { "набор" } else if o.is_some() { "общее" } else { "нигде" },
                "costs": than_pays,
            })
        })
        .collect();
    Ok(json!({
        "roles": out,
        "asked": out.len(),
        "silent": stay_silent,
        "ambiguous": ambiguous,
        "catchesNothing": blind,
        "why": "молчащая роль ВЫКЛЮЧАЕТ правило, а не обнуляет его находки: снаружи это \
                неотличимо от зелёного. Объявляется дверью `scheme-term-set`",
    }))
}

pub(crate) fn violator(detail: &str) -> String {
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
pub(crate) async fn execute_method(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    method_kind: &str,
    method: &str,
) -> Verdict {
    execute_method_upto(client, project, method_kind, method, 5, 0).await
}

/// То же, но со своим числом примеров.
///
/// Пятёрка хороша там, где ответ читают мельком: подсказка следующего шага,
/// строка готовности. Гейту мало: его пункт открывают затем, чтобы починить, а
/// чинить по пяти именам из тридцати восьми нельзя. Замер гейта хранится, и
/// хранить в нём двести имён вместо пяти ничего не стоит.
pub(crate) async fn execute_method_upto(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    method_kind: &str,
    method: &str,
    limit: usize,
    since: i64,
) -> Verdict {
    match method_kind {
        // `$2` ПОДСТАВЛЯЕТСЯ ТОЛЬКО ТЕМ, КТО ЕГО СПРАШИВАЕТ. Отдать лишний довод
        // запросу, который его не поминает, — отказ базы: «bind message supplies
        // 2 parameters, but prepared statement requires 1». Граница нужна
        // считанным правилам, и знать о ней остальным незачем.
        "query" if !method.trim().is_empty() => match if method.contains("$2") {
            client.query(method, &[&project, &since]).await
        } else {
            client.query(method, &[&project]).await
        } {
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
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let steps = client
        .query(
            "SELECT ord, question, method_kind, method, when_query, when_why, owner_kind, owner,
                    touches, work_run, subject_query, subject_why, unit
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
    // ВСЯ ОТКРЫТАЯ РАБОТА, а не только самая ранняя ступень. Лестница строго
    // упорядочена, а работа — нет: у `tot-ade` одна неотвечаемая ступень
    // («карты проекта нет») закрывала собой предполёт, у которого три задачи
    // готовы прямо сейчас. Дефект на стороне сервера останавливал проект, чьи
    // задачи готовы. Порядок остаётся правдой — `at` по-прежнему самая ранняя
    // невыполненная, — но «работать не над чем» перестаёт быть ответом, когда
    // работа есть.
    let mut open_work: Vec<Value> = Vec::new();
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
        let work_run: String = row.get(9);
        let subject_query: String = row.get(10);
        let subject_why: String = row.get(11);
        let unit: String = row.get(12);

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

        // ПУСТОЙ ПРЕДМЕТ — не «пройдено». Ступень «в плане документов не
        // осталось ненаписанных» проходилась потому, что плана нет вовсе.
        let verdict = if subject_empty(&client, project, &subject_query).await {
            Verdict {
                state: "unknown",
                violations: 0,
                detail: vec![],
                why: if subject_why.trim().is_empty() {
                    "предмет ступени пуст: мерить нечего, и это не «пройдено»".to_owned()
                } else {
                    subject_why.clone()
                },
            }
        } else {
            execute_method(&*client, project, &method_kind, &method).await
        };
        journal.push((ord, verdict.state.to_owned(), verdict.detail.join(" · ")));

        match verdict.state {
            "passed" => {
                passed.push(ord);
                continue;
            }
            // НЕОТВЕЧАЕМАЯ СТУПЕНЬ НЕ БЫВАЕТ ТЕКУЩЕЙ. Прежде ручка говорила
            // одновременно «вот твоя ступень» и «мерить её нечем»: агент, верящий
            // первому, шёл делать работу, про которую вторая половина ответа
            // сказала, что её сделать нельзя.
            //
            // Она остаётся в `unanswerable` со своей причиной — прятать её
            // нельзя, — но текущей работой не объявляется.
            "unknown" | "stale" => {
                unanswerable.push(json!({ "ord": ord, "why": verdict.why, "question": question,
                                          "owner": owner, "touches": touches }));
                if touches == "corpus" {
                    corpus_open = true;
                }
                continue;
            }
            _ => {
                if touches == "corpus" {
                    corpus_open = true;
                }
            }
        }

        let here_name = violator(verdict.detail.first().map(String::as_str).unwrap_or(""));
        let here_run = if work_run.trim().is_empty() || here_name.is_empty() {
            Value::Null
        } else {
            json!(work_run.replace(WORK_RUN_NAME, &here_name))
        };
        open_work.push(json!({
            "ord": ord,
            "question": question,
            "state": verdict.state,
            "violations": verdict.violations,
            "detail": verdict.detail,
            "owner": owner,
            "touches": touches,
            "run": here_run,
            "kind": unit,
        }));

        if at.is_none() {
            // Репозиторная ступень при открытой фазе набора не выдаётся, и
            // причина называется: иначе отказ читается как «нечего делать».
            if touches == "repository" && corpus_open {
                at = Some(json!({
                    "ord": ord, "question": question, "state": "held",
                    "ownerKind": owner_kind, "owner": owner, "touches": touches, "kind": unit,
                    "why": "ступень пишет в репозиторий, а фаза набора ещё открыта",
                }));
            } else {
                // ПЕРВАЯ ЕДИНИЦА РАБОТЫ, а не заголовок ступени. «ord 5 ·
                // открытых вопросов не осталось · violations 10» — и всё: какой
                // вопрос брать первым и чем он закрывается, не сказано. Соседняя
                // ручка `next-task` устроена наоборот и отдаёт задачу со всем
                // контекстом внутри.
                //
                // Команду даёт САМА СТУПЕНЬ колонкой `work_run`, `{name}` —
                // первое слово находки. Пусто — команды нет, и `next-step` её не
                // выдумывает: угаданная команда хуже отсутствующей.
                let first = verdict.detail.first().cloned().unwrap_or_default();
                let name = violator(&first);
                let run = if work_run.trim().is_empty() || name.is_empty() {
                    Value::Null
                } else {
                    json!(work_run.replace(WORK_RUN_NAME, &name))
                };
                at = Some(json!({
                    "ord": ord, "question": question, "state": verdict.state,
                    "ownerKind": owner_kind, "owner": owner, "touches": touches,
                    "why": verdict.why, "violations": verdict.violations, "detail": verdict.detail,
                    // Команда и вид стоят у ступени так же, как у всякой открытой
                    // работы в `openWork`. Прежде `run` жил только внутри `first`,
                    // и читающий `at.run` находил пустоту рядом с заполненной
                    // командой: два ответа на один вопрос, один из них пустой.
                    "run": run, "kind": unit,
                    "first": json!({ "unit": first, "name": name, "run": run, "kind": unit }),
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
        // Ступени, у которых работа есть ПРЯМО СЕЙЧАС, — включая те, что стоят
        // позади текущей. Ответ «работать не над чем» при семидесяти шести
        // готовых задачах был неправдой о проекте, а не о лестнице.
        "openWork": open_work,
        // Номер ступени — НЕ мера пройденного, и читать его так значит ошибиться.
        // «Где мы» отвечает на «что чинить первым»: лестница останавливается на
        // самой ранней невыполненной. Проект с чистым набором и ненаписанным
        // кодом стоит на девятой, проект с написанным кодом и одной корпусной
        // придиркой — на шестой, и второй при этом дальше по продукту.
        //
        // Поэтому рядом стоит счёт: сколько ступеней выполнено из скольких.
        // Два числа отвечают на два разных вопроса, и ни одно не выдаёт себя за
        // другое.
        // НОМЕР СТУПЕНИ ЧИТАЕТСЯ КАК ПОЗИЦИЯ, А ОН ЕЮ НЕ ЯВЛЯЕТСЯ. `passed`
        // несплошной — пятая, седьмая и восьмая могут быть пройдены при текущей
        // четвёртой, — а `at.ord` рядом с `of: 13` читается как «4 из 13», и
        // читается так всеми. Поэтому рядом едет счёт, который номером не
        // притворяется: выполнено, неотвечаемо, осталось.
        "progress": json!({
            "met": passed.len(),
            "unanswerable": unanswerable.len(),
            "left": total_steps.saturating_sub(passed.len() + skipped.len() + unanswerable.len()),
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
pub(crate) async fn measure_process(
    pool: &Pool,
    project: &str,
    set_name: &str,
    process: &str,
) -> Result<Value, crate::db::Fail> {
    let mut answer = compute_next_step(pool, project, set_name, process, true).await?;
    // КРАСНЫЙ ГЕЙТ ТЕКУЩЕЙ ФАЗЫ — вне очереди ступеней.
    //
    // Ступени про гейты — шестая и одиннадцатая, а пятая спрашивает про открытые
    // вопросы. Пока хоть один вопрос открыт, лестница НИКОГДА не назовёт красный
    // гейт: измерено на myack, где `phases` печатал `gateState: failed` у двух
    // фаз, а `next-step` всю дорогу отвечал «отвечайте на вопросы». Две ручки
    // знали разное об одном проекте, и не соединяло их ничто.
    //
    // Здесь гейт называется НЕЗАВИСИМО от того, куда дошла лестница: работа в
    // нём есть уже сейчас, и ждать своей ступени ей незачем.
    {
        let client = crate::db::conn(pool).await?;
        let rows = client
            .query(
                "SELECT ph.id, ph.title, ph.gate, g.id, i.item, g.violations
                   FROM phase ph
                   JOIN project_gates g ON g.project_id = $1 AND g.phase = ph.gate
                   JOIN gate_item i ON i.phase = g.phase AND i.id = g.id
                  WHERE ph.ord = (SELECT min(p2.ord) FROM phase p2
                                   -- ЗДЕСЬ `p2.gate <> ''` НУЖЕН, и это не та же
                                   -- проверка, что у `phase_open`. Та спрашивает
                                   -- про ПРЕДШЕСТВЕННИКА — «пустой гейт не
                                   -- пройден, значит держит»; эта спрашивает про
                                   -- СОБСТВЕННЫЙ гейт фазы — «где мы». У
                                   -- хвостовой фазы гейта может не быть законно,
                                   -- и без этого условия она навсегда становится
                                   -- первой непройденной: проект, доделавший всё,
                                   -- никогда не смог бы сказать, что доделал.
                                   WHERE p2.gate <> ''
                                     AND coalesce((SELECT s.computed FROM gate_state s
                                                    WHERE s.project_id = $1 AND s.gate = p2.gate),
                                                  'open') <> 'passed')
                    AND g.result->>'computed' = 'failed'
                  ORDER BY g.violations DESC NULLS LAST, g.id",
                &[&project],
            )
            .await?;
        // СТАДИЯ ПЕРВОЙ СТРОКОЙ. На вопрос «где мы» отвечает фаза и состояние её
        // гейта, а не номер ступени: порядок ступеней хорош для «что делать
        // дальше», а «где мы» он отвечает не о том. Прежде `phases` знал и
        // печатал, `next-step` молчал — две ручки об одном проекте в одну
        // секунду.
        let stage = client
            .query_opt(
                "SELECT ph.id, ph.title, ph.gate,
                        (SELECT count(*) FROM project_gates g
                          WHERE g.project_id = $1 AND g.phase = ph.gate
                            AND g.result->>'computed' = 'failed') AS красных,
                        (SELECT coalesce(sum(g.violations), 0) FROM project_gates g
                          WHERE g.project_id = $1 AND g.phase = ph.gate
                            AND g.result->>'computed' = 'failed') AS нарушений
                   FROM phase ph
                  WHERE ph.ord = (SELECT min(p2.ord) FROM phase p2
                                   -- ЗДЕСЬ `p2.gate <> ''` НУЖЕН, и это не та же
                                   -- проверка, что у `phase_open`. Та спрашивает
                                   -- про ПРЕДШЕСТВЕННИКА — «пустой гейт не
                                   -- пройден, значит держит»; эта спрашивает про
                                   -- СОБСТВЕННЫЙ гейт фазы — «где мы». У
                                   -- хвостовой фазы гейта может не быть законно,
                                   -- и без этого условия она навсегда становится
                                   -- первой непройденной: проект, доделавший всё,
                                   -- никогда не смог бы сказать, что доделал.
                                   WHERE p2.gate <> ''
                                     AND coalesce((SELECT s.computed FROM gate_state s
                                                    WHERE s.project_id = $1 AND s.gate = p2.gate),
                                                  'open') <> 'passed')",
                &[&project],
            )
            .await?;
        if let Some(o) = answer.as_object_mut() {
            o.insert(
                "stage".into(),
                match &stage {
                    Some(r) => json!({
                        "phase": r.get::<_, String>(0),
                        "title": r.get::<_, String>(1),
                        "gate": r.get::<_, String>(2),
                        "gateState": if r.get::<_, i64>(3) > 0 { "failed" } else { "open" },
                        "redItems": r.get::<_, i64>(3),
                        "violations": r.get::<_, i64>(4),
                        "why": "это ответ на «где мы». Ступень ниже — ответ на «что делать дальше», \
                                и порядок у неё свой",
                    }),
                    None => json!({ "why": "все фазы пройдены: гейт не держит ни одна" }),
                },
            );
            o.insert(
                "redGate".into(),
                if rows.is_empty() {
                    Value::Null
                } else {
                    json!({
                        "phase": rows[0].get::<_, String>(0),
                        "title": rows[0].get::<_, String>(1),
                        "gate": rows[0].get::<_, String>(2),
                        "items": rows.iter().map(|r| json!({
                            "id": r.get::<_, String>(3),
                            "item": r.get::<_, String>(4),
                            "violations": r.get::<_, Option<i32>>(5),
                        })).collect::<Vec<_>>(),
                        "why": "красный гейт текущей фазы: работа в нём есть независимо от того, \
                                куда дошла лестница",
                    })
                },
            );
            // ЖДУТ СВОЕЙ ФАЗЫ — отдельным списком, а не в общем счёте работы.
            //
            // Ступени про задачи считают задачи ОТКРЫТЫХ фаз: иначе лестница
            // звала владельца отправлять 121 задачу фазы, которую гейт ещё не
            // открыл, и предполёт — освежать по ним вердикты. Спрятать их
            // совсем нельзя — работа настоящая; поэтому они названы здесь, и
            // рядом назван гейт, который их откроет.
            let waiting = client
                .query(
                    "SELECT tp.phase, coalesce(h.gate, ''), count(*)
                       FROM task_phase tp
                       LEFT JOIN LATERAL (
                            SELECT CASE WHEN p2.gate = ''
                                         THEN 'гейт фазы ' || p2.id || ' не объявлен'
                                         ELSE p2.gate END AS gate
                              FROM phase p2
                             WHERE p2.ord < tp.phase_ord
                               AND coalesce((SELECT s.computed FROM gate_state s
                                              WHERE s.project_id = tp.project_id
                                                AND s.gate = p2.gate), 'open') <> 'passed'
                             ORDER BY p2.ord LIMIT 1) h ON true
                      WHERE tp.project_id = $1 AND tp.state <> 'closed' AND tp.open IS NOT TRUE
                      GROUP BY 1, 2 ORDER BY 1",
                    &[&project],
                )
                .await?;
            o.insert(
                "waitingForPhase".into(),
                json!(waiting.iter().map(|r| json!({
                    "phase": r.get::<_, Option<String>>(0),
                    "opensWith": r.get::<_, String>(1),
                    "tasks": r.get::<_, i64>(2),
                    "why": match r.get::<_, Option<String>>(0) {
                        None => "вид задачи не отображён ни на одну фазу: отображение не объявлено",
                        Some(_) => "фаза не открыта: её задачи не работа, пока не пройден названный гейт",
                    },
                })).collect::<Vec<_>>()),
            );
        }
    }
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
    let client = crate::db::conn(pool).await?;
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
/// Лестница ЦЕЛИКОМ, с состоянием каждой ступени.
///
/// `process-state` отдавал вопрос, способ-род, владельца и сторону — и **ни
/// запроса, ни пробы, ни вычисленного состояния, ни числа нарушений**. То есть
/// ступень нельзя было ни просмотреть, ни сверить, ни починить, не имея прямого
/// доступа к базе.
///
/// Это дословно тот дефект, который у пункта гейта закрыт: пункт отдаёт `id`,
/// `query`, `probe`, `probeRuns`. Лестница того же не получила,
/// и расхождение двух ручек об одном предмете — ступень 4 говорит «пройдена»,
/// `plan` говорит «пять ненаписанных» — разрешалось чтением кода, а не вопросом
/// к харнесу.
///
/// Состояние СЧИТАЕТСЯ ЗДЕСЬ, тем же единственным исполнителем, что у гейта.
/// Второй записи о нём не заводится: она разошлась бы с первой.
///
/// Условная ступень отвечает `skipped` с причиной. Пропуск — не «пройдено»:
/// слить их значило бы посчитать невыполненное выполненным.
/// Пуст ли ПРЕДМЕТ ступени.
///
/// Ступень «в плане документов не осталось ненаписанных» проходится, когда плана
/// нет ВОВСЕ: у `tot-ade` в нём ноль позиций, запрос возвращает ноль строк, и
/// лестница объявляет пройденным то, чего не смотрела. Ровно тот случай, против
/// которого написана вся доктрина «незнание называется словом».
///
/// Сходится и с самотестом: ступень, чей предмет пуст, проба не роняет — пустое
/// множество не ломается подсадкой.
///
/// Предмет ОБЪЯВЛЯЕТСЯ. Не объявлен — проверять нечего, и ступень отвечает как
/// прежде: выводить предмет из запроса значило бы гадать.
async fn subject_empty(
    client: &deadpool_postgres::Client,
    project: &str,
    subject_query: &str,
) -> bool {
    if subject_query.trim().is_empty() {
        return false;
    }
    match client.query(subject_query, &[&project]).await {
        Ok(rows) => rows.is_empty(),
        // Запрос предмета, который не исполнился, — не «предмет пуст». Судит
        // тогда сам способ, и его отказ будет виден своим словом.
        Err(_) => false,
    }
}

/// Кто и чем виноват: поля двери `blame-set`.
pub(crate) struct Blame<'a> {
    pub rule: &'a str,
    pub entity_id: &'a str,
    pub blame: &'a str,
    pub fixed_by: &'a str,
    pub why: &'a str,
    pub decided_by: &'a str,
}

/// Объявить, ЧЕЙ предмет спора у находки.
///
/// Находка остаётся красной: признак делит счёт, а не уменьшает его. `fixedBy`
/// обязателен — признак без указания, чем это чинится, ничем не отличается от
/// жалобы, и следующий проход запишет её заново.
pub(crate) async fn set_blame(pool: &Pool, project: &str, fields: Blame<'_>, drop_it: bool) -> Result<Value, crate::db::Fail> {
    let Blame { rule, entity_id, blame, fixed_by, why, decided_by } = fields;
    let client = crate::db::conn(pool).await?;
    if drop_it {
        let gone = client
            .execute(
                "DELETE FROM finding_blame WHERE project_id = $1 AND rule = $2 AND entity_id = $3",
                &[&project, &rule, &entity_id],
            )
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" } }));
    }
    if rule.trim().is_empty() || entity_id.trim().is_empty() {
        return Ok(json!({ "status": "nameless",
                          "why": "признак ставится НАХОДКЕ: нужны правило и ключ, которым она адресуется" }));
    }
    if !matches!(blame, "harness" | "corpus") {
        return Ok(json!({ "status": "unknown_blame", "why":
            "чей предмет спора — `harness` (не читает сервер) либо `corpus` (работа набора)" }));
    }
    if fixed_by.trim().is_empty() {
        return Ok(json!({ "status": "no_fix", "why":
            "не сказано, чем это чинится. Признак без этого — жалоба: находка останется \
             красной, а следующий проход запишет её заново, не зная, что с ней делали." }));
    }
    // Правило обязано СУЩЕСТВОВАТЬ. Признак, поставленный правилу, которого нет,
    // не увидит никто, а выглядеть будет как объявленный.
    let known: i64 = client
        .query_one("SELECT count(*) FROM gate_item WHERE id = $1", &[&rule])
        .await?
        .get(0);
    if known == 0 {
        return Ok(json!({ "status": "no_rule",
                          "why": format!("пункта гейта «{rule}» нет: признак ставить некому") }));
    }
    client
        .execute(
            "INSERT INTO finding_blame (project_id, rule, entity_id, blame, fixed_by, why, decided_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (project_id, rule, entity_id) DO UPDATE SET blame = EXCLUDED.blame,
               fixed_by = EXCLUDED.fixed_by, why = EXCLUDED.why, decided_by = EXCLUDED.decided_by",
            &[&project, &rule, &entity_id, &blame, &fixed_by, &why, &decided_by],
        )
        .await?;
    Ok(json!({ "status": "declared", "rule": rule, "entityId": entity_id, "blame": blame,
               "fixedBy": fixed_by,
               "means": "находка остаётся красной: счёт разделён, а не уменьшен" }))
}

pub(crate) async fn process_state(
    pool: &Pool,
    project: &str,
    process: &str,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT ord, question, method_kind, method, when_query, when_why,
                    owner_kind, owner, touches, probe, work_run, subject_query, subject_why
               FROM harness_process_step
              WHERE set_name = 'godzy' AND process = $1 ORDER BY ord",
            &[&process],
        )
        .await?;
    let mut out = Vec::new();
    for r in &rows {
        let ord: i32 = r.get(0);
        let method_kind: String = r.get(2);
        let method: String = r.get(3);
        let when_query: String = r.get(4);
        let when_why: String = r.get(5);
        let probe: String = r.get(9);

        // Проба ступени — тот же вопрос, что у пункта гейта: роняли ли её. Пустая
        // проба и непроверяемая — разное, и `null` здесь значит «пробы нет»,
        // а не «не роняется».
        let probe_runs: Option<bool> = if probe.trim().is_empty() {
            None
        } else {
            Some(client.prepare(probe.trim()).await.is_ok())
        };

        let mut skipped = false;
        if !when_query.trim().is_empty() {
            skipped = match client.query(when_query.as_str(), &[&project]).await {
                Ok(found) => found.is_empty(),
                Err(_) => false,
            };
        }
        // Порядок колонок ЗДЕСЬ свой: перед `work_run` стоит `probe`, и те же
        // номера, что у обходчика, читали бы соседнее поле.
        let subject_query: String = r.get(11);
        let subject_why: String = r.get(12);
        let empty = subject_empty(&client, project, &subject_query).await;
        // Находки отдаются ЦЕЛИКОМ. Пятёрка хороша там, где ответ читают мельком;
        // эту ручку открывают затем, чтобы чинить, и «нарушений 76, показано 5»
        // — это работа по половине предмета вслепую.
        let verdict = if skipped || empty {
            None
        } else {
            Some(execute_method_upto(&*client, project, &method_kind, &method, 200, 0).await)
        };
        out.push(json!({
            "ord": ord,
            "question": r.get::<_, String>(1),
            "methodKind": method_kind,
            "method": method,
            "probe": probe,
            "probeRuns": probe_runs,
            "whenQuery": when_query,
            "whenWhy": when_why,
            "ownerKind": r.get::<_, String>(6),
            "owner": r.get::<_, String>(7),
            "touches": r.get::<_, String>(8),
            "workRun": r.get::<_, String>(10),
            "subjectQuery": subject_query,
            "computed": match &verdict {
                Some(v) => v.state,
                None if skipped => "skipped",
                None => "unknown",
            },
            "violations": verdict.as_ref().map(|v| v.violations),
            "detail": verdict.as_ref().map(|v| v.detail.clone()),
            "why": match &verdict {
                Some(v) => v.why.clone(),
                None if skipped => when_why,
                None => if subject_why.trim().is_empty() {
                    "предмет ступени пуст: мерить нечего, и это не «пройдено»".to_owned()
                } else {
                    subject_why
                },
            },
        }));
    }
    Ok(json!({
        "process": process,
        "steps": out,
        "means": "состояние ступени считается ТЕМ ЖЕ исполнителем, что у пункта гейта; \
                  `skipped` — условие ступени не выполнено, и это не «пройдено»",
    }))
}

/// Чем заняться, пока фаза закрыта: ближайшая ступень лестницы.
///
/// Лестница — тот же вопрос «что делать дальше», только не про задачу, а про
/// процесс, и держащий гейт стоит на ней ступенью. Ответ берётся у неё целиком,
/// чтобы двух порядков работ не было.
async fn step_instead(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
) -> Result<Value, crate::db::Fail> {
    let asked_step = position(client, project, "godzy").await?;
    Ok(json!({
        "why": "фаза закрыта, но работа есть: её называет лестница — это работа фазы НИЖЕ, та самая, что откроет гейт",
        "step": asked_step["at"]["question"],
        "ord": asked_step["at"]["ord"],
        "first": asked_step["at"]["first"],
        "state": asked_step["at"]["state"],
    }))
}

/// Держит ли лестница выдачу задач — и чем.
///
/// `next-step` и `next-task` — разные вопросы, «где мы и что держит» против «что
/// брать внутри волны», и сливать их нельзя. Но без связи вторая дверь была
/// способом обойти порядок, не заметив, что обходишь: лестница стояла на
/// корпусном гейте, а `next-task` называл кодовую задачу.
async fn ladder_holds(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    own_phase: Option<i32>,
) -> Result<Option<Value>, crate::db::Fail> {
    let task: Option<i32> = client
        .query_one(
            "SELECT min(ord) FROM harness_process_step
              WHERE set_name = 'godzy' AND process = 'godzy' AND unit = 'task'",
            &[],
        )
        .await?
        .get(0);
    let Some(task) = task else {
        return Ok(Some(json!({
            "why": "задачная ступень лестницы не объявлена: где по порядку начинаются задачи, сказать \
                    нечем, и это не «можно всё». Объявляется дверью `step-method-set` доводом `unit=task`",
        })));
    };
    let asked_step = position(client, project, "godzy").await?;
    let red_earlier: Vec<String> = client
        .query(
            "SELECT g.phase || ' · ' || i.item FROM project_gates g
               JOIN gate_item i ON i.phase = g.phase AND i.id = g.id
              WHERE g.project_id = $1 AND g.state = 'failed'
                AND g.phase NOT IN (SELECT ph.gate FROM phase ph WHERE ph.ord >= $2)
              ORDER BY g.phase, i.item",
            &[&project, &own_phase.unwrap_or(i32::MAX)],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    Ok(decision_ladder(&asked_step, i64::from(task), &red_earlier))
}

fn decision_ladder(asked_step: &Value, task: i64, red_earlier: &[String]) -> Option<Value> {
    if asked_step["checkedAt"].is_null() || !asked_step["openWork"].is_array() {
        return Some(json!({
            "why": "положение лестницы этим кодом ни разу не считали: держит ли она задачи, сказать нечем, и это не «можно всё»",
        }));
    }
    let (step, hold, total) = holding_step(asked_step, task, red_earlier)?;
    let at = &asked_step["at"];
    Some(json!({
        "ord": step["ord"],
        "owner": step["owner"],
        "question": step["question"],
        "run": step["run"],
        "first": if step["ord"] == at["ord"] { at["first"].clone() } else { Value::Null },
        "holding": hold.iter().take(5).collect::<Vec<_>>(),
        "holdingCount": total,
        "stale": asked_step["stale"],
        "why": format!(
            "ступень {} не пройдена, владелец `{}`: «{}». Задачи не запрашиваются, пока не пройдены все \
             ступени до {task}-й — сперва то, что держит эта",
            step["ord"], step["owner"].as_str().unwrap_or(""), step["question"].as_str().unwrap_or("")),
    }))
}

fn holding_step(asked_step: &Value, task: i64, red_earlier: &[String]) -> Option<(Value, Vec<String>, i64)> {
    let corpus_without_answer_earlier = |ord: i64| {
        asked_step["unanswerable"].as_array().is_some_and(|u| {
            u.iter().any(|s| s["touches"] == "corpus" && s["ord"].as_i64().is_some_and(|o| o < ord))
        })
    };
    asked_step["openWork"]
        .as_array()?
        .iter()
        .filter_map(|s| Some((s["ord"].as_i64()?, s)))
        .filter(|&(ord, s)| {
            ord < task
                && s["state"] == "failed"
                && !(s["touches"] == "repository" && corpus_without_answer_earlier(ord))
        })
        .find_map(|(_, s)| {
            if s["kind"] == "gate" {
                (!red_earlier.is_empty())
                    .then(|| (s.clone(), red_earlier.to_vec(), red_earlier.len() as i64))
            } else {
                Some((
                    s.clone(),
                    serde_json::from_value(s["detail"].clone()).unwrap_or_default(),
                    s["violations"].as_i64().unwrap_or(1),
                ))
            }
        })
}

/// Метка в команде ступени, вместо которой подставляется первое слово находки.
///
/// Объявлена ОДНАЖДЫ: она же стоит в описании двери, в доводе к колонке и в
/// самих записях набора, и разъехавшись, они перестают подставляться молча.
pub(crate) const WORK_RUN_NAME: &str = "{name}";

/// Как дверь объясняет эту метку. Словами двери, а не второй копией строки:
/// описание и подстановка разъезжаются молча, и набор пишет команду с меткой,
/// которой никто не подставит.
pub(crate) fn step_run_hint() -> String {
    format!("команда, которой видна единица работы ступени; `{WORK_RUN_NAME}` — первое слово находки")
}

pub(crate) async fn next_step(pool: &Pool, project: &str, process: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    position(&*client, project, process).await
}

/// Сохранённое положение лестницы — на уже взятом соединении.
///
/// Кто держит своё соединение, второго из пула не берёт: слотов восемь, срока
/// ожидания нет, и вложенный захват — это тупик, а не медленность.
async fn position(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    process: &str,
) -> Result<Value, crate::db::Fail> {
    let row = client
        .query_opt(
            // Устарело то, что считали РАНЬШЕ последней правки, — кто бы ни
            // считал. Сравнение с одним прогоном работника называло устаревшим
            // положение, только что посчитанное дверью `gate-measure`.
            "SELECT p.result, p.checked_at,
                    coalesce((SELECT d.dirty_at > greatest(coalesce(d.ran_at, 0), p.checked_at) FROM gate_dirty d
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

pub(crate) async fn process_history(pool: &Pool, project: &str, process: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn progress(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn task_status(pool: &Pool, project: &str, task: Option<&str>) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn status_anomaly(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let all = task_status(pool, project, None).await?;
    let list: Vec<Value> = all["tasks"]
        .as_array()
        .map(|a| a.iter().filter(|t| t["anomaly"] == json!(true)).cloned().collect())
        .unwrap_or_default();
    Ok(json!({ "count": list.len(), "anomalies": list }))
}

/// Плитка конвейера: сколько задач на каждом статусе, и сколько неизвестно.
pub(crate) async fn task_pipeline(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
                // Псевдоним кириллицей: объявленный факт может завести свой
                // `t` и перекрыть внешний — см. `task_board`.
                &format!("SELECT count(*) FROM project_plan_tasks задача WHERE задача.project_id = $1 AND EXISTS ({})",
                         fact.replace("$2", "задача.id")),
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

/// Доска: где стоит каждая задача разработки и какие ступени она миновала.
///
/// Плитка конвейера считает ступени порознь: «закрыта 17» и «проверена 82» —
/// два независимых числа, и по ним не видно, что это разные задачи. Доска
/// ставит каждую задачу в одну колонку — самую дальнюю, которой она достигла,
/// — и потому показывает то, чего плитка показать не может: ступень, через
/// которую перешагнули.
///
/// Ступени берутся из `kind_status`, а не из списка в коде: набор объявляет их
/// сам, и седьмая ступень появится здесь без правки этой функции.
///
/// Зеркала (`kind = 'red'`) в доску не идут. Они не проходят те же ступени, и
/// смешивать их с задачами разработки — это ровно та ошибка, из-за которой
/// «222 задачи» много недель значили 139 задач и 83 их пары.
pub(crate) async fn task_board(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let statuses = client
        .query("SELECT ord, name, title, fact, durable FROM kind_status WHERE kind = 'task' ORDER BY ord", &[])
        .await?;

    // Сами задачи: только разработка, с этапом, названием и зеркалом.
    let rows = client
        .query(
            "SELECT t.id, t.title, t.milestone_id,
                    (SELECT r.id FROM red_task r
                      WHERE r.project_id = t.project_id AND r.parent_task = t.id LIMIT 1) AS mirror
               FROM project_plan_tasks t
              WHERE t.project_id = $1 AND t.kind = 'dev'
              ORDER BY t.milestone_id, t.ord, t.id",
            &[&project],
        )
        .await?;
    let mirrors: i64 = client
        .query_one(
            "SELECT count(*) FROM project_plan_tasks WHERE project_id = $1 AND kind = 'red'",
            &[&project],
        )
        .await?
        .get(0);

    // По ступени — множество достигших её задач, одним запросом на ступень.
    // Ступень без факта не «никем не достигнута», а неизмерима: её ответ —
    // `null`, и в подсчёт пропусков она не входит.
    let mut ladder: Vec<(String, String, bool, Option<std::collections::HashSet<String>>)> = Vec::new();
    for s in &statuses {
        let name: String = s.get(1);
        let title: String = s.get(2);
        let fact: String = s.get(3);
        let durable: bool = s.get(4);
        if fact.trim().is_empty() {
            ladder.push((name, title, durable, None));
            continue;
        }
        // Псевдоним внешней задачи — кириллицей, и это не украшение. Факт
        // «проанализирована» объявляет ВНУТРИ себя `JOIN project_plan_tasks t`;
        // с внешним `t` он перекрывал его, `$2` указывал на внутреннюю строку,
        // и ступень выходила достигнутой у всех 139. Латинский псевдоним может
        // совпасть с любым в объявленном факте — кириллический не может.
        let q = format!(
            "SELECT задача.id FROM project_plan_tasks задача
              WHERE задача.project_id = $1 AND задача.kind = 'dev' AND EXISTS ({})",
            fact.replace("$2", "задача.id")
        );
        match client.query(q.as_str(), &[&project]).await {
            Ok(r) => ladder.push((name, title, durable, Some(r.iter().map(|x| x.get::<_, String>(0)).collect()))),
            // Запрос не выполнился — это тоже «нечем мерить», а не «никого нет».
            Err(_) => ladder.push((name, title, durable, None)),
        }
    }

    let total = rows.len() as i64;
    let mut at_count: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut torn_count: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut tasks = Vec::new();
    let mut torn_total: i64 = 0;

    for r in &rows {
        let id: String = r.get(0);
        let title: String = r.get(1);
        let milestone: Option<String> = r.get(2);
        let mirror: Option<String> = r.get(3);

        let mut at: Option<&str> = None;
        let mut at_ord = 0usize;
        for (i, (name, _, _, set)) in ladder.iter().enumerate() {
            if set.as_ref().is_some_and(|s| s.contains(&id)) {
                at = Some(name.as_str());
                at_ord = i;
            }
        }
        // Пропущенная ступень — накопительная, измеримая, стоящая раньше
        // достигнутой и не достигнутая. Неизмеримую сюда не пишем:
        // «неизвестно» не улика. Ненакопительную — тоже: «ветки нет сейчас»
        // не значит «работы не было».
        let missed: Vec<String> = ladder
            .iter()
            .take(at_ord)
            .filter(|(_, _, durable, set)| *durable && set.as_ref().is_some_and(|s| !s.contains(&id)))
            .map(|(name, _, _, _)| name.clone())
            .collect();
        let torn = !missed.is_empty();
        if torn {
            torn_total += 1;
        }
        if let Some(a) = at {
            *at_count.entry(a.to_owned()).or_insert(0) += 1;
            if torn {
                *torn_count.entry(a.to_owned()).or_insert(0) += 1;
            }
        }
        tasks.push(json!({
            "id": id, "title": title, "milestone": milestone, "mirror": mirror,
            "at": at, "torn": torn, "missed": missed,
        }));
    }

    let columns: Vec<Value> = ladder
        .iter()
        .map(|(name, title, durable, set)| match set {
            Some(s) => json!({
                "status": name, "title": title,
                "at": at_count.get(name).copied().unwrap_or(0),
                "torn": torn_count.get(name).copied().unwrap_or(0),
                "reached": s.len() as i64, "unknown": 0, "durable": durable,
            }),
            None => json!({
                "status": name, "title": title,
                "at": Value::Null, "torn": 0, "reached": Value::Null, "unknown": total,
                "durable": durable, "why": "факт этого статуса никто не записывает",
            }),
        })
        .collect();

    Ok(json!({
        "total": total, "mirrors": mirrors, "torn": torn_total,
        "columns": columns, "tasks": tasks,
    }))
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
pub(crate) async fn waves(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let tasks = client
        .query(
            "SELECT t.id, t.milestone_id, t.title, t.kind, t.state, t.ord,
                    coalesce(r.checks, 0) AS checks,
                    (SELECT count(*) FROM task_requirement q
                      WHERE q.project_id = t.project_id AND q.task_id = t.id) AS requirements,
                    (SELECT v.verdict FROM preflight_verdict v
                      WHERE v.project_id = t.project_id AND v.task_id = t.id
                      ORDER BY v.at DESC LIMIT 1) AS preflight,
                    -- ФАЗА — В КАРТОЧКЕ, потому что очередь строят отсюда.
                    -- Барьер стоит в `next-task`, а диспетчер этапа берёт волну
                    -- и раздаёт её по исполнителям: доска знала про фазу, а
                    -- очередь — нет, и мимо барьера уходило ровно то, что он
                    -- держит.
                    tp.phase, tp.open AS phase_open
               FROM project_plan_tasks t
               LEFT JOIN red_task r ON r.project_id = t.project_id AND r.id = t.id
               LEFT JOIN task_phase tp ON tp.project_id = t.project_id AND tp.task_id = t.id
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
            "phase": r.get::<_, Option<String>>(9),
            // Пусто — вид задачи не отображён ни на одну фазу: «не объявлено», а
            // не «можно раздавать».
            "phaseOpen": r.get::<_, Option<bool>>(10),
            "wave": wave[i],
            "waits": depends[i].iter().filter(|&&j| tasks[j].get::<_, String>(4) != "closed").count(),
        }));
    }
    let open_red = cards
        .iter()
        .filter(|c| c["kind"] == "red" && c["state"] != "closed")
        .count();
    // Сколько в волне того, что раздавать нельзя. Счёт рядом с карточками, а не
    // вместо них: спрятать такие задачи значило бы потерять работу, которая
    // станет работой, как только гейт откроется.
    let held = cards
        .iter()
        .filter(|c| c["state"] != "closed" && c["phaseOpen"] != serde_json::Value::Bool(true))
        .count();
    Ok(json!({
        "cards": cards,
        "total": cards.len(),
        // Барьер красной фазы: весь трек проверок предшествует всему коду.
        "openRed": open_red,
        "heldByPhase": held,
        "why": if held > 0 {
            "карточки с `phaseOpen` не `true` в очередь не идут: их фаза не открыта либо              вид задачи не отображён ни на одну фазу"
        } else { "" },
    }))
}

/// Фазы с полной картиной: документы, гейт и задачи — каждой своим счётом.
///
/// До сих пор в плане были только вехи кода, и трек проверок в картину не
/// входил. Теперь фаза знает все три свои части, и **ни одна не складывается с
/// другой**: документы считаются по плану, гейт вычисляется своими запросами,
/// задачи — по состоянию из истории.
async fn compute_phases(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    // Гейт считается ДО того, как это соединение взято: он берёт своё, и два
    // сразу на один ответ — способ запереть пул на себе же.
    let gates = gate(pool, project, None).await?;
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT id, ord, title, gate, plan_level, task_kind FROM phase ORDER BY ord",
            &[],
        )
        .await?;
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
pub(crate) async fn measure_phases(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let computed = compute_phases(pool, project).await?;
    let empty = Vec::new();
    let list = computed["phases"].as_array().unwrap_or(&empty);
    let now = now_ms();
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn phases(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
            "SELECT coalesce((SELECT d.dirty_at > greatest(coalesce(d.ran_at, 0),
                                       (SELECT min(f.checked_at) FROM phase_state f WHERE f.project_id = $1))
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

pub(crate) async fn coverage(pool: &Pool, kinds: &crate::kinds::Kinds, project: &str) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn history(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    limit: i64,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT revision, bytes, content_hash, written_at, written_by, count(*) OVER ()
               FROM project_document_revisions
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
              ORDER BY revision DESC LIMIT $4",
            &[&project, &kind, &name, &limit],
        )
        .await?;
    let total: i64 = rows.first().map(|r| r.get(5)).unwrap_or(0);
    Ok(json!({
        "count": rows.len(),
        "total": total,
        "truncated": (rows.len() as i64) < total,
        "revisions": rows.iter().map(|r| json!({
            "revision": r.get::<_, i64>(0),
            "bytes": r.get::<_, i32>(1),
            "hash": r.get::<_, String>(2),
            "at": r.get::<_, i64>(3),
            "by": r.get::<_, String>(4),
        })).collect::<Vec<_>>(),
    }))
}

pub(crate) async fn confirm_entity(
    pool: &Pool,
    project: &str,
    kind: &str,
    id: &str,
    why: &str,
    by: &str,
) -> Result<Value, crate::db::Fail> {
    if why.trim().is_empty() {
        return Ok(json!({ "status": "no_why",
            "why": "подтверждение без довода — отметка, а не ревью: скажите, что перечитано и почему закрытие в силе" }));
    }
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.query(
        "SELECT 1 FROM entity_stamp WHERE project_id = $1 AND kind = $2 AND id = $3 FOR UPDATE",
        &[&project, &kind, &id],
    )
    .await?;
    let causes: Vec<String> = tx
        .query(
            "SELECT DISTINCT stale_link FROM entity_live
              WHERE project_id = $1 AND kind = $2 AND id = $3 AND live_state = 'reopened'",
            &[&project, &kind, &id],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    if causes.is_empty() {
        return Ok(json!({ "status": "not_reopened", "kind": kind, "id": id,
            "why": "запись не переоткрыта: подтверждать нечего" }));
    }
    let at = now_ms();
    tx.execute(
        "UPDATE entity_stamp SET confirmed_at = $4 WHERE project_id = $1 AND kind = $2 AND id = $3",
        &[&project, &kind, &id, &at],
    )
    .await?;
    tx.execute(
        "INSERT INTO entity_confirm (project_id, kind, id, at, by_whom, why, cause)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&project, &kind, &id, &at, &by, &why, &causes.join(", ")],
    )
    .await?;
    tx.commit().await?;
    Ok(json!({ "status": "confirmed", "kind": kind, "id": id, "causes": causes, "at": at }))
}

pub(crate) async fn declare_retired_term(
    pool: &Pool,
    project: &str,
    term: &str,
    retired_by: &str,
    declared_in: &str,
    drop_it: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let term = term.trim();
    if term.is_empty() {
        return Ok(json!({ "status": "nameless", "why": "снимается слово, а оно не названо" }));
    }
    if drop_it {
        let gone = client
            .execute("DELETE FROM term_retired WHERE project_id = $1 AND term = $2", &[&project, &term])
            .await?;
        return Ok(json!({ "status": if gone > 0 { "dropped" } else { "not_found" }, "term": term }));
    }
    if retired_by.trim().is_empty() {
        return Ok(json!({ "status": "no_source",
            "why": "не сказано, чем слово снято: снятие без решения или статьи неотличимо от вкуса" }));
    }
    client
        .execute(
            "INSERT INTO term_retired (project_id, term, retired_by, declared_in) VALUES ($1, $2, $3, $4)
             ON CONFLICT (project_id, term) DO UPDATE
                SET retired_by = EXCLUDED.retired_by, declared_in = EXCLUDED.declared_in",
            &[&project, &term, &retired_by, &declared_in],
        )
        .await?;
    Ok(json!({ "status": "declared", "term": term, "retiredBy": retired_by }))
}

/// Текст сущности, каким он был на названной правке.
pub(crate) async fn at_revision(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    revision: i64,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
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
pub(crate) async fn blocks(
    pool: &Pool,
    project: &str,
    entity_kind: &str,
    entity_name: &str,
    anchor: Option<&str>,
    own: bool,
) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;

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

/// Примерка: набор копируется, правка кладётся в копию, гейт меряется по ней.
///
/// Харнес силён в «правильно ли сейчас» и нем в «будет ли правильно, если я
/// сделаю так». Агент правит вслепую и цену узнаёт после — отсюда и реестр
/// вопросов, работающий переполнением: спросить дешевле, чем проверить.
///
/// Сравнивается не с «сейчас», а с самой копией ДО правки: обе стороны
/// пересчитаны на копии одним кругом одного кода.
pub(crate) async fn what_if(
    pool: &Pool,
    kinds: &std::sync::Arc<crate::kinds::Kinds>,
    project: &str,
    author: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, crate::db::Fail> {
    static NUMBER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let copy = format!(
        "примерка·{project}·{}·{}-{}",
        now_ms(),
        std::process::id(),
        NUMBER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let done = try_on(pool, kinds, project, &copy, author, tool, args).await;
    // КОПИЯ СНИМАЕТСЯ ВСЕГДА, и число снятого называется: молчаливая уборка,
    // которая не отработала, оставляет набор-призрак в каждой таблице сразу.
    let removed = forget(pool, &copy).await;
    let mut answer = done?;
    match removed {
        Ok(n) => answer["снято строк копии"] = json!(n),
        Err(e) => {
            tracing::warn!("примерка не убралась за собой: копия {copy}, {}", e.says());
            answer["копия осталась"] = json!(copy);
            answer["почему осталась"] = json!(e.says());
        }
    }
    Ok(answer)
}

async fn forget(pool: &Pool, copy: &str) -> Result<i64, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    Ok(client.query_one("SELECT project_forget($1)", &[&copy]).await?.get(0))
}

async fn try_on(
    pool: &Pool,
    kinds: &std::sync::Arc<crate::kinds::Kinds>,
    project: &str,
    copy: &str,
    author: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, crate::db::Fail> {
    let start = std::time::Instant::now();
    let row_count: i64 = {
        let client = crate::db::conn(pool).await?;
        client.query_one("SELECT project_copy($1, $2)", &[&project, &copy]).await?.get(0)
    };
    // ОТМЕТКА «НАДО ПЕРЕСЧИТАТЬ» КОПИИ НЕ НАСЛЕДУЕТСЯ.
    //
    // Она копируется вместе с набором, и сборщик — увидев её — берётся считать
    // копию наперегонки с примеркой, ради которой копия и заведена. Хуже того,
    // дописывает он её ПОСЛЕ снятия: остаётся набор-призрак, которого нет среди
    // проектов и который уже никто не уберёт. Так и вышло дважды, пока сборщик
    // не научили брать только настоящие наборы. Учить его было правильно; не
    // оставлять ему повода — дешевле, и одно другому не мешает.
    {
        let client = crate::db::conn(pool).await?;
        client.execute("DELETE FROM gate_dirty WHERE project_id = $1", &[&copy]).await?;
    }
    crate::watch::recount(pool, copy).await?;
    let before = snapshot_gate(pool, copy).await?;

    // Дверь зовётся по копии ТЕМ ЖЕ кодом: у примерки нет своей ветки, которая
    // могла бы разойтись с настоящей. Имя набора — единственное, что меняется.
    // Будущее кладётся в короб: `call` зовёт примерку, примерка зовёт `call`, и
    // без короба у состояния этой цепочки нет конечного размера.
    let door_answer = crate::mcp::Mcp {
        pool: pool.clone(),
        kinds: kinds.clone(),
        project: copy.to_owned(),
        author: author.to_owned(),
    };
    let answer_doors = Box::pin(door_answer.call(tool, args)).await;
    // ЗАНЯТОСТЬ — НЕ ПРИГОВОР КОПИИ. Отказ по существу на копии значит, что
    // и на подлиннике будет отказ; перегрузка не значит ничего, а примерка —
    // самый тяжёлый ход и первым упирается в потолок.
    if crate::door::busy_said(&answer_doors) {
        let said = answer_doors["content"][0]["text"].as_str().unwrap_or("").to_owned();
        forget(pool, copy).await.ok();
        return Err(crate::db::Fail::Busy(format!(
            "примерка не сделана: {said}. Копия убрана, набор не тронут"
        )));
    }
    let refusal = answer_doors.get("isError").and_then(Value::as_bool).unwrap_or(false);
    // Дверь отвечает подробно, и подробность её — о копии: счёт проекций копии
    // никому не нужен, а читать примерку мешает. Берётся слово исхода, а при
    // отказе — весь довод, потому что довод и есть ответ.
    let fully = answer_doors["content"][0]["text"].as_str().unwrap_or("").to_owned();
    let said = serde_json::from_str::<Value>(&fully)
        .ok()
        .and_then(|v| v["status"].as_str().map(str::to_owned))
        .unwrap_or_else(|| fully.clone());
    if refusal {
        return Ok(json!({
            "примерено": false, "дверь": tool, "отказ": fully,
            "почему": "дверь отказала на копии — на подлиннике отказала бы так же, \
                       и мерить нечего",
        }));
    }

    crate::watch::recount(pool, copy).await?;
    let after = snapshot_gate(pool, copy).await?;

    let mut now_red = Vec::new();
    let mut went_out = Vec::new();
    let mut moved = Vec::new();
    let mut changed = Vec::new();
    let mut not_with_than_compare = Vec::new();
    for ((phase, rule), (was, was_found, _)) in &before {
        let Some((became, now_found, than)) = after.get(&(phase.clone(), rule.clone())) else {
            continue;
        };
        let name_said = format!("{phase} · {rule}");
        // НЕСВЕЖЕЕ «ДО» НЕ СРАВНИВАЕТСЯ, А НАЗЫВАЕТСЯ. Пункт, у которого до
        // правки не было ответа, после неё покажется покрасневшим от неё — хотя
        // краснел он и без всякой правки. Одной оговорки внизу мало: находка
        // уже прочитана к тому месту, где она опровергается.
        if was == "unknown" || was == "stale" {
            not_with_than_compare.push(json!({ "пункт": name_said, "до правки": was,
                                           "после": became, "находок после": now_found }));
            continue;
        }
        match (was.as_str(), became.as_str()) {
            (_, "failed") if was != "failed" => {
                now_red.push(json!({ "пункт": name_said, "находок": now_found, "чем": than }))
            }
            ("failed", "passed") => went_out.push(json!({ "пункт": name_said })),
            ("failed", "failed") if was_found != now_found => moved.push(json!({
                "пункт": name_said, "было": was_found, "стало": now_found })),
            _ if was != became => changed.push(json!({ "пункт": name_said, "было": was, "стало": became })),
            _ => {}
        }
    }
    // НОВЫЙ ПУНКТ — ТОЖЕ ИСХОД. Правка, заводящая сущность нового вида, приносит
    // с ней и пункты, которых у набора не было: без этого они прошли бы молча.
    for ((phase, item), (became, found, than)) in &after {
        if became == "failed" && !before.contains_key(&(phase.clone(), item.clone())) {
            now_red.push(json!({ "пункт": format!("{phase} · {item}"),
                                       "находок": found, "чем": than, "пункт новый": true }));
        }
    }

    Ok(json!({
        "примерено": true,
        "дверь": tool,
        "сказала": said,
        "стало красным": now_red,
        "погасло": went_out,
        "сдвинулось": moved,
        "сменилось": changed,
        "цена": {
            "строк скопировано": row_count,
            "мс": start.elapsed().as_millis() as i64,
        },
        "не с чем сравнить": not_with_than_compare,
    }))
}

async fn snapshot_gate(
    pool: &Pool,
    project: &str,
) -> Result<std::collections::HashMap<(String, String), (String, String, String)>, crate::db::Fail>
{
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            // Имена находок приезжают вместе со счётом: «покраснел пункт, находок
            // три» не говорит, ЧТО чинить, а примерка затем и заводится, чтобы
            // сказать это до правки. Берутся первые три — перечень целиком
            // бывает в сотни строк и хоронит ответ.
            "SELECT phase, id, coalesce(result->>'computed', 'unknown'),
                    coalesce(result->>'violations', ''),
                    coalesce((SELECT string_agg(д, ' | ')
                                FROM (SELECT jsonb_array_elements_text(result->'detail') AS д
                                       LIMIT 3) x), '')
               FROM project_gates WHERE project_id = $1",
            &[&project],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| ((r.get(0), r.get(1)), (r.get(2), r.get(3), r.get(4))))
        .collect())
}

#[cfg(test)]
mod ladder {
    use super::{holding_step, decision_ladder};
    use serde_json::json;

    #[test]
    fn failed_corpus_and_unanswerable_not_those_not_release_red() {
        let asked_step = json!({
            "corpusPhaseOpen": true,
            "unanswerable": [{ "ord": 7, "touches": "repository" }, { "ord": 12, "touches": "corpus" }],
            "openWork": [
                { "ord": 6, "state": "failed", "kind": "gate", "touches": "corpus", "violations": 1,
                  "detail": ["G4 · все задачи кода закрыты"] },
                { "ord": 8, "state": "failed", "kind": "milestone", "touches": "repository", "violations": 1,
                  "detail": ["M3 — задач нет"] },
            ],
        });
        let (step, hold, total) = holding_step(&asked_step, 9, &[]).unwrap();
        assert_eq!((step["ord"].as_i64(), hold, total), (Some(8), vec!["M3 — задач нет".to_owned()], 1));
    }

    #[test]
    fn step_from_task_not_hold() {
        let asked_step = json!({
            "unanswerable": [],
            "openWork": [
                { "ord": 9, "state": "failed", "kind": "task", "touches": "repository", "violations": 3 },
                { "ord": 11, "state": "failed", "kind": "gate", "touches": "repository", "violations": 2 },
            ],
        });
        assert!(holding_step(&asked_step, 9, &["corpus · x".to_owned()]).is_none());
    }

    #[test]
    fn corpus_holds_gate_step() {
        let asked_step = json!({ "unanswerable": [], "openWork": [
            { "ord": 6, "state": "failed", "kind": "gate", "touches": "corpus", "violations": 4 },
        ]});
        let red = vec!["corpus · x".to_owned(), "corpus · y".to_owned()];
        let (step, hold, total) = holding_step(&asked_step, 9, &red).unwrap();
        assert_eq!((step["ord"].as_i64(), hold, total), (Some(6), red, 2));
    }

    #[test]
    fn repository_after_unanswerable_corpus_not_holds() {
        let asked_step = json!({
            "corpusPhaseOpen": true,
            "unanswerable": [{ "ord": 4, "touches": "corpus" }],
            "openWork": [
                { "ord": 8, "state": "failed", "kind": "milestone", "touches": "repository", "violations": 1 },
            ],
        });
        assert!(holding_step(&asked_step, 9, &[]).is_none());
    }

    #[test]
    fn corpus_after_unanswerable_corpus_holds() {
        let asked_step = json!({
            "unanswerable": [{ "ord": 4, "touches": "corpus" }],
            "openWork": [
                { "ord": 5, "state": "failed", "kind": "question", "touches": "corpus", "violations": 2,
                  "detail": ["Q-1 — открыт", "Q-2 — открыт"] },
            ],
        });
        let (step, hold, total) = holding_step(&asked_step, 9, &[]).unwrap();
        assert_eq!((step["ord"].as_i64(), hold.len(), total), (Some(5), 2, 2));
    }

    #[test]
    fn uncounted_position_holds_and_stale_decides_by_last() {
        let stale = json!({ "checkedAt": 1, "stale": true, "unanswerable": [], "openWork": [] });
        assert!(decision_ladder(&stale, 9, &[]).is_none());
        assert!(decision_ladder(&json!({ "checkedAt": 1 }), 9, &[]).is_some());
        assert!(decision_ladder(&json!({ "openWork": [] }), 9, &[]).is_some());
    }

    #[test]
    fn first_unit_only_at_current_step() {
        let asked_step = |at: i64| {
            json!({ "checkedAt": 1, "stale": false, "unanswerable": [],
                    "at": { "ord": at, "first": { "name": "Q-1" } },
                    "openWork": [{ "ord": 5, "state": "failed", "kind": "question", "touches": "corpus", "violations": 1,
                                   "detail": ["Q-1 — открыт"] }] })
        };
        let holds_rows = decision_ladder(&asked_step(5), 9, &[]).unwrap();
        assert_eq!(
            (holds_rows["first"]["name"].as_str(), holds_rows["holding"][0].as_str()),
            (Some("Q-1"), Some("Q-1 — открыт"))
        );
        assert!(decision_ladder(&asked_step(4), 9, &[]).unwrap()["first"].is_null());
    }
}

#[cfg(test)]
mod redo {
    use super::{clear_redone, closings_to_judge, closings_without_commit, declare_task, judge_closings,
                judging_closings, PHASE_VIEWS};
    use deadpool_postgres::Pool;

    const P: &str = "p";

    async fn db() -> Pool {
        let url = std::env::var("MH_TEST_DB_URL")
            .expect("MH_TEST_DB_URL: адрес пустой базы; тест заводит только временные таблицы своего соединения");
        let pool = crate::db::pool(&url, 1).expect("пул тестовой базы");
        let c = pool.get().await.expect("соединение с тестовой базой");
        let foreign: bool = c
            .query_one("SELECT to_regclass('public.project_documents') IS NOT NULL", &[])
            .await
            .unwrap()
            .get(0);
        assert!(!foreign, "MH_TEST_DB_URL ведёт в базу с набором: тесту нужна пустая");
        c.batch_execute(&format!(
            "SET search_path TO pg_temp;
             CREATE TEMP TABLE projects (id text PRIMARY KEY);
             CREATE TEMP TABLE phase (id text PRIMARY KEY, ord integer NOT NULL, title text NOT NULL,
                                      gate text NOT NULL DEFAULT '', task_kind text NOT NULL DEFAULT '');
             CREATE TEMP TABLE project_gates (project_id text NOT NULL, phase text NOT NULL, id text NOT NULL,
                                              result jsonb);
             CREATE TEMP TABLE gate_item (phase text NOT NULL, id text NOT NULL);
             CREATE TEMP TABLE project_plan_tasks (project_id text NOT NULL, id text NOT NULL, kind text NOT NULL,
                                                   state text NOT NULL DEFAULT 'not_started', closing_commit text);
             CREATE TEMP TABLE task_state (project_id text NOT NULL, task_id text NOT NULL, state text NOT NULL,
                                           closing_commit text NOT NULL DEFAULT '', seen_at bigint NOT NULL,
                                           closed_at bigint NOT NULL DEFAULT 0,
                                           PRIMARY KEY (project_id, task_id));
             CREATE TEMP TABLE task_closing_judged (project_id text NOT NULL, task_id text NOT NULL,
                                                    closing_commit text NOT NULL,
                                                    PRIMARY KEY (project_id, task_id, closing_commit));
             CREATE TEMP TABLE task_redo (project_id text NOT NULL, task_id text NOT NULL, noticed_at bigint NOT NULL,
                                          phase text NOT NULL DEFAULT '', gate text NOT NULL DEFAULT '',
                                          PRIMARY KEY (project_id, task_id));
             INSERT INTO projects VALUES ('{P}');
             INSERT INTO phase VALUES ('Ф2', 2, 'проект', 'G2', ''), ('Ф3', 3, 'тесты', 'G3', 'red');
             INSERT INTO gate_item VALUES ('G2', 'пункт');
             INSERT INTO project_gates VALUES ('{P}', 'G2', 'пункт', '{{\"computed\": \"passed\"}}');"
        ))
        .await
        .unwrap();
        c.batch_execute(PHASE_VIEWS).await.unwrap();
        drop(c);
        pool
    }

    async fn sql(pool: &Pool, query: &str) {
        pool.get().await.unwrap().batch_execute(query).await.unwrap();
    }

    async fn plan(pool: &Pool, task: &str, kind: &str) {
        sql(pool, &format!("INSERT INTO project_plan_tasks (project_id, id, kind) VALUES ('{P}', '{task}', '{kind}')")).await;
    }

    async fn close(pool: &Pool, task: &str, commit: &str, at: i64) {
        sql(pool, &format!(
            "INSERT INTO task_state (project_id, task_id, state, closing_commit, seen_at, closed_at)
             VALUES ('{P}', '{task}', 'closed', '{commit}', {at}, {at})
             ON CONFLICT (project_id, task_id) DO UPDATE
               SET state = 'closed', closing_commit = EXCLUDED.closing_commit, closed_at = EXCLUDED.closed_at"
        ))
        .await;
    }

    async fn rebuild(pool: &Pool) {
        sql(pool, &format!(
            "UPDATE project_plan_tasks t SET state = s.state, closing_commit = s.closing_commit
               FROM task_state s WHERE s.project_id = t.project_id AND s.task_id = t.id AND t.project_id = '{P}'"
        ))
        .await;
        clear_redone(&pool.get().await.unwrap(), P).await.unwrap();
    }

    async fn gate(pool: &Pool, passed: bool) {
        let computed = if passed { "passed" } else { "failed" };
        sql(pool, &format!(
            "UPDATE project_gates SET result = jsonb_build_object('computed', '{computed}')
              WHERE project_id = '{P}' AND phase = 'G2'"
        ))
        .await;
    }

    async fn cycle(pool: &Pool, passed: bool) {
        rebuild(pool).await;
        judging_closings(pool, P, || async {
            gate(pool, passed).await;
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn debt(pool: &Pool, task: &str) -> Option<i64> {
        pool.get()
            .await
            .unwrap()
            .query_opt("SELECT noticed_at FROM task_redo WHERE project_id = $1 AND task_id = $2", &[&P, &task])
            .await
            .unwrap()
            .map(|r| r.get(0))
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn closed_in_turn_stays_clear_when_the_gate_reddens_later() {
        let db = db().await;
        plan(&db, "X", "red").await;
        close(&db, "X", "c1", 100).await;
        cycle(&db, true).await;
        cycle(&db, false).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "X").await, None);
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_closing_is_judged_by_the_measurement_after_it() {
        let db = db().await;
        gate(&db, false).await;
        plan(&db, "A", "red").await;
        close(&db, "A", "a1", 100).await;
        cycle(&db, true).await;
        assert_eq!(debt(&db, "A").await, None, "гейт, красный до закрытия и зелёный после, долга не ставит");

        plan(&db, "B", "red").await;
        close(&db, "B", "b1", 100).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "B").await, Some(100), "гейт, красный после закрытия, ставит долг");
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_closing_seen_during_a_measurement_waits_for_the_next() {
        let db = db().await;
        plan(&db, "W", "red").await;
        rebuild(&db).await;
        judging_closings(&db, P, || async {
            close(&db, "W", "w1", 100).await;
            rebuild(&db).await;
            gate(&db, false).await;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(debt(&db, "W").await, None);
        cycle(&db, false).await;
        assert_eq!(debt(&db, "W").await, Some(100));
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_closing_replaced_during_a_measurement_waits_for_the_next() {
        let db = db().await;
        plan(&db, "Z", "red").await;
        close(&db, "Z", "z1", 100).await;
        rebuild(&db).await;
        judging_closings(&db, P, || async {
            close(&db, "Z", "z2", 200).await;
            gate(&db, false).await;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(debt(&db, "Z").await, None, "подача, пришедшая до пересборки, — уже не то закрытие");
        judging_closings(&db, P, || async {
            rebuild(&db).await;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(debt(&db, "Z").await, None, "план за время замера сменил закрытие");
        cycle(&db, false).await;
        assert_eq!(debt(&db, "Z").await, Some(200));

        plan(&db, "R", "red").await;
        close(&db, "R", "r1", 100).await;
        rebuild(&db).await;
        judging_closings(&db, P, || async {
            sql(&db, "UPDATE task_state SET state = 'claimed' WHERE task_id = 'R'").await;
            rebuild(&db).await;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(debt(&db, "R").await, None, "переоткрытая за время замера задача не судится");
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn two_measurements_judge_a_closing_once() {
        let db = db().await;
        plan(&db, "D", "red").await;
        close(&db, "D", "d1", 100).await;
        rebuild(&db).await;
        let first = closings_to_judge(&db.get().await.unwrap(), P).await.unwrap();
        let second = closings_to_judge(&db.get().await.unwrap(), P).await.unwrap();
        gate(&db, true).await;
        judge_closings(&db.get().await.unwrap(), P, &first).await.unwrap();
        gate(&db, false).await;
        judge_closings(&db.get().await.unwrap(), P, &second).await.unwrap();
        assert_eq!(debt(&db, "D").await, None);
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_task_missing_from_one_push_keeps_its_judgement() {
        let db = db().await;
        plan(&db, "M", "red").await;
        close(&db, "M", "m1", 100).await;
        cycle(&db, true).await;
        sql(&db, "DELETE FROM task_state WHERE task_id = 'M'").await;
        cycle(&db, false).await;
        close(&db, "M", "m1", 100).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "M").await, None);
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn debt_is_cleared_only_by_a_later_closing_at_an_open_phase() {
        let db = db().await;
        plan(&db, "Y", "red").await;
        close(&db, "Y", "c1", 100).await;
        cycle(&db, false).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "Y").await, Some(100));
        cycle(&db, true).await;
        assert_eq!(debt(&db, "Y").await, Some(100), "позеленевший гейт долга не гасит");
        gate(&db, false).await;
        close(&db, "Y", "c0", 50).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "Y").await, Some(100), "закрытие коммитом раньше замеченного долг не снимает");
        close(&db, "Y", "c2", 200).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "Y").await, Some(200), "закрыта заново при закрытой фазе — долг заново");
        close(&db, "Y", "c3", 300).await;
        cycle(&db, true).await;
        assert_eq!(debt(&db, "Y").await, None);
        cycle(&db, false).await;
        assert_eq!(debt(&db, "Y").await, None);
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn leaving_the_plan_does_not_launder_debt() {
        let db = db().await;
        plan(&db, "V", "red").await;
        close(&db, "V", "c1", 100).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "V").await, Some(100));
        sql(&db, "DELETE FROM project_plan_tasks WHERE id = 'V'").await;
        cycle(&db, true).await;
        plan(&db, "V", "red").await;
        cycle(&db, true).await;
        assert_eq!(debt(&db, "V").await, Some(100));
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn an_unmapped_kind_waits_for_its_phase() {
        let db = db().await;
        plan(&db, "U", "other").await;
        close(&db, "U", "u1", 100).await;
        cycle(&db, false).await;
        let judged: i64 = db
            .get()
            .await
            .unwrap()
            .query_one("SELECT count(*) FROM task_closing_judged WHERE task_id = 'U'", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(judged, 0, "вид без фазы не судится");
        sql(&db, "INSERT INTO phase VALUES ('Ф4', 4, 'прочее', 'G3', 'other')").await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "U").await, Some(100));
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_plan_row_without_a_commit_does_not_break_the_measurement() {
        let db = db().await;
        sql(&db, &format!("INSERT INTO project_plan_tasks (project_id, id, kind, state) VALUES ('{P}', 'N', 'red', 'closed')")).await;
        cycle(&db, false).await;
        assert_eq!(debt(&db, "N").await, None, "закрытие без истории не опознать, и судом оно не считается");
    }

    #[test]
    fn a_push_closing_without_a_commit_is_named() {
        let states = vec![
            ("A".to_owned(), "closed".to_owned(), "abc".to_owned(), 1),
            ("B".to_owned(), "closed".to_owned(), " ".to_owned(), 1),
            ("C".to_owned(), "claimed".to_owned(), "".to_owned(), 1),
        ];
        assert_eq!(closings_without_commit(&states), vec!["B".to_owned()]);
    }

    #[tokio::test]
    async fn a_task_is_not_declared_closed_or_claimed() {
        let pool = crate::db::pool("postgres://nobody@127.0.0.1:1/nothing", 1).unwrap();
        for state in ["closed", "claimed"] {
            let fields = super::Task { id: "T", milestone: "M1", ord: 1, title: "заголовок",
                                kind: "dev", state, size: "" };
            let out = declare_task(&pool, P, fields, false).await.unwrap();
            assert_eq!(out["status"], "state_from_history", "{state}: до базы дело не доходит");
        }
    }
}

#[cfg(test)]
mod work_run_token {
    use super::{step_run_hint, DDL, WORK_RUN_NAME};

    /// Метка подстановки живёт В ЗАПИСЯХ набора, а не только в коде. Пока её
    /// меняли правкой буквы в исходнике, все объявленные ступени переставали
    /// подставлять имя — молча, и `next-step` отдавал команду с меткой внутри.
    #[test]
    fn the_token_moves_together_with_the_rows_that_hold_it() {
        assert_eq!(WORK_RUN_NAME, "{name}");
        assert!(
            DDL.contains("replace(work_run, '{имя}', '{name}')"),
            "перенос записанных ступеней на новую метку идёт схемой"
        );
        assert!(step_run_hint().contains(WORK_RUN_NAME), "дверь объясняет ту же метку, что подставляет");
        let command = "mh call question id={name}";
        assert_eq!(command.replace(WORK_RUN_NAME, "Q-1"), "mh call question id=Q-1");
        let from_old_memory = "mh call question id={имя}".replace("{имя}", WORK_RUN_NAME);
        assert_eq!(from_old_memory.replace(WORK_RUN_NAME, "Q-1"), "mh call question id=Q-1");
    }
}

#[cfg(test)]
mod kind_tables {
    use super::own_tables;

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn only_holders_keyed_by_id_are_the_kinds_own_tables() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let pool = crate::db::pool(&url, 1).expect("пул тестовой базы");
        let mut c = pool.get().await.expect("соединение с тестовой базой");
        let foreign: bool = c
            .query_one("SELECT to_regclass('public.kind_layout') IS NOT NULL", &[])
            .await
            .unwrap()
            .get(0);
        assert!(!foreign, "MH_TEST_DB_URL ведёт в базу с раскладкой видов: тесту нужна пустая");
        let tx = c.transaction().await.unwrap();
        tx.batch_execute(
            "CREATE TABLE public.kind_layout (name text PRIMARY KEY, spec jsonb NOT NULL);
             CREATE TABLE public.holder_links (project_id text, entity_kind text, target text);
             CREATE TABLE public.holder_rows (project_id text, id text);
             CREATE TABLE public.holder_more (project_id text, id text);
             CREATE TABLE public.holder_bare (id text);
             INSERT INTO public.kind_layout VALUES
               ('index', '{\"holds\": [\"holder_links\", \"holder_more\", \"holder_bare\", \"holder_rows\", \"no_such_table\"]}'),
               ('mockup', '{\"holds\": []}'),
               ('bare', '{}');",
        )
        .await
        .unwrap();
        assert_eq!(own_tables(&tx, "index").await.unwrap(), vec!["holder_more".to_owned(), "holder_rows".to_owned()]);
        assert!(own_tables(&tx, "mockup").await.unwrap().is_empty());
        assert!(own_tables(&tx, "bare").await.unwrap().is_empty());
        assert!(own_tables(&tx, "unknown").await.unwrap().is_empty());
        tx.rollback().await.unwrap();
    }
}

#[cfg(test)]
mod rename {
    use super::{name_in_text, renaming, RENAME_IN_COLUMNS};

    async fn empty_base() -> deadpool_postgres::Object {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let pool = crate::db::pool(&url, 1).expect("пул тестовой базы");
        let c = pool.get().await.expect("соединение с тестовой базой");
        let foreign: bool = c
            .query_one(
                "SELECT to_regclass('public.project_documents') IS NOT NULL OR to_regclass('public.kind_layout') IS NOT NULL",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!foreign, "MH_TEST_DB_URL ведёт в базу с набором: тесту нужна пустая");
        c
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_name_is_renamed_where_it_stands_whole() {
        let mut c = empty_base().await;
        let tx = c.transaction().await.unwrap();
        tx.batch_execute(
            "CREATE TABLE public.project_notes (project_id text, name text, note text);
             INSERT INTO public.project_notes VALUES
               ('p', '20-surface', 'см. 20-surface. и ../20-surface, 20-surface/ и `20-surface`'),
               ('p', '20-surface/configure', 'ссылка на index:20-surface/configure'),
               ('p', 'docs/20-surface.md', 'x-20-surface-y 20-surface.md v1.20-surface'),
               ('p', 'a.b', 'axb'),
               ('p', 'OQ-01', 'OQ-01/OQ-02, FR-01...OQ-01, v1.OQ-01, OQ-010, OQ-01-й, вопросOQ-01 (OQ-01)'),
               ('q', '20-surface', 'чужой набор: 20-surface');",
        )
        .await
        .unwrap();
        tx.batch_execute(RENAME_IN_COLUMNS).await.unwrap();
        for (name, slash, to) in [("20-surface", true, "surface"), ("a.b", true, "ab"), ("OQ-01", false, "Q-01")] {
            tx.query("SELECT * FROM rename_matching_in_columns('p', $1, $2, false)", &[&name_in_text(name, slash), &to])
                .await
                .unwrap();
        }
        let got: Vec<(String, String, String)> = tx
            .query("SELECT project_id, name, note FROM public.project_notes ORDER BY project_id, name COLLATE \"C\"", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
        let row = |p: &str, n: &str, note: &str| (p.to_owned(), n.to_owned(), note.to_owned());
        assert_eq!(got, vec![
            row("p", "20-surface/configure", "ссылка на index:20-surface/configure"),
            row("p", "Q-01", "Q-01/OQ-02, FR-01...Q-01, v1.OQ-01, OQ-010, OQ-01-й, вопросOQ-01 (Q-01)"),
            row("p", "ab", "axb"),
            row("p", "docs/20-surface.md", "x-20-surface-y 20-surface.md v1.20-surface"),
            row("p", "surface", "см. surface. и ../20-surface, surface/ и `surface`"),
            row("q", "20-surface", "чужой набор: 20-surface"),
        ]);
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_previous_server_still_starts_over_the_new_function() {
        let mut c = empty_base().await;
        let tx = c.transaction().await.unwrap();
        let previous = "CREATE OR REPLACE FUNCTION rename_in_columns(п text, было text, стало text, слить boolean DEFAULT false)
             RETURNS TABLE(таблица text, колонка text, строк bigint, снято bigint) AS $$ BEGIN RETURN; END $$
             LANGUAGE plpgsql;";
        tx.batch_execute(previous).await.unwrap();
        tx.batch_execute(RENAME_IN_COLUMNS).await.unwrap();
        tx.batch_execute(RENAME_IN_COLUMNS).await.unwrap();
        tx.batch_execute(previous).await.unwrap();
        tx.batch_execute(RENAME_IN_COLUMNS).await.unwrap();
        let functions: Vec<String> = tx
            .query("SELECT proname::text FROM pg_proc WHERE proname LIKE 'rename%in_columns' ORDER BY 1", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect();
        assert_eq!(functions, vec!["rename_matching_in_columns".to_owned()]);
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn renaming_moves_the_name_and_nothing_that_is_not_a_name() {
        let mut c = empty_base().await;
        let tx = c.transaction().await.unwrap();
        tx.batch_execute(
            r#"CREATE TABLE public.kind_layout (name text PRIMARY KEY, spec jsonb NOT NULL);
             CREATE TABLE public.project_documents (project_id text, entity_kind text, entity_name text,
               content text, revision int, updated_at bigint, UNIQUE (project_id, entity_kind, entity_name));
             CREATE TABLE public.project_document_revisions (project_id text, entity_kind text, entity_name text,
               content text, content_hash text, bytes int, revision int, written_at bigint, written_by text,
               UNIQUE (project_id, entity_kind, entity_name, revision));
             CREATE TABLE public.project_questions (project_id text, id text, state text);
             INSERT INTO public.kind_layout VALUES
               ('index', '{"id": "^[A-Za-z0-9][A-Za-z0-9/._-]*$"}'),
               ('question', '{"id": "^Q-\\d+$", "holds": ["project_questions"]}'),
               ('term', '{"id": "^.+$"}');
             INSERT INTO public.project_documents VALUES
               ('p', 'index', '20-surface', 'экраны', 1, 0),
               ('p', 'index', '20-surface/configure', 'выше: [экраны](index:20-surface)', 1, 0),
               ('p', 'index', 'p', 'имя как у набора', 1, 0),
               ('p', 'term', 'словарь', 'T-1 и OQ-01/OQ-02', 1, 0);
             INSERT INTO public.project_document_revisions VALUES
               ('p', 'index', '20-surface', 'экраны', 'h', 6, 1, 0, 'x'),
               ('p', 'index', 'gone', 'было', 'h', 4, 1, 0, 'x');
             INSERT INTO public.kind_layout VALUES
               ('run', '{"id": "^([MV][0-9]+(-T[0-9a-z]+)?|v[0-9]+|v[0-9]+/[A-Za-z0-9-]+(/[A-Za-z0-9-]+)?)$"}');
             INSERT INTO public.project_documents VALUES
               ('p', 'run', 'v1/M0', 'этап', 1, 0),
               ('p', 'run', 'v1/M0/M0-T1', 'задача этапа v1/M0', 1, 0);
             INSERT INTO public.project_questions VALUES ('p', 'OQ-01', 'open'), ('p', 'OQ-02', 'open');"#,
        )
        .await
        .unwrap();
        tx.batch_execute(RENAME_IN_COLUMNS).await.unwrap();

        let refused = renaming(&tx, "p", "index", "index", "catalog", false).await.unwrap();
        assert_eq!(refused["status"], "kind_word");

        let taken = renaming(&tx, "p", "index", "20-surface/configure", "20-surface", false).await.unwrap();
        assert_eq!(taken["status"], "taken", "{taken}");
        assert!(taken["why"].as_str().unwrap().contains("целиком"), "{taken}");
        let history = renaming(&tx, "p", "index", "20-surface", "gone", false).await.unwrap();
        assert_eq!(history["status"], "history_taken", "{history}");
        let run = renaming(&tx, "p", "run", "v1/M0", "v2/M0", false).await.unwrap();
        assert_eq!(run["status"], "renamed", "{run}");
        let moved = renaming(&tx, "p", "index", "20-surface", "20-screens", false).await.unwrap();
        assert_eq!(moved["status"], "renamed", "{moved}");
        assert_eq!(moved["revisions"], 1, "{moved}");
        let project = renaming(&tx, "p", "index", "p", "p2", false).await.unwrap();
        assert_eq!(project["status"], "renamed", "{project}");
        let question = renaming(&tx, "p", "question", "OQ-01", "Q-01", false).await.unwrap();
        assert_eq!(question["records"], 1, "{question}");
        let term = renaming(&tx, "p", "term", "T-1", r"T-\1\&", false).await.unwrap();
        assert_eq!(term["status"], "renamed", "{term}");

        let documents: Vec<(String, String, String, String)> = tx
            .query("SELECT project_id, entity_kind, entity_name, content FROM public.project_documents
                     ORDER BY entity_kind, entity_name COLLATE \"C\"", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect();
        let doc = |k: &str, n: &str, t: &str| ("p".to_owned(), k.to_owned(), n.to_owned(), t.to_owned());
        assert_eq!(documents, vec![
            doc("index", "20-screens", "экраны"),
            doc("index", "20-surface/configure", "выше: [экраны](index:20-screens)"),
            doc("index", "p2", "имя как у набора"),
            doc("run", "v1/M0/M0-T1", "задача этапа v2/M0"),
            doc("run", "v2/M0", "этап"),
            doc("term", "словарь", r"T-\1\& и Q-01/OQ-02"),
        ]);
        let questions: Vec<(String, String, String)> = tx
            .query("SELECT project_id, id, state FROM public.project_questions ORDER BY 2", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
        let q = |id: &str| ("p".to_owned(), id.to_owned(), "open".to_owned());
        assert_eq!(questions, vec![q("OQ-02"), q("Q-01")]);
        let history: Vec<(String, String, i32)> = tx
            .query("SELECT entity_kind, entity_name, revision FROM public.project_document_revisions
                     ORDER BY entity_kind, entity_name COLLATE \"C\", revision", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
        let rev = |k: &str, n: &str, r: i32| (k.to_owned(), n.to_owned(), r);
        assert_eq!(history, vec![
            rev("index", "20-screens", 1),
            rev("index", "20-surface/configure", 2),
            rev("index", "gone", 1),
            rev("run", "v1/M0/M0-T1", 2),
            rev("term", "словарь", 2),
            rev("term", "словарь", 3),
        ]);
        tx.rollback().await.unwrap();
    }
}
