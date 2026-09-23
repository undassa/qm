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
        .wait_timeout(Some(std::time::Duration::from_secs(3)))
        .create_timeout(Some(std::time::Duration::from_secs(10)))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .map_err(|e| format!("пул не собрался: {e}"))
}

/// Отказ со стороны базы: занято или ответ самой базы.
///
/// Прежде соединение брали `expect`, и перегрузка кончалась паникой в 245
/// местах — с сообщением «пул отдал соединение» ровно тогда, когда он его НЕ
/// отдал. Клиент ждал тридцать секунд и получал обрыв вместо отказа: 22 агента
/// предполёта tot-ade так держали сервер час (2026-09-17). Дверь обязана
/// отвечать или отказывать, а не падать.
#[derive(Debug)]
pub enum Fail {
    /// Все соединения заняты: сервер перегружен, и повторить стоит.
    Busy(String),
    /// База не принимает соединение: повторять бессмысленно, чинить надо базу.
    /// Отдельно от занятости, потому что оператор по этим словам идёт в разные
    /// места, а клиент по ним решает, повторять ли.
    Down(String),
    Db(tokio_postgres::Error),
    /// Набор говорит то, что спроецировать нельзя, — и отказ называет что
    /// именно. Отдельно от `Db`, потому что чинить надо не базу и не повтор, а
    /// документ: сообщение Postgres об этом молчит, и час уходит на поиск
    /// строки, которую здесь можно назвать по имени.
    Corpus(String),
}

impl From<tokio_postgres::Error> for Fail {
    fn from(e: tokio_postgres::Error) -> Self {
        Fail::Db(e)
    }
}

impl From<deadpool_postgres::PoolError> for Fail {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        use deadpool_postgres::{PoolError, TimeoutType};
        match e {
            PoolError::Backend(e) => Fail::Db(e),
            // Занятость — это ТОЛЬКО ожидание свободного места. Ожидание
            // СОЗДАНИЯ соединения значит, что база его не приняла: назвать это
            // перегрузкой — отправить чинить нагрузку, которой нет.
            PoolError::Timeout(TimeoutType::Wait) => Fail::Busy(
                "все соединения с базой заняты: сервер перегружен, повторите через несколько секунд"
                    .to_owned(),
            ),
            PoolError::Timeout(TimeoutType::Create) => {
                Fail::Down("база не принимает соединение: ответа на попытку подключиться нет".to_owned())
            }
            PoolError::Timeout(TimeoutType::Recycle) => {
                Fail::Down("база не отдала соединение обратно в пул: проверка соединения не ответила".to_owned())
            }
            PoolError::Closed => Fail::Down("пул соединений закрыт: сервер останавливается".to_owned()),
            e => Fail::Down(format!("соединение не получено: {e}")),
        }
    }
}

/// Что сказать спросившему. У ответа базы берётся её собственное слово:
/// `to_string` печатает «db error», а причина лежит внутри.
pub trait Says {
    fn says(&self) -> String;
}

impl Says for tokio_postgres::Error {
    fn says(&self) -> String {
        match self.as_db_error() {
            Some(d) => format!("{}: {}", d.severity(), d.message()),
            None => self.to_string(),
        }
    }
}

impl Says for Fail {
    fn says(&self) -> String {
        match self {
            Fail::Busy(why) | Fail::Down(why) | Fail::Corpus(why) => why.clone(),
            Fail::Db(e) => e.says(),
        }
    }
}

impl Fail {
    pub(crate) fn busy(&self) -> bool {
        matches!(self, Fail::Busy(_))
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.says())
    }
}

tokio::task_local! {
    static DEPTH: std::cell::Cell<u32>;
}

/// Сколько раз соединение брали, уже держа другое. Ноль — правило соблюдено.
pub(crate) static NESTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Сколько раз отказали из-за перегрузки: не отдали соединение или не пустили
/// в дверь. Счёт живёт в памяти минуту, а потом ложится строкой в базу —
/// иначе от перегрузки не остаётся следа, который можно прочесть потом.
pub(crate) static BUSY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Снять счётчики и обнулить: возвращается то, что накопилось с прошлого раза.
pub(crate) fn strain() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (BUSY.swap(0, Relaxed), NESTED.swap(0, Relaxed))
}

/// Соединение из пула. Единственное место, где оно берётся.
///
/// Второе соединение при живом первом — способ запереть пул на себе же:
/// шестнадцать запросов держат по одному и ждут второго, и ни один не может
/// его получить. Такое место считается и называется в журнале; чинится оно
/// передачей соединения вниз, а не размером пула.
pub(crate) async fn conn(pool: &Pool) -> Result<Held, Fail> {
    let depth = DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if depth > 0 {
        NESTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!("соединение взято при живом соединении (глубина {})", depth + 1);
    }
    let held = match pool.get().await {
        Ok(held) => held,
        Err(e) => {
            let fail = Fail::from(e);
            if fail.busy() {
                BUSY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            return Err(fail);
        }
    };
    let _ = DEPTH.try_with(|d| d.set(depth + 1));
    Ok(Held { held })
}

/// Взятое соединение: считает глубину, пока живо.
#[derive(Debug)]
pub struct Held {
    held: deadpool_postgres::Object,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = DEPTH.try_with(|d| d.set(d.get().saturating_sub(1)));
    }
}

impl std::ops::Deref for Held {
    type Target = deadpool_postgres::Object;

    fn deref(&self) -> &Self::Target {
        &self.held
    }
}

impl std::ops::DerefMut for Held {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.held
    }
}

/// Считать глубину можно только внутри этой обёртки: счёт живёт у задачи.
pub async fn counting<T>(work: impl std::future::Future<Output = T>) -> T {
    DEPTH.scope(std::cell::Cell::new(0), work).await
}

#[cfg(test)]
mod tests {
    use super::{conn, pool, Fail, Says, NESTED};

    #[tokio::test]
    #[ignore = "нужна база Postgres: MH_TEST_DB_URL"]
    async fn a_full_pool_refuses_instead_of_panicking() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес базы");
        let pool = pool(&url, 1).expect("пул тестовой базы");
        let held = conn(&pool).await.expect("первое соединение");
        let start = std::time::Instant::now();
        let busy = conn(&pool).await.expect_err("второе соединение: пул пуст");
        let waited = start.elapsed();
        assert!(busy.busy(), "{}", busy.says());
        assert!(busy.says().contains("перегружен"), "{}", busy.says());
        // Срок ожидания — часть ответа, а не мелочь настройки: тридцать секунд
        // превращали перегрузку в час простоя, и цифру держит этот предел.
        assert!(waited < std::time::Duration::from_secs(5), "ждали {waited:?}: отказ должен быть скорым");
        drop(held);
        assert!(conn(&pool).await.is_ok(), "освободившееся соединение снова выдаётся");
    }

    /// Второе соединение при живом первом — то, чем запирается пул. Оно
    /// считается, и счёт виден: правило «одно соединение на запрос» иначе
    /// держится только внимательностью читающего.
    #[tokio::test]
    #[ignore = "нужна база Postgres: MH_TEST_DB_URL"]
    async fn a_second_connection_while_holding_one_is_counted() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес базы");
        let pool = pool(&url, 4).expect("пул тестовой базы");
        let was = NESTED.load(std::sync::atomic::Ordering::Relaxed);
        super::counting(async {
            let first = conn(&pool).await.expect("первое соединение");
            assert_eq!(NESTED.load(std::sync::atomic::Ordering::Relaxed), was, "одно соединение — не вложенность");
            let second = conn(&pool).await.expect("второе соединение");
            assert_eq!(NESTED.load(std::sync::atomic::Ordering::Relaxed), was + 1, "второе при живом первом считается");
            drop(second);
            drop(first);
        })
        .await;
        super::counting(async {
            let _by_queue = conn(&pool).await.expect("соединение");
        })
        .await;
        super::counting(async {
            let _again = conn(&pool).await.expect("соединение");
            assert_eq!(NESTED.load(std::sync::atomic::Ordering::Relaxed), was + 1, "после освобождения счёт не растёт");
        })
        .await;
    }

    #[test]
    fn a_wrong_address_is_not_busyness() {
        let pool = pool("postgres://nobody@127.0.0.1:1/nothing", 1).expect("пул собрался");
        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("рантайм")
            .block_on(conn(&pool));
        match out {
            Err(Fail::Db(_)) => {}
            Err(e) => panic!("отказ базы назван занятостью: {}", e.says()),
            Ok(_) => panic!("несуществующий адрес отдал соединение"),
        }
    }
}
