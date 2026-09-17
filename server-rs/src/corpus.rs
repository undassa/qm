//! Выборки по набору: связи, разделы, поиск, предметные сущности.
//!
//! Всё это — то, ради чего набор переехал в базу: «кто ссылается на этот
//! документ» перестало быть обходом тысячи файлов и стало запросом по индексу.

use deadpool_postgres::Pool;
use serde_json::{json, Value};

/// Разделы документа: заголовок, якорь, уровень.
///
/// Адресуется ВИДОМ И ИМЕНЕМ, а не путём: путь остаётся полем происхождения и
/// ключом быть перестал. Пустое имя законно — у одиночки его заменяет вид.
pub(crate) async fn sections(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
) -> Result<Vec<Value>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // У раздела два размера: со вложенными подразделами и без них. Оглавлению
    // нужен второй — иначе вес корневого заголовка равен всему документу, и
    // читателю он не говорит ничего. `own_last` обрывает раздел там, где
    // начинается следующий заголовок внутри него.
    let rows = client
        .query(
            "WITH s AS (
               SELECT ord, level, title, anchor, first_block, last_block,
                      COALESCE((SELECT min(n.first_block) - 1
                                  FROM project_document_sections n
                                 WHERE n.project_id = d.project_id AND n.entity_kind = d.entity_kind
                                      AND n.entity_name = d.entity_name
                                   AND n.first_block > d.first_block
                                   AND n.first_block <= d.last_block),
                               last_block) AS own_last
                 FROM project_document_sections d
                WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3)
             SELECT s.ord, s.level, s.title, s.anchor,
                    (SELECT count(*) FROM project_document_blocks b
                      WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3
                        AND b.ord BETWEEN s.first_block AND s.own_last AND b.kind <> 'blank'),
                    (SELECT COALESCE(sum(length(b.raw)), 0) FROM project_document_blocks b
                      WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3
                        AND b.ord BETWEEN s.first_block AND s.own_last AND b.kind <> 'blank'),
                    (SELECT count(*) FROM project_document_blocks b
                      WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3
                        AND b.ord BETWEEN s.first_block AND s.own_last AND b.kind = 'table'),
                    (SELECT count(*) FROM project_document_blocks b
                      WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3
                        AND b.ord BETWEEN s.first_block AND s.own_last AND b.kind = 'code')
               FROM s ORDER BY s.ord",
            &[&project, &kind, &name],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "ord": r.get::<_, i32>(0),
                "level": r.get::<_, i32>(1),
                "title": r.get::<_, String>(2),
                "anchor": r.get::<_, String>(3),
                "blocks": r.get::<_, i64>(4),
                "chars": r.get::<_, i64>(5),
                "tables": r.get::<_, i64>(6),
                "code": r.get::<_, i64>(7),
            })
        })
        .collect())
}

/// Поиск по содержимому. Отдаёт документ и строку, в которой нашлось.
pub(crate) async fn search(
    pool: &Pool,
    project: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<Value>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            // Порядок ответа — не мелочь. Ищущий «ADR-0138» ищет ДОКУМЕНТ с таким
            // именем, а не четыре чужих, где эта строка встретилась в прозе.
            // Поиск только по тексту отдавал именно чужие: имя документа
            // упоминается в нём реже, чем в ссылающихся на него.
            //
            // Потому вес: точное имя, потом имя с начала, потом имя где угодно,
            // и только затем текст. Внутри веса — по имени, чтобы порядок не
            // плясал между запросами.
            "SELECT entity_kind, entity_name, bytes, excerpt FROM (
               SELECT entity_kind, entity_name, bytes,
                      (SELECT string_agg(line, E'\\n')
                         FROM (SELECT line FROM unnest(string_to_array(content, E'\\n')) AS line
                                WHERE line ILIKE '%' || $2 || '%' LIMIT 3) hits) AS excerpt,
                      CASE
                        WHEN lower(entity_name) = lower($2) THEN 0
                        WHEN entity_name ILIKE $2 || '%' THEN 1
                        WHEN entity_name ILIKE '%' || $2 || '%' THEN 2
                        WHEN lower(entity_kind) = lower($2) THEN 3
                        ELSE 4
                      END AS weight
                 FROM project_documents
                WHERE project_id = $1
                  AND (content ILIKE '%' || $2 || '%'
                       OR entity_name ILIKE '%' || $2 || '%'
                       OR entity_kind ILIKE '%' || $2 || '%')
             ) ranked
             ORDER BY weight, entity_kind, entity_name LIMIT $3",
            &[&project, &query, &limit],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "kind": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "bytes": r.get::<_, i32>(2),
                "excerpt": r.get::<_, Option<String>>(3),
            })
        })
        .collect())
}

/// Тело раздела — из тех же блоков, которыми его режет запись.
///
/// Своего разбора здесь нет и быть не должно. Заменяя раздел, донор берёт блоки
/// от `first_block` (не включая) до `last_block` и склеивает их `raw` без
/// разделителя; читать иначе — значит читать не тот раздел, который потом
/// запишется. Строчный разбор рядом с блочным уже успел разойтись: он вернул
/// границу, при записи которой документ рос на 300 байт за круг.
pub(crate) async fn section_body(
    pool: &Pool,
    project: &str,
    kind: &str,
    name: &str,
    anchor: &str,
) -> Result<Option<String>, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT coalesce((SELECT string_agg(b.raw, '' ORDER BY b.ord)
                                FROM project_document_blocks b
                               WHERE b.project_id = s.project_id
                                 AND b.entity_kind = s.entity_kind AND b.entity_name = s.entity_name
                                 AND b.ord > s.first_block AND b.ord <= s.last_block), '')
               FROM project_document_sections s
              WHERE s.project_id = $1 AND s.entity_kind = $2 AND s.entity_name = $3
                AND s.anchor = $4",
            &[&project, &kind, &name, &anchor],
        )
        .await?;
    Ok(rows.first().map(|r| r.get::<_, String>(0)))
}
