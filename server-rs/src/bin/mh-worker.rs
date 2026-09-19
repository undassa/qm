//! Воркер пульта как служба: круг за кругом отвечает в беседах наборов и
//! ведёт прогоны задач. Конфигурация — тот же runner.json, что у прежнего
//! воркера на Python: наборы, их адреса проектов и деревья.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("MH_RUNNER_CONFIG").ok())
        .unwrap_or_else(|| "scripts/runner.json".into());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("mh-worker: конфиг {path} не читается: {e}");
        std::process::exit(2);
    });
    let config: mh_server::worker::Config = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("mh-worker: конфиг {path} не разбирается: {e}");
        std::process::exit(2);
    });
    for b in &config.projects {
        if !std::path::Path::new(&b.repo).is_dir() {
            eprintln!("mh-worker: нет дерева {} — воркеру нечего открыть", b.repo);
            std::process::exit(2);
        }
    }
    let url = std::env::var("MH_DB_URL").unwrap_or_else(|_| {
        eprintln!("mh-worker: не задан MH_DB_URL; без этого воркер не поднимается");
        std::process::exit(2);
    });
    let pool = match mh_server::db::pool(&url, 4) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("mh-worker: {why}");
            std::process::exit(2);
        }
    };
    let kinds = match mh_server::kinds::Kinds::from_db(&pool).await {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("mh-worker: виды не раскладываются: {e:?}");
            std::process::exit(2);
        }
    };
    let claude = std::env::var("MH_WORKER_CLAUDE").unwrap_or_else(|_| "claude".into());
    let worker = mh_server::worker::Worker::new(pool, kinds, claude);
    println!("воркер пульта: наборов {}, круг {} с", config.projects.len(), mh_server::worker::TICK_S);
    loop {
        for bundle in &config.projects {
            // Круг не должен уносить воркер: следующий круг важнее.
            worker.chats(bundle).await;
            worker.drive_runs(bundle).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(mh_server::worker::TICK_S)).await;
    }
}
