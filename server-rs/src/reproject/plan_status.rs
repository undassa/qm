//! Доска состояний `50-plan/v1/status.md` — второе объявление того же факта:
//! состояние задачи записано ещё и полем в самой задаче. Два места расходятся
//! молча, поэтому доска разбирается отдельно и сверяется с планом.

use super::plan::state_of;
use super::rows::{at, cells, headings, title_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "board";
const NAME: &str = "status";
const HEADER: [&str; 3] = ["Задача", "Состояние", "Коммиты"];
static TASK_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?([MV]\d+-T[0-9a-z]+)`?\s*$").expect("образец задачи"));
/// В ячейке коммита бывает несколько хешей и прочерк.
static COMMIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b([0-9a-f]{7,40})\b").expect("образец коммита"));

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let heads = headings(pool, project, KIND, NAME).await?;
    let mut out: Vec<(String, String, &str, String)> = Vec::new();
    let mut seen = HashSet::new();

    for (block, rows) in &blocks {
        let Some(head) = rows.get(&0) else { continue };
        if !HEADER.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name) {
            continue;
        }
        // Заголовок раздела над таблицей — это и есть этап: «M0», «M1»…
        let milestone = title_of(&heads, *block);
        for (ord, row) in rows {
            if *ord == 0 {
                continue;
            }
            let Some(m) = TASK_ID.captures(at(row, 0)) else { continue };
            let id = m[1].to_owned();
            if !seen.insert(id.clone()) {
                continue;
            }
            // Состояние читается тем же разбором, что и поле задачи: словарь один.
            let (state, _) = state_of(at(row, 1));
            let commit = COMMIT.captures(at(row, 2)).map(|c| c[1].to_owned()).unwrap_or_default();
            out.push((id, milestone.clone(), state, commit));
        }
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_plan_status WHERE project_id = $1", &[&project]).await?;
    for (task, milestone, state, commit) in &out {
        tx.execute(
            "INSERT INTO project_plan_status(project_id, task_id, milestone, state, commit_hash, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[&project, task, milestone, state, commit, &KIND, &NAME],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(out.len())
}
