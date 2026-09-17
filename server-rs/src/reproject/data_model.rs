//! Проект базы: таблицы и миграции из `30-design/data-model.md`.
//!
//! Большинство таблиц описано не строкой таблицы описаний, а разделом
//! «### `signals` — …» и блоком кода под ним. Считать такие неописанными значит
//! выдумать находку на почти весь проект базы — поэтому блоки кода разбираются
//! тоже, но только по именам, уже известным из миграций и описаний: выдумать
//! таблицу из случайного слова так нельзя.

use super::rows::{at, cells, headings, title_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "data-model";
const NAME: &str = "";
const MIGRATION_HEADER: [&str; 3] = ["№", "Файл", "Таблицы"];
/// Описание колонок встречается под несколькими шапками — все они об одном.
const COLUMN_HEADERS: [[&str; 2]; 2] = [["Таблица", "Ключевые колонки"], ["Таблица", "Колонки"]];
static TABLE_NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*`?([a-z][a-z0-9_]*)`?\s*$").expect("образец имени"));
static NUMBER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+$").expect("образец номера"));
static LINE_TABLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?([a-z][a-z0-9_]*)`?\s+(\S[\s\S]*)$").expect("образец строки кода"));
static SINGLE_NAME_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`?([a-z][a-z0-9_]*)`?\s*(?:[—–-][\s\S]*)?$").expect("образец заголовка"));
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("образец пробелов"));

struct Table {
    name: String,
    migration: String,
    migration_file: String,
    columns: String,
}

fn header_is(head: &[String], wanted: &[&str]) -> bool {
    wanted.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name)
}

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<(usize, usize), crate::db::Fail> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let mut migrations: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut tables: HashMap<String, Table> = HashMap::new();

    for rows in blocks.values() {
        let Some(head) = rows.get(&0) else { continue };

        if header_is(head, &MIGRATION_HEADER) {
            for (ord, row) in rows {
                if *ord == 0 {
                    continue;
                }
                let number = at(row, 0).trim().to_owned();
                if !NUMBER.is_match(&number) {
                    continue;
                }
                // Имена таблиц в ячейке разделены точкой-разделителем или запятой.
                let named = at(row, 2)
                    .split(['·', ','])
                    .filter_map(|p| TABLE_NAME.captures(p).map(|m| m[1].to_owned()))
                    .collect();
                migrations.push((number, at(row, 1).trim().to_owned(), named));
            }
            continue;
        }

        if !COLUMN_HEADERS.iter().any(|w| header_is(head, w)) {
            continue;
        }
        for (ord, row) in rows {
            if *ord == 0 {
                continue;
            }
            let Some(m) = TABLE_NAME.captures(at(row, 0)) else { continue };
            let name = m[1].to_owned();
            let columns = at(row, 1).trim().to_owned();
            // Первое описание главное; более позднее лишь дополняет пустое.
            match tables.get_mut(&name) {
                Some(known) => {
                    if known.columns.is_empty() && !columns.is_empty() {
                        known.columns = columns;
                    }
                }
                None => {
                    order.push(name.clone());
                    tables.insert(name.clone(), Table { name, migration: String::new(), migration_file: String::new(), columns });
                }
            }
        }
    }

    for (number, file, named) in &migrations {
        for name in named {
            match tables.get_mut(name) {
                Some(table) if table.migration.is_empty() => {
                    table.migration = number.clone();
                    table.migration_file = file.clone();
                }
                Some(_) => {}
                None => {
                    order.push(name.clone());
                    tables.insert(
                        name.clone(),
                        Table {
                            name: name.clone(),
                            migration: number.clone(),
                            migration_file: file.clone(),
                            columns: String::new(),
                        },
                    );
                }
            }
        }
    }

    // Блоки кода дополняют то, чего не дали таблицы описаний.
    let known: HashSet<String> = tables.keys().cloned().collect();
    for (name, columns) in columns_from_code(pool, project, &known).await? {
        if let Some(table) = tables.get_mut(&name) {
            if table.columns.is_empty() {
                table.columns = columns;
            }
        }
    }

    let mut all: Vec<&Table> = order.iter().filter_map(|n| tables.get(n)).collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_db_tables WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_db_migrations WHERE project_id = $1", &[&project]).await?;
    for t in &all {
        tx.execute(
            "INSERT INTO project_db_tables(project_id, name, migration, migration_file, columns, entity_kind, entity_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[&project, &t.name, &t.migration, &t.migration_file, &t.columns, &KIND, &NAME],
        )
        .await?;
    }
    for (number, file, named) in &migrations {
        tx.execute(
            "INSERT INTO project_db_migrations(project_id, number, file, tables) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING",
            &[&project, number, file, &(named.len() as i32)],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((all.len(), migrations.len()))
}

/// Колонки из блоков кода. Форм три: под заголовком с именем таблицы, под
/// заголовком с несколькими именами и под обычным заголовком раздела, где имя
/// стоит в начале строки.
async fn columns_from_code(
    pool: &Pool,
    project: &str,
    known: &HashSet<String>,
) -> Result<HashMap<String, String>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let blocks = client
        .query(
            "SELECT ord, kind, raw FROM project_document_blocks
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 ORDER BY ord",
            &[&project, &KIND, &NAME],
        )
        .await?;
    drop(client);
    let heads = headings(pool, project, KIND, NAME).await?;
    let mut out: HashMap<String, String> = HashMap::new();

    for b in &blocks {
        if b.get::<_, String>(1) != "code" {
            continue;
        }
        let raw: String = b.get(2);
        let lines: Vec<&str> = raw
            .split('\n')
            .filter(|l| !l.trim_start().starts_with("```"))
            .filter(|l| !l.trim().is_empty())
            .collect();
        let mut matched = false;
        for line in &lines {
            let Some(m) = LINE_TABLE.captures(line) else { continue };
            let name = m[1].to_owned();
            if !known.contains(&name) {
                continue;
            }
            matched = true;
            out.entry(name).or_insert_with(|| SPACES.replace_all(&m[2], " ").trim().to_owned());
        }
        if matched {
            continue;
        }
        // Строки имени не назвали — тогда блок целиком про таблицу из заголовка.
        let title = title_of(&heads, b.get::<_, i32>(0));
        let Some(m) = SINGLE_NAME_TITLE.captures(&title) else { continue };
        let single = m[1].to_owned();
        if !known.contains(&single) || out.contains_key(&single) {
            continue;
        }
        let body = SPACES.replace_all(&lines.join(" "), " ").trim().to_owned();
        if !body.is_empty() {
            out.insert(single, body);
        }
    }
    Ok(out)
}
