//! Шапка этапа против его же перечней.
//!
//! Шапку правят руками, перечни — тоже, и расходятся они молча: правка одного
//! перечня оставляет в шапке прежнее число, а читают шапку первой. Слом
//! измерен: проход оставил в шапке `11/8/12` при перечнях `12/9/12`, объявив
//! правку сделанной.
//!
//! Слова шапки и заголовки блоков — из словаря схемы: другой набор назовёт их
//! иначе, и зашитое слово сделало бы правило знающим один проект.

use deadpool_postgres::Pool;

/// Роль шапки и роль блока: их четыре пары, и связаны они именем роли.
const PAIRS: [(&str, &str, &str); 4] = [
    ("milestone.head.requirements", "milestone.block.requirements", "требования"),
    ("milestone.head.stories", "milestone.block.stories", "истории"),
    ("milestone.head.screens", "milestone.block.screens", "экраны"),
    ("milestone.head.checks", "milestone.block.checks", "проверки"),
];

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let terms = crate::scheme::Terms::load_at(&*client, project).await?;
    let docs = client
        .query(
            "SELECT entity_name, content FROM project_documents
              WHERE project_id = $1 AND entity_kind = 'milestone'",
            &[&project],
        )
        .await?;
    let fields = client
        .query(
            "SELECT entity_name, name, value FROM project_document_fields
              WHERE project_id = $1 AND entity_kind = 'milestone'",
            &[&project],
        )
        .await?;

    let mut rows: Vec<(String, String, i32, i32)> = Vec::new();
    for d in &docs {
        let milestone: String = d.get(0);
        let content: String = d.get(1);
        for (head_role, block_role, what) in PAIRS {
            let (Some(head_word), Some(block_word)) = (terms.one(head_role), terms.one(block_role))
            else {
                continue;
            };
            // Сказано шапкой.
            let said = fields.iter().find_map(|f| {
                let n: String = f.get(0);
                let k: String = f.get(1);
                if n == milestone && k == head_word {
                    f.get::<_, String>(2).trim().parse::<i32>().ok()
                } else {
                    None
                }
            });
            let Some(said) = said else { continue };
            // Названо перечнем: имена внутри свёрнутого блока.
            let marker = format!("<details><summary>{block_word}");
            let Some(after) = content.split(marker.as_str()).nth(1) else { continue };
            let block = after.split("</details>").next().unwrap_or("");
            let mut names = std::collections::HashSet::new();
            // Зачёркнутое не считается: тот же блок `plan.rs` читает через
            // `ids::said`, и счёт, включающий снятое, разошёлся бы с составом.
            for line in block.lines() {
                for id in super::ids::said(line) {
                    names.insert(id);
                }
            }
            rows.push((milestone.clone(), what.to_owned(), said, names.len() as i32));
        }
    }

    // Читающее соединение отпущено: писать и читать одновременно здесь нечем,
    // а два соединения на одну проекцию запирают пул на себе же.
    drop(client);
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    tx.execute("DELETE FROM project_milestone_count WHERE project_id = $1", &[&project]).await?;
    for (m, what, said, listed) in &rows {
        tx.execute(
            "INSERT INTO project_milestone_count(project_id, milestone_id, what, said, listed)
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
            &[&project, m, what, said, listed],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}
