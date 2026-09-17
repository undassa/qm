//! Чтение корпуса из `project_documents`.
//!
//! Наполняет таблицу отдельная работа — перенос файлов в базу. Сервер файловую
//! систему не читает вовсе: чего нет в базе, того нет. Это не ограничение, а
//! граница — иначе у корпуса появилось бы два источника, и расходились бы они
//! молча.

use deadpool_postgres::Pool;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Summary {
    pub bytes: i32,
    pub revision: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    #[serde(rename = "updatedBy")]
    pub updated_by: String,
}

#[derive(Debug, Serialize)]
pub struct Document {
    #[serde(flatten)]
    pub summary: Summary,
    pub content: String,
}

/// Перечень путей. Префикс сужает выборку до каталога.
///
/// Хвостовая косая в префиксе снимается: `list(p, "10-intent/")` и
/// `list(p, "10-intent")` должны отвечать одинаково. Прежняя поверхность на этом
/// уже обжигалась — запрос с косой возвращал пусто, и пустота выглядела как
/// «в каталоге ничего нет», а не как «спросили не так».
/// Один документ целиком. `None` — документа нет; это `not_found`, а не ошибка.
pub async fn read(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
) -> Result<Option<Document>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT bytes, revision, updated_at, updated_by, content
               FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?;
    Ok(rows.first().map(|r| Document {
        summary: Summary {
            bytes: r.get(0),
            revision: r.get(1),
            updated_at: r.get(2),
            updated_by: r.get(3),
        },
        content: r.get(4),
    }))
}
