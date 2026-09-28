//! Пересчёт по изменению: набор правят — гейты меряют заново.
//!
//! Раньше состояние гейта считалось при каждом открытии доски: тридцать восемь
//! запросов на заход, и ни одного — когда документ действительно менялся. Теперь
//! наоборот. Правка ставит отметку в `gate_dirty`, а этот работник её снимает:
//! пересобирает проекции и меряет пункты, складывая измеренное в `project_gates`.
//!
//! Отметка одна на проект, и это сделано нарочно: десять правок подряд стоят
//! одного прогона. Работник тоже один — два одновременных пересчёта писали бы в
//! одни строки и оставили бы смесь двух замеров, о которой никто бы не узнал.
//!
//! Между `reproject` и `rebuild` набор неполон — первая команда снимает то, что
//! кладёт вторая. Замер идёт ПОСЛЕ обеих, и до его конца доска показывает
//! прошлый результат со своим временем: устаревшее, названное устаревшим, лучше
//! свежего наполовину.

use crate::db::Says;
use serde_json::json;
use deadpool_postgres::Pool;

/// Как часто заглядывать в отметку. Две секунды — это задержка между правкой и
/// пересчётом; пара `reproject`+`rebuild` занимает около секунды, так что чаще
/// смотреть незачем, а реже — заметно человеку.
const TICK: std::time::Duration = std::time::Duration::from_secs(2);
/// Через сколько кругов сборщик снова убирает брошенные копии примерок.
const FORGET_EVERY: u32 = 900;
/// Сколько живёт запись о натуге: месяц. Сравнивают её с соседними днями, а не
/// с прошлым годом.
const TRACE_LIVES_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const RETRY_MS: i64 = 60_000;
const TRYON_ABANDONED_MS: i64 = 600_000;

/// Отметить, что набор изменился и гейты пора мерить заново.
///
/// Ставится с уже совершённой записи или той же транзакцией, что и запись
/// (`mark`), но не раньше неё: отметка на неудавшейся правке заставила бы
/// считать то же самое второй раз.
/// Объявление ОБЩЕЕ — значит и пересчёт общий.
///
/// Пункт гейта, фаза и ступень лестницы одни на все проекты: разрабатываем по
/// одной схеме. Пометить один проект после правки общего объявления значит
/// оставить остальные с прежним числом пунктов — и разница прочитается как
/// разница проектов, а не как непосчитанное. Так и вышло: у одного набора
/// стояло шестьдесят три пункта, у другого сорок, и выглядело это отставанием.
pub(crate) async fn touch_all(pool: &Pool, reason: &str) -> Result<(), crate::db::Fail> {
    let client = match crate::db::conn(pool).await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("отметка всем наборам не поставлена ({reason}): {}", e.says());
            return Err(e);
        }
    };
    let rows = match client.query("SELECT id FROM projects", &[]).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("отметка всем наборам не поставлена ({reason}): {}", e.says());
            return Err(crate::db::Fail::Db(e));
        }
    };
    drop(client);
    let mut lost: Vec<(crate::db::Fail, String)> = Vec::new();
    for r in &rows {
        let id: String = r.get(0);
        if let Err(e) = touch(pool, &id, reason).await {
            lost.push((e, id));
        }
    }
    if !lost.is_empty() {
        let names = lost.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>().join(", ");
        let first = lost.into_iter().next().map(|(e, _)| e).expect("список не пуст");
        // Причина берётся ПЕРВАЯ НАСТОЯЩАЯ, а не назначается занятостью: база,
        // не принявшая соединение, и перегрузка лечатся по-разному.
        let why = format!("отметка «пересчитать» не поставлена наборам {names}: {}", first.says());
        return Err(match first {
            crate::db::Fail::Busy(_) => crate::db::Fail::Busy(why),
            _ => crate::db::Fail::Down(why),
        });
    }
    Ok(())
}

/// Отметка «пересчитать» для набора.
///
/// Отказ ВОЗВРАЩАЕТСЯ, а не только пишется в журнал: правка уже записана, и
/// потерянная отметка значит, что гейт будет отдавать прежний замер, пока
/// кто-нибудь не тронет набор снова. Сказать об этом обязан тот, кто правил.
pub(crate) async fn touch(pool: &Pool, project: &str, reason: &str) -> Result<(), crate::db::Fail> {
    let client = match crate::db::conn(pool).await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("отметка «пересчитать» набора {project} потеряна: {}", e.says());
            return Err(e);
        }
    };
    // Ошибка здесь не роняет правку намеренно: пометка — не часть правки. Уронить
    // записанный документ из-за неудавшейся отметки значило бы обменять
    // сохранённое на своевременность пересчёта.
    if let Err(e) = mark(&*client, project, reason).await {
        tracing::warn!("отметка «пересчитать» набора {project} потеряна: {}", e.says());
        return Err(e);
    }
    Ok(())
}

pub(crate) async fn mark(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    reason: &str,
) -> Result<u64, crate::db::Fail> {
    client
        .execute(
            "INSERT INTO gate_dirty(project_id, dirty_at, reason) VALUES ($1,$2,$3)
             ON CONFLICT (project_id) DO UPDATE SET dirty_at = $2, reason = $3",
            &[&project, &crate::projector::now_ms(), &reason],
        )
        .await.map_err(Into::into)
}

/// Время по часам БАЗЫ, в миллисекундах. Аренду пишут и читают разные
/// процессы — сервер, `mh-server mcp`, подкоманды, — и сравнивать срок одного
/// с началом круга другого по их собственным часам значило бы доверить
/// порядок событий расхождению часов.
const DB_MS: &str = "(extract(epoch from clock_timestamp()) * 1000)::bigint";

/// Срок аренды пересборки и шаг, которым её продлевает идущая пересборка.
///
/// ponytail: потолок назван. Процесс, убитый посреди пересборки, откладывает
/// замеры набора до `REBUILD_LEASE_MS` (30 с): гейт всё это время стоит на
/// прошлом честном замере. База, не отвечавшая дольше срока, даёт замеру пройти
/// посреди живой пересборки. Срок короткий и продлевается, потому что длинный
/// без продления держал бы замеры после любого оборванного запроса.
#[cfg(not(test))]
const REBUILD_LEASE_MS: i64 = 30_000;
#[cfg(not(test))]
const REBUILD_BEAT: std::time::Duration = std::time::Duration::from_secs(10);
// Под тестом срок короткий: продление иначе не проверить за разумное время.
#[cfg(test)]
const REBUILD_LEASE_MS: i64 = 1_500;
#[cfg(test)]
const REBUILD_BEAT: std::time::Duration = std::time::Duration::from_millis(250);

/// Право писать проекции. Выдаёт его только `rebuilding`, а пишущие проекции
/// функции — `reproject::reproject`, `projector::rebuild_before`,
/// `projector::rebuild`, `store::reparse_all` — без него не зовутся: «каждый
/// писатель берёт аренду» держит компилятор, а не обещание: писатель, забывший
/// аренду, не соберётся.
///
/// Право отдаётся заимствованием и не копируется: сохранённое в сторону, оно
/// пережило бы аренду и открыло бы запись без неё.
pub struct Lease(());

/// Взятая аренда. Снимается и тогда, когда пересборку бросили на полпути:
/// оборванный клиентом запрос роняет будущее, и без снятия при сбросе аренда
/// жила бы до конца срока, откладывая все замеры набора.
struct Held {
    id: i64,
    pool: Pool,
    beat: tokio::task::JoinHandle<()>,
    released: bool,
}

/// Продлить аренду. Снятую не трогает: продление, пришедшее позже снятия
/// (задача продления оборвана посреди запроса), оживило бы её навсегда.
async fn renew(pool: &Pool, id: i64) -> Result<(), crate::db::Fail> {
    crate::db::conn(pool)
        .await?
        .execute(
            &format!("UPDATE projection_rebuild SET until = {DB_MS} + $2 WHERE id = $1 AND NOT ended"),
            &[&id, &REBUILD_LEASE_MS],
        )
        .await?;
    Ok(())
}

async fn release(pool: &Pool, id: i64) -> Result<(), crate::db::Fail> {
    crate::db::conn(pool)
        .await?
        .execute(&format!("UPDATE projection_rebuild SET until = {DB_MS}, ended = true WHERE id = $1"), &[&id])
        .await?;
    Ok(())
}

impl Drop for Held {
    fn drop(&mut self) {
        self.beat.abort();
        if self.released {
            return;
        }
        let (pool, id) = (self.pool.clone(), self.id);
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                if let Err(e) = release(&pool, id).await {
                    tracing::warn!("брошенная пересборка: аренда {id} не снята, истечёт сама: {}", e.says());
                }
            });
        }
    }
}

/// Пересобрать проекции набора под арендой: пока `work` идёт, замер этого
/// набора не сохраняется (`rebuilt_since`).
pub async fn rebuilding<T, E>(pool: &Pool, project: &str, work: impl AsyncFnOnce(&Lease) -> Result<T, E>) -> Result<T, E>
where
    E: From<crate::db::Fail>,
{
    let id: i64 = {
        let client = crate::db::conn(pool).await?;
        client
            // Строки живут, пока их может спросить круг замера, — с запасом.
            .execute(&format!("DELETE FROM projection_rebuild WHERE until < {DB_MS} - 600000"), &[])
            .await
            .map_err(crate::db::Fail::from)?;
        client
            .query_one(
                &format!("INSERT INTO projection_rebuild (project_id, until) VALUES ($1, {DB_MS} + $2) RETURNING id"),
                &[&project, &REBUILD_LEASE_MS],
            )
            .await
            .map_err(crate::db::Fail::from)?
            .get(0)
    };
    let beat = {
        let pool = pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REBUILD_BEAT).await;
                if let Err(e) = renew(&pool, id).await {
                    tracing::warn!("аренда пересборки {id} не продлена: {}", e.says());
                }
            }
        })
    };
    let mut held = Held { id, pool: pool.clone(), beat, released: false };
    let out = work(&Lease(())).await;
    held.beat.abort();
    // Снятие ЖДЁТСЯ, а не отдаётся сбросу: замер, который тот же путь зовёт
    // следом, иначе застал бы собственную аренду и отложил бы сам себя.
    // Неудача не роняет пересборку: она уже сделана, а аренда истечёт сама.
    if let Err(e) = release(pool, id).await {
        tracing::warn!("аренда пересборки набора {project} не снята, истечёт сама: {}", e.says());
    }
    held.released = true;
    out
}

/// Время по часам базы — начало круга замера для `rebuilt_since`.
pub(crate) async fn db_now(client: &impl deadpool_postgres::GenericClient) -> Result<i64, crate::db::Fail> {
    Ok(client.query_one(&format!("SELECT {DB_MS}"), &[]).await?.get(0))
}

/// Шла ли пересборка набора хоть миг позже `since` (по часам базы).
///
/// Спрашивается «позже», а не «идёт ли сейчас»: пересборка, начавшаяся и
/// кончившаяся посреди круга замера, успела переписать то, что круг уже прочёл.
pub(crate) async fn rebuilt_since(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    since: i64,
) -> Result<bool, crate::db::Fail> {
    Ok(client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM projection_rebuild WHERE project_id = $1 AND until > $2)",
            &[&project, &since],
        )
        .await?
        .get(0))
}

/// Работник: смотрит отметку и, если набор менялся, пересчитывает.
pub fn spawn(pool: Pool) {
    tokio::spawn(async move {
        // Уборка брошенных копий — КРУГАМИ, а не однажды при запуске: занятый
        // в момент старта пул отменял её на всю жизнь процесса, и копии
        // примерок оставались до следующего перезапуска.
        let mut till_forget = 0u32;
        let _ = touch_all(&pool, "запуск").await;
        loop {
            if till_forget == 0 {
                crate::db::counting(forget_abandoned_tryons(&pool)).await;
                till_forget = FORGET_EVERY;
            }
            till_forget -= 1;
            tokio::time::sleep(TICK).await;
            remember_strain(&pool).await;
            remember_door_calls(&pool).await;
            remember_visits(&pool).await;
            if let Err(e) = crate::db::counting(round(&pool)).await {
                // Пересчёт, упавший молча, — это доска, застывшая без объяснения.
                tracing::warn!("пересчёт гейтов не прошёл: {}", e.says());
            }
        }
    });
}

/// Положить накопленный счёт вызовов дверей в базу: строка на день, дверь,
/// автора и исход.
///
/// Записью на каждый вызов это было написано сначала. Ревью намерило цену —
/// соединение из пула на пути ответа, ~35 тысяч строк в сутки от одной
/// открытой вкладки и накрутка счётчика «занято», который читают как
/// перегрузку. Счёт отвечает на тот же вопрос, а размер его ограничен числом
/// дверей: срок жизни оттого и не нужен.
async fn remember_door_calls(pool: &Pool) {
    let counted = crate::mcp::take_calls();
    if counted.is_empty() {
        return;
    }
    let Ok(client) = crate::db::conn(pool).await else {
        crate::mcp::return_calls(counted);
        return;
    };
    // Номер суток от эпохи UTC, а не дата базы: читателю, написавшему
    // `where day = current_date`, вернулся бы ноль, неотличимый от «вызовов не
    // было». Сдвиг на границе суток не больше круга сборщика.
    let day = (crate::projector::now_ms() / 86_400_000) as i32;
    // НЕЗАПИСАННОЕ ВОЗВРАЩАЕТСЯ В ПАМЯТЬ ЦЕЛИКОМ, И ЭТО НЕ МЕЛОЧЬ. Первая
    // редакция возвращала счёт только при отказе взять соединение, а отказ
    // самой записи на середине круга проглатывала — то есть теряла счёт ровно
    // в тот круг, когда база была занята и знать это было особенно нужно.
    // `remember_strain` десятью строками ниже так и делает; довод у
    // `return_calls` это и обещает.
    let mut lost = std::collections::HashMap::new();
    for ((project, door, author, failed), (calls, sum, max)) in counted {
        if let Err(e) = client
            .execute(
                "INSERT INTO door_call (day, project_id, door, author, failed, calls, millis_sum, millis_max) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
                 ON CONFLICT (day, project_id, door, author, failed) DO UPDATE SET \
                   calls = door_call.calls + EXCLUDED.calls, \
                   millis_sum = door_call.millis_sum + EXCLUDED.millis_sum, \
                   millis_max = greatest(door_call.millis_max, EXCLUDED.millis_max)",
                &[&day, &project, &door, &author, &failed, &calls, &sum, &max],
            )
            .await
        {
            tracing::warn!("счёт вызовов двери «{door}» не записан: {}", e.says());
            lost.insert((project, door, author, failed), (calls, sum, max));
        }
    }
    if !lost.is_empty() {
        crate::mcp::return_calls(lost);
    }
}

/// Положить накопленные следы входа в базу (`projector::VISITS`).
///
/// ОДНИМ ОПЕРАТОРОМ на обе таблицы: он либо лёг весь, либо не лёг вовсе, и
/// тогда вернуть в память можно всё снятое, не гадая, какая половина записана.
/// Прибавки, `least` и `greatest` вместо присваивания — потому что сервер
/// бывает не один (сине-зелёная выкатка), и каждый несёт свою долю счёта.
async fn remember_visits(pool: &Pool) {
    let taken = crate::projector::take_visits();
    if taken.edge.is_empty() && taken.keys.is_empty() {
        return;
    }
    let Ok(client) = crate::db::conn(pool).await else {
        crate::projector::return_visits(taken);
        return;
    };
    // Строки берутся В ОДНОМ ПОРЯДКЕ на любом сервере: при сине-зелёной выкатке
    // два сервера сбрасывают пересекающиеся имена, и обход `HashMap` в разном
    // порядке давал бы взаимную блокировку — postgres снял бы один из сбросов.
    let mut keys: Vec<_> = taken.keys.iter().collect();
    keys.sort();
    let (shas, seen_at): (Vec<&String>, Vec<i64>) = keys.into_iter().map(|(k, at)| (k, *at)).unzip();
    let mut edge: Vec<_> = taken.edge.iter().collect();
    edge.sort_by(|a, b| a.0.cmp(b.0));
    let mut who = Vec::new();
    let (mut first, mut last, mut ok, mut no) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (p, (f, l, o, n)) in edge {
        who.push(p);
        first.push(*f);
        last.push(*l);
        ok.push(*o);
        no.push(*n);
    }
    if let Err(e) = client
        .execute(
            "WITH k AS (
               UPDATE session_key s SET last_seen = greatest(coalesce(s.last_seen, 0), u.at)
                 FROM unnest($1::text[], $2::bigint[]) AS u(sha, at)
                WHERE s.secret_sha = u.sha)
             INSERT INTO edge_seen (principal, first_at, last_at, requests, refused)
             SELECT * FROM unnest($3::text[], $4::bigint[], $5::bigint[], $6::bigint[], $7::bigint[])
             ON CONFLICT (principal) DO UPDATE SET
               first_at = least(edge_seen.first_at, EXCLUDED.first_at),
               last_at = greatest(edge_seen.last_at, EXCLUDED.last_at),
               requests = edge_seen.requests + EXCLUDED.requests,
               refused = edge_seen.refused + EXCLUDED.refused",
            &[&shas, &seen_at, &who, &first, &last, &ok, &no],
        )
        .await
    {
        tracing::warn!("след входа не записан: {}", e.says());
        crate::projector::return_visits(taken);
    }
}

/// Положить накопленную натугу в базу: отказы «занято» и вложенные взятия.
///
/// Пишется только когда есть что писать, и своим соединением — если его нет,
/// счёт остаётся в памяти до следующего круга, а не теряется.
async fn remember_strain(pool: &Pool) {
    let (busy, nested) = crate::db::strain();
    if busy == 0 && nested == 0 {
        return;
    }
    let Ok(client) = crate::db::conn(pool).await else {
        crate::db::BUSY.fetch_add(busy, std::sync::atomic::Ordering::Relaxed);
        crate::db::NESTED.fetch_add(nested, std::sync::atomic::Ordering::Relaxed);
        return;
    };
    tracing::warn!("натуга: отказов «занято» {busy}, вложенных взятий соединения {nested}");
    let now = crate::projector::now_ms();
    if let Err(e) = client
        .execute(
            "INSERT INTO server_strain (at, busy, nested) VALUES ($1,$2,$3)",
            &[&now, &(busy as i64), &(nested as i64)],
        )
        .await
    {
        // Счёт возвращается в память: потерянный вместе с неудавшейся записью,
        // он делает перегрузку невидимой ровно там, где она случилась.
        tracing::warn!("след перегрузки не записан: {}", e.says());
        crate::db::BUSY.fetch_add(busy, std::sync::atomic::Ordering::Relaxed);
        crate::db::NESTED.fetch_add(nested, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    // След живёт месяц: дверь читает последние полсотни строк, а таблица без
    // срока — это та же куча, которую однажды придётся разгребать руками.
    if let Err(e) = client
        .execute("DELETE FROM server_strain WHERE at < $1", &[&(now - TRACE_LIVES_MS)])
        .await
    {
        tracing::warn!("старый след перегрузки не убран: {}", e.says());
    }
}

async fn round(pool: &Pool) -> Result<(), crate::db::Fail> {
    let client = match crate::db::conn(pool).await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("сборщик без соединения: {}", e.says());
            return Ok(());
        }
    };
    let due = client
        .query(
            // ТОЛЬКО НАСТОЯЩИЕ НАБОРЫ. Отметка ставится по имени набора, а имя
            // бывает и не набором: примерка копирует набор под своим именем, и
            // вместе с ним — эту отметку. Сборщик подхватил бы копию и пересобрал
            // её ВТОРЫМ кругом, наперегонки с той примеркой, ради которой она и
            // заведена. То же и с остатками оборвавшегося прогона: работать по
            // ним значит тратить время на набор, которого нет.
            "SELECT d.project_id, d.dirty_at, d.reason FROM gate_dirty d
              WHERE (d.ran_at IS NULL OR d.dirty_at > d.ran_at)
                AND d.dirty_at <= $1
                AND EXISTS (SELECT 1 FROM projects p WHERE p.id = d.project_id)",
            &[&crate::projector::now_ms()],
        )
        .await?;
    drop(client);
    for r in &due {
        let project: String = r.get(0);
        // Взятая отметка запоминается ДО работы: правка, пришедшая во время
        // пересчёта, поставит время новее — и следующий круг посчитает снова.
        // Записать «посчитано сейчас» в конце значило бы проглотить её.
        let taken: i64 = r.get(1);
        let reason: String = r.get(2);
        let began = std::time::Instant::now();
        // ЗАМКА ПРОЕКТА ЗДЕСЬ НЕТ, И ЭТО РЕШЕНИЕ, А НЕ УПУЩЕНИЕ.
        //
        // Самотест судит пробы по набору, который в это же время переписывает
        // пересчёт, — и пять раз за сессию называл сломанной целую пробу. Замок
        // на круг это чинил и приносил хуже: `pg_advisory_lock` живёт сессией, а
        // пул отдаёт соединение обратно без `DISCARD ALL` (`db.rs`,
        // `RecyclingMethod::Fast`). Любой из шести `?` между захватом и снятием
        // — а падение пересчёта здесь не выдумка, о нём говорят два довода в
        // `reproject` — возвращал в пул соединение с непогашенным замком. Дальше
        // либо счётчик навсегда не нулевой, либо следующий круг ждёт замка без
        // срока: доска замирает, и ни строчки о том, почему.
        //
        // Восемь соединений в пуле, а один `reproject` берёт их сорок девять
        // подряд — ожидающие держат слоты, держащий не может взять следующий, и
        // это уже не ожидание, а тупик. Такое место в пересборке было и без
        // всякого замка: `plan` брала второе соединение, не отпустив первого с
        // открытой транзакцией. Оно убрано — читает та же транзакция.
        //
        // Поэтому не замок сессии, а аренда с концом — `rebuilding`: строка со
        // сроком, которую переживает падение и которая сама истекает. Её берёт
        // и всякий, кто пишет проекции мимо сборщика, — `finish_write` делает
        // это на КАЖДУЮ правку документа. Гонку пересборки с замером она не
        // запрещает, а обезвреживает: замер, заставший пересборку, не
        // сохраняется и оставляет набор помеченным к пересчёту.
        let measured = {
            let (pool, project) = (pool.clone(), project.clone());
            match tokio::spawn(async move { crate::db::counting(recount(&pool, &project)).await }).await {
                Ok(Ok(out)) => Ok(out),
                Ok(Err(e)) => Err(e.says()),
                Err(e) => Err(format!("пересчёт упал: {e}")),
            }
        };
        let spent = began.elapsed().as_millis() as i32;
        let retry_at = measured.is_err().then(|| crate::projector::now_ms() + RETRY_MS);
        let failure = format!("срыв пересчёта: {}", measured.as_ref().err().map_or("", String::as_str));
        // Итог пересчёта записывается с повтором: потерянный, он значит, что
        // круг будет сделан заново на следующем тике — и так под нагрузкой
        // бесконечно, потому что именно нагрузка и мешает его записать.
        let mut client = None;
        for attempt in 0..3 {
            match crate::db::conn(pool).await {
                Ok(c) => {
                    client = Some(c);
                    break;
                }
                Err(e) => {
                    tracing::warn!("итог пересчёта набора {project} не записан ({attempt}): {}", e.says());
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }
        let Some(client) = client else {
            tracing::warn!("итог пересчёта набора {project} потерян: круг будет сделан заново");
            continue;
        };
        if let Err(e) = client
            .execute(
                "UPDATE gate_dirty SET ran_at = $2, ran_ms = $3,
                        reason = CASE WHEN $4::bigint IS NOT NULL AND dirty_at <= $2 THEN $5 ELSE reason END,
                        dirty_at = CASE WHEN $4::bigint IS NOT NULL AND dirty_at <= $2 THEN $4 ELSE dirty_at END
                  WHERE project_id = $1",
                &[&project, &taken, &spent, &retry_at, &failure],
            )
            .await
        {
            tracing::warn!("отметка пересчёта набора {project} не записана: {e}");
        }
        drop(client);
        match measured {
            Ok(out) if out.get("deferred").is_some() => {
                tracing::info!("замер набора {project} отложен: шла пересборка проекций, {spent} мс")
            }
            Ok(out) => tracing::info!(
                "гейты пересчитаны: проект {project}, повод «{reason}», пунктов {}, провалено {}, {spent} мс",
                out["measured"], out["failed"]
            ),
            Err(e) => tracing::warn!("пересчёт набора {project} не прошёл, повтор через {} с: {e}", RETRY_MS / 1000),
        }
    }
    Ok(())
}

async fn forget_abandoned_tryons(pool: &Pool) {
    let client = match crate::db::conn(pool).await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("брошенные копии примерки не убраны: {}", e.says());
            return;
        }
    };
    let cutoff = crate::projector::now_ms() - TRYON_ABANDONED_MS;
    let rows = match client
        .query(
            "SELECT DISTINCT project_id FROM project_documents
              WHERE project_id LIKE 'примерка·%'
                AND CASE WHEN split_part(project_id, '·', 3) ~ '^[0-9]+$'
                         THEN split_part(project_id, '·', 3)::bigint < $1 ELSE false END",
            &[&cutoff],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("брошенные примерки не найдены: {e}");
            return;
        }
    };
    for r in &rows {
        let copy: String = r.get(0);
        match client.query_one("SELECT project_forget($1)", &[&copy]).await {
            Ok(n) => tracing::info!("брошенная примерка {copy} снята: строк {}", n.get::<_, i64>(0)),
            Err(e) => tracing::warn!("брошенная примерка {copy} не снялась: {e}"),
        }
    }
}

pub(crate) async fn recount(pool: &Pool, project: &str) -> Result<serde_json::Value, crate::db::Fail> {
    rebuilding(pool, project, async |lease| {
        crate::reproject::reproject(pool, project, lease).await?;
        crate::projector::rebuild_before(pool, project, lease).await?;
        crate::projector::rebuild(pool, project, lease).await
    })
    .await?;
    measure(pool, project).await
}

/// Гейты, лестница, фазы — одним замером и в этом порядке.
///
/// Ступени 4, 7 и 10 читают состояние пунктов гейта, фаза — состояние своего
/// гейта. Посчитанные раньше, они прочли бы прошлый круг и разошлись бы с
/// доской на один шаг — расхождение, невидимое глазом.
///
/// Дверь `gate-measure` и пересборка после записи мерили только гейты: лестница
/// ждала этого работника, и `next-step`, спрошенный сразу, отвечал прежним.
///
/// Закрытия задач судятся вокруг замера гейтов: взятые до него — по фазе, какой
/// он её оставил.
pub(crate) async fn measure(pool: &Pool, project: &str) -> Result<serde_json::Value, crate::db::Fail> {
    // Начало круга — ДО первого чтения и по часам базы: каждая из трёх записей
    // ниже откладывается, если пересборка шла хоть миг после него.
    let since = db_now(&*crate::db::conn(pool).await?).await?;
    let out = crate::projector::measure_gates(pool, project, since).await?;
    let mut deferred = out.get("deferred").is_some();
    if !deferred {
        deferred = crate::projector::measure_process(pool, project, "godzy", "godzy", since).await?
            .get("deferred").is_some();
    }
    if !deferred {
        deferred = crate::projector::measure_phases(pool, project, since).await?.get("deferred").is_some();
    }
    // Отложенный замер оставляет набор помеченным: иначе сборщик, записав свой
    // круг сделанным, не вернулся бы к набору до следующей правки, и гейт
    // отвечал бы замером, снятым до пересборки.
    //
    // ПОТОЛОК НАЗВАН: два цикла «пересборка + замер» одного набора, идущие
    // разом, откладывают друг друга всякий раз — ревью подсадило это четырежды
    // из четырёх. Сегодня сборщик один и правки идут поодиночке, и это
    // держится; сине-зелёная выкатка с двумя живыми сборщиками или второй
    // сборщик требуют другой формы — атомарной пересборки или очереди замеров.
    if deferred {
        touch(pool, project, "замер отложен: шла пересборка проекций").await?;
        return Ok(json!({ "deferred": true, "gates": out,
                          "why": "замер отложен: во время круга шла пересборка проекций, \
                                  и прочитанное могло быть полусобранным; сохранён прошлый \
                                  круг, набор помечен к пересчёту" }));
    }
    Ok(out)
}

/// Замер, заставший пересборку, — против гонки заявки 116.
///
/// Порча, которую ловят проверки: круг замера сохраняет находки и вердикты,
/// прочитанные посреди пересборки, а аренда не держится, пока пересборка
/// идёт, или держится дольше неё.
#[cfg(test)]
mod lease {
    use deadpool_postgres::Pool;
    use std::time::Duration;

    const P: &str = "П";

    /// Своя схема с одним пунктом гейта и одной ступенью лестницы. `slow` —
    /// пункт мерится две секунды: в это окно проверка успевает начать и
    /// кончить пересборку.
    async fn schema(name: &str, slow: bool) -> Pool {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}options=-c%20search_path%3D{name}", if url.contains('?') { '&' } else { '?' });
        let pool = crate::db::pool(&format!("{url}{apart}"), 6).expect("пул тестовой базы");
        {
            let client = pool.get().await.expect("соединение с тестовой базой");
            client
                .batch_execute(&format!("DROP SCHEMA IF EXISTS {name} CASCADE; CREATE SCHEMA {name};"))
                .await
                .expect("своя схема заводится");
        }
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        let query = if slow { "SELECT $1::text AS detail FROM pg_sleep(2)" } else { "SELECT $1::text AS detail" };
        pool.get()
            .await
            .expect("соединение")
            .batch_execute(&format!(
                "INSERT INTO gate_item (phase, item, id, kind, query) VALUES ('G1', 'пункт', 'пункт', 'query', '{query}');
                 INSERT INTO harness_process (set_name, name) VALUES ('godzy', 'godzy');
                 INSERT INTO harness_process_step (set_name, process, ord, question, method_kind, owner_kind, touches)
                 VALUES ('godzy', 'godzy', 1, 'ступень', 'unknown', 'none', 'corpus');"
            ))
            .await
            .expect("пункт и ступень объявляются");
        pool
    }

    async fn drop_schema(pool: &Pool, name: &str) {
        pool.get()
            .await
            .expect("соединение")
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {name} CASCADE"))
            .await
            .expect("схема снимается");
    }

    async fn count(pool: &Pool, sql: &str) -> i64 {
        pool.get().await.expect("соединение").query_one(sql, &[]).await.expect("счёт").get(0)
    }

    async fn live(pool: &Pool) -> bool {
        let client = crate::db::conn(pool).await.expect("соединение");
        let now = super::db_now(&*client).await.expect("часы базы");
        super::rebuilt_since(&*client, P, now).await.expect("аренда")
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_measure_during_a_rebuild_is_not_stored() {
        let pool = schema("lease_during", false).await;
        let during = super::rebuilding(&pool, P, async |_| {
            let during = super::measure(&pool, P).await?;
            // Лестница и фазы откладываются сами, а не только следом за гейтами:
            // пересборка может начаться и после коммита круга гейтов.
            let since = super::db_now(&*crate::db::conn(&pool).await?).await?;
            assert_eq!(crate::projector::measure_process(&pool, P, "godzy", "godzy", since).await?["deferred"], true,
                       "положение лестницы посреди пересборки не сохраняется");
            assert_eq!(crate::projector::measure_phases(&pool, P, since).await?["deferred"], true,
                       "фазы посреди пересборки не сохраняются");
            Ok::<_, crate::db::Fail>(during)
        })
        .await
        .expect("пересборка с замером внутри");
        assert_eq!(during["deferred"], true, "замер посреди пересборки откладывается: {during}");
        assert_eq!(count(&pool, "SELECT count(*) FROM project_gates").await, 0,
                   "отложенный замер не пишет ни строки в `project_gates`");
        assert_eq!(count(&pool, "SELECT count(*) FROM process_run").await, 0,
                   "вердикты ступеней посреди пересборки не ложатся в `process_run`");
        assert_eq!(count(&pool, "SELECT count(*) FROM gate_dirty").await, 1,
                   "отложенный замер оставляет набор помеченным к пересчёту");

        // Аренда снята — тот же замер сохраняется целиком.
        let after = super::measure(&pool, P).await.expect("замер после пересборки");
        assert!(after.get("deferred").is_none(), "после пересборки замер не откладывается: {after}");
        assert_eq!(count(&pool, "SELECT count(*) FROM project_gates").await, 1, "замер после пересборки сохранён");
        assert!(count(&pool, "SELECT count(*) FROM process_run").await > 0, "и вердикты ступеней тоже");
        drop_schema(&pool, "lease_during").await;
    }

    /// Пересборка, начавшаяся и кончившаяся посреди круга, откладывает его:
    /// спрашивается «с начала круга», а не «идёт ли сейчас».
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_rebuild_inside_a_round_defers_it() {
        let pool = schema("lease_inside", true).await;
        let round = tokio::spawn({
            let pool = pool.clone();
            async move { super::measure(&pool, P).await }
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        super::rebuilding(&pool, P, async |_| Ok::<_, crate::db::Fail>(())).await.expect("пустая пересборка");
        let out = round.await.expect("круг не упал").expect("круг замера");
        assert_eq!(out["deferred"], true, "круг, внутри которого прошла пересборка, отложен: {out}");
        assert_eq!(count(&pool, "SELECT count(*) FROM project_gates").await, 0);
        drop_schema(&pool, "lease_inside").await;
    }

    /// Пересборка дольше срока держит аренду до конца: продление идёт.
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_rebuild_longer_than_the_term_keeps_its_lease() {
        let pool = schema("lease_long", false).await;
        let held = super::rebuilding(&pool, P, async |_| {
            tokio::time::sleep(Duration::from_millis(3 * super::REBUILD_LEASE_MS as u64)).await;
            Ok::<_, crate::db::Fail>(live(&pool).await)
        })
        .await
        .expect("долгая пересборка");
        assert!(held, "через три срока аренда пересборки ещё жива");
        assert!(!live(&pool).await, "по концу пересборки аренда снята");
        drop_schema(&pool, "lease_long").await;
    }

    /// Продление, пришедшее после снятия, аренду не оживляет.
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_renewal_after_release_does_not_revive_the_lease() {
        let pool = schema("lease_revive", false).await;
        super::rebuilding(&pool, P, async |_| Ok::<_, crate::db::Fail>(())).await.expect("пересборка");
        let id = count(&pool, "SELECT max(id) FROM projection_rebuild").await;
        super::renew(&pool, id).await.expect("запоздалое продление");
        assert!(!live(&pool).await, "снятая аренда не ожила от запоздалого продления");
        drop_schema(&pool, "lease_revive").await;
    }

    /// Брошенная пересборка снимает аренду: оборванный клиентом запрос не
    /// держит замеры набора до конца срока.
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn an_abandoned_rebuild_releases_its_lease() {
        let pool = schema("lease_dropped", false).await;
        let abandoned = tokio::time::timeout(
            Duration::from_millis(500),
            super::rebuilding(&pool, P, async |_| {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok::<_, crate::db::Fail>(())
            }),
        )
        .await;
        assert!(abandoned.is_err(), "пересборка брошена по сроку");
        // Снятие при сбросе идёт отдельной задачей. Ждётся признак `ended`, а не
        // «аренда не жива»: истёкшая по сроку тоже не жива, и проверка прошла бы
        // без снятия.
        let mut ended = 0;
        for _ in 0..50 {
            ended = count(&pool, "SELECT count(*) FROM projection_rebuild WHERE ended").await;
            if ended == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(ended, 1, "брошенная пересборка снимает аренду");
        drop_schema(&pool, "lease_dropped").await;
    }
}
