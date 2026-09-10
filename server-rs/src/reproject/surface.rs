//! Поверхность: экраны, истории и их требования.
//!
//! Требование ОБЪЯВЛЕНО первой ячейкой строки таблицы; всё остальное в разделе —
//! упоминание. Разница не косметическая: история называет одно требование, а
//! внутри его описания стоит имя другого, и обход всего тела заводил историю на
//! требование, которое она лишь поминает.

use super::needs::STORY_REQUIREMENTS_SECTION;
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Имя истории — её собственный идентификатор: `US-ONB-01`.
static STORY_NAME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^US-[A-Z0-9]+-\d+$").expect("образец истории"));
static SCREEN_IN_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?(SCR-[A-Z0-9]+-\d+)`?\b").expect("образец имени экрана"));
static SCREEN_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(SCR-[A-Z0-9]+-\d+)\b").expect("образец ссылки"));
/// Экран, названный НОМЕРОМ: «Экраны | 34, 10».
///
/// Один набор зовёт экран `SCR-ONB-01`, второй — числом, которым экран назван
/// сам: `# 34 · Память агента`. Второй способ не читался вовсе, и все 37
/// экранов набора tot выглядели никем не востребованными.
static SCREEN_NUMBER: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(\d{2})\b").expect("образец номера экрана"));
static TASK_NAME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^([MV]\d+-T[0-9a-z]+)$").expect("образец задачи"));
static REQUIREMENT_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)\b").expect("образец требования"));
static SEPARATOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[\s|:-]+$").expect("образец разделителя"));

/// Первые ячейки строк таблицы, склеенные переводом строки: место, где имя
/// объявлено, а не помянуто. Строки не-таблицы и разделитель `|---|` отброшены.
fn declared_column(body: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in body.split('\n') {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || SEPARATOR.is_match(trimmed) {
            continue;
        }
        if let Some(first) = trimmed[1..].split('|').next() {
            out.push(first.trim());
        }
    }
    out.join("\n")
}

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize, usize, usize), tokio_postgres::Error> {
    const KINDS: [&str; 3] = ["story", "screen", "task"];
    let named = super::runs::named_of_kinds(pool, project, &KINDS).await?;
    // Область экрана — объявленная величина: её держит `screen_area`, и больше
    // никто. Прежде она бралась из каталога, в котором лежал файл.
    let areas: HashMap<String, String> = {
        let client = pool.get().await.expect("пул отдал соединение");
        client
            .query(
                "SELECT screen_id, area FROM screen_area WHERE project_id = $1",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect()
    };
    let titles = super::runs::section_titles(pool, project, &KINDS).await?;
    // Номер экрана — то, чем экран назвал себя в заголовке: `# 34 · Память
    // агента`. По нему на экран и ссылаются.
    let by_number: HashMap<String, String> = {
        let client = pool.get().await.expect("пул отдал соединение");
        client
            .query(
                // Граница слова в Postgres пишется `\y`, а не `\b`: с `\b`
                // образец не совпадал ни с одним заголовком.
                "SELECT substring(title from '^\\s*([0-9]{2})\\y'), id FROM project_screens
                  WHERE project_id = $1 AND title ~ '^\\s*[0-9]{2}\\y'",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect()
    };
    let fields = super::plan::fields(pool, project, false).await?;
    let declared = super::needs::section_bodies(pool, project, STORY_REQUIREMENTS_SECTION, "story").await?;
    let empty_fields = HashMap::new();
    let empty_titles: Vec<String> = Vec::new();

    let mut stories: Vec<(String, String, String, String, String, String, String, String)> = Vec::new();
    let mut screens: Vec<(String, String, String, String, String)> = Vec::new();
    let mut references: Vec<(String, &str, String)> = Vec::new();
    let mut story_requirements: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_requirement = HashSet::new();

    for e in &named {
        let path = &e.1;
        let key = format!("{} {}", e.0, e.1);
        let title_of = || {
            titles.get(e).and_then(|t| t.first()).cloned().unwrap_or_else(|| path.clone())
        };
        let f = fields.get(&key).unwrap_or(&empty_fields);
        let got = |name: &str| f.get(name).map(String::as_str).unwrap_or("");

        if e.0 == "story" {
        if STORY_NAME.is_match(path) {
            let id = e.1.clone();
            stories.push((
                id.clone(),
                super::title_without_name(&title_of(), &id),
                e.0.clone(),
                e.1.clone(),
                // Область истории — её же фича: поле «Фича» стоит у всех 188 и
                // совпадает с каталогом, из которого область бралась прежде.
                got("Фича").trim().to_owned(),
                got("Персона").trim().to_owned(),
                got("Фаза пути").trim().to_owned(),
                got("Фича").trim().to_owned(),
            ));
            for c in SCREEN_ID.captures_iter(got("Экраны")) {
                if seen.insert(format!("{id} {}", &c[1])) {
                    references.push((id.clone(), "story", c[1].to_owned()));
                }
            }
            for c in SCREEN_NUMBER.captures_iter(got("Экраны")) {
                let Some(screen) = by_number.get(&c[1]) else { continue };
                if seen.insert(format!("{id} {screen}")) {
                    references.push((id.clone(), "story", screen.clone()));
                }
            }
            let body = declared_column(declared.get(path).map(String::as_str).unwrap_or(""));
            for c in REQUIREMENT_ID.captures_iter(&body) {
                if seen_requirement.insert(format!("{id} {}", &c[1])) {
                    story_requirements.push((id.clone(), c[1].to_owned()));
                }
            }
            continue;
        }
        }

        if e.0 == "screen" {
            // Экран объявлен заголовком: один документ держит и один экран, и
            // восемь. Имя экрана стоит в заголовке, а не в имени документа.
            let mut declared = 0usize;
            for title in titles.get(e).unwrap_or(&empty_titles) {
                let Some(id) = SCREEN_IN_TITLE.captures(title) else { continue };
                let id = id[1].to_owned();
                let area = areas.get(&id).cloned().unwrap_or_default();
                let title = super::title_without_name(&title, &id);
                screens.push((id, title, e.0.clone(), e.1.clone(), area));
                declared += 1;
            }
            if declared > 0 {
                continue;
            }
        }

        if e.0 == "task" {
        if let Some(m) = TASK_NAME.captures(path) {
            let task = m[1].to_owned();
            for c in SCREEN_ID.captures_iter(got("Экраны")) {
                if seen.insert(format!("{task} {}", &c[1])) {
                    references.push((task.clone(), "task", c[1].to_owned()));
                }
            }
            for c in SCREEN_NUMBER.captures_iter(got("Экраны")) {
                let Some(screen) = by_number.get(&c[1]) else { continue };
                if seen.insert(format!("{task} {screen}")) {
                    references.push((task.clone(), "task", screen.clone()));
                }
            }
        }
        }
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    // Имя поля — из словаря схемы: зашитое, оно знает один набор.
    let terms = crate::scheme::Terms::load(pool, project).await?;
    let screen_requirements: Vec<(String, String)> = if let Some(field) =
        terms.one("field.requirements").map(str::to_owned)
    {
        let c2 = pool.get().await.expect("пул отдал соединение");
        let rows = c2
            .query(
                "SELECT f.entity_name, f.value, s.id FROM project_document_fields f
                   JOIN project_screens s ON s.project_id = f.project_id
                        AND s.entity_kind = f.entity_kind AND s.entity_name = f.entity_name
                  WHERE f.project_id = $1 AND f.entity_kind = 'screen' AND f.name = $2",
                &[&project, &field],
            )
            .await?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in &rows {
            let value: String = r.get(1);
            let screen: String = r.get(2);
            for line in value.lines() {
                if super::runs::gone(line) {
                    continue;
                }
                for name in super::ids::expand(line) {
                    if !(name.starts_with("FR-") || name.starts_with("NFR-")) {
                        continue;
                    }
                    if seen.insert(format!("{screen} {name}")) {
                        out.push((screen.clone(), name));
                    }
                }
            }
        }
        out
    } else {
        Vec::new()
    };
    let tx = client.transaction().await?;
    // Связи пересобираются целиком — они выводятся всегда. У сущностей стирается
    // только выведенное: объявленная прямо история не выводится ниоткуда, и
    // пересборка, стирающая её, стирала бы запись, а не свой прошлый вывод.
    for table in ["project_screen_references", "project_story_requirements"] {
        tx.execute(&format!("DELETE FROM {table} WHERE project_id = $1"), &[&project]).await?;
    }
    for table in ["project_stories", "project_screens"] {
        tx.execute(&format!("DELETE FROM {table} WHERE project_id = $1 AND origin = 'projected'"),
                   &[&project]).await?;
    }
    for (id, title, kind, name, area, persona, phase, feature) in &stories {
        tx.execute(
            "INSERT INTO project_stories(project_id, id, title, entity_kind, entity_name, area,
                                         persona, phase, feature)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[&project, id, title, kind, name, area, persona, phase, feature],
        )
        .await?;
    }
    // Отложенность экрана объявлена в нём самом: «**Вне первой версии**».
    tx.execute(
        "UPDATE project_screens s SET out_of_version = CASE
            WHEN d.content ~ 'Вне первой версии' OR d.content ~ 'Вне v[0-9]' THEN 'объявлен отложенным'
            ELSE '' END
           FROM project_documents d
          WHERE d.project_id = s.project_id AND d.entity_kind = s.entity_kind
            AND d.entity_name = s.entity_name AND s.project_id = $1",
        &[&project],
    )
    .await?;
    for (id, title, kind, name, area) in &screens {
        tx.execute(
            "INSERT INTO project_screens(project_id, id, title, entity_kind, entity_name, area)
             VALUES ($1,$2,$3,$4,$5,$6)",
            &[&project, id, title, kind, name, area],
        )
        .await?;
    }
    // Тело экрана — из секции его же документа, названной его именем. Без
    // текста экран выпадал из каскада «связь обновилась — вопрос переоткрыт»,
    // хотя описание есть у всех 69.
    tx.execute(
        "UPDATE project_screens s SET spec = coalesce(z.тело,'')
           FROM (SELECT sc.id, sc.project_id,
                        (SELECT string_agg(b.raw, E'\n' ORDER BY b.ord)
                           FROM project_document_blocks b
                          WHERE b.project_id = sec.project_id AND b.entity_kind = sec.entity_kind
                            AND b.entity_name = sec.entity_name
                            AND b.ord > sec.ord AND b.ord <= sec.last_block
                            AND b.kind NOT IN ('heading','blank')) тело
                   FROM project_screens sc
                   JOIN LATERAL (SELECT * FROM project_document_sections d
                                  WHERE d.project_id = sc.project_id AND d.entity_kind = 'screen'
                                    AND d.title LIKE sc.id || '%' LIMIT 1) sec ON true
                  WHERE sc.project_id = $1) z
          WHERE z.project_id = s.project_id AND z.id = s.id",
        &[&project],
    )
    .await?;

    // Требования экрана: раскрытый перечень поля «Требования». Без этой связи
    // правило «задача, чьи требования видны человеку, называет экраны» отвечало
    // бы нулём при шестидесяти восьми экранах — то есть молчало бы.
    tx.execute("DELETE FROM project_screen_requirements WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    for (screen, requirement) in &screen_requirements {
        tx.execute(
            "INSERT INTO project_screen_requirements(project_id, screen_id, requirement_id, origin)
             VALUES ($1,$2,$3,'projected') ON CONFLICT DO NOTHING",
            &[&project, screen, requirement],
        )
        .await?;
    }
    for (story, requirement) in &story_requirements {
        tx.execute(
            "INSERT INTO project_story_requirements(project_id, story_id, requirement_id) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING",
            &[&project, story, requirement],
        )
        .await?;
    }
    for (src, kind, screen) in &references {
        tx.execute(
            "INSERT INTO project_screen_references(project_id, source, source_kind, screen_id)
             VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&project, src, kind, screen],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((stories.len(), screens.len(), references.len(), story_requirements.len()))
}
