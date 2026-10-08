//! План: версии, этапы, задачи и зависимости между ними.
//!
//! Состояние задачи здесь берётся из ПОЛЯ документа — так делает донор, и порт
//! обязан совпасть с ним до строки. Что поле это второй источник истины и что
//! состояние правильнее выводить из закрывающего трейлера — правда, но правда
//! отдельной работы: сначала перенос без изменения смысла, потом смена смысла.
//! Красные задачи (`kind='red'`) кладёт сюда сборка сервера, не эта проекция.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Этап задачи — то, что стоит в её имени до `-T`: у `M0-T1` это `M0`.
///
/// Прежде этап читался из каталога (`50-plan/v1/M0/M0-T1.md`). Каталог говорил
/// то же самое, что имя задачи, — только окольно.
static TASK_MILESTONE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^([MV](?:\d+|[A-Z]))-T").expect("образец этапа в имени задачи"));
/// Имя задачи в поле «Зависит от» — с кавычками кода и без них.
///
/// Требование кавычек стоило 57 зависимостей из 338: половина набора пишет
/// `M1-T2`, половина — просто M1-T2, и вторая половина читалась как пустота.
/// Широта образца здесь безопасна: имя, которого нет задачей, отбрасывается
/// разрешением по таблице, а не образцом.
static IDENTIFIER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([A-Za-z]+\d*-[A-Za-z0-9-]+)").expect("образец идентификатора"));
static COMMIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b([0-9a-f]{7,40})\b").expect("образец коммита"));
static TEST_WORDS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*(тест|тесты|проверк|test)").expect("образец вида"));

/// Состояние и коммит из поля. Словарь один на план и на доску состояний.
pub(crate) fn state_of(field: &str) -> (&'static str, Option<String>) {
    let head = field.split('·').next().unwrap_or("").trim().to_lowercase();
    let commit = COMMIT.captures(field).map(|m| m[1].to_owned());
    if head.starts_with("закрыт") || head.starts_with("closed") {
        return ("closed", commit);
    }
    if head.starts_with("в работе") || head.starts_with("claimed") {
        return ("claimed", None);
    }
    ("not_started", None)
}

/// Поле «Вид» сильнее: им переопределяется то, что говорит идентификатор.
fn kind_of(field: &str, task: &str) -> &'static str {
    if !field.trim().is_empty() {
        return if TEST_WORDS.is_match(field) { "test" } else { "dev" };
    }
    // Трек проверок живёт в своём пространстве имён: `V3-T17` — тесты к `M3-T17`.
    if task.starts_with('V') && task.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
        "test"
    } else {
        "dev"
    }
}

/// Поля документов набора: имя → значение. Последнее объявление сильнее.
///
/// Сырое или очищенное — не вкусовщина: план читает `value_raw`, потому что имена
/// зависимостей стоят в обратных кавычках и образец их требует, а поверхность
/// читает `value`, потому что персона и фаза — это текст, а не разметка. Донор
/// делает ровно так, и порт обязан различать их так же.
pub(crate) async fn fields(
    pool: &Pool,
    project: &str,
    raw: bool,
) -> Result<HashMap<String, HashMap<String, String>>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let sql = if raw {
        "SELECT entity_kind || ' ' || entity_name, name, value_raw FROM project_document_fields
          WHERE project_id = $1 ORDER BY entity_kind, entity_name, section_ord, ord"
    } else {
        "SELECT entity_kind || ' ' || entity_name, name, value FROM project_document_fields
          WHERE project_id = $1 ORDER BY entity_kind, entity_name, section_ord, ord"
    };
    let rows = client.query(sql, &[&project]).await?;
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    for r in &rows {
        out.entry(r.get(0)).or_default().insert(r.get(1), r.get(2));
    }
    Ok(out)
}

/// Строка задачи плана: имя, этап, порядок, заголовок, вид, состояние.
type PlanTaskRow = (String, String, i32, String, String, String, String, &'static str, &'static str, Option<String>);


/// Заголовок документа — первый его раздел; если разделов нет, донор берёт путь.
pub(crate) async fn titles(pool: &Pool, project: &str) -> Result<HashMap<String, String>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT DISTINCT ON (entity_kind, entity_name) entity_kind || ' ' || entity_name, title
               FROM project_document_sections
              WHERE project_id = $1 ORDER BY entity_kind, entity_name, ord",
            &[&project],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<(usize, usize, usize, usize), crate::db::Fail> {
    let named = super::runs::named_of_kinds(pool, project, &["version", "milestone", "task"]).await?;
    // Блок «Требования» этапа — свёрнутый `<details>`. Читается он целиком и
    // построчно, мимо зачёркнутых имён: такое имя названо, чтобы сказать,
    // что его больше нет.
    let terms = crate::scheme::Terms::load(pool, project).await?;
    let milestone_requirements: Vec<(String, String)> = {
        let client = crate::db::conn(pool).await?;
        let rows = client
            .query(
                "SELECT entity_name, content FROM project_documents
                  WHERE project_id = $1 AND entity_kind = 'milestone'",
                &[&project],
            )
            .await?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in &rows {
            let milestone: String = r.get(0);
            let content: String = r.get(1);
            let Some(marker) = terms.one("marker.milestone-requirements") else { continue };
            let Some(block) = content.split(marker).nth(1) else { continue };
            let block = block.split("</details>").next().unwrap_or("");
            for line in block.lines() {
                for name in super::ids::said(line) {
                    if !name.starts_with("FR-") {
                        continue;
                    }
                    if seen.insert(format!("{milestone} {name}")) {
                        out.push((milestone.clone(), name));
                    }
                }
            }
        }
        out
    };

    // ПРОЧИЕ ПЕРЕЧНИ ЭТАПА — проверки, истории, экраны. Читаются теми же
    // объявленными маркерами, что и требования, и ложатся в одну таблицу с
    // колонкой рода. Отбора по приставке имени здесь НЕТ: что за сущность,
    // говорит маркер блока, а не форма имени — иначе пришлось бы зашить
    // `TC-`, `US-`, `SCR-` рядом с уже зашитым `FR-`.
    let milestone_links: Vec<(String, String, String)> = {
        let client = crate::db::conn(pool).await?;
        let rows = client
            .query(
                "SELECT entity_name, content FROM project_documents
                  WHERE project_id = $1 AND entity_kind = 'milestone'",
                &[&project],
            )
            .await?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (role, row_kind) in [
            ("marker.milestone-checks", "check"),
            ("marker.milestone-stories", "story"),
            ("marker.milestone-screens", "screen"),
        ] {
            let Some(marker) = terms.one(role) else { continue };
            for r in &rows {
                let milestone: String = r.get(0);
                let content: String = r.get(1);
                let Some(block) = content.split(marker).nth(1) else { continue };
                let block = block.split("</details>").next().unwrap_or("");
                for line in block.lines() {
                    for name in super::ids::said(line) {
                        if seen.insert(format!("{milestone} {row_kind} {name}")) {
                            out.push((milestone.clone(), row_kind.to_owned(), name));
                        }
                    }
                }
            }
        }
        out
    };
    // Какому выпуску принадлежит этап, говорит сам выпуск: его документ
    // перечисляет этапы таблицей, первой колонкой. Прежде это читалось из
    // каталога, в котором лежал файл.
    //
    // Этап, названный двумя выпусками, достаётся ЗАВЕДЁННОМУ ПОЗЖЕ (по первой
    // правке имени, равные — по имени): так стоит переименование, новый
    // документ пишется рядом со старым. Без порядка победителя решал порядок
    // строк запроса. Отказ здесь не годится: tot-ade сейчас ровно в таком
    // состоянии (0.0.9 и v1 перечисляют все 15 этапов), и отказ остановил бы
    // его пересборку до снятия v1.
    let milestone_version: HashMap<String, String> = {
        let client = crate::db::conn(pool).await?;
        client
            .query(
                "SELECT c.value, c.entity_name FROM project_document_cells c
                  WHERE c.project_id = $1 AND c.entity_kind = 'version'
                    AND c.col = 0 AND c.row_ord > 0 AND c.value ~ '^[MV](?:[0-9]+|[A-Z])$'
                  ORDER BY (SELECT min(r.written_at) FROM project_document_revisions r
                             WHERE r.project_id = c.project_id AND r.entity_kind = 'version'
                               AND r.entity_name = c.entity_name) NULLS FIRST, c.entity_name",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect()
    };
    let fields = fields(pool, project, true).await?;
    // Этапы, которые план знает: из документов и объявленные ручкой. Веха задачи
    // ищется среди НИХ, а не своим образцом имени: образец — копия раскладки,
    // а поле пишут прозой («**M9** — выпуск 0.1.2, редактор»), и десять таких
    // задач tot-ade держали пересборку всего плана (2026-10-07).
    let (known_milestones, held_version): (HashSet<String>, HashMap<String, String>) = {
        let client = crate::db::conn(pool).await?;
        let declared = client
            .query("SELECT id FROM project_plan_milestones WHERE project_id = $1 AND origin = 'declared'",
                   &[&project])
            .await?;
        // Этап, получивший документ после объявления, остаётся в своём выпуске,
        // пока таблица выпуска его не назовёт: иначе выпуск выходил пустым, и
        // пересборка падала о внешний ключ (tot-ade, документ M9, 2026-10-07).
        let held = client
            .query("SELECT id, version_id FROM project_plan_milestones WHERE project_id = $1", &[&project])
            .await?;
        (
            named.iter().filter(|e| e.0 == "milestone").map(|e| e.1.clone())
                .chain(declared.iter().map(|r| r.get::<_, String>(0)))
                .collect(),
            held.iter().map(|r| (r.get(0), r.get(1))).collect(),
        )
    };
    let titles = titles(pool, project).await?;
    let empty = HashMap::new();

    let mut versions: Vec<(String, String, String)> = Vec::new();
    let mut milestones: Vec<(String, String, i32, String, String, String)> = Vec::new();
    let mut tasks: Vec<PlanTaskRow> = Vec::new();
    let mut deps: Vec<(String, String)> = Vec::new();

    for e in &named {
        let key = format!("{} {}", e.0, e.1);
        if e.0 == "version" {
            versions.push((e.1.clone(), e.0.clone(), e.1.clone()));
            continue;
        }
        let title = titles.get(&key).cloned().unwrap_or_else(|| e.1.clone());
        if e.0 == "milestone" {
            let version = milestone_version.get(&e.1).or_else(|| held_version.get(&e.1)).cloned().unwrap_or_default();
            milestones.push((e.1.clone(), version, milestones.len() as i32, title,
                             e.0.clone(), e.1.clone()));
            continue;
        }
        if e.0 != "task" {
            continue;
        }
        let id = e.1.clone();
        let f = fields.get(&key).unwrap_or(&empty);
        let got = |name: &str| f.get(name).map(String::as_str).unwrap_or("");
        // Состояние задачи в план НЕ КЛАДЁТСЯ из документа.
        //
        // Отметка «сделано» — заявление; трейлер на приземлившемся коммите —
        // свидетельство. Набор говорит это дважды, и цена измерена: 83 вопроса
        // закрыли основанием «держатель написан», тридцать пять пришлось
        // возвращать. Здесь задача рождается незапущенной, а состояние кладёт
        // поверх подача датчика — из закрывающих трейлеров. Что сказал при этом
        // документ, спрашивается отдельно: `state-disagreements`.
        let _ = state_of(got("Состояние"));
        let (state, commit): (&str, Option<String>) = ("not_started", None);
        // Веху задача ОБЪЯВЛЯЕТ полем «Веха», и это ответ. Прежде она
        // выводилась из имени документа, а имя у задач набора строчное
        // (`m1-t1`) при вехе `M1`: образец не совпадал, веха выходила пустой и
        // вся пересборка падала о внешний ключ. Имя остаётся запасным ответом —
        // но регистр в нём больше не решает.
        // Поле без имени этапа, но не пустое, остаётся как написано и падает
        // проверкой неизвестной вехи ниже: запасной ответ по имени задачи
        // молча увёл бы её в другой этап, чем назвал автор.
        let field = got("Веха");
        let named_in_field = field
            .split(|c: char| !c.is_alphanumeric() && c != '-')
            .find(|w| known_milestones.contains(*w));
        let milestone = match named_in_field {
            Some(m) => m.to_owned(),
            None if field.trim_matches(|c: char| c.is_whitespace() || "*—-".contains(c)).is_empty() => TASK_MILESTONE
                .captures(&id)
                .map(|m| m[1].to_uppercase())
                .unwrap_or_default(),
            None => field.trim().to_owned(),
        };
        tasks.push((
            id.clone(),
            milestone,
            tasks.len() as i32,
            title,
            e.0.clone(),
            e.1.clone(),
            got("Размер").to_owned(),
            kind_of(got("Вид"), &id),
            state,
            commit,
        ));
        for d in IDENTIFIER.captures_iter(got("Зависит от")) {
            deps.push((id.clone(), d[1].to_owned()));
        }
    }

    // Имя зависимости приводится к имени задачи, а не сравнивается с ним
    // буквально: набор пишет `M1-T1`, а задача зовётся `m1-t1`, и все 338
    // объявленных зависимостей отбрасывались как «ссылка в никуда» — план
    // выходил одним валом из семидесяти семи задач.
    let known: HashMap<String, String> =
        tasks.iter().map(|t| (t.0.to_lowercase(), t.0.clone())).collect();
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    // Версия БЫЛА вершиной каскада, и удаление её уносило этапы, задачи и
    // зависимости — включая объявленные ручкой, которых ни один документ не
    // называет. Так пропадали 77 задач трека проверок и 338 зависимостей:
    // пересборка стирала их и не выводила обратно, потому что вывести их не из
    // чего. Теперь стирается только выведенное, а совпавшее — переписывается.
    // Удаляется только то, что документы ПЕРЕСТАЛИ называть, и это не
    // придирка. Версия — вершина каскада: снос её строки уносит этапы, задачи и
    // зависимости независимо от их происхождения. Удаление «всего выведенного»
    // с последующей вставкой того же самого стоило 77 объявленных красных задач
    // и 338 зависимостей — каскад отработал в зазоре между DELETE и INSERT.
    let version_ids: Vec<String> = versions.iter().map(|v| v.0.clone()).collect();
    let milestone_ids: Vec<String> = milestones.iter().map(|m| m.0.clone()).collect();
    // ПАДЕНИЕ НАЗЫВАЕТ ВИНОВНОГО. Задача с вехой, которой в плане нет, роняет
    // вставку о внешний ключ, и сообщение Postgres говорит только имя
    // ограничения: «violates foreign key constraint … _milestone_id_fkey».
    // Набор при этом судит по ПРЕЖНИМ проекциям и показывает двадцать красных
    // пунктов вместо четырёх — то есть выглядит как двадцать находок, а не как
    // одна сломанная пересборка.
    //
    // Цена измерена: час работы набора ушёл на поиск задачи, которую здесь
    // можно назвать одной строкой. Виновником оказался документ подсадки
    // самотеста, утёкший из транзакции: веха выводилась из имени `M9-T9990`,
    // такой вехи нет.
    // Этап, объявленный ручкой (`milestone-add`), документа не имеет и в
    // перечне выше не стоит, но в плане он есть: задача из документа с такой
    // вехой роняла пересборку, хотя внешний ключ её принял бы (tot-ade, M9).
    let declared_milestones: Vec<String> = tx
        .query("SELECT id FROM project_plan_milestones WHERE project_id = $1 AND origin = 'declared'",
               &[&project])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let unknown: Vec<String> = tasks
        .iter()
        .filter(|t| !milestone_ids.contains(&t.1) && !declared_milestones.contains(&t.1))
        .map(|t| format!("{} → веха «{}»", t.0, t.1))
        .collect();
    if !unknown.is_empty() {
        return Err(crate::db::Fail::Corpus(format!(
            "пересборка плана не пройдёт: {} задач(и) называют веху, которой план не знает — {}.              План вех строится из документов вех; либо веха не заведена, либо поле «Веха» задачи              называет не то. Проекции остались прежними, и гейт считает по ним",
            unknown.len(),
            unknown.join(" · ")
        )));
    }
    let task_ids: Vec<String> = tasks.iter().map(|t| t.0.clone()).collect();
    for (id, kind, name) in &versions {
        tx.execute(
            "INSERT INTO project_plan_versions(project_id, id, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (project_id, id) DO UPDATE SET entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, origin = 'projected'",
            &[&project, id, kind, name],
        ).await?;
    }
    // Листья дерева задач: разбор блока живёт в своём модуле, а сюда приходит
    // готовым перечнем.
    //
    // ЧИТАЕТСЯ ТОЙ ЖЕ ТРАНЗАКЦИЕЙ, А НЕ ВТОРЫМ СОЕДИНЕНИЕМ. Здесь стоял
    // `pool.get()` при живом внешнем соединении с открытой транзакцией — и это
    // единственное такое место во всей пересборке. В пуле восемь слотов, а
    // `deadpool` собран без срока ожидания: восемь пересборок разом — а дверь
    // `put` без `deferProjection` зовёт пересборку на КАЖДУЮ запись — заняли бы
    // все восемь внешними соединениями и встали бы навсегда, ожидая девятого.
    // Не «медленно», а молча и насмерть.
    //
    // На ответ это не влияет: `project_plan_tasks` эта транзакция трогает
    // ниже, так что читается ровно то же зафиксированное состояние.
    // Требования этапа: раскрытый перечень блока «Требования». Требование
    // принадлежит ровно ОДНОМУ этапу — без этой связи этапы становятся
    // тематическими заголовками, а не планом: новое требование не попадает
    // ни в один и молчит.
    tx.execute("DELETE FROM project_milestone_links WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    for (milestone, row_kind, target) in &milestone_links {
        tx.execute(
            "INSERT INTO project_milestone_links(project_id, milestone_id, kind, target, origin)
             VALUES ($1,$2,$3,$4,'projected') ON CONFLICT DO NOTHING",
            &[&project, milestone, row_kind, target],
        )
        .await?;
    }
    tx.execute("DELETE FROM project_milestone_requirements WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    for (milestone, requirement) in &milestone_requirements {
        tx.execute(
            "INSERT INTO project_milestone_requirements(project_id, milestone_id, requirement_id, origin)
             VALUES ($1,$2,$3,'projected') ON CONFLICT DO NOTHING",
            &[&project, milestone, requirement],
        )
        .await?;
    }
    tx.execute("DELETE FROM project_plan_milestones
                 WHERE project_id = $1 AND origin = 'projected' AND id <> ALL($2)",
               &[&project, &milestone_ids]).await?;
    for (id, version, ord, title, kind, name) in &milestones {
        tx.execute(
            "INSERT INTO project_plan_milestones(project_id, id, version_id, ord, title,
                                                 entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (project_id, id) DO UPDATE SET version_id = EXCLUDED.version_id,
               ord = EXCLUDED.ord, title = EXCLUDED.title, entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, origin = 'projected'",
            &[&project, id, version, ord, title, kind, name],
        ).await?;
    }
    // Ушедший выпуск снимается ПОСЛЕ того, как вехи переехали к новому. Снятый
    // раньше, он каскадом уносил свои вехи, их задачи и рёбра в этой же
    // транзакции: спроецированное возвращалось вставкой ниже, а объявленное —
    // задачи и рёбра, повешенные дверями под эти вехи, — пропадало насовсем
    // (tot-ade при переименовании v1: M5-T416, V5-T416 и 18 объявленных рёбер).
    // Ушедший выпуск не уносит ни одного этапа. Этап, оставшийся под ним, —
    // удержанный с прежним выпуском или объявленный ручкой без документа, —
    // уходил каскадом со своими задачами и рёбрами, молча, в зафиксированной
    // транзакции (ревью переименования v1 → 0.0.9, 2026-10-08). Отказ называет
    // этапы: их выпуск называет документ выпуска или объявление.
    let stranded = tx.query(
        "SELECT m.id, m.version_id FROM project_plan_milestones m
           JOIN project_plan_versions v ON v.project_id = m.project_id AND v.id = m.version_id
          WHERE m.project_id = $1 AND v.origin = 'projected' AND v.id <> ALL($2)
          ORDER BY m.id",
        &[&project, &version_ids]).await?;
    if !stranded.is_empty() {
        let names: Vec<String> = stranded.iter()
            .map(|r| format!("{} (выпуск {})", r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect();
        return Err(crate::db::Fail::Corpus(format!(
            "выпуск уходит из документов, а под ним остаются этапы: {} — назовите их в таблице \
             другого выпуска или объявите под другим выпуском", names.join(", "))));
    }
    tx.execute("DELETE FROM project_plan_versions
                 WHERE project_id = $1 AND origin = 'projected' AND id <> ALL($2)",
               &[&project, &version_ids]).await?;
    // СНОСИТСЯ ТОЛЬКО СВОЁ. Красные задачи кладёт `rebuild` из `red_task`, а этот
    // проход сносил их заодно: их нет в его перечне, значит под «лишние» они
    // подходили.
    //
    // Цена измерена, а не предположена: опрос в 400 замеров во время одного
    // `reproject` увидел ноль красных задач в 382 из них. Восемьдесят три задачи
    // пропадали на всё время между двумя проходами, и всякий, кто в это окно
    // мерил гейт, считал по плану без трека проверок. Так четырежды за сессию
    // «ломался» пункт `check-track-has-tasks` — он честно отвечал «трек пуст»,
    // потому что трек и вправду был пуст. А подкоманда `mh-server reproject
    // --half`, которая пересборку не зовёт, оставляла проект без красных задач
    // насовсем.
    //
    // Снос красных стоит теперь ВПЛОТНУЮ к их вставке, в той же транзакции
    // `rebuild`: и окна нет, и призраков нет. Условие на вид здесь — не особый
    // случай, а граница прохода: он владеет задачами из документов вида `task`.
    tx.execute("DELETE FROM project_plan_tasks
                 WHERE project_id = $1 AND origin = 'projected' AND id <> ALL($2)
                   AND entity_kind <> 'red-task'",
               &[&project, &task_ids]).await?;
    for (id, milestone, ord, title, ekind, ename, size, kind, state, commit) in &tasks {
        tx.execute(
            "INSERT INTO project_plan_tasks(project_id, id, milestone_id, ord, title,
                                            entity_kind, entity_name, size,
                                            kind, state, closing_commit)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (project_id, id) DO UPDATE SET milestone_id = EXCLUDED.milestone_id,
               ord = EXCLUDED.ord, title = EXCLUDED.title, entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, size = EXCLUDED.size, kind = EXCLUDED.kind,
               origin = 'projected'",
            &[&project, id, milestone, ord, title, ekind, ename, size, kind, state, commit],
        ).await?;
    }
    // Состояние задачи здесь не трогается умышленно: его кладёт подача датчика
    // из закрывающих трейлеров, и переписать его выведенным «не запущена»
    // значило бы стереть свидетельство заявлением.
    tx.execute("DELETE FROM project_plan_task_deps
                 WHERE project_id = $1 AND origin = 'projected' AND task_id = ANY($2)",
               &[&project, &task_ids]).await?;
    let mut written = 0;
    for (task, on) in &deps {
        let Some(on) = known.get(&on.to_lowercase()) else { continue };
        if task == on {
            continue;
        }
        written += tx
            .execute(
                "INSERT INTO project_plan_task_deps(project_id, task_id, depends_on) VALUES ($1,$2,$3)
                 ON CONFLICT (project_id, task_id, depends_on) DO UPDATE SET origin = 'projected'",
                &[&project, task, on],
            )
            .await?;
    }
    tx.commit().await?;
    Ok((versions.len(), milestones.len(), tasks.len(), written as usize))
}

/// Разметка дерева задачи — из ЕЁ ЖЕ документа, какого бы вида он ни был.
///
/// Читалось только из документов вида `task`, и зеркала не давали ни строки:
/// `V3-T19` объявляла `+ crates/tot-config/…`, а правило
/// `task-tree-op-matches-disk` отвечало «ни одна зависимость задачи его не
/// создаёт» — и было право по своим данным.
async fn tree_leaves(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    let terms = crate::scheme::Terms::load_at(&tx, project).await?;
    let rows = tx
        .query(
            "SELECT t.id, d.content FROM project_plan_tasks t
               JOIN project_documents d ON d.project_id = t.project_id
                    AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name
              WHERE t.project_id = $1",
            &[&project],
        )
        .await?;
    tx.execute("DELETE FROM project_task_tree_leaf WHERE project_id = $1", &[&project]).await?;
    let mut written = 0usize;
    for r in &rows {
        let task: String = r.get(0);
        let content: String = r.get(1);
        for (i, l) in super::tree_leaf::leaves_of(&content, terms.one("section.tree").unwrap_or(""))
            .into_iter()
            .enumerate()
        {
            tx.execute(
                "INSERT INTO project_task_tree_leaf(project_id, task_id, ord, dir, leaf, is_path,
                                                    exempt, target_dir, op, path)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
                &[&project, &task, &(i as i32), &l.dir, &l.leaf, &l.is_path, &l.exempt, &l.target,
                  &l.op.to_string(), &l.path],
            )
            .await?;
            written += 1;
        }
    }
    tx.commit().await?;
    Ok(written)
}

/// ПОСЛЕПЛАНОВЫЙ ПРОХОД: всё, чему нужен ПОЛНЫЙ список задач.
///
/// Красные задачи попадают в `project_plan_tasks` отдельным шагом сборки, ПОЗЖЕ
/// разбора плана. Всякий шаг, который читает список задач раньше, зеркала не
/// видит — и молчит об этом. За одно утро это случилось четырежды: волна у
/// зеркал выходила пустой и читалась как круг; ребро `M3-T19 → V3-T19`
/// отбрасывалось; разметка дерева зеркал не попадала в таблицу вовсе.
///
/// Поэтому не четыре починки по месту, а одно место с именем: сюда переносится
/// то, чему нужен полный план, и порядок внутри назван — листья, связи, волна.
/// Волна последняя: она из связей и выводится.
pub(crate) async fn after(pool: &Pool, project: &str) -> Result<serde_json::Value, crate::db::Fail> {
    let leaves = tree_leaves(pool, project).await?;
    let edges = deps(pool, project).await?;
    let depth = waves(pool, project).await?;
    Ok(serde_json::json!({ "project_task_tree_leaf": leaves, "project_plan_task_deps": edges, "волн": depth }))
}

/// Объявленные зависимости — ПОСЛЕДНИМ шагом сборки, и место здесь важнее шага.
///
/// Имя зависимости разрешается по списку задач. Пока это делалось при разборе
/// плана, в списке стояли только задачи, разобранные тем же проходом, — то есть
/// одни `dev`. Красные задачи попадают в `project_plan_tasks` ПОЗЖЕ, отдельным
/// шагом, и ребро на зеркало отбрасывалось молча: `M3-T19` объявила «Зависит от
/// `M3-T12` · `V3-T19`», а ждала одну `M3-T12`. Молча — потому что
/// неразрешённое имя просто пропускалось, и разницы между «ссылка в никуда» и
/// «ещё не вставлено» не было.
///
/// Теперь список берётся из самой таблицы задач, когда в ней уже все. Тот же
/// разбор, то же место в конце, что и у волны, и по той же причине.
pub(crate) async fn deps(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    let known: HashMap<String, String> = tx
        .query("SELECT id FROM project_plan_tasks WHERE project_id = $1", &[&project])
        .await?
        .iter()
        .map(|r| {
            let id: String = r.get(0);
            (id.to_lowercase(), id)
        })
        .collect();
    let said = tx
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                AND f.entity_name = t.entity_name AND f.name = 'Зависит от'
              WHERE t.project_id = $1",
            &[&project],
        )
        .await?;
    tx.execute("DELETE FROM project_plan_task_deps WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    let mut written = 0;
    for r in &said {
        let (task, field): (String, String) = (r.get(0), r.get(1));
        for d in IDENTIFIER.captures_iter(&field) {
            let Some(on) = known.get(&d[1].to_lowercase()) else { continue };
            if &task == on {
                continue;
            }
            written += tx
                .execute(
                    "INSERT INTO project_plan_task_deps(project_id, task_id, depends_on) VALUES ($1,$2,$3)
                     ON CONFLICT (project_id, task_id, depends_on) DO UPDATE SET origin = 'projected'",
                    &[&project, &task, on],
                )
                .await?;
        }
    }
    tx.commit().await?;
    Ok(written as usize)
}

/// Глубина задачи по объявленным зависимостям: 1 у той, что никого не ждёт.
///
/// КРУГ НЕ ПОЛУЧАЕТ НОМЕРА ВОВСЕ, и это не осторожность. Прежний счёт ходил
/// проходами с пределом и утверждал в доводе, что «круг не углубляет» — проба
/// показала обратное: два взаимно ждущих друг друга дошли до седьмой волны на
/// трёх задачах. Число там было выдумано: задачу из круга не начать ни первой,
/// ни седьмой. Пустая волна — честный ответ «порядка нет», а сам круг называет
/// гейт правилом `task-dependency-points-back`.
///
/// Считается снятием слоёв: волну получает тот, у кого все ожидаемые её уже
/// получили. Кто не получил ни за один проход — стоит в круге либо за ним.
pub(crate) fn depth(ids: &[String], edges: &[(String, String)]) -> Vec<Option<i32>> {
    let index: std::collections::HashMap<&str, usize> =
        ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let mut waits: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    for (from, to) in edges {
        if let (Some(&a), Some(&b)) = (index.get(from.as_str()), index.get(to.as_str())) {
            if a != b {
                waits[a].push(b);
            }
        }
    }
    let mut wave: Vec<Option<i32>> = vec![None; ids.len()];
    loop {
        let mut moved = false;
        for i in 0..ids.len() {
            if wave[i].is_some() {
                continue;
            }
            let mut deepest = 0;
            let mut ready = true;
            for &j in &waits[i] {
                match wave[j] {
                    Some(w) => deepest = deepest.max(w),
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if ready {
                wave[i] = Some(deepest + 1);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    wave
}

/// Разложить задачи по волнам и положить волну на задачу.
///
/// Считается ПОСЛЕ связей: глубина берётся из `project_plan_task_deps` и из
/// «жду весь этап», а они пишутся шагом выше и уже зафиксированы.
pub(crate) async fn waves(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    let ids: Vec<String> = tx
        .query("SELECT id FROM project_plan_tasks WHERE project_id = $1 ORDER BY milestone_id, ord, id",
               &[&project])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let edges: Vec<(String, String)> = tx
        .query(
            "SELECT task_id, depends_on FROM project_plan_task_deps WHERE project_id = $1
             UNION ALL
             SELECT md.task_id, t.id FROM task_milestone_dep md
               JOIN project_plan_tasks t
                 ON t.project_id = md.project_id AND t.milestone_id = md.milestone_id
              WHERE md.project_id = $1 AND t.id <> md.task_id",
            &[&project],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    let wave = depth(&ids, &edges);
    for (id, w) in ids.iter().zip(wave.iter()) {
        tx.execute("UPDATE project_plan_tasks SET wave = $3 WHERE project_id = $1 AND id = $2",
                   &[&project, id, w]).await?;
    }
    tx.commit().await?;
    Ok(wave.iter().flatten().copied().max().unwrap_or(0) as usize)
}

#[cfg(test)]
mod waves_depth {
    use super::depth;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }
    fn edges(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    /// Принято сломом нарочно: уберите повторные проходы — цепочка соберётся
    /// не до конца, и `M1-T3` останется во второй волне вместо третьей.
    #[test]
    fn a_task_waits_one_wave_longer_than_the_deepest_it_waits_for() {
        let t = ids(&["M1-T1", "M1-T2", "M1-T3", "M2-T1"]);
        // Порядок рёбер нарочно обратный цепочке: проход не должен зависеть от него.
        let w = depth(&t, &edges(&[("M1-T3", "M1-T2"), ("M1-T2", "M1-T1"), ("M2-T1", "M1-T2")]));
        assert_eq!(w, vec![Some(1), Some(2), Some(3), Some(3)], "глубина цепочки и ветки поперёк этапа");
    }

    /// Круг и всё, что стоит за ним, волны не получают: числа там нет.
    #[test]
    fn a_cycle_and_whatever_waits_behind_it_get_no_wave() {
        let t = ids(&["A", "B", "C", "D"]);
        // A и B ждут друг друга, C ждёт A, D не ждёт никого.
        let w = depth(&t, &edges(&[("A", "B"), ("B", "A"), ("C", "A"), ("D", "нет-такой")]));
        assert_eq!(w, vec![None, None, None, Some(1)], "круг и заложник круга без номера: {w:?}");
    }

    /// Задача, ждущая саму себя, и ребро в никуда не участвуют.
    #[test]
    fn a_self_edge_and_an_unknown_name_are_ignored() {
        let t = ids(&["A", "B"]);
        assert_eq!(depth(&t, &edges(&[("A", "A"), ("B", "нет-такой")])), vec![Some(1), Some(1)]);
    }
}

/// Задача из документа под этапом, объявленным ручкой, с вехой, написанной
/// прозой, — как карточки M9 у tot-ade, на которых пересборка плана стояла.
#[cfg(test)]
mod milestone_field {
    const P: &str = "p";
    const SCHEMA: &str = "plan_milestone_field";

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_prose_milestone_under_a_declared_milestone_projects() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}options=-c%20search_path%3D{SCHEMA}", if url.contains('?') { '&' } else { '?' });
        let pool = crate::db::pool(&format!("{url}{apart}"), 4).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE; CREATE SCHEMA {SCHEMA};"))
            .await.expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        crate::projector::declare_version(&pool, P, "0.1.2", false).await.expect("выпуск");
        crate::projector::declare_milestone(&pool, P, "M9", "0.1.2", 1, "каркас", false).await.expect("этап");
        // У объявленного этапа появился документ, а таблица выпуска его не
        // называет: выпуск остаётся прежним.
        pool.get().await.expect("соединение").execute(
            "INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                            bytes, revision, updated_at, updated_by)
             VALUES ($1, 'milestone', 'M9', '# M9 · каркас', '', 0, 1, 0, 't')", &[&P])
            .await.expect("документ этапа");
        // Имя задачи этапа не называет: иначе запасной ответ по имени скрыл бы
        // непрочитанное поле.
        // Документ и его поле кладутся строками: разбор документа в поля — не
        // предмет этой проверки, а `value_raw` хранит веху так, как её
        // написали, со звёздочками.
        let client = pool.get().await.expect("соединение");
        client.execute(
            "INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                            bytes, revision, updated_at, updated_by)
             VALUES ($1, 'task', 'window-1', '# window-1 · Окно', '', 0, 1, 0, 't')", &[&P])
            .await.expect("документ задачи");
        client.execute(
            "INSERT INTO project_document_fields (project_id, entity_kind, entity_name, section_ord, ord,
                                                  name, shape, value_raw, value)
             VALUES ($1, 'task', 'window-1', 0, 0, 'Веха', 'row', $2, $3)",
            &[&P, &"**M9** — выпуск **0.1.2**, редактор", &"M9 — выпуск 0.1.2, редактор"])
            .await.expect("поле вехи");
        drop(client);
        super::project(&pool, P).await.map_err(|e| crate::db::Says::says(&e)).expect("пересборка плана проходит");
        let milestone: String = pool.get().await.expect("соединение")
            .query_one("SELECT milestone_id FROM project_plan_tasks WHERE project_id = $1 AND id = 'window-1'", &[&P])
            .await.expect("задача в плане").get(0);
        assert_eq!(milestone, "M9");
        let version: String = pool.get().await.expect("соединение")
            .query_one("SELECT version_id FROM project_plan_milestones WHERE project_id = $1 AND id = 'M9'", &[&P])
            .await.expect("этап в плане").get(0);
        assert_eq!(version, "0.1.2", "этап с документом остаётся в объявленном выпуске");
        // Веха прозой без имени этапа — отказ с её словами, а не этап по имени.
        let client = pool.get().await.expect("соединение");
        client.execute(
            "INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                            bytes, revision, updated_at, updated_by)
             VALUES ($1, 'task', 'M9-T2', '# M9-T2 · Окно', '', 0, 1, 0, 't')", &[&P])
            .await.expect("документ задачи");
        client.execute(
            "INSERT INTO project_document_fields (project_id, entity_kind, entity_name, section_ord, ord,
                                                  name, shape, value_raw, value)
             VALUES ($1, 'task', 'M9-T2', 0, 0, 'Веха', 'row', $2, $2)",
            &[&P, &"выпуск 0.1.2"])
            .await.expect("поле вехи");
        drop(client);
        let refused = super::project(&pool, P).await.map(|_| ()).map_err(|e| crate::db::Says::says(&e));
        let why = refused.expect_err("веха без имени этапа не уходит в этап по имени задачи");
        assert!(why.contains("M9-T2 → веха «выпуск 0.1.2»"), "отказ называет задачу и её слова: {why}");
        pool.get().await.expect("соединение")
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"))
            .await.expect("схема снимается");
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_milestone_held_by_a_leaving_version_refuses_instead_of_vanishing() {
        const SCHEMA: &str = "plan_milestone_leaving";
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}options=-c%20search_path%3D{SCHEMA}", if url.contains('?') { '&' } else { '?' });
        let pool = crate::db::pool(&format!("{url}{apart}"), 4).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE; CREATE SCHEMA {SCHEMA};"))
            .await.expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        // Выпуск v1 пришёл из документа, которого больше нет: пересборка его снимет.
        crate::projector::declare_version(&pool, P, "v1", false).await.expect("выпуск");
        crate::projector::declare_milestone(&pool, P, "M7", "v1", 1, "этап", false).await.expect("этап");
        crate::projector::declare_task(&pool, P, crate::projector::Task {
            id: "M7-T1", milestone: "M7", ord: 1, title: "объявленная", kind: "dev", state: "", size: "",
        }, false).await.expect("объявленная задача");
        let client = pool.get().await.expect("соединение");
        client.execute("UPDATE project_plan_versions SET origin = 'projected' WHERE project_id = $1", &[&P])
            .await.expect("выпуск выведенный");
        client.execute(
            "INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                            bytes, revision, updated_at, updated_by)
             VALUES ($1, 'milestone', 'M7', '# M7 · этап', '', 0, 1, 0, 't')", &[&P])
            .await.expect("документ этапа");
        drop(client);
        // Второй этап объявлен ручкой и документа не имеет: перечень пересборки
        // его не видит, и защита только перечисленных его не прикрыла бы.
        crate::projector::declare_milestone(&pool, P, "M8", "v1", 2, "без документа", false).await.expect("этап");
        crate::projector::declare_task(&pool, P, crate::projector::Task {
            id: "M8-T1", milestone: "M8", ord: 1, title: "объявленная", kind: "dev", state: "", size: "",
        }, false).await.expect("объявленная задача");
        let rebuilt = super::project(&pool, P).await.map(|_| ()).map_err(|e| crate::db::Says::says(&e));
        let why = rebuilt.expect_err("этап без оставшегося выпуска роняет пересборку, а не уходит каскадом");
        assert!(why.contains("M7 (выпуск v1)") && why.contains("M8 (выпуск v1)"), "отказ называет этапы: {why}");
        let left: i64 = pool.get().await.expect("соединение")
            .query_one("SELECT count(*) FROM project_plan_tasks WHERE project_id = $1 AND id IN ('M7-T1', 'M8-T1')", &[&P])
            .await.expect("счёт").get(0);
        assert_eq!(left, 2, "объявленные задачи под этапами целы");
        pool.get().await.expect("соединение")
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"))
            .await.expect("схема снимается");
    }
}
