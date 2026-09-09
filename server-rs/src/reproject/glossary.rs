//! Словарь: «Термин | Идентификатор | Что значит».
//!
//! Русское имя и машинное — одно понятие, и расхождение между ними ловится
//! только тем, что они лежат рядом. Раздел над таблицей — область термина.

use super::rows::{at, cells, headings, title_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "glossary";
const NAME: &str = "";
const HEADER: [&str; 3] = ["Термин", "Идентификатор", "Что значит"];
/// Идентификатор — одно машинное слово, иногда с точкой или дефисом.
static IDENTIFIER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?([A-Za-z][A-Za-z0-9_.-]*)`?\s*$").expect("образец имени"));

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let heads = headings(pool, project, KIND, NAME).await?;

    let mut terms: Vec<(String, String, String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for (block, rows) in &blocks {
        // Только таблицы словаря: в документе есть и таблица переименований
        // «Сегодня | Становится | Цена», где машинное имя стоит во второй колонке.
        let Some(head) = rows.get(&0) else { continue };
        if !HEADER.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name) {
            continue;
        }
        let area = title_of(&heads, *block);
        for (ord, row) in rows {
            if *ord == 0 {
                continue;
            }
            let Some(m) = IDENTIFIER.captures(at(row, 1)) else { continue };
            let id = m[1].to_owned();
            let term = at(row, 0).trim().to_owned();
            if term.is_empty() {
                continue;
            }
            // Одно машинное имя объявляется один раз; повтор — находка, не строка.
            if !seen.insert(id.clone()) {
                continue;
            }
            terms.push((id, term, at(row, 2).trim().to_owned(), area.clone()));
        }
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_terms WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    for (id, term, meaning, area) in &terms {
        tx.execute(
            "INSERT INTO project_terms(project_id, id, term, meaning, area, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[&project, id, term, meaning, area, &KIND, &NAME],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(terms.len())
}
