//! Числа, сказанные документом прослеживаемости.
//!
//! Документ пишет «`FR` — 272», «myack-app — 181 тестовая функция». Правило
//! сверяет это с измеренным, и чтобы сверять РАВЕНСТВОМ, сказанное обязано
//! лежать значением. Иначе каждое правило вынимало бы число из ячейки само —
//! своим способом, и два правила разошлись бы в чтении одной таблицы.

use deadpool_postgres::Pool;

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let terms = crate::scheme::Terms::load_at(&*client, project).await?;
    let rows = client
        .query(
            "SELECT block_ord, row_ord, col, value FROM project_document_cells
              WHERE project_id = $1 AND entity_kind = 'traceability'
              ORDER BY block_ord, row_ord, col",
            &[&project],
        )
        .await?;

    // Шапка блока — строка ноль: она даёт имена колонок. Первая ячейка строки —
    // предмет. Числом считается только то, что числом записано целиком:
    // «**1** — ST-63, отложена» это проза с числом, а не число.
    let mut head: std::collections::HashMap<(i32, i32), String> = Default::default();
    let mut subject: std::collections::HashMap<(i32, i32), String> = Default::default();
    let mut said: Vec<(i32, String, String, i32, bool)> = Vec::new();
    let totals = terms.all("word.total");
    for r in &rows {
        let (block, row, col): (i32, i32, i32) = (r.get(0), r.get(1), r.get(2));
        let value: String = r.get(3);
        let clean = value.trim().trim_matches('`').trim().to_owned();
        if row == 0 {
            head.insert((block, col), clean);
            continue;
        }
        if col == 0 {
            // Предмет — первое слово: `SIT ситуации` это подсистема `SIT`,
            // `ST` — потребности сторон` это род `ST`.
            let first = clean.split_whitespace().next().unwrap_or("").trim_matches('`').to_owned();
            subject.insert((block, row), if first.is_empty() { clean } else { first });
            continue;
        }
        let Some(subj) = subject.get(&(block, row)) else { continue };
        let Some(name) = head.get(&(block, col)) else { continue };
        let Ok(n) = clean.parse::<i32>() else { continue };
        let low = subj.to_lowercase();
        let total = totals.iter().any(|t| t.to_lowercase() == low);
        said.push((block, subj.clone(), name.clone(), n, total));
    }

    // Читающее соединение отпущено: писать и читать одновременно здесь нечем,
    // а два соединения на одну проекцию запирают пул на себе же.
    drop(client);
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_traceability_said WHERE project_id = $1", &[&project]).await?;
    for (block, subj, name, n, total) in &said {
        tx.execute(
            "INSERT INTO project_traceability_said(project_id, block_ord, subject, column_name,
                                                   said, is_total)
             VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
            &[&project, block, subj, name, n, total],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(said.len())
}
