//! Области и выжимки прогонов — один разбор на две проекции, как у донора.
//!
//! Область (`10-intent/functional/<имя>.md`) перечисляет свои истории
//! заголовками разделов; прогон (`60-runs/v1/M0/M0-T1.md`) рассказывает, как
//! выполнялась задача. Общего у них только источник: заголовки разделов.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Имя прогона — имя того, о чём он: `M0` у прогона этапа, `M0-T1` у прогона
/// задачи. Выпуск прогона — выпуск его этапа; прежде он читался из каталога
/// (`60-runs/v1/M0/M0-T1.md`), а каталог говорил то же, что этап.
static RUN_OF_TASK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([MV]\d+)-T[0-9a-z]+$").expect("образец прогона задачи"));
static RUN_OF_MILESTONE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([MV]\d+)$").expect("образец прогона этапа"));
/// Заголовки разделов хранятся без разметки, поэтому обратных кавычек в них нет.
static STORY_IN_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?(US-[A-Z0-9]+-\d+)`?\b").expect("образец истории"));
const LEFT_OPEN: &str = "Оставлено открытым";

/// Заголовки разделов сущности: первый — заголовок документа, остальные — его разделы.
///
/// Отбор идёт ВИДОМ, а не корнем пути: корень был способом сказать «эти
/// документы», а вид говорит это прямо и записан у каждого.
pub async fn section_titles(
    pool: &Pool,
    project: &str,
    kinds: &[&str],
) -> Result<HashMap<(String, String), Vec<String>>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let list: Vec<String> = kinds.iter().map(|k| (*k).to_owned()).collect();
    let rows = client
        .query(
            "SELECT s.entity_kind, s.entity_name, s.title FROM project_document_sections s
              WHERE s.project_id = $1 AND s.entity_kind = ANY($2)
              ORDER BY s.entity_kind, s.entity_name, s.ord",
            &[&project, &list],
        )
        .await?;
    let mut out: HashMap<(String, String), Vec<String>> = HashMap::new();
    for r in &rows {
        out.entry((r.get(0), r.get(1))).or_default().push(r.get(2));
    }
    Ok(out)
}

/// Сущности названных видов, в порядке имени.
pub async fn named_of_kinds(
    pool: &Pool,
    project: &str,
    kinds: &[&str],
) -> Result<Vec<(String, String)>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let list: Vec<String> = kinds.iter().map(|k| (*k).to_owned()).collect();
    let rows = client
        .query(
            "SELECT entity_kind, entity_name FROM project_documents
              WHERE project_id = $1 AND entity_kind = ANY($2)
              ORDER BY entity_kind, entity_name",
            &[&project, &list],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Оговорка об удалении: имя в такой строке названо, чтобы сказать, что его
/// больше нет.
pub fn gone(line: &str) -> bool {
    GONE.is_match(line)
}

static GONE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)удал|отмен|снят|прежн|историч|устар").expect("образец оговорки")
});

pub async fn project(
    pool: &Pool,
    project: &str,
) -> Result<(usize, usize, usize), tokio_postgres::Error> {
    let named = named_of_kinds(pool, project, &["feature", "run"]).await?;
    let titles = section_titles(pool, project, &["feature", "run"]).await?;
    // Выпуск этапа объявлен самим выпуском: его документ перечисляет этапы.
    let milestone_version: std::collections::HashMap<String, String> = {
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
    // Заголовок документа — первый его раздел; без разделов остаётся имя.
    let title_of = |e: &(String, String)| -> String {
        titles.get(e).and_then(|t| t.first()).cloned().unwrap_or_else(|| e.1.clone())
    };
    let empty: Vec<String> = Vec::new();

    let mut features: Vec<(String, String, String, String, i32)> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for e in &named {
        if e.0 != "feature" {
            continue;
        }
        let id = e.1.clone();
        if id == "README" {
            continue;
        }
        let mut stories = 0;
        for title in titles.get(e).unwrap_or(&empty) {
            let Some(sm) = STORY_IN_TITLE.captures(title) else { continue };
            let story = sm[1].to_owned();
            if seen.insert(format!("{id} {story}")) {
                links.push((id.clone(), story));
                stories += 1;
            }
        }
        features.push((id, title_of(e), e.0.clone(), e.1.clone(), stories));
    }

    // Требования фичи названы ПЕРЕЧНЕМ: `FR-CFG-01…15, FR-EXT-04`. Пока их
    // никто не раскрывал, таблица связей стояла пустой при девятнадцати
    // фичах и трёхстах требованиях — и всякая проверка покрытия по ней
    // отвечала «ни одно требование не названо», что неправда.
    let mut requirement_links: Vec<(String, String)> = Vec::new();
    {
        let client = pool.get().await.expect("пул отдал соединение");
        // Читается ВЕСЬ текст документа фичи, а не поле «Требования»: фича
        // называет требование и прозой раздела, и таблицей, и подписью к
        // экрану. Одно поле дало бы «требование не описано» там, где оно
        // описано подробнее всего.
        let rows = client
            .query(
                "SELECT entity_name, content FROM project_documents
                  WHERE project_id = $1 AND entity_kind = 'feature' AND entity_name <> 'README'",
                &[&project],
            )
            .await?;
        let mut seen_link = HashSet::new();
        for r in &rows {
            let feature: String = r.get(0);
            let value: String = r.get(1);
            // Построчно и мимо оговорок. «`FR-STP-11…13` удалены» — это не
            // называние требования, а запись о его удалении: посчитать её
            // связью значит завести шесть связей на то, чего нет.
            let named: Vec<String> = value
                .lines()
                .filter(|l| !gone(l))
                .flat_map(super::ids::expand)
                .collect();
            for name in named {
                // Только `FR`. Нефункциональные требования сквозные: они не
                // принадлежат фиче и не обязаны быть в ней названы, а связь
                // «NFR в этой фиче» ничего не значит.
                if !name.starts_with("FR-") {
                    continue;
                }
                if seen_link.insert(format!("{feature} {name}")) {
                    requirement_links.push((feature.clone(), name));
                }
            }
        }
    }

    let mut runs: Vec<(String, String, String, String, String, String, bool, bool, i32)> = Vec::new();
    for e in &named {
        if e.0 != "run" {
            continue;
        }
        let task = RUN_OF_TASK.captures(&e.1);
        let milestone = if task.is_some() { None } else { RUN_OF_MILESTONE.captures(&e.1) };
        let (id, milestone_id, is_milestone) = match (&task, &milestone) {
            (Some(t), _) => (e.1.clone(), t[1].to_owned(), false),
            (None, Some(m)) => (e.1.clone(), m[1].to_owned(), true),
            _ => continue,
        };
        let version = milestone_version.get(&milestone_id).cloned().unwrap_or_default();
        let list = titles.get(e).unwrap_or(&empty);
        runs.push((
            id,
            title_of(e),
            e.0.clone(),
            e.1.clone(),
            version,
            milestone_id,
            is_milestone,
            list.iter().any(|t| t.trim() == LEFT_OPEN),
            list.len() as i32,
        ));
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_feature_stories WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_features WHERE project_id = $1", &[&project]).await?;
    for (id, title, kind, name, stories) in &features {
        tx.execute(
            "INSERT INTO project_features(project_id, id, title, entity_kind, entity_name, stories)
             VALUES ($1,$2,$3,$4,$5,$6)",
            &[&project, id, title, kind, name, stories],
        )
        .await?;
    }
    // Связи фичи с требованием: пересборка сносит только СВОИ строки. Строка,
    // объявленная дверью, живёт своим происхождением и здесь не трогается.
    tx.execute("DELETE FROM project_feature_requirements WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    for (feature, requirement) in &requirement_links {
        tx.execute(
            "INSERT INTO project_feature_requirements(project_id, feature_id, requirement_id, origin)
             VALUES ($1,$2,$3,'projected') ON CONFLICT DO NOTHING",
            &[&project, feature, requirement],
        )
        .await?;
    }
    for (feature, story) in &links {
        tx.execute(
            "INSERT INTO project_feature_stories(project_id, feature_id, story_id) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING",
            &[&project, feature, story],
        )
        .await?;
    }
    tx.execute("DELETE FROM project_runs_log WHERE project_id = $1", &[&project]).await?;
    for (id, title, kind, name, version, milestone, is_milestone, left_open, sections) in &runs {
        tx.execute(
            "INSERT INTO project_runs_log(project_id, id, title, entity_kind, entity_name, version,
                                          milestone, is_milestone, left_open, sections)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[&project, id, title, kind, name, version, milestone, is_milestone, left_open, sections],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((features.len(), links.len(), runs.len()))
}
