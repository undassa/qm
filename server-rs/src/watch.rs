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
        // Правильная форма — не замок сессии, а аренда с концом: отметка в
        // `gate_dirty` со сроком, которую переживает падение и которая сама
        // истекает. Она же нужна и тем, кто пишет проекции мимо сборщика:
        // `finish_write` делает это на КАЖДУЮ правку документа. Пока её нет,
        // гонка остаётся — названной, а не прикрытой.
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
    crate::reproject::reproject(pool, project).await?;
    crate::projector::rebuild_before(pool, project).await?;
    crate::projector::rebuild(pool, project).await?;
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
    let out = crate::projector::measure_gates(pool, project).await?;
    crate::projector::measure_process(pool, project, "godzy", "godzy").await?;
    crate::projector::measure_phases(pool, project).await?;
    Ok(out)
}
