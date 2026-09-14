//! Маршруты и формы ответов.
//!
//! Всё здесь — чтение. Записи нет не потому, что до неё не дошли руки: пока не
//! названо, кто и по какому правилу правит документ, запись завела бы второй
//! источник истины рядом с git.

use std::{sync::Arc, time::SystemTime};

use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use deadpool_postgres::Pool;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{corpus, documents, entities, entities::Miss, identity, projects};

/// Личность, утверждённая краем.
///
/// Портал ушёл вместе с донором, и подписывать личность стало нечем. Кто-то
/// должен пускать людей: пускает Caddy своим `basic_auth` — тем самым, что уже
/// стережёт `/board` и `/mh` на этой машине, и о котором в его же настройке
/// сказано «кто пройдёт эту строку, владеет машиной».
///
/// Сервер верит краю не на слово: край предъявляет общий секрет, а сам сервер
/// слушает только петлю, и снаружи в него не постучаться минуя Caddy. Это
/// **меньше**, чем подпись портала: она удостоверяла личность, а это —
/// утверждение края о ней. Разница названа здесь, чтобы её не забыли.
#[derive(Clone)]
pub struct Edge {
    pub secret: String,
}

#[derive(Clone)]
pub struct App {
    pub pool: Pool,
    pub secret: Arc<Vec<u8>>,
    pub kinds: Arc<crate::kinds::Kinds>,
    /// Пусто — краю не верим вовсе, и остаётся только подписанная личность.
    pub edge: Option<Edge>,
}

/// Кто пишет. Личность проверена посредником, сюда она доходит уже разобранной.
#[derive(Clone)]
pub struct Author(pub String);

/// Отказ называется словом, а не только числом: интерфейс уже различает
/// `not_found` и `upstream_error`, и по одному лишь коду он этого не сможет.
pub enum Failure {
    NoKind(String),
    NoEntity(String, String),
    /// Вид есть, проекции у него нет. Отказ с именем, а не пустой список.
    Unprojected(String),
    NotFound,
    NoSuchSection,
    /// Документ успели поправить: писавший видел не ту правку, что лежит сейчас.
    Conflict,
    InvalidPath,
    Unauthorized,
    Expired,
    Upstream(String),
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (code, name, note) = match self {
            Failure::NoKind(k) => (StatusCode::NOT_FOUND, "no_such_kind", format!("нет такого вида: {k}")),
            Failure::NoEntity(k, id) => (
                StatusCode::NOT_FOUND,
                "no_such_entity",
                if id.is_empty() {
                    format!("документа вида {k} в наборе нет: у одиночного вида имени не бывает, \
                             заводить надо документ")
                } else { format!("нет такой сущности: {k} {id}") },
            ),
            // Отдельное слово: «нет проекции» — это не «ничего не нашлось».
            // Агент, принявший первое за второе, построит задачу без контекста.
            Failure::Unprojected(k) => (
                StatusCode::NOT_IMPLEMENTED,
                "unprojected",
                format!("вид {k} в базу не спроецирован: ответ неизвестен, а не пуст"),
            ),
            Failure::NotFound => (StatusCode::NOT_FOUND, "not_found", String::new()),
            // «Нет такого раздела» — не «нет такого документа»: по первому идут
            // проверять якорь, по второму — путь.
            Failure::NoSuchSection => (StatusCode::NOT_FOUND, "no_such_section", String::new()),
            Failure::Conflict => (
                StatusCode::CONFLICT,
                "conflict",
                "документ изменился с той правки, которую вы видели".to_owned(),
            ),
            Failure::InvalidPath => (StatusCode::BAD_REQUEST, "invalid_path", String::new()),
            Failure::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", String::new()),
            // Протухший токен — не то же, что чужой: первый лечится перевыпуском,
            // второй значит, что стучится не тот. Одно слово на оба увело бы
            // интерфейс чинить не ту беду.
            Failure::Expired => (StatusCode::UNAUTHORIZED, "identity_expired", String::new()),
            Failure::Upstream(e) => (StatusCode::BAD_GATEWAY, "upstream_error", e),
        };
        let mut body = json!({ "error": name });
        if !note.is_empty() {
            body["message"] = json!(note);
        }
        (code, Json(body)).into_response()
    }
}

impl From<Miss> for Failure {
    fn from(m: Miss) -> Self {
        match m {
            Miss::NoKind(k) => Failure::NoKind(k),
            Miss::NoEntity(k, id) => Failure::NoEntity(k, id),
            Miss::Unprojected(k) => Failure::Unprojected(k),
            Miss::Refused(почему) => Failure::Upstream(почему),
            Miss::Db(e) => Failure::Upstream(e),
        }
    }
}

impl From<tokio_postgres::Error> for Failure {
    fn from(e: tokio_postgres::Error) -> Self {
        Failure::Upstream(e.to_string())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Личность лежит либо в заголовке, либо в печенье `mh_identity`.
///
/// Заголовок — для того, кто зовёт ручку сам. Печенье — для браузера: он
/// заголовков не добавляет, а собственного входа у этого сервера пока нет, и
/// без печенья интерфейс не смог бы открыть ни одного документа. Это временный
/// путь, и он назван временным в `DESIGN.md`, чтобы не остаться навсегда.
///
/// Подделки с чужой страницы это не открывает: все ручки — чтение, а ответ
/// чужому источнику не отдаётся, потому что заголовков CORS сервер не ставит.
pub fn token_of(header: Option<&str>, cookie: Option<&str>) -> Option<String> {
    if let Some(token) = header.map(str::trim).filter(|t| !t.is_empty()) {
        return Some(token.to_owned());
    }
    cookie?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == "mh_identity")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|t| !t.is_empty())
}

/// Личность проверяется на каждом запросе к `/api`. Её отсутствие — это отказ, а
/// не гость: у сервера нет анонимного режима, и заводить его втихую значило бы
/// открыть корпус всем, кто дошёл до порта.
async fn require_identity(State(app): State<App>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let token = token_of(
        headers.get("x-portal-identity").and_then(|v| v.to_str().ok()),
        headers.get("cookie").and_then(|v| v.to_str().ok()),
    );
    let Some(token) = token else {
        // Край мог пустить человека сам. Проверяется общий секрет, а не сам факт
        // заголовка: заголовок подделает кто угодно, дотянувшийся до порта.
        if let Some(edge) = &app.edge {
            let shown = headers.get("x-mh-edge").and_then(|v| v.to_str().ok()).unwrap_or("");
            let principal = headers
                .get("x-mh-principal")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            // Секрет отвечает «это край», но не «этому человеку можно»: имя край
            // подставляет какое настроено. Допущенные объявлены в базе, и вход
            // записывается — как допущенный, так и отказанный.
            if !shown.is_empty() && shown == edge.secret && !principal.is_empty() {
                match crate::projector::edge_admits(&app.pool, &principal).await {
                    Ok(true) => {
                        let mut request = request;
                        request.extensions_mut().insert(Author(principal));
                        return next.run(request).await;
                    }
                    Ok(false) => return Failure::Unauthorized.into_response(),
                    Err(e) => return Failure::Upstream(format!("вход не проверен: {e}")).into_response(),
                }
            }
        }
        return Failure::Unauthorized.into_response();
    };
    match identity::verify(&token, &app.secret, now_ms()) {
        Ok(claims) => {
            // Имя писавшего попадёт в летопись правок, поэтому берётся из
            // подписанной личности, а не из тела запроса: тело подделает кто угодно.
            let mut request = request;
            request.extensions_mut().insert(Author(claims.p));
            next.run(request).await
        }
        Err(identity::Refusal::Expired) => Failure::Expired.into_response(),
        Err(_) => Failure::Unauthorized.into_response(),
    }
}



#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<i64>,
}





async fn contexts(State(app): State<App>) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(json!({ "contexts": projects::list(&app.pool).await? })))
}

#[derive(serde::Deserialize)]
struct WhoseQuery {
    path: String,
}

/// Чей это репозиторий. Маршрут БЕЗ проекта в пути — в этом и смысл: клиент
/// спрашивает, кому принадлежит дерево, в котором стоит, вместо того чтобы
/// читать ответ файлом из того же дерева.
async fn whose(
    State(app): State<App>,
    Query(q): Query<WhoseQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(crate::projector::whose_repo(&app.pool, &q.path).await?))
}

/// Объявить проект: имя и репозиторий.
async fn project_declare(
    State(app): State<App>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, Failure> {
    let g = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    Ok(Json(crate::projector::declare_project(&app.pool, &g("id"), &g("name"), &g("repo"),
                                              &g("by"),
                                                 body.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await?))
}

async fn search(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let hits = corpus::search(&app.pool, &project, &q.q, limit).await?;
    Ok(Json(json!({ "hits": hits, "count": hits.len() })))
}

/// Дописать к ответу писателя пересборку собственных проекций.
///
/// Донор пересобирает свои таблицы внутри записи; эти — следом, в том же
/// вызове. Иначе вернулось бы ровно то окно, ради закрытия которого пересборка
/// и делается синхронной: документ новый, часть проекций вчерашняя.
async fn with_own_projections(
    app: &App,
    project: &str,
    mut out: serde_json::Value,
) -> Result<serde_json::Value, Failure> {
    let written = matches!(out.get("status").and_then(|s| s.as_str()), Some("written") | Some("deleted"));
    if written {
        let own = finish_write(app, project).await?;
        if let Some(map) = out.as_object_mut() {
            map.insert("own".into(), own);
        }
    }
    Ok(out)
}

/// Конвейер после записи: до-проход → предметные проекции → после-проход.
///
/// Порядок существен. Предметные проекции читают `task_requirement`, а она
/// считается до-проходом: посчитанная после них, она отдала бы им прошлый круг.
///
/// Здесь больше нет донора. Тридцать таблиц, которые считал Node-процесс,
/// считаются в этом же сервере и сверены с ним до хеша упорядоченного дампа —
/// на пустых таблицах, а не поверх донорских.
pub async fn finish_write(app: &App, project: &str) -> Result<serde_json::Value, Failure> {
    let before = crate::projector::rebuild_before(&app.pool, project).await?;
    let subject = crate::reproject::reproject(&app.pool, project).await?;
    let after = crate::projector::rebuild(&app.pool, project).await?;
    let generated = crate::projector::check_generated(&app.pool, project).await?;
    Ok(json!({ "before": before, "subject": subject, "after": after, "generated": generated }))
}

/// Ответ писателя переводится в отказ там, где он отказ.
fn writer_answer(out: serde_json::Value) -> Result<Json<serde_json::Value>, Failure> {
    match out.get("status").and_then(|s| s.as_str()) {
        Some("conflict") => Err(Failure::Conflict),
        Some("not_found") => Err(Failure::NotFound),
        Some("no_such_section") => Err(Failure::NoSuchSection),
        Some("invalid_path") => Err(Failure::InvalidPath),
        _ => match out.get("error").and_then(|e| e.as_str()) {
            Some(e) => Err(Failure::Upstream(e.to_owned())),
            None => Ok(Json(out)),
        },
    }
}

async fn reproject(
    State(app): State<App>,
    Path(project): Path<String>,
) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(finish_write(&app, &project).await?))
}

#[derive(Deserialize)]
struct EntityQuery {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    /// `brief=true` — про сущность, но без её текста: его берут блоками.
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Deserialize)]
struct EntitySectionQuery {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    anchor: String,
}

#[derive(Deserialize)]
struct EntityPut {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    content: String,
    #[serde(rename = "expectedRevision")]
    expected_revision: Option<i64>,
}

#[derive(Deserialize)]
struct EntitySectionPut {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    anchor: String,
    body: String,
    #[serde(rename = "expectedRevision")]
    expected_revision: Option<i64>,
}

async fn list_kinds(
    State(app): State<App>,
    Path(project): Path<String>,
) -> Result<Json<serde_json::Value>, Failure> {
    let mut out = Vec::new();
    for (name, k) in &app.kinds.0 {
        let count = match entities::ids(&app.pool, &app.kinds, &project, name).await {
            Ok(list) => json!(list.len()),
            // Не ноль: ноль читается как «ничего нет», а здесь неизвестно.
            Err(Miss::Unprojected(_)) => json!(null),
            Err(e) => return Err(e.into()),
        };
        out.push(json!({
            "kind": name,
            "shape": if k.is_inner() { "inner" } else { "document" },
            "single": k.single,
            "in": k.in_kind,
            "count": count,
            "projected": count != json!(null),
        }));
    }
    Ok(Json(json!({ "kinds": out })))
}

async fn entity_ids(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<EntityQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let list = entities::ids(&app.pool, &app.kinds, &project, &q.kind).await?;
    Ok(Json(json!({ "kind": q.kind, "ids": list })))
}

async fn read_entity(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<EntityQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let v = entities::entity(&app.pool, &app.kinds, &project, &q.kind, q.id.as_deref()).await?;
    Ok(Json(if q.brief.as_deref() == Some("true") { entities::without_body(v) } else { v }))
}

async fn entity_sections(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<EntityQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let (kind, name) = entities::locate(&app.pool, &app.kinds, &project, &q.kind, q.id.as_deref()).await?;
    Ok(Json(json!({ "sections": corpus::sections(&app.pool, &project, &kind, &name).await? })))
}

async fn entity_section(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<EntitySectionQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let (kind, name) = entities::locate(&app.pool, &app.kinds, &project, &q.kind, q.id.as_deref()).await?;
    let Some(document) = documents::read(&app.pool, &project, &kind, &name).await? else {
        return Err(Failure::NoEntity(q.kind.clone(), q.id.clone().unwrap_or_default()));
    };
    let body = corpus::section_body(&app.pool, &project, &kind, &name, &q.anchor)
        .await?
        .ok_or(Failure::NoSuchSection)?;
    Ok(Json(json!({
        "kind": q.kind, "id": q.id, "anchor": q.anchor,
        "body": body, "revision": document.summary.revision
    })))
}

async fn entity_backlinks(
    State(app): State<App>,
    Path(project): Path<String>,
    Query(q): Query<EntityQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let links = entities::backlinks(&app.pool, &app.kinds, &project, &q.kind, q.id.as_deref()).await?;
    Ok(Json(json!({ "backlinks": links, "count": links.len() })))
}

async fn put_entity(
    State(app): State<App>,
    Path(project): Path<String>,
    author: Option<axum::extract::Extension<Author>>,
    Json(body): Json<EntityPut>,
) -> Result<Json<serde_json::Value>, Failure> {
    let (kind, name) = entities::locate(&app.pool, &app.kinds, &project, &body.kind, body.id.as_deref()).await?;
    let who = author.map(|a| a.0 .0).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0);
    let out = crate::store::put(&app.pool, &project, &kind, &name, &body.content, &who, body.expected_revision, now).await?;
    writer_answer(with_own_projections(&app, &project, out).await?)
}

async fn put_entity_section(
    State(app): State<App>,
    Path(project): Path<String>,
    author: Option<axum::extract::Extension<Author>>,
    Json(body): Json<EntitySectionPut>,
) -> Result<Json<serde_json::Value>, Failure> {
    let (kind, name) = entities::locate(&app.pool, &app.kinds, &project, &body.kind, body.id.as_deref()).await?;
    let who = author.map(|a| a.0 .0).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0);
    let out = crate::store::put_section(&app.pool, &project, &kind, &name, &body.anchor, &body.body, &who,
                                        body.expected_revision, now).await?;
    writer_answer(with_own_projections(&app, &project, out).await?)
}

async fn rm_entity(
    State(app): State<App>,
    Path(project): Path<String>,
    author: Option<axum::extract::Extension<Author>>,
    Json(body): Json<EntityQuery>,
) -> Result<Json<serde_json::Value>, Failure> {
    let (kind, name) = entities::locate(&app.pool, &app.kinds, &project, &body.kind, body.id.as_deref()).await?;
    let who = author.map(|a| a.0 .0).unwrap_or_default();
    let _ = who;
    let out = crate::store::remove(&app.pool, &project, &kind, &name).await?;
    writer_answer(with_own_projections(&app, &project, out).await?)
}

/// Инструменты — одним диспетчером, и он тот же, что у MCP.
///
/// Иначе поверхностей стало бы две: интерфейс спрашивал бы своим маршрутом,
/// харнес — своим инструментом, и разошлись бы они молча. Здесь имя одно, ответ
/// один, реализация одна.
///
/// Наружу отдаётся только чтение: запись идёт своими маршрутами, где есть
/// `expectedRevision` и разбор отказов.
/// Перечень ручек — тот же, что отдаёт MCP.
///
/// Нужен клиенту: он ходит в сервер по сети и не должен знать про ручки ничего
/// своего. Второй перечень рядом с первым однажды разошёлся бы, и разошёлся бы молча.
async fn list_tools(
    State(app): State<App>,
    Path(project): Path<String>,
    author: Option<axum::extract::Extension<Author>>,
) -> Result<Json<serde_json::Value>, Failure> {
    let mcp = crate::mcp::Mcp {
        pool: app.pool.clone(),
        kinds: app.kinds.clone(),
        project,
        author: author.map(|a| a.0 .0).unwrap_or_else(|| "клиент".into()),
    };
    let tools = mcp.tools();
    Ok(Json(json!({ "count": tools.len(), "tools": tools })))
}

/// Вызов ручки С ТЕЛОМ — тем самым и пишущей.
///
/// У адресной строки доводы приходят строками, и число от слова там отличают
/// догадкой; ожидаемую правку так не передать вовсе. Оттого запись и была
/// закрыта для `GET`, а не потому, что запись по сети опасна: край утверждает
/// личность, и допущенные объявлены в базе.
async fn call_tool_body(
    State(app): State<App>,
    Path((project, name)): Path<(String, String)>,
    author: Option<axum::extract::Extension<Author>>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<serde_json::Value>, Failure> {
    let args = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    let mcp = crate::mcp::Mcp {
        pool: app.pool.clone(),
        kinds: app.kinds.clone(),
        project,
        author: author.map(|a| a.0 .0).unwrap_or_else(|| "клиент".into()),
    };
    // ОБОЛОЧКА СНИМАЕТСЯ, как и на чтении. Прежде POST отдавал конверт MCP
    // целиком, а GET — сам предмет: отказ двери лежал строкой внутри
    // `content[0].text`, и читающий видел поле `content`, а не `status`.
    // Запись отчиталась бы успехом на отказе — молчание вместо причины.
    let out = mcp.call(&name, &args).await;
    let text = out["content"][0]["text"].as_str().unwrap_or("").to_owned();
    if out["isError"] == json!(true) {
        return Err(Failure::Upstream(text));
    }
    Ok(Json(
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "text": text })),
    ))
}

async fn call_tool(
    State(app): State<App>,
    Path((project, name)): Path<(String, String)>,
    Query(args): Query<std::collections::HashMap<String, String>>,
    author: Option<axum::extract::Extension<Author>>,
) -> Result<Json<serde_json::Value>, Failure> {
    const WRITES: &[&str] = &["put", "put-section", "rm", "task-state-push", "method-set"];
    if WRITES.contains(&name.as_str()) {
        return Err(Failure::Upstream(format!(
            "инструмент {name} пишет: у записи свой маршрут, с ожидаемой правкой и разбором отказов"
        )));
    }
    let mut map = serde_json::Map::new();
    for (k, v) in args {
        // Числа приходят строками из адреса; поле, которое инструмент ждёт
        // числом, разбирается здесь, а не угадывается им.
        map.insert(
            k,
            v.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| match v.as_str() {
                "true" => json!(true),
                "false" => json!(false),
                other => json!(other),
            }),
        );
    }
    let mcp = crate::mcp::Mcp {
        pool: app.pool.clone(),
        kinds: app.kinds.clone(),
        project,
        author: author.map(|a| a.0 .0).unwrap_or_else(|| "интерфейс".into()),
    };
    let out = mcp.call(&name, &Value::Object(map)).await;
    // Ответ инструмента — текст в оболочке MCP; наружу отдаётся сам предмет.
    let text = out["content"][0]["text"].as_str().unwrap_or("").to_owned();
    if out["isError"] == json!(true) {
        return Err(Failure::Upstream(text));
    }
    Ok(Json(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text }))))
}

pub fn routes(app: App) -> Router {
    Router::new()
        .route("/api/contexts", get(contexts))
        .route("/api/whose", get(whose))
        .route("/api/projects", axum::routing::post(project_declare))
        .route("/api/projects/:project/search", get(search))
        .route("/api/projects/:project/reproject", axum::routing::post(reproject))
        .route("/api/projects/:project/kinds", get(list_kinds))
        .route("/api/projects/:project/tool/:name", get(call_tool))
        .route("/api/projects/:project/tool/:name", axum::routing::post(call_tool_body))
        .route("/api/projects/:project/tools", get(list_tools))
        .route("/api/projects/:project/ids", get(entity_ids))
        .route("/api/projects/:project/entity", get(read_entity))
        .route("/api/projects/:project/entity", axum::routing::put(put_entity))
        .route("/api/projects/:project/entity", axum::routing::delete(rm_entity))
        .route("/api/projects/:project/entity/sections", get(entity_sections))
        .route("/api/projects/:project/entity/section", get(entity_section))
        .route("/api/projects/:project/entity/section", axum::routing::patch(put_entity_section))
        .route("/api/projects/:project/entity/backlinks", get(entity_backlinks))
        .layer(middleware::from_fn_with_state(app.clone(), require_identity))
        .layer(tower_http::catch_panic::CatchPanicLayer::new())
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::token_of;

    #[test]
    fn заголовок_главнее_печенья() {
        assert_eq!(token_of(Some("из-заголовка"), Some("mh_identity=из-печенья")).as_deref(), Some("из-заголовка"));
    }

    #[test]
    fn печенье_читается_когда_заголовка_нет() {
        assert_eq!(token_of(None, Some("a=1; mh_identity=токен; b=2")).as_deref(), Some("токен"));
    }

    #[test]
    fn чужое_печенье_личностью_не_считается() {
        // `mh_identity_other` начинается так же; поиск по префиксу принял бы его
        // за нашу и отдал бы корпус по чужому имени.
        assert_eq!(token_of(None, Some("mh_identity_other=токен")), None);
    }

    #[test]
    fn пустое_значение_не_личность() {
        assert_eq!(token_of(Some("  "), Some("mh_identity=")), None);
    }
}
