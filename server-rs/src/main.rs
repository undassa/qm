//! Сервер харнеса: отдаёт интерфейсу корпус из Postgres.
//!
//! Рубеж первый — только чтение документов: `contexts`, `documents`, `document`.
//! Границы и порядок работы описаны в `DESIGN.md` рядом.


use mh_server::{api, db, instrument, kinds, mcp, parse, projector, reproject, watch};
use mh_server::db::Says;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use tower_http::services::{ServeDir, ServeFile};

/// Переменная окружения, без которой сервер не поднимается.
///
/// Не пускать всех при отсутствии секрета — единственное безопасное поведение:
/// «пока не настроено, пускаем» превращает недонастроенный сервер в открытый.
fn required(names: &[&str]) -> String {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    eprintln!("mh-server: не задано {}; без этого сервер не поднимается", names.join(" или "));
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    // Логи — в stderr, и это не вкус. У подкоманды `mcp` stdout занят
    // протоколом: одна строка лога, попавшая туда, делает ответ неразбираемым,
    // и выглядит это как поломка сервера, а не как поломка вывода.
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let url = required(&["MH_DB_URL"]);
    // Тот же секрет, что у портала, и под тем же именем: портал выписывает
    // личность, этот сервер её проверяет. Завести для одного секрета второе имя
    // значит однажды сменить его в одном месте и не сменить в другом — и вход
    // сломается тихо, отказом «чужая подпись» на верных токенах.
    let secret = required(&["MH_IDENTITY_SECRET", "PORTAL_IDENTITY_SECRET"]);
    let port: u16 = std::env::var("MH_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8200);
    let web = PathBuf::from(std::env::var("MH_WEB_DIR").unwrap_or_else(|_| "../web/dist".into()));

    // Сколько соединений держать — зависит от машины: на ней живут и донорские
    // службы того же Postgres, и число агентов меняется. Умолчание рассчитано
    // на `max_connections` по умолчанию (100) с запасом для соседей.
    let size: usize = std::env::var("MH_DB_POOL").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(16);
    let pool = match db::pool(&url, size) {
        Ok(pool) => pool,
        Err(why) => {
            eprintln!("mh-server: {why}");
            std::process::exit(2);
        }
    };

    // Таблицы — ДО раскладки видов: раскладка теперь одна из них.
    if let Err(e) = projector::ensure(&pool).await {
        eprintln!("mh-server: таблицы проекций не заводятся: {e}");
        std::process::exit(2);
    }

    // Прибор — из репозитория, и раскладывается он ДО всего остального: пункт
    // гейта, снятый слиянием, не должен пережить выкладку ни одним замером.
    //
    // РАСКЛАДЫВАЕТ ТОЛЬКО ВЫКЛАДКА, а не всякая подкоманда. Подкоманду зовут из
    // любого дерева и любой сборки; устаревший двоичный файл вернул бы прибор к
    // своему объявлению и снял бы замеры того, чего в нём нет, — тихий откат
    // выкладки от команды, которая просто хотела спросить дверь.
    if std::env::args().nth(1).is_none() {
        match instrument::apply(&pool).await {
            Ok(v) => eprintln!("mh-server: прибор разложен: {v}"),
            Err(why) => {
                eprintln!("mh-server: прибор не раскладывается: {why}");
                std::process::exit(2);
            }
        }
    }

    // Раскладка видов — ИЗ БАЗЫ, куда её положило объявление репозитория.
    //
    // Файла рядом (`MH_CORPUS_LAYOUT`) больше нет. Он заводил раскладку из чужого
    // дерева — что в ней лежит, зависело от того, чей путь стоял в окружении, — и
    // писал её РАЗОБРАННОЙ: в записи семнадцать ключей, а разбор знает десять,
    // и `reopens` с `proves` он стирал молча. Проверено на себе 2026-09-18:
    // сорок восемь значений у двадцати видов, восстановлены из часового снимка.
    let kinds = match kinds::Kinds::from_db(&pool).await {
        Ok(k) if !k.is_empty() => k,
        Ok(_) => {
            eprintln!("mh-server: раскладка видов пуста, хотя прибор только что разложен: \
                       это несогласие базы с объявлением, а не пустой проект");
            std::process::exit(2);
        }
        Err(why) => {
            eprintln!("mh-server: {why}");
            std::process::exit(2);
        }
    };

    let app = api::App {
        admit_cap: size.saturating_sub(1).max(1),
        admit: Arc::new(tokio::sync::Semaphore::new(size.saturating_sub(1).max(1))),
        pool,
        secret: Arc::new(secret.into_bytes()),
        kinds: Arc::new(kinds),
        edge: std::env::var("MH_EDGE_SECRET").ok().filter(|s| !s.trim().is_empty()).map(|secret| api::Edge { secret }),
    };

    // Подкоманда `key-add`: выдать сессии её собственный секрет.
    //
    // ПОДКОМАНДОЙ, А НЕ ДВЕРЬЮ, и это существенно. Дверь стояла бы за общим
    // секретом края — то есть всякая сессия выписывала бы себе ключ с любым
    // именем, и смысл «имя из секрета» пропал бы в первый же день. Выдаёт тот,
    // кто дотянулся до машины; после переезда сессий в контейнеры это будет
    // только пускатель и владелец.
    //
    // Секрет печатается ОДИН раз: в базе лежит отпечаток, и повторить его
    // нечем — это не потеря, а свойство.
    if std::env::args().nth(1).as_deref() == Some("key-add") {
        let mut rest = std::env::args().skip(2);
        let (session, principal) = (rest.next().unwrap_or_default(), rest.next().unwrap_or_default());
        let project = rest.next().unwrap_or_default();
        let why = rest.collect::<Vec<_>>().join(" ");
        if session.is_empty() || principal.is_empty() {
            eprintln!("mh-server key-add <сессия> <принципал> [набор] [зачем]");
            std::process::exit(2);
        }
        match projector::key_add(&app.pool, &session, &principal, &project, "key-add", &why).await {
            Ok((secret, said)) => {
                println!("{said}");
                println!("MH_EDGE_SECRET={secret}");
                eprintln!("mh-server: секрет показан один раз; в базе лежит только его отпечаток");
            }
            Err(e) => {
                eprintln!("ключ не выдан: {}", e.says());
                std::process::exit(1);
            }
        }
        return;
    }

    // Подкоманда `key-drop`: снять ключи сессии. Строка остаётся — «кто ходил
    // этим ключом» спрашивают после снятия, а не до.
    if std::env::args().nth(1).as_deref() == Some("key-drop") {
        let mut rest = std::env::args().skip(2);
        let session = rest.next().unwrap_or_default();
        let why = rest.collect::<Vec<_>>().join(" ");
        if session.is_empty() {
            eprintln!("mh-server key-drop <сессия> [почему]");
            std::process::exit(2);
        }
        match projector::key_drop(&app.pool, &session, &why).await {
            Ok(n) => println!("снято ключей: {n}"),
            Err(e) => {
                eprintln!("ключ не снят: {}", e.says());
                std::process::exit(1);
            }
        }
        return;
    }

    // Подкоманда `parse-check`: сверка порта разбора с тем, что в базе оставил
    // донор. Одноразовая по замыслу, но остаётся: порт, сошедшийся однажды,
    // может разойтись при первой же правке.
    if std::env::args().nth(1).as_deref() == Some("parse-check") {
        let project = std::env::var("MH_PROJECT").unwrap_or_default();
        if project.is_empty() {
            eprintln!("mh-server parse-check: не задан MH_PROJECT — сверять нечего");
            std::process::exit(2);
        }
        match parse::check_against_donor(&app.pool, &project).await {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
            Err(e) => {
                eprintln!("сверка не прошла: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Подкоманда `call`: любая дверь, позванная одним прогоном. Затем же, зачем
    // `rebuild` и `reproject`, — прогнать её отдельно от сети и посмотреть, что
    // она даёт, не поднимая сервер и не трогая работающий.
    //
    // Сборщик сюда не доезжает: подкоманды возвращаются до `watch::spawn`, и
    // второй пересчёт наперегонки с боевым не заводится.
    if std::env::args().nth(1).as_deref() == Some("call") {
        let project = std::env::var("MH_PROJECT").unwrap_or_default();
        let Some(name) = std::env::args().nth(2) else {
            eprintln!("mh-server call: не названа дверь");
            std::process::exit(2);
        };
        let args = match std::env::args().nth(3) {
            None => serde_json::json!({}),
            Some(json) => match serde_json::from_str(&json) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("mh-server call: доводы не разбираются как JSON: {e}");
                    std::process::exit(2);
                }
            },
        };
        // Глубина соединений считается и здесь: подкоманда зовёт те же двери,
        // а счёт живёт у задачи — без обёртки он видел бы ноль там, где двое.
        let out = db::counting(
            mcp::Mcp {
                pool: app.pool.clone(),
                kinds: app.kinds.clone(),
                project,
                author: "cli".into(),
            }
            .call(&name, &args),
        )
        .await;
        println!("{}", out["content"][0]["text"].as_str().unwrap_or(""));
        if out["isError"] == serde_json::json!(true) {
            std::process::exit(1);
        }
        return;
    }

    // Подкоманда `rebuild`: собственные проекции сервера, без записи документа.
    // Нужна затем же, зачем `parse-check`, — прогнать сборку отдельно от правки
    // и посмотреть, что она даёт.
    if std::env::args().nth(1).as_deref() == Some("rebuild") {
        let project = std::env::var("MH_PROJECT").unwrap_or_default();
        // Пустой проект — не «проект по умолчанию», а не заданный: сборка с ним
        // пишет строки, которые потом не удаляет никакая пересборка, потому что
        // они не принадлежат ни одному проекту. Так уже вышло: шесть снятых
        // терминов и семь заявленных предметов легли под пустым именем.
        if project.is_empty() {
            eprintln!("mh-server rebuild: не задан MH_PROJECT — собирать нечего");
            std::process::exit(2);
        }
        if let Err(e) = db::counting(projector::rebuild_before(&app.pool, &project)).await {
            eprintln!("подготовка не прошла: {}", e.says());
            std::process::exit(1);
        }
        match projector::rebuild(&app.pool, &project).await {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
            Err(e) => {
                eprintln!("сборка не прошла: {}", e.says());
                std::process::exit(1);
            }
        }
        return;
    }

    // Подкоманда `reproject`: перенесённые проекции — те, что прежде считал
    // донор. Сверяется снимком таблиц до и после.
    if std::env::args().nth(1).as_deref() == Some("reproject") {
        let project = std::env::var("MH_PROJECT").unwrap_or_default();
        if project.is_empty() {
            eprintln!("mh-server reproject: не задан MH_PROJECT — пересобирать нечего");
            std::process::exit(2);
        }
        // Исход записывается и здесь: подкоманда — та же пересборка, и упавшая
        // она оставляет те же недособранные проекции.
        match db::counting(reproject::reproject(&app.pool, &project)).await {
            Ok(v) => {
                projector::note_reproject(&app.pool, &project, true, "").await;
                // ПАРА, А НЕ ПОЛОВИНА. Пересборка снимает из плана красные задачи
                // и состояния — их кладёт СБОРКА, и между двумя командами база
                // неполна. Дверь `mh call reproject` делает обе половины и всегда
                // делала; подкоманда останавливалась на первой и печатала успех.
                //
                // Стоило дня: красные задачи «исчезли», пункт, читающий их,
                // замолчал, и гейт от этого позеленел.
                //
                // Половина — по явному слову, и она говорит, чем это кончится.
                let half = std::env::args().any(|a| a == "--half");
                let after = if half {
                    serde_json::json!({
                        "warning": "СДЕЛАНА ПОЛОВИНА. В плане сейчас нет красных задач и \
                                    состояний: их кладёт `mh-server rebuild`. Пока он не \
                                    прогнан, всё прочитанное соврёт.",
                    })
                } else {
                    if let Err(e) = db::counting(projector::rebuild_before(&app.pool, &project)).await {
                        eprintln!("подготовка сборки не прошла: {}", e.says());
                        std::process::exit(1);
                    }
                    match projector::rebuild(&app.pool, &project).await {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("сборка не прошла: {}", e.says());
                            std::process::exit(1);
                        }
                    }
                };
                println!("{}", serde_json::to_string_pretty(
                    &serde_json::json!({ "reproject": v, "rebuild": after })).unwrap_or_default());
            }
            Err(e) => {
                let said = e.says();
                projector::note_reproject(&app.pool, &project, false, &said).await;
                eprintln!("пересборка не прошла: {said}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Пересчёт гейтов при изменении набора. Живёт он только здесь, в сетевом
    // сервере. Правку, пришедшую от клиента, он всё равно увидит: клиент ходит
    // сюда же по сети, и отметка ложится в ту же таблицу.
    watch::spawn(app.pool.clone());

    let index = web.join("index.html");
    let router = api::routes(app)
        // ЗАГОЛОВКИ КЭША СТАВЯТСЯ ОДНИМ СЛОЕМ НА ОБА ПУТИ, а не вложенными
        // роутерами: вложение сдвинуло разбор адреса, и `/next/` — ровно тот,
        // куда корень и приводит человека, — начал отвечать 401. Проверено
        // курлом сразу после правки, до того как это увидел кто-то живой.
        .nest_service("/next", ServeDir::new(&web).fallback(ServeFile::new(&index)))
        // Корень — это то, куда портал приводит человека после входа. Пустой
        // 404 на этом месте читается как «всё сломалось», хотя интерфейс жив
        // одной строкой ниже по адресу.
        .route("/", axum::routing::get(|| async { axum::response::Redirect::to("/next/") }));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mh-server: порт {port} не занять: {e}");
            std::process::exit(2);
        }
    };
    println!("mh-server: слушает http://{addr}/next/");
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("mh-server: остановился: {e}");
        std::process::exit(1);
    }
}
