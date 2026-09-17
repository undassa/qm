//! Потребности заинтересованных сторон — начало цепочки:
//! потребность → история → требование → проверка → задача.
//!
//! Объявлены таблицами в `10-intent/strs.md`, сгруппованными по темам. История
//! называет потребность в двух местах: разделом «Потребности…» (объявление) и
//! колонкой в таблице требований (цитата). Они расходятся, и это находка, а не
//! повод слить их в одну связь — потому у связи есть вид.

use super::rows::{at, cells, headings, title_of, ord_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const REGISTER_KIND: &str = "strs";
const REGISTER_NAME: &str = "";
pub const STORY_NEEDS_SECTION: &str = "Потребности, из которых это выросло";
pub const STORY_REQUIREMENTS_SECTION: &str = "Требования, на которые опирается";
static NEED_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(ST-(\d+))\s*$").expect("образец потребности"));
static NEED_IN_TEXT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(ST-\d+)\b").expect("образец упоминания"));
/// Имя истории — её собственный идентификатор: `US-ONB-01`.
static STORY_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(US-[A-Z0-9]+-\d+)$").expect("образец истории"));

/// «О» обязательно · «Ж» желательно · «П» можно после — так объявлено в документе.
fn priority_of(mark: &str) -> &'static str {
    match mark.trim() {
        "О" => "must",
        "Ж" => "should",
        "П" => "later",
        _ => "unknown",
    }
}

/// Тела одноимённых разделов по документам: первый раздел с таким заголовком.
pub async fn section_bodies(
    pool: &Pool,
    project: &str,
    title: &str,
    kind: &str,
) -> Result<HashMap<String, String>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT DISTINCT ON (s.entity_name) s.entity_name,
                    coalesce((SELECT string_agg(b.raw, '' ORDER BY b.ord)
                                FROM project_document_blocks b
                               WHERE b.project_id = s.project_id AND b.entity_kind = s.entity_kind AND b.entity_name = s.entity_name
                                 AND b.ord > s.first_block AND b.ord <= s.last_block), '')
               FROM project_document_sections s
              WHERE s.project_id = $1 AND s.title = $2 AND s.entity_kind = $3
              ORDER BY s.entity_name, s.ord",
            &[&project, &title, &kind],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize), crate::db::Fail> {
    let blocks = cells(pool, project, REGISTER_KIND, REGISTER_NAME, false).await?;
    let heads = headings(pool, project, REGISTER_KIND, REGISTER_NAME).await?;

    let mut needs: Vec<(String, i32, String, String, String, &str, String, Option<i32>)> = Vec::new();
    let mut claimed = HashSet::new();
    for (block, rows) in &blocks {
        for row in rows.values() {
            let Some(m) = NEED_ID.captures(at(row, 0)) else { continue };
            // Тот же ST встречается и в прозе раздела «Открытые вопросы»:
            // объявлением считается первое вхождение.
            if !claimed.insert(m[1].to_owned()) {
                continue;
            }
            needs.push((
                m[1].to_owned(),
                m[2].parse().unwrap_or(0),
                at(row, 1).trim().to_owned(),
                at(row, 2).trim().to_owned(),
                at(row, 3).trim().to_owned(),
                priority_of(at(row, 4)),
                title_of(&heads, *block),
                ord_of(&heads, *block),
            ));
        }
    }
    needs.sort_by_key(|n| n.1);

    let stories = super::runs::named_of_kinds(pool, project, &["story"]).await?;
    let declared = section_bodies(pool, project, STORY_NEEDS_SECTION, "story").await?;
    let cited = section_bodies(pool, project, STORY_REQUIREMENTS_SECTION, "story").await?;
    let mut links: Vec<(String, String, &str)> = Vec::new();
    let mut seen = HashSet::new();
    for e in &stories {
        let Some(m) = STORY_ID.captures(&e.1) else { continue };
        let story = m[1].to_owned();
        for (body, kind) in [(declared.get(&e.1), "declared"), (cited.get(&e.1), "cited")] {
            for hit in NEED_IN_TEXT.captures_iter(body.map(String::as_str).unwrap_or("")) {
                let need = hit[1].to_owned();
                if seen.insert((need.clone(), story.clone(), kind)) {
                    links.push((need, story.clone(), kind));
                }
            }
        }
    }

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_need_stories WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_needs WHERE project_id = $1", &[&project]).await?;
    for (id, number, text, sides, sources, priority, theme, секция) in &needs {
        tx.execute(
            "INSERT INTO project_needs(project_id, id, number, text, sides, sources, theme, priority,
                                        entity_kind, entity_name, section_ord)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[&project, id, number, text, sides, sources, theme, priority, &REGISTER_KIND, &REGISTER_NAME, секция],
        )
        .await?;
    }
    for (need, story, kind) in &links {
        tx.execute(
            "INSERT INTO project_need_stories(project_id, need_id, story_id, kind) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING",
            &[&project, need, story, kind],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((needs.len(), links.len()))
}
