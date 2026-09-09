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
use std::collections::HashMap;

/// Этап задачи — то, что стоит в её имени до `-T`: у `M0-T1` это `M0`.
///
/// Прежде этап читался из каталога (`50-plan/v1/M0/M0-T1.md`). Каталог говорил
/// то же самое, что имя задачи, — только окольно.
static TASK_MILESTONE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^([MV]\d+)-T").expect("образец этапа в имени задачи"));
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
pub fn state_of(field: &str) -> (&'static str, Option<String>) {
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
pub async fn fields(
    pool: &Pool,
    project: &str,
    raw: bool,
) -> Result<HashMap<String, HashMap<String, String>>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
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

/// Заголовок документа — первый его раздел; если разделов нет, донор берёт путь.
pub async fn titles(pool: &Pool, project: &str) -> Result<HashMap<String, String>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
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

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize, usize, usize), tokio_postgres::Error> {
    let named = super::runs::named_of_kinds(pool, project, &["version", "milestone", "task"]).await?;
    // Блок «Требования» этапа — свёрнутый `<details>`. Читается он целиком и
    // построчно, мимо строк с оговоркой об удалении: имя в такой строке
    // названо, чтобы сказать, что его больше нет.
    let terms = crate::scheme::Terms::load(pool).await?;
    let milestone_requirements: Vec<(String, String)> = {
        let client = pool.get().await.expect("пул отдал соединение");
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
                if super::runs::gone(line) {
                    continue;
                }
                for name in super::ids::expand(line) {
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
    // Какому выпуску принадлежит этап, говорит сам выпуск: его документ
    // перечисляет этапы таблицей, первой колонкой. Прежде это читалось из
    // каталога, в котором лежал файл.
    let milestone_version: HashMap<String, String> = {
        let client = pool.get().await.expect("пул отдал соединение");
        client
            .query(
                "SELECT c.value, c.entity_name FROM project_document_cells c
                  WHERE c.project_id = $1 AND c.entity_kind = 'version'
                    AND c.col = 0 AND c.row_ord > 0 AND c.value ~ '^[MV][0-9]+$'",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect()
    };
    let fields = fields(pool, project, true).await?;
    let titles = titles(pool, project).await?;
    let empty = HashMap::new();

    let mut versions: Vec<(String, String, String)> = Vec::new();
    let mut milestones: Vec<(String, String, i32, String, String, String)> = Vec::new();
    let mut tasks: Vec<(String, String, i32, String, String, String, String, &str, &str, Option<String>)> = Vec::new();
    let mut deps: Vec<(String, String)> = Vec::new();

    for e in &named {
        let key = format!("{} {}", e.0, e.1);
        if e.0 == "version" {
            versions.push((e.1.clone(), e.0.clone(), e.1.clone()));
            continue;
        }
        let title = titles.get(&key).cloned().unwrap_or_else(|| e.1.clone());
        if e.0 == "milestone" {
            let version = milestone_version.get(&e.1).cloned().unwrap_or_default();
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
        let milestone = match got("Веха").trim() {
            "" => TASK_MILESTONE
                .captures(&id)
                .map(|m| m[1].to_uppercase())
                .unwrap_or_default(),
            declared => declared.to_owned(),
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
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
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
    let task_ids: Vec<String> = tasks.iter().map(|t| t.0.clone()).collect();
    tx.execute("DELETE FROM project_plan_versions
                 WHERE project_id = $1 AND origin = 'projected' AND id <> ALL($2)",
               &[&project, &version_ids]).await?;
    for (id, kind, name) in &versions {
        tx.execute(
            "INSERT INTO project_plan_versions(project_id, id, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (project_id, id) DO UPDATE SET entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name",
            &[&project, id, kind, name],
        ).await?;
    }
    // Листья дерева задач: разбор блока живёт в своём модуле, а сюда приходит
    // готовым перечнем.
    let tree_leaves: Vec<(String, i32, String, String, bool, bool, String)> = {
        let client = pool.get().await.expect("пул отдал соединение");
        let rows = client
            .query(
                "SELECT t.id, d.content FROM project_plan_tasks t
                   JOIN project_documents d ON d.project_id = t.project_id
                        AND d.entity_kind = 'task' AND d.entity_name = t.entity_name
                  WHERE t.project_id = $1",
                &[&project],
            )
            .await?;
        let mut out = Vec::new();
        for r in &rows {
            let task: String = r.get(0);
            let content: String = r.get(1);
            for (i, l) in super::tree_leaf::leaves_of(&content, terms.one("section.tree").unwrap_or("")).into_iter().enumerate() {
                let target = super::tree_leaf::target_dir(&l.dir, &l.leaf);
                out.push((task.clone(), i as i32, l.dir, l.leaf, l.is_path, l.exempt, target));
            }
        }
        out
    };

    // Требования этапа: раскрытый перечень блока «Требования». Требование
    // принадлежит ровно ОДНОМУ этапу — без этой связи этапы становятся
    // тематическими заголовками, а не планом: новое требование не попадает
    // ни в один и молчит.
    tx.execute("DELETE FROM project_task_tree_leaf WHERE project_id = $1", &[&project]).await?;
    for (task, ord, dir, leaf, is_path, exempt, target) in &tree_leaves {
        tx.execute(
            "INSERT INTO project_task_tree_leaf(project_id, task_id, ord, dir, leaf, is_path,
                                                exempt, target_dir)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            &[&project, task, ord, dir, leaf, is_path, exempt, target],
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
               entity_name = EXCLUDED.entity_name",
            &[&project, id, version, ord, title, kind, name],
        ).await?;
    }
    tx.execute("DELETE FROM project_plan_tasks
                 WHERE project_id = $1 AND origin = 'projected' AND id <> ALL($2)",
               &[&project, &task_ids]).await?;
    for (id, milestone, ord, title, ekind, ename, size, kind, state, commit) in &tasks {
        tx.execute(
            "INSERT INTO project_plan_tasks(project_id, id, milestone_id, ord, title,
                                            entity_kind, entity_name, size,
                                            kind, state, closing_commit)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (project_id, id) DO UPDATE SET milestone_id = EXCLUDED.milestone_id,
               ord = EXCLUDED.ord, title = EXCLUDED.title, entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, size = EXCLUDED.size, kind = EXCLUDED.kind",
            &[&project, id, milestone, ord, title, ekind, ename, size, kind, state, commit],
        ).await?;
    }
    // Состояние задачи здесь не трогается умышленно: его кладёт подача датчика
    // из закрывающих трейлеров, и переписать его выведенным «не запущена»
    // значило бы стереть свидетельство заявлением.
    let mut written = 0;
    for (task, on) in &deps {
        let Some(on) = known.get(&on.to_lowercase()) else { continue };
        if task == on {
            continue;
        }
        written += tx
            .execute(
                "INSERT INTO project_plan_task_deps(project_id, task_id, depends_on) VALUES ($1,$2,$3)
                 ON CONFLICT DO NOTHING",
                &[&project, task, on],
            )
            .await?;
    }
    tx.commit().await?;
    Ok((versions.len(), milestones.len(), tasks.len(), written as usize))
}
