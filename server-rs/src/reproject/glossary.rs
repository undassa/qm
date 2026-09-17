//! Словарь: «Термин | Идентификатор | Что значит».
//!
//! Русское имя и машинное — одно понятие, и расхождение между ними ловится
//! только тем, что они лежат рядом. Раздел над таблицей — область термина.

use super::rows::{at, cells, headings, ord_of, title_of};
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

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let heads = headings(pool, project, KIND, NAME).await?;

    let mut terms: Vec<(String, String, String, String, Option<i32>)> = Vec::new();
    let mut seen = HashSet::new();
    for (block, rows) in &blocks {
        // Только таблицы словаря: в документе есть и таблица переименований
        // «Сегодня | Становится | Цена», где машинное имя стоит во второй колонке.
        let Some(head) = rows.get(&0) else { continue };
        if !HEADER.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name) {
            continue;
        }
        let area = title_of(&heads, *block);
        let section = ord_of(&heads, *block);
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
            terms.push((id, term, at(row, 2).trim().to_owned(), area.clone(), section));
        }
    }

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_terms WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // ОБЪЯВЛЕННОЕ ТЕМ ЖЕ ИМЕНЕМ ПОГЛОЩАЕТСЯ ДОКУМЕНТОМ. Чистка снимала только
    // строки происхождения `projected`, а объявленная дверью оставалась — и
    // вставка документа с тем же именем падала на первичном ключе. Пересборка
    // обрывалась целиком, гейт продолжал отдавать числа по недособранным
    // проекциям, и час уходил на ложные находки.
    //
    // Документ — полнее объявления: все колонки, которые заполняет дверь, он
    // считает сам. Побеждает он.
    let ids: Vec<String> = terms.iter().map(|t| t.0.clone()).collect();
    tx.execute("DELETE FROM project_terms WHERE project_id = $1 AND id = ANY($2)",
               &[&project, &ids]).await?;
    for (id, term, meaning, area, section) in &terms {
        tx.execute(
            "INSERT INTO project_terms(project_id, id, term, meaning, area, entity_kind, entity_name, section_ord)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            &[&project, id, term, meaning, area, &KIND, &NAME, section],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(terms.len())
}
