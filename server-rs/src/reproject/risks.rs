//! Риски: открытые описаны разделом и таблицей, улаженные — перечнем.
//!
//! Таблица открытого риска необычной формы: влияние и вероятность стоят в самой
//! шапке («В / Вер | высокое / среднее»), а строки ниже — владелец, признак,
//! источник. Принятые как цена решения и закрытые перечислены отдельно, и там у
//! риска другое состояние — открытым его считать нельзя.

use super::rows::{at, cells, headings, title_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "risks";
const NAME: &str = "";
const HEAD: &str = "В / Вер";
static SECTION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(R-(\d+))\s*[·—–-]\s*(.+?)\s*$").expect("образец раздела риска"));
static RISK_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*`?(R-(\d+))`?\s*$").expect("образец имени риска"));

struct Risk {
    id: String,
    number: i32,
    title: String,
    state: &'static str,
    impact: String,
    probability: String,
    owner: String,
    trigger: String,
    source: String,
    settled_by: String,
}

/// «высокое / среднее» — влияние и вероятность одной ячейкой.
fn split_impact(value: &str) -> (String, String) {
    let mut parts = value.split('/').map(str::trim);
    (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned())
}

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let heads = headings(pool, project, KIND, NAME).await?;
    let mut risks: HashMap<String, Risk> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (block, rows) in &blocks {
        let Some(head) = rows.get(&0) else { continue };

        // Открытый риск: собственный раздел плюс таблица со шапкой «В / Вер».
        if at(head, 0).trim() == HEAD {
            let heading = title_of(&heads, *block);
            let Some(m) = SECTION.captures(&heading) else { continue };
            let field = |name: &str| -> String {
                for row in rows.values() {
                    if at(row, 0).trim() == name {
                        return at(row, 1).trim().to_owned();
                    }
                }
                String::new()
            };
            let (impact, probability) = split_impact(at(head, 1));
            let id = m[1].to_owned();
            order.push(id.clone());
            risks.insert(
                id.clone(),
                Risk {
                    id,
                    number: m[2].parse().unwrap_or(0),
                    title: m[3].to_owned(),
                    state: "open",
                    impact,
                    probability,
                    owner: field("Владелец"),
                    trigger: field("Признак"),
                    source: field("Источник"),
                    settled_by: String::new(),
                },
            );
            continue;
        }

        // Улаженные: перечень «Риск | Где записан» или «Риск | Чем закрыт».
        let column = at(head, 2).trim();
        let state = match column {
            "Чем закрыт" => "closed",
            "Где записан" => "accepted",
            _ => continue,
        };
        for (ord, row) in rows {
            if *ord == 0 {
                continue;
            }
            let Some(m) = RISK_ID.captures(at(row, 0)) else { continue };
            let id = m[1].to_owned();
            if risks.contains_key(&id) {
                continue;
            }
            order.push(id.clone());
            risks.insert(
                id.clone(),
                Risk {
                    id,
                    number: m[2].parse().unwrap_or(0),
                    title: at(row, 1).trim().to_owned(),
                    state,
                    impact: String::new(),
                    probability: String::new(),
                    owner: String::new(),
                    trigger: String::new(),
                    source: String::new(),
                    settled_by: at(row, 2).trim().to_owned(),
                },
            );
        }
    }

    let mut all: Vec<&Risk> = order.iter().filter_map(|id| risks.get(id)).collect();
    all.sort_by_key(|r| r.number);

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_risks WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    for r in &all {
        tx.execute(
            "INSERT INTO project_risks(project_id, id, number, title, state, impact, probability,
                                       owner, trigger_sign, source, settled_by, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
            &[&project, &r.id, &r.number, &r.title, &r.state, &r.impact, &r.probability,
              &r.owner, &r.trigger, &r.source, &r.settled_by, &KIND, &NAME],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(all.len())
}
