//! Список проектов — то, что интерфейс называет «контекстами».
//!
//! Прежняя поверхность отдавала под этим именем смесь: личный контекст, каналы
//! слака, сессии. React-интерфейсу из всего этого нужен только проект, и он сам
//! отбрасывает остальное по форме `group:web-project-<id>`. Здесь отдаётся
//! сразу то, что нужно: лишнее, которое получатель всё равно выбрасывает, — это
//! не совместимость, а работа впустую.

use deadpool_postgres::Pool;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct Context {
    #[serde(rename = "scopeId")]
    pub scope_id: String,
    pub name: Option<String>,
    /// Сколько документов лежит в наборе проекта.
    pub documents: i64,
    /// Когда последний раз правили; `null` — ни одного документа нет.
    ///
    /// Ноль здесь был бы враньём: он читается как «правили в 1970-м», тогда как
    /// правды нет вовсе. Пустое значение переносит незнание в интерфейс,
    /// который один и может сказать это словом.
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<i64>,
}

/// Область проекта кодируется идентификатором; интерфейс разбирает её обратно.
pub(crate) fn scope_of(project_id: &str) -> String {
    format!("group:web-project-{project_id}")
}

pub(crate) async fn list(pool: &Pool) -> Result<Vec<Context>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // Архивированный проект в списке не показывается: он есть, но работой не является.
    //
    // Соединение внешнее, и это существенно: проект без единого документа —
    // законное состояние (набор в базу ещё не переносили), и выкидывать его из
    // списка значит прятать проект, в который как раз и надо зайти.
    let rows = client
        .query(
            "SELECT p.id, p.json->>'name', count(d.entity_name)::bigint, max(d.updated_at)
               FROM projects p
               LEFT JOIN project_documents d ON d.project_id = p.id
              WHERE p.json->>'archivedAt' IS NULL
              GROUP BY p.id, p.json->>'name'
              ORDER BY p.json->>'name'",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Context {
            scope_id: scope_of(&r.get::<_, String>(0)),
            name: r.get(1),
            documents: r.get(2),
            updated_at: r.get(3),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_is_built_the_way_the_ui_reads_it() {
        assert_eq!(scope_of("308ed7a2"), "group:web-project-308ed7a2");
    }
}
