//! Сущность вместо адреса.
//!
//! У проекта есть конституция, требование `FR-ORG-12`, решение `ADR-0157` — так
//! их и спрашивают. Путь остаётся подробностью того, что набор когда-то лежал
//! файлами: он живёт внутри этого модуля и наружу не выходит.

use crate::db::Says;
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
    /// Сервер перегружен: соединений с базой не осталось. Не отказ по
    /// существу — повторить стоит, и ответ говорит, через сколько.
    Busy(String),
    /// Дверь отказала ПО СУЩЕСТВУ: довода нет, ключ не тот, правило запрещает.
    ///
    /// Прежде такие отказы шли через `Db` и печатались словами «база не
    /// ответила» — неправдой, и неправдой дорогой: читающий шёл проверять
    /// соединение вместо того, чтобы прочесть довод, который стоял тут же.
    Refused(String),
    Db(String),
}

impl From<tokio_postgres::Error> for Miss {
    fn from(e: tokio_postgres::Error) -> Self {
        Miss::Db(e.says())
    }
}

impl std::fmt::Display for Miss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Miss::NoKind(k) => write!(f, "нет такого вида: {k}"),
            Miss::NoEntity(k, id) => write!(f, "нет такой сущности: {k} {id}"),
            Miss::Unprojected(k) => write!(f, "вид {k} в базу не спроецирован"),
            Miss::Busy(why) | Miss::Refused(why) | Miss::Db(why) => f.write_str(why),
        }
    }
}

impl Miss {
    /// Тот же отказ, но с названным шагом. Занятость при этом остаётся
    /// занятостью: пока шаг дописывался склейкой в текст, `reproject` отвечал
    /// «база не ответила» и кодом отказа по существу — на перегрузке, и ровно
    /// на той двери, которую советуют позвать, когда что-то не досчиталось.
    pub fn step(self, step: &str) -> Self {
        match self {
            Miss::Busy(why) => Miss::Busy(format!("{step}: {why}")),
            Miss::Db(why) => Miss::Db(format!("{step}: {why}")),
            Miss::Refused(why) => Miss::Refused(format!("{step}: {why}")),
            other => other,
        }
    }
}

impl From<crate::db::Fail> for Miss {
    fn from(e: crate::db::Fail) -> Self {
        if e.busy() { Miss::Busy(e.says()) } else { Miss::Db(e.says()) }
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
    let client = crate::db::conn(pool).await?;
    locate_at(&*client, kinds, project, kind, id).await
}

/// То же на ГОТОВОМ соединении.
///
/// Соединение берётся один раз на запрос и передаётся вниз. Пока каждый
/// помощник брал своё, один ответ о сущности держал три сразу — своё, своё у
/// связей и своё у их перечня, — и шестнадцать одновременных чтений запирали
/// пул целиком: каждое держало первое и ждало второго.
pub async fn locate_at(
    client: &impl deadpool_postgres::GenericClient,
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
        return Box::pin(locate_at(client, kinds, project, &owner, None)).await;
    }
    if !k.single && id.is_none() {
        return Err(Miss::NoEntity(kind.to_owned(), String::new()));
    }
    let name = if k.single { "" } else { id.unwrap_or_default() };
    // Образец имени применяется и здесь: без него вид отдаёт то, чего не
    // называет его же перечень.
    if !k.single && !matches_id_at(client, project, kind, k, name).await? {
        return Err(Miss::NoEntity(kind.to_owned(), name.to_owned()));
    }
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
    let client = crate::db::conn(pool).await?;
    if k.single {
        // ЕДИНИЦА ВЫДУМЫВАЛАСЬ. Одиночному виду перечень отдавал имя вида
        // независимо от того, есть ли документ: `goals-list` сообщал
        // `count: 1, ids: ["goals"]` при нуле документов. Это не пустота в
        // ответе, а придуманная строка, и она читается как знание — тот же
        // род, что «пустой список вместо ответа».
        //
        // Контроль, которым это поймано: у вида, где документ есть, ответ не
        // менялся, а у `postmortem` он честно пуст.
        let есть = client
            .query_opt(
                "SELECT 1 FROM project_documents WHERE project_id = $1 AND entity_kind = $2",
                &[&project, &kind],
            )
            .await?
            .is_some();
        return Ok(if есть { vec![kind.to_owned()] } else { Vec::new() });
    }
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
    // Образец берётся ТОТ ЖЕ, что у двери сущности: пока перечень сверялся
    // только общей раскладкой, документ с именем по образцу набора попадал в
    // перечень, а `entity` и `document-add` его же отвергали.
    //
    // Образцы читаются ОДИН раз на перечень, а не на имя.
    let own = образцы_вида(&*client, project, kind).await?;
    let mut out = Vec::new();
    for r in &rows {
        let name: String = r.get(0);
        if подходит_образцу(&own, kind, k, &name)? {
            out.push(name);
        }
    }
    Ok(out)
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

/// Мета связей сущности: чем она связана и как разложена.
///
/// Сущность отдавалась ТЕКСТОМ либо СТРОКОЙ — и ни одной связи. Двадцать шесть
/// тысяч записей связей лежали в таблицах, а метод о них молчал: чтобы узнать,
/// на что опирается требование, надо было ЗНАТЬ про `links-of` и `backlinks` и
/// позвать их отдельно. Знание, которого нет у того, кто спрашивает впервые.
///
/// Здесь — счёт, а не содержимое: перечень в двести имён забил бы ответ. Счёт
/// говорит, что связи ЕСТЬ и сколько их; за именами идут в названную дверь.
/// «Перечня связей у этого вида нет» — это пусто, а не отказ сущности.
///
/// `links_at` знает семь видов и остальным отвечает «не спроецирован». Пока
/// этот ответ шёл наверх как есть, дверь сущности объявляла невыводимым видом
/// живой вопрос, лежащий в таблице. Перегрузка и ошибка базы отказом остаются:
/// пустые связи при них — тихая неправда, по которой считают гейты.
fn связи_или_пусто(ответ: Result<Value, Miss>) -> Result<Value, Miss> {
    match ответ {
        Ok(v) => Ok(v),
        Err(Miss::Unprojected(_)) => Ok(Value::Null),
        Err(e) => Err(e),
    }
}

async fn relations_of(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    id: &str,
    owner_kind: &str,
    owner_name: &str,
    // Внутренняя сущность живёт строкой в чужом документе, и разделы с именами
    // принадлежат ЕМУ. Отдать их как свои значило бы приписать требованию
    // четыреста имён, которых оно не называет, — весь `srs` разом.
    inner: bool,
) -> Result<Value, Miss> {
    let mut out = serde_json::Map::new();

    // Связи по видам — тем же перечнем, что отдаёт `links-of`: вторая правда о
    // связях разошлась бы с первой.
    if !id.is_empty() {
        // Связи — часть ответа, а не украшение: проглоченный отказ отдавал
        // сущность без связей, и по ней считали гейты. Но «связей у вида нет»
        // — это не отказ сущности: `links_at` знает семь видов, и для
        // остальных его «не спроецирован» отвечало бы за ВЕСЬ ответ, хотя
        // вопрос лежит в таблице и прекрасно читается.
        let v = связи_или_пусто(crate::projector::links_at(client, project, kind, id).await)?;
        if !v.is_null() {
            if let Some(sets) = v.get("sets").and_then(|s| s.as_object()) {
                let counted: serde_json::Map<String, Value> = sets
                    .iter()
                    .filter(|(_, v)| v.as_array().map(|a| !a.is_empty()).unwrap_or(false))
                    .map(|(k, v)| (k.clone(), json!(v.as_array().map(|a| a.len()).unwrap_or(0))))
                    .collect();
                if !counted.is_empty() {
                    out.insert("links".into(), Value::Object(counted));
                    out.insert("linksBy".into(), json!(format!("mh call links-of kind={kind} id={id}")));
                }
            }
        }
    }

    // Кто ссылается сюда.
    if let Ok(rows) = client
        .query(
            "SELECT count(*) FROM project_document_links l
              WHERE l.project_id = $1 AND l.target_kind = $2 AND l.target_name = $3",
            &[&project, &owner_kind, &owner_name],
        )
        .await
    {
        if let Some(r) = rows.first() {
            let n: i64 = r.get(0);
            if n > 0 {
                out.insert("backlinks".into(), json!(n));
                out.insert(
                    "backlinksBy".into(),
                    json!(format!("mh call backlinks kind={owner_kind} id={owner_name}")),
                );
            }
        }
    }

    // Как документ разложен: разделы и блоки — мета структуры, а не текста.
    if let Ok(rows) = client
        .query(
            "SELECT (SELECT count(*) FROM project_document_sections s
                      WHERE s.project_id = $1 AND s.entity_kind = $2 AND s.entity_name = $3),
                    (SELECT count(*) FROM project_document_blocks b
                      WHERE b.project_id = $1 AND b.entity_kind = $2 AND b.entity_name = $3),
                    (SELECT count(*) FROM project_document_fields f
                      WHERE f.project_id = $1 AND f.entity_kind = $2 AND f.entity_name = $3)",
            &[&project, &owner_kind, &owner_name],
        )
        .await
    {
        if let Some(r) = rows.first() {
            let (sec, blk, fld): (i64, i64, i64) = (r.get(0), r.get(1), r.get(2));
            if sec + blk + fld > 0 {
                out.insert(
                    "parts".into(),
                    json!({ "sections": sec, "blocks": blk, "fields": fld,
                            "of": if inner { format!("{owner_kind} {owner_name}") } else { String::new() },
                            "why": if inner {
                                "это разбор ДОКУМЕНТА-ХОЗЯИНА: у внутренней сущности своего документа нет"
                            } else { "" },
                            "by": format!("mh call sections kind={owner_kind} id={owner_name}") }),
                );
            }
        }
    }

    // Имена, которые сущность НАЗЫВАЕТ: связь, выведенная разбором, а не прозой.
    if let Ok(rows) = client
        .query(
            "SELECT count(*) FROM project_named_id n
              WHERE n.project_id = $1 AND n.entity_kind = $2 AND n.entity_name = $3",
            &[&project, &owner_kind, &owner_name],
        )
        .await
    {
        if let Some(r) = rows.first() {
            let n: i64 = r.get(0);
            if n > 0 {
                out.insert("names".into(), json!(n));
                if inner {
                    out.insert("namesOf".into(), json!(format!("{owner_kind} {owner_name}")));
                }
            }
        }
    }

    Ok(if out.is_empty() { Value::Null } else { Value::Object(out) })
}

/// ЖИВОЕ состояние записи: переоткрыта ли она правкой того, на чём стоит.
///
/// Объявленное состояние говорит, чем запись закончили; живое — можно ли этому
/// ещё верить. Каскад считал это верно, а наружу отдавала одна дверь из пяти:
/// `mh call task` показывал `closed` при переоткрытой задаче.
async fn живое(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    id: &str,
) -> Result<Value, crate::db::Fail> {
    let Some(r) = client
        .query_opt(
            "SELECT live_state, why, coalesce(depth,0), stale_link
               FROM entity_live
              WHERE project_id = $1 AND kind = $2 AND id = $3",
            &[&project, &kind, &id],
        )
        .await?
    else {
        return Ok(Value::Null);
    };
    let состояние: String = r.get(0);
    if состояние != "reopened" {
        return Ok(json!({ "state": состояние }));
    }
    Ok(json!({ "state": состояние, "why": r.get::<_, String>(1),
            "through": r.get::<_, i32>(2), "cause": r.get::<_, Option<String>>(3),
            "means": "запись стоит на том, что изменили после неё: закрытость держится памятью" }))
}

/// ГДЕ сущность названа: документ, секция и её заголовок.
///
/// Обратная сторона `names`: та считает, КОГО сущность называет. Спросить у
/// экрана, какая секция его задаёт, было нечем — связь стояла на уровне
/// документа, а `ui-spec` это сорок килобайт и двадцать шесть секций. Ответ
/// выводится соединением таблиц, а не поиском подстроки в прозе.
async fn сказано_в(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    id: &str,
) -> Result<Value, crate::db::Fail> {
    let rows = client
        .query(
            "SELECT n.entity_kind, n.entity_name, s.ord, s.title, n.caveated, n.role
               FROM named_id_role n
               LEFT JOIN project_document_sections s
                 ON s.project_id = n.project_id AND s.entity_kind = n.entity_kind
                AND s.entity_name = n.entity_name AND s.ord = n.section_ord
              WHERE n.project_id = $1 AND n.said_id = $2
              ORDER BY n.entity_kind, n.entity_name, s.ord",
            &[&project, &id],
        )
        .await?;
    if rows.is_empty() {
        return Ok(Value::Null);
    }
    let всего = rows.len();
    // «Определяет» идёт первым: на вопрос «где это задано» ответ один, а
    // упоминаний два десятка, и без порядка первый ответ был случайным.
    let определяет: Vec<Value> = rows
        .iter()
        .filter(|r| r.get::<_, String>(5) == "defines")
        .map(|r| json!({ "kind": r.get::<_, String>(0), "name": r.get::<_, String>(1),
                         "section": r.get::<_, Option<i32>>(2),
                         "title": r.get::<_, Option<String>>(3).unwrap_or_default() }))
        .collect();
    let места: Vec<Value> = rows
        .iter()
        .take(40)
        .map(|r| {
            let kind: String = r.get(0);
            let name: String = r.get(1);
            let ord: Option<i32> = r.get(2);
            let title: Option<String> = r.get(3);
            json!({
                "kind": kind, "name": name,
                "section": ord,
                "title": title.unwrap_or_default(),
                "caveated": r.get::<_, bool>(4),
                "role": r.get::<_, String>(5),
                "by": format!("mh call section kind={} id={} ord={}", kind, name,
                              ord.map(|o| o.to_string()).unwrap_or_default()),
            })
        })
        .collect();
    Ok(json!({ "count": всего, "definedIn": определяет, "where": места,
               "means": if всего > 40 { "показаны первые сорок" } else { "" } }))
}

/// Строка сущности из её предметной таблицы — без проекта и адреса.
///
/// Строка приходит текстом JSON: типа `json` в tokio-postgres нет без
/// отдельной возможности, а лишняя возможность ради одной колонки — плата
/// большая, чем один разбор.
async fn row_of(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    id: &str,
) -> Result<Option<Value>, Miss> {
    let Some((table, id_col, _)) = table_of(kind) else { return Ok(None) };
    let sql = format!(
        "SELECT row_to_json(x)::text FROM (SELECT * FROM {table} WHERE project_id = $1 AND {id_col} = $2) x"
    );
    let rows = client.query(&sql, &[&project, &id]).await?;
    let Some(r) = rows.first() else { return Ok(None) };
    let mut row: Value = serde_json::from_str(&r.get::<_, String>(0)).map_err(|e| Miss::Db(e.to_string()))?;
    if let Some(map) = row.as_object_mut() {
        // Ни проекта, ни адреса наружу: первое известно спрашивающему,
        // второго у сущности нет.
        map.remove("project_id");
        map.remove("path");
    }
    Ok(Some(row))
}

pub async fn entity(pool: &Pool, kinds: &Kinds, project: &str, kind: &str, id: Option<&str>) -> Result<Value, Miss> {
    let Some(k) = kinds.get(kind) else {
        return Err(Miss::NoKind(kind.to_owned()));
    };
    let client = crate::db::conn(pool).await?;

    if k.is_inner() {
        let id = id.ok_or_else(|| Miss::NoEntity(kind.to_owned(), String::new()))?;
        if table_of(kind).is_none() {
            return Err(Miss::Unprojected(kind.to_owned()));
        }
        let row = row_of(&*client, project, kind, id)
            .await?
            .ok_or_else(|| Miss::NoEntity(kind.to_owned(), id.to_owned()))?;
        // Внутренняя сущность живёт строкой в чужом документе — связи считаются
        // по нему же: у неё своего документа нет.
        // «Хозяина не нашли» — это пустые имена, а «спросить не вышло» —
        // отказ. Одним `unwrap_or_else` перегрузка превращалась в связи,
        // посчитанные для чужого хозяина, и ответ приходил без ошибки.
        let (ok, oi) = match Box::pin(locate_at(&*client, kinds, project, kind, Some(id))).await {
            Ok(where_) => where_,
            Err(e @ (Miss::Busy(_) | Miss::Db(_))) => return Err(e),
            Err(_) => (String::new(), String::new()),
        };
        let rel = relations_of(&*client, project, kind, id, &ok, &oi, true).await?;
        return Ok(json!({ "kind": kind, "id": id, "entity": row, "relations": rel,
                          "live": живое(&*client, project, kind, id).await?,
                          "saidIn": сказано_в(&*client, project, id).await? }));
    }

    let (owner_kind, owner_name) = locate_at(&*client, kinds, project, kind, id).await?;
    // Сущность записана СТРОКОЙ В ЧУЖОМ документе: вопрос `Q-83` живёт в
    // реестре. Дверь отдавала реестр целиком — 88 КБ, — а состояния, ответа и
    // того, чем вопрос закрыт, в ответе не было. Отдаётся строка сущности, а
    // документ — ссылкой, как у внутренней.
    if let (Some(id), true) = (id, owner_kind != kind) {
        if let Some(row) = row_of(&*client, project, kind, id).await? {
            let doc = client
                .query_opt(
                    "SELECT revision, updated_at, updated_by FROM project_documents
                      WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                    &[&project, &owner_kind, &owner_name],
                )
                .await?;
            let revision: Option<i64> = doc.as_ref().map(|r| r.get(0));
            let rel = relations_of(&*client, project, kind, id, &owner_kind, &owner_name, true).await?;
            return Ok(json!({ "kind": kind, "id": id, "entity": row, "relations": rel,
                              "revision": revision,
                              "updatedAt": doc.as_ref().map(|r| r.get::<_, i64>(1)),
                              "updatedBy": doc.as_ref().map(|r| r.get::<_, String>(2)),
                              "document": { "kind": owner_kind, "name": owner_name, "revision": revision },
                              "live": живое(&*client, project, kind, id).await?,
                              "saidIn": сказано_в(&*client, project, id).await? }));
        }
    }
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
    let rel = relations_of(&*client, project, kind, id.unwrap_or(""), &owner_kind, &owner_name, false).await?;
    Ok(json!({
        "kind": kind,
        "id": id.unwrap_or(kind),
        "content": content,
        "relations": rel,
        "live": живое(&*client, project, kind, id.unwrap_or(&owner_name)).await?,
        "saidIn": сказано_в(&*client, project, id.unwrap_or(&owner_name)).await?,
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
    let client = crate::db::conn(pool).await?;

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

    let (owner_kind, owner_name) = locate_at(&*client, kinds, project, kind, id).await?;
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
) -> Result<bool, Miss> {
    let client = crate::db::conn(pool).await?;
    matches_id_at(&*client, project, kind, k, id).await
}

fn сломан_запомнить(куда: &mut Option<Miss>, kind: &str, образец: &str, e: &regex::Error) {
    if куда.is_none() {
        *куда = Some(Miss::Refused(format!(
            "образец имени вида {kind} не разбирается: {образец} — {e}"
        )));
    }
}

pub async fn matches_id_at(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
    k: &crate::kinds::Kind,
    id: &str,
) -> Result<bool, Miss> {
    let own = образцы_вида(client, project, kind).await?;
    подходит_образцу(&own, kind, k, id)
}

/// Образцы имени, объявленные НАБОРОМ. Читаются один раз на перечень: по
/// запросу на имя перечень из трёхсот документов давал триста одинаковых
/// запросов в схему и держал соединение всё это время — той самой хваткой,
/// от которой лечится пул.
pub async fn образцы_вида(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
) -> Result<Vec<String>, Miss> {
    Ok(client
        .query("SELECT value FROM scheme($1) WHERE role = $2", &[&project, &format!("id.{kind}")])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect())
}

/// Подходит ли имя: объявленным образцам набора, а без них — общей раскладке.
///
/// Образцы читаются, ПОКА имя не подошло: один кривой образец в объявлении
/// иначе уносит весь вид, включая имена, которые подходят соседнему. Сам
/// кривой образец называется, только если без него имя не подошло, — иначе
/// сломанное объявление отвечало «нет такой сущности» на КАЖДОЕ имя вида, и
/// чинить шли имя вместо образца.
pub fn подходит_образцу(own: &[String], kind: &str, k: &crate::kinds::Kind, id: &str) -> Result<bool, Miss> {
    if own.is_empty() {
        return Ok(matches_id(k, id));
    }
    let mut сломан: Option<Miss> = None;
    for образец in own {
        match regex::Regex::new(образец) {
            Ok(re) if re.is_match(id) => return Ok(true),
            Ok(_) => {}
            Err(e) => сломан_запомнить(&mut сломан, kind, образец, &e),
        }
    }
    match сломан {
        Some(e) => Err(e),
        None => Ok(false),
    }
}

#[cfg(test)]
mod links_tests {
    use super::{связи_или_пусто, Miss};

    /// «Связей у вида нет» — не приговор сущности. `links_at` знает семь видов,
    /// и его «не спроецирован» отвечал за ВЕСЬ ответ: живой вопрос, лежащий в
    /// таблице, дверь объявляла невыводимым видом.
    #[test]
    fn a_kind_without_links_is_still_an_entity() {
        let пусто = связи_или_пусто(Err(Miss::Unprojected("question".to_owned())))
            .expect("вид без перечня связей — не отказ");
        assert!(пусто.is_null(), "у вида без перечня связей ответ — пусто");
        let есть = связи_или_пусто(Ok(serde_json::json!({ "sets": {} }))).expect("связи отдаются как есть");
        assert_eq!(есть, serde_json::json!({ "sets": {} }));
        assert!(
            matches!(связи_или_пусто(Err(Miss::Busy("занято".to_owned()))), Err(Miss::Busy(_))),
            "перегрузка остаётся отказом: пустые связи при ней — тихая неправда"
        );
        assert!(
            matches!(связи_или_пусто(Err(Miss::Db("нет таблицы".to_owned()))), Err(Miss::Db(_))),
            "ошибка базы остаётся отказом"
        );
    }
}
