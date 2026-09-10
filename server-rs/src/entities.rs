//! Сущность вместо адреса.
//!
//! У проекта есть конституция, требование `FR-ORG-12`, решение `ADR-0157` — так
//! их и спрашивают. Путь остаётся подробностью того, что набор когда-то лежал
//! файлами: он живёт внутри этого модуля и наружу не выходит.

use deadpool_postgres::Pool;
use serde_json::{json, Value};

use crate::kinds::{table_of, Kinds};

#[derive(Debug)]
pub enum Miss {
    /// Такого вида нет вовсе.
    NoKind(String),
    /// Вид есть, сущности с таким именем нет.
    NoEntity(String, String),
    /// Вид есть, но проекции у него нет: ответ неизвестен, а не пуст.
    Unprojected(String),
    Db(String),
}

impl From<tokio_postgres::Error> for Miss {
    fn from(e: tokio_postgres::Error) -> Self {
        Miss::Db(e.to_string())
    }
}

/// Как документ зовут: вид и имя, под которыми он записан.
///
/// **Правило адресации одно, и оно здесь.** Прежде их было два: имя спрашивалось
/// у базы, а не нашедшись — выводилось раскладкой из пути (корень вида плюс имя
/// плюс `.md`). Пока путь был ключом, второе было нужно; теперь ключ — имя, и
/// вывод из пути стал бы вторым правилом, расходящимся с первым молча. Ровно так
/// один документ отзывался на два имени: `story README` и `index 10-intent/use-cases`.
///
/// Пустое имя — законный ответ: у одиночки имени нет, его заменяет вид.
pub async fn locate(
    pool: &Pool,
    kinds: &Kinds,
    project: &str,
    kind: &str,
    id: Option<&str>,
) -> Result<(String, String), Miss> {
    let Some(k) = kinds.get(kind) else {
        return Err(Miss::NoKind(kind.to_owned()));
    };
    // Внутренняя сущность живёт в своём документе-хозяине: у требования нет
    // своего документа, оно объявлено строкой внутри `srs`.
    if k.is_inner() {
        if id.is_none() {
            return Err(Miss::NoEntity(kind.to_owned(), String::new()));
        }
        let owner = k.in_kind.clone().ok_or_else(|| Miss::NoKind(kind.to_owned()))?;
        return Box::pin(locate(pool, kinds, project, &owner, None)).await;
    }
    if !k.single && id.is_none() {
        return Err(Miss::NoEntity(kind.to_owned(), String::new()));
    }
    let name = if k.single { "" } else { id.unwrap_or_default() };
    // Образец имени применяется и здесь: без него вид отдаёт то, чего не
    // называет его же перечень.
    if !k.single && !matches_id_of(pool, project, kind, k, name).await {
        return Err(Miss::NoEntity(kind.to_owned(), name.to_owned()));
    }
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT entity_kind, entity_name FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?;
    if let Some(r) = rows.first() {
        return Ok((r.get(0), r.get(1)));
    }
    // Своего документа у сущности нет — значит она объявлена ВНУТРИ чужого, и
    // какого именно, знает предметная таблица. Так живут три экрана
    // `SCR-SHELL-01·07·08` в одном `shell`: вид объявлен документным, а набор
    // держит их втроём. Это не второе правило адресации документа, а ответ на
    // другой вопрос — «в каком документе это объявлено», — и отвечает на него
    // та же таблица, что объявляет сущность.
    if let Some((table, id_col, _)) = table_of(kind) {
        let sql = if kind == "task" {
            format!(
                "SELECT entity_kind, entity_name FROM {table}
                  WHERE project_id = $1 AND {id_col} = $2 AND kind <> 'red' AND entity_kind <> ''"
            )
        } else {
            format!(
                "SELECT entity_kind, entity_name FROM {table}
                  WHERE project_id = $1 AND {id_col} = $2 AND entity_kind <> ''"
            )
        };
        if let Some(r) = client.query(&sql, &[&project, &name]).await?.first() {
            return Ok((r.get(0), r.get(1)));
        }
    }
    // Прежде здесь стояло `k.under.is_none()` — «нет каталога». Каталог в
    // точности совпадал с «документный и не одиночный», то есть условие целиком
    // было НЕДОСТИЖИМО: `!inner && (single||inner) && !single`. Условие,
    // отвечающее «никогда», — это не условие.
    if !k.is_inner() && table_of(kind).is_none() && !k.single {
        return Err(Miss::Unprojected(kind.to_owned()));
    }
    Err(Miss::NoEntity(kind.to_owned(), name.to_owned()))
}

/// Имена сущностей вида.
///
/// Документный вид перечисляется САМИМИ документами: вид у каждого записан, и
/// обход путей под корнем стал бы вторым правилом рядом с первым. Внутренний —
/// своей предметной таблицей: собственного документа у него нет.
pub async fn ids(pool: &Pool, kinds: &Kinds, project: &str, kind: &str) -> Result<Vec<String>, Miss> {
    let Some(k) = kinds.get(kind) else {
        return Err(Miss::NoKind(kind.to_owned()));
    };
    if k.single {
        return Ok(vec![kind.to_owned()]);
    }
    let client = pool.get().await.expect("пул отдал соединение");
    if let Some((table, id_col, _)) = table_of(kind) {
        let sql = if kind == "task" {
            format!("SELECT DISTINCT {id_col} FROM {table} WHERE project_id = $1 AND kind <> 'red' ORDER BY 1")
        } else {
            format!("SELECT DISTINCT {id_col} FROM {table} WHERE project_id = $1 ORDER BY 1")
        };
        let rows = client.query(&sql, &[&project]).await?;
        return Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect());
    }
    // Перечня имён не бывает у одиночного вида и у внутреннего: у первого
    // экземпляр один и без имени, у второго имена живут в чужом документе.
    // Прежде это спрашивалось через «есть ли каталог» — файловым следом того,
    // что и так сказано формой.
    if k.single || k.is_inner() {
        return Err(Miss::Unprojected(kind.to_owned()));
    }
    let rows = client
        .query(
            "SELECT entity_name FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 ORDER BY 1",
            &[&project, &kind],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let name: String = r.get(0);
            matches_id(k, &name).then_some(name)
        })
        .collect())
}

/// Сущность целиком.
///
/// Документ-сущность отдаётся своим текстом; внутренняя — своей строкой.
/// Сущность без тела: сколько в ней байт вместо самого текста.
///
/// Байт, а не знаков: `content.len()` в Rust считает байты, и назвать их
/// знаками значило бы соврать на каждом тире — так же, как это поле уже
/// названо у записи не разметкой.
///
/// Читалка берёт текст блоками — теми, что разобраны при записи. Присылать ей
/// вдобавок `content` значит гнать один документ дважды, и второй экземпляр
/// выбрасывается. Одно место на обоих вызывающих: ручку и MCP.
pub fn without_body(mut v: Value) -> Value {
    if let Some(len) = v.get("content").and_then(|c| c.as_str()).map(str::len) {
        v["bytes"] = json!(len);
        if let Some(m) = v.as_object_mut() {
            m.remove("content");
        }
    }
    v
}

pub async fn entity(pool: &Pool, kinds: &Kinds, project: &str, kind: &str, id: Option<&str>) -> Result<Value, Miss> {
    let Some(k) = kinds.get(kind) else {
        return Err(Miss::NoKind(kind.to_owned()));
    };
    let client = pool.get().await.expect("пул отдал соединение");

    if k.is_inner() {
        let id = id.ok_or_else(|| Miss::NoEntity(kind.to_owned(), String::new()))?;
        let Some((table, id_col, _)) = table_of(kind) else {
            return Err(Miss::Unprojected(kind.to_owned()));
        };
        // Строка приходит текстом JSON: типа `json` в tokio-postgres нет без
        // отдельной возможности, а лишняя возможность ради одной колонки — плата
        // большая, чем один разбор.
        let sql = format!(
            "SELECT row_to_json(x)::text FROM (SELECT * FROM {table} WHERE project_id = $1 AND {id_col} = $2) x"
        );
        let rows = client.query(&sql, &[&project, &id]).await?;
        let raw: String = rows
            .first()
            .map(|r| r.get::<_, String>(0))
            .ok_or_else(|| Miss::NoEntity(kind.to_owned(), id.to_owned()))?;
        let mut row: Value = serde_json::from_str(&raw).map_err(|e| Miss::Db(e.to_string()))?;
        if let Some(map) = row.as_object_mut() {
            // Ни проекта, ни адреса наружу: первое известно спрашивающему,
            // второго у сущности нет.
            map.remove("project_id");
            map.remove("path");
        }
        return Ok(json!({ "kind": kind, "id": id, "entity": row }));
    }

    let (owner_kind, owner_name) = locate(pool, kinds, project, kind, id).await?;
    let rows = client
        .query(
            "SELECT content, revision, updated_at, updated_by FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &owner_kind, &owner_name],
        )
        .await?;
    let row = rows
        .first()
        .ok_or_else(|| Miss::NoEntity(kind.to_owned(), id.unwrap_or(kind).to_owned()))?;
    let content: String = row.get(0);
    // Расширение берётся у ИМЕНИ, а не у файла. Правило то же, по которому имя
    // и составлено: `.md` подразумевается и в имени опущено, любое другое стоит
    // в имени явно — `mockup build/operations-os-web-standalone.html`.
    let format = owner_name.rsplit('.').next().unwrap_or("").to_owned();
    // Запись не разметкой отдаётся описанием, а не текстом: вывалить мегабайт
    // собранного html в ответ — значит забить им весь разговор и ничего не
    // сообщить. Что это и сколько в нём — сообщается, содержимое берут разделом.
    if format != "md" && format != owner_name {
        return Ok(json!({
            "kind": kind, "id": id.unwrap_or(kind),
            "format": format, "bytes": content.len(),
            "revision": row.get::<_, i64>(1),
            "why": "запись не разметкой: отдаётся описанием, а не текстом",
        }));
    }
    Ok(json!({
        "kind": kind,
        "id": id.unwrap_or(kind),
        "content": content,
        "revision": row.get::<_, i64>(1),
        "updatedAt": row.get::<_, i64>(2),
        "updatedBy": row.get::<_, String>(3),
    }))
}

/// Кто ссылается на сущность — сущностями, а не файлами.
///
/// Разрешается соединением с предметными таблицами. Что не разрешилось,
/// остаётся видом одиночки либо адресом: выдуманное имя хуже честного адреса.
pub async fn backlinks(
    pool: &Pool,
    kinds: &Kinds,
    project: &str,
    kind: &str,
    id: Option<&str>,
) -> Result<Vec<Value>, Miss> {
    let k = kinds.get(kind).ok_or_else(|| Miss::NoKind(kind.to_owned()))?;
    let client = pool.get().await.expect("пул отдал соединение");

    if k.is_inner() {
        // Внутреннюю сущность видно упоминанием: где её имя стоит ячейкой.
        let id = id.ok_or_else(|| Miss::NoEntity(kind.to_owned(), String::new()))?;
        let like = format!("%{id}%");
        let rows = client
            .query(
                "SELECT DISTINCT c.entity_kind, c.entity_name FROM project_document_cells c
                  WHERE c.project_id = $1 AND c.value LIKE $2 ORDER BY 1, 2",
                &[&project, &like],
            )
            .await?;
        let mut out = Vec::new();
        for r in &rows {
            let (from_kind, from_name): (String, String) = (r.get(0), r.get(1));
            out.push(json!({
                "from": if from_name.is_empty() { from_kind } else { format!("{from_kind} {from_name}") },
                "label": id,
            }));
        }
        return Ok(out);
    }

    let (owner_kind, owner_name) = locate(pool, kinds, project, kind, id).await?;
    // Ссылающийся называется СВОИМ именем, записанным рядом со ссылкой, а не
    // выводится из адреса при каждом запросе. Вывод на каждом запросе — это
    // ровно то, чем 82 красные задачи полгода звались `task`: пока имя не стало
    // записанным ответом базы, ошибку в нём нечем было увидеть.
    let rows = client
        .query(
            "SELECT l.entity_kind, l.entity_name, l.label FROM project_document_links l
              WHERE l.project_id = $1 AND l.target_kind = $2 AND l.target_name = $3
              ORDER BY l.entity_kind, l.entity_name, l.ord",
            &[&project, &owner_kind, &owner_name],
        )
        .await?;
    let mut out = Vec::new();
    for r in &rows {
        let (from_kind, from_name): (String, String) = (r.get(0), r.get(1));
        out.push(json!({
            "from": if from_name.is_empty() { from_kind } else { format!("{from_kind} {from_name}") },
            "label": r.get::<_, Option<String>>(2),
        }));
    }
    Ok(out)
}

/// Подходит ли имя под объявленный образец. Образца нет — подходит любое.
pub fn matches_id(k: &crate::kinds::Kind, id: &str) -> bool {
    match k.id.as_deref() {
        Some(pattern) => regex::Regex::new(pattern).map(|re| re.is_match(id)).unwrap_or(true),
        None => true,
    }
}

/// Образец имени вида — С УЧЁТОМ СЛОВА НАБОРА.
///
/// Раскладка видов объявлена ОДНА на все наборы, и образец имени вопроса в ней
/// `^Q-\d+$`. `tot-ade` зовёт свои вопросы `OQ-01…OQ-80`: перечень их отдавал,
/// а `question id=OQ-01` отказывал — ни один другой вид так себя не ведёт.
/// Скилл ответов начинает работу словами «посмотри на открытые вопросы» и на
/// первом же шаге получал отказ.
///
/// Слово даёт набор: роль `id.<вид>` в словаре схемы. Не объявлена — образец
/// раскладки, как и было.
pub async fn matches_id_of(
    pool: &Pool,
    project: &str,
    kind: &str,
    k: &crate::kinds::Kind,
    id: &str,
) -> bool {
    let client = pool.get().await.expect("пул отдал соединение");
    let own = client
        .query(
            "SELECT value FROM scheme($1) WHERE role = $2",
            &[&project, &format!("id.{kind}")],
        )
        .await
        .unwrap_or_default();
    if own.is_empty() {
        return matches_id(k, id);
    }
    own.iter().any(|r| {
        regex::Regex::new(&r.get::<_, String>(0))
            .map(|re| re.is_match(id))
            .unwrap_or(false)
    })
}
