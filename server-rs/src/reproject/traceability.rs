//! Матрица трассируемости — единственный документ, делающий проверяемые
//! утверждения обо всём остальном: сколько в подсистеме требований и сколько из
//! них описано проверками. Числа написаны руками, а значит устаревают молча.
//!
//! Часть колонок сверить нечем: «названо телом контракта» и «в коде» говорят про
//! контракт и репозиторий, которых в наборе нет. Они сохраняются как заявленное.

use super::rows::{at, cells};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "traceability";
const NAME: &str = "";
const HEADER: [&str; 3] = ["Подсистема", "FR", "TC описано"];
static TOTAL_ROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(?i:всего|итого)\s*$").expect("образец итога"));
/// «SIT ситуации» → код подсистемы SIT, дальше человеческое название.
static AREA: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*([A-Z]{2,4})\b\s*(.*)$").expect("образец подсистемы"));
static COUNT: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(\d+)").expect("образец числа"));

pub async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let mut claims: Vec<(String, String, i32, i32, Option<i32>, String)> = Vec::new();

    // Таблица находится по заголовку, а не по номеру блока: документ правят.
    for rows in blocks.values() {
        let Some(head) = rows.get(&0) else { continue };
        if !HEADER.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name) {
            continue;
        }
        for (ord, row) in rows {
            if *ord == 0 || TOTAL_ROW.is_match(at(row, 0)) {
                continue;
            }
            let Some(m) = AREA.captures(at(row, 0)) else { continue };
            let num = |v: &str| COUNT.captures(v).and_then(|c| c[1].parse::<i32>().ok());
            claims.push((
                m[1].to_owned(),
                m[2].trim().to_owned(),
                num(at(row, 1)).unwrap_or(0),
                num(at(row, 2)).unwrap_or(0),
                num(at(row, 3)),
                at(row, 4).trim().to_owned(),
            ));
        }
    }

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_traceability_claims WHERE project_id = $1", &[&project]).await?;
    for (area, title, requirements, described, in_contract, in_code) in &claims {
        tx.execute(
            "INSERT INTO project_traceability_claims(project_id, area, title, requirements,
                                                     described, in_contract, in_code, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[&project, area, title, requirements, described, in_contract, in_code, &KIND, &NAME],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(claims.len())
}
