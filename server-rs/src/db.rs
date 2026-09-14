//! Пул к Postgres и единственное место, где решается, как до него добраться.
//!
//! Адрес читается из `MH_DB_URL`, а не из `DATABASE_URL`. Имя выбрано разным
//! намеренно: в этом окружении `DATABASE_URL` уже что-то значит для прежнего
//! рантайма, и сервер, молча подхвативший чужую переменную, однажды прочитает
//! не ту базу — а выглядеть это будет как пустой корпус, а не как ошибка настройки.

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::{Config, NoTls};

pub fn pool(url: &str, size: usize) -> Result<Pool, String> {
    let config: Config = url.parse().map_err(|e| format!("MH_DB_URL не разбирается: {e}"))?;
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig { recycling_method: RecyclingMethod::Fast },
    );
    Pool::builder(manager)
        .max_size(size)
        .wait_timeout(Some(std::time::Duration::from_secs(30)))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .map_err(|e| format!("пул не собрался: {e}"))
}
