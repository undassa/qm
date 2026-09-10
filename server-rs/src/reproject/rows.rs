//! Общее для проекций, читающих таблицы документов.
//!
//! Ячейка уже разобрана при записи и лежит в `project_document_cells` — здесь
//! её только собирают в строки. Собственного разбора таблиц ни одна проекция не
//! заводит: второй разбор тех же документов разошёлся бы с первым молча.

use deadpool_postgres::Pool;
use std::collections::BTreeMap;

/// Таблицы документа: блок → строка → колонки (значения ячеек).
pub type Blocks = BTreeMap<i32, BTreeMap<i32, Vec<String>>>;

/// Ячейки одного документа. `raw` берётся, когда проекции нужна разметка ссылки.
pub async fn cells(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    raw: bool,
) -> Result<Blocks, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT block_ord, row_ord, col, value, raw FROM project_document_cells
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
              ORDER BY block_ord, row_ord, col",
            &[&project, &kind, &name],
        )
        .await?;
    let mut out: Blocks = BTreeMap::new();
    for r in &rows {
        let (b, row, col): (i32, i32, i32) = (r.get(0), r.get(1), r.get(2));
        let v: String = if raw { r.get(4) } else { r.get(3) };
        let line = out.entry(b).or_default().entry(row).or_default();
        // Колонки приходят по порядку, но дыры возможны: столбец без ячейки —
        // пустая строка, а не сдвиг соседей на одну позицию влево.
        while line.len() <= col as usize {
            line.push(String::new());
        }
        line[col as usize] = v;
    }
    Ok(out)
}

/// Заголовки разделов документа с их порядковым номером блока.
pub async fn headings(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
) -> Result<Vec<(i32, String)>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT ord, title FROM project_document_sections
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 ORDER BY ord",
            &[&project, &kind, &name],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Заголовок раздела, под которым стоит блок: ближайший заголовок выше него.
pub fn title_of(headings: &[(i32, String)], block: i32) -> String {
    let mut title = String::new();
    for (ord, t) in headings {
        if *ord > block {
            break;
        }
        title = t.clone();
    }
    title
}

/// Номер секции, в которой стоит блок, — тот же `ord`, каким она лежит в
/// `project_document_sections`.
///
/// Рядом с `title_of` номер лежал всё это время и не брался: строка сущности
/// знала документ, но не место в нём. Пока места нет, собрать документ обратно
/// из таблицы нельзя — неизвестен ни порядок, ни к какому разделу строка
/// относится.
pub fn ord_of(headings: &[(i32, String)], block: i32) -> Option<i32> {
    let mut ord = None;
    for (o, _) in headings {
        if *o > block {
            break;
        }
        ord = Some(*o);
    }
    ord
}

/// Значение ячейки строки, которой в таблице может не быть.
pub fn at(row: &[String], col: usize) -> &str {
    row.get(col).map(String::as_str).unwrap_or("")
}
