//! Прибор харнеса — объявления репозитория, а не строки, которые правит дверь.
//!
//! Правило гейта — тот же код: им меряют работу всех наборов сразу. Пока оно
//! жило одной строкой в базе, менять его мог всякий, у кого есть секрет края, а
//! проверить правку было нечем — ни ревью, ни истории, ни отката. Объявление
//! лежит в дереве, вшивается в двоичный файл сборкой и раскладывается в базу
//! стартом: меняется правило слиянием в ствол и выкладкой, и никак иначе.
//!
//! Вшивается, а не читается с диска рядом: файл рядом с сервером расходится с
//! тем, из чего собран сервер, и расхождение это молчит.

use deadpool_postgres::Pool;
use serde::Deserialize;
use serde_json::{json, Value};

include!(concat!(env!("OUT_DIR"), "/instrument.rs"));

const KINDS: [&str; 4] = ["query", "command", "manual", "unknown"];

/// Пусто — то, что разбирать не надо: способ-команду читает оболочка.
static EMPTY: &String = &String::new();

/// Строка, которой снимается необъявленное: пара «гейт — имя», а не имя.
const UNDECLARED: &str = "NOT EXISTS (SELECT 1 FROM unnest($1::text[], $2::text[]) AS d(phase, id)
                                       WHERE d.phase = g.phase AND d.id = g.id)";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    gates: Vec<Gate>,
    items: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    phase: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    phase: String,
    id: String,
    title: String,
    kind: String,
    #[serde(default)]
    why: String,
    #[serde(default)]
    owner: String,
    #[serde(default, rename = "subjectWhy")]
    subject_why: String,
    #[serde(default)]
    since: i64,
}

/// Фаза работы: порядок, гейт, уровень плана и вид задач, который она пускает.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Phase {
    id: String,
    ord: i32,
    title: String,
    #[serde(default)]
    gate: String,
    #[serde(default, rename = "planLevel")]
    plan_level: String,
    #[serde(default, rename = "taskKind")]
    task_kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Phases {
    phases: Vec<Phase>,
}

/// Ступень лестницы: чем она мерится, кому принадлежит и что трогает.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    ord: i32,
    question: String,
    #[serde(rename = "methodKind")]
    method_kind: String,
    #[serde(rename = "ownerKind")]
    owner_kind: String,
    #[serde(default)]
    owner: String,
    touches: String,
    #[serde(default, rename = "workRun")]
    work_run: String,
    #[serde(default, rename = "whenWhy")]
    when_why: String,
    #[serde(default, rename = "subjectWhy")]
    subject_why: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ladder {
    set: String,
    process: String,
    title: String,
    steps: Vec<Step>,
}

/// Ступень вместе с запросами, разложенными по файлам рядом.
struct Rung {
    step: Step,
    method: String,
    probe: String,
    when_query: String,
    subject: String,
}

/// Объявленный пункт вместе с текстами, разложенными по файлам рядом.
struct Rule {
    item: Item,
    query: String,
    probe: String,
    subject: String,
}

/// Имя файла пункта: двоеточие в имени правила — не разделитель каталогов.
fn file_of(id: &str) -> String {
    id.replace(':', ".")
}

fn text(name: &str) -> Option<&'static str> {
    FILES.iter().find(|(f, _)| *f == name).map(|(_, body)| *body)
}

/// Раскладка видов: чем набор вправе быть. Объявление лежит тем же складом,
/// каким его читает `Kinds`, — и раскладывается той же мерой.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    kinds: std::collections::BTreeMap<String, Value>,
}

/// Гейты с их пунктами и файлы, которые пошли в дело.
struct Gates {
    gates: Vec<Gate>,
    rules: Vec<Rule>,
    used: Vec<String>,
}

/// Лестница: чья она, как зовётся и из чего сложена.
struct Rungs {
    set: String,
    process: String,
    title: String,
    rungs: Vec<Rung>,
    used: Vec<String>,
}

/// Прибор целиком, как его объявляет репозиторий.
struct Instrument {
    kinds: std::collections::BTreeMap<String, Value>,
    gates: Vec<Gate>,
    rules: Vec<Rule>,
    phases: Vec<Phase>,
    set: String,
    process: String,
    title: String,
    rungs: Vec<Rung>,
}

/// Прочесть и проверить ВСЁ объявление разом: половина прибора хуже прежнего целиком.
fn declared() -> Result<Instrument, String> {
    let Gates { gates, rules, mut used } = read()?;
    checked(&gates, &rules)?;
    let phases = phases()?;
    let Rungs { set, process, title, rungs, used: ladder_files } = ladder()?;
    used.extend(ladder_files);
    used.push("kinds.json".to_owned());
    no_orphan_files(&used)?;
    let raw = text("kinds.json").ok_or("объявления видов нет: instrument/kinds.json")?;
    let layout: Layout =
        serde_json::from_str(raw).map_err(|e| format!("instrument/kinds.json не разбирается: {e}"))?;
    if layout.kinds.len() < 10 {
        return Err(format!("объявлено {} видов: набору нечем быть", layout.kinds.len()));
    }
    // Объявление вида обязано разбираться ТЕМ ЖЕ складом, каким его читает
    // сервер: раскладка, которую он не поймёт, остановит его на следующем старте
    // — и остановит уже после записи.
    for (name, spec) in &layout.kinds {
        serde_json::from_value::<crate::kinds::Kind>(spec.clone())
            .map_err(|e| format!("вид {name} не разбирается: {e}"))?;
    }
    for p in &phases {
        if !p.gate.is_empty() && !gates.iter().any(|g| g.phase == p.gate) {
            return Err(format!("фаза {} стоит на гейте {}, которого нет в объявлении", p.id, p.gate));
        }
    }
    Ok(Instrument { kinds: layout.kinds, gates, rules, phases, set, process, title, rungs })
}

fn read() -> Result<Gates, String> {
    let raw = text("gates.json").ok_or("объявления гейтов нет: instrument/gates.json")?;
    let declared: Declared =
        serde_json::from_str(raw).map_err(|e| format!("instrument/gates.json не разбирается: {e}"))?;
    let mut rules = Vec::new();
    let mut used = vec!["gates.json".to_owned(), "phases.json".to_owned()];
    for item in declared.items {
        let base = format!("gate/{}/{}", item.phase, file_of(&item.id));
        let mut take = |suffix: &str| -> String {
            let name = format!("{base}{suffix}");
            match text(&name) {
                Some(body) => {
                    used.push(name);
                    body.trim().to_owned()
                }
                None => String::new(),
            }
        };
        let (query, probe, subject) = (take(".sql"), take(".probe.sql"), take(".subject.sql"));
        rules.push(Rule { item, query, probe, subject });
    }
    Ok(Gates { gates: declared.gates, rules, used })
}

fn ladder() -> Result<Rungs, String> {
    let raw = text("ladder.json").ok_or("объявления лестницы нет: instrument/ladder.json")?;
    let declared: Ladder =
        serde_json::from_str(raw).map_err(|e| format!("instrument/ladder.json не разбирается: {e}"))?;
    if declared.steps.is_empty() {
        return Err("объявлено ноль ступеней: лестнице нечем отвечать «что делать дальше»".into());
    }
    let (set, process, title) =
        (declared.set.clone(), declared.process.clone(), declared.title.clone());
    let mut rungs = Vec::new();
    let mut used = vec!["ladder.json".to_owned()];
    let mut seen: Vec<i32> = Vec::new();
    for step in declared.steps {
        if seen.contains(&step.ord) {
            return Err(format!("ступень {} объявлена дважды", step.ord));
        }
        seen.push(step.ord);
        let base = format!("ladder/{:02}", step.ord);
        let mut take = |suffix: &str| match text(&format!("{base}{suffix}")) {
            Some(body) => {
                used.push(format!("{base}{suffix}"));
                body.trim().to_owned()
            }
            None => String::new(),
        };
        let (method, probe) = (take(".sql"), take(".probe.sql"));
        let (when_query, subject) = (take(".when.sql"), take(".subject.sql"));
        if step.question.trim().is_empty() {
            return Err(format!("у ступени {} нет вопроса: без него она ничего не спрашивает", step.ord));
        }
        // Род способа и наличие способа обязаны сходиться — ровно как у пункта
        // гейта: ступень без способа честно отвечает «мерить нечем», и молчать
        // об этом она не должна.
        if (step.method_kind == "query" || step.method_kind == "command") != !method.is_empty() {
            return Err(format!(
                "у ступени {} род «{}» и {}способ", step.ord, step.method_kind,
                if method.is_empty() { "не объявлен " } else { "объявлен " }));
        }
        if !["query", "command", "unknown"].contains(&step.method_kind.as_str()) {
            return Err(format!("у ступени {} род способа «{}»", step.ord, step.method_kind));
        }
        if !["skill", "agent", "none"].contains(&step.owner_kind.as_str()) {
            return Err(format!("у ступени {} хозяин рода «{}»", step.ord, step.owner_kind));
        }
        if !["corpus", "repository"].contains(&step.touches.as_str()) {
            return Err(format!("ступень {} трогает «{}»: бывает corpus либо repository", step.ord, step.touches));
        }
        if !when_query.is_empty() && step.when_why.is_empty() {
            return Err(format!("у ступени {} есть условие и нет довода: пропуск обязан быть виден с причиной", step.ord));
        }
        if subject.is_empty() != step.subject_why.is_empty() {
            return Err(format!("у ступени {} предмет и довод пустого предмета объявлены порознь", step.ord));
        }
        rungs.push(Rung { step, method, probe, when_query, subject });
    }
    Ok(Rungs { set, process, title, rungs, used })
}

fn phases() -> Result<Vec<Phase>, String> {
    let raw = text("phases.json").ok_or("объявления фаз нет: instrument/phases.json")?;
    let declared: Phases =
        serde_json::from_str(raw).map_err(|e| format!("instrument/phases.json не разбирается: {e}"))?;
    if declared.phases.is_empty() {
        return Err("объявлено ноль фаз: без них лестница не знает, где проект стоит".into());
    }
    let mut seen: Vec<&str> = Vec::new();
    for p in &declared.phases {
        if p.id.is_empty() || p.title.is_empty() {
            return Err(format!("фаза {} объявлена без имени или заголовка", p.id));
        }
        if seen.contains(&p.id.as_str()) {
            return Err(format!("фаза {} объявлена дважды", p.id));
        }
        seen.push(&p.id);
    }
    // ПОРЯДОК РАЗЛИЧАЕТ ФАЗЫ. По нему решают, какая раньше, и две с одним числом
    // сделали бы «предыдущую фазу» делом случая.
    let mut ords: Vec<i32> = declared.phases.iter().map(|p| p.ord).collect();
    ords.sort_unstable();
    ords.dedup();
    if ords.len() != declared.phases.len() {
        return Err("две фазы объявлены с одним порядком: какая из них раньше — решал бы случай".into());
    }
    Ok(declared.phases)
}

/// ФАЙЛ БЕЗ ХОЗЯИНА — НЕ УКРАШЕНИЕ. Запрос, который никто не объявил, выглядит
/// работающим правилом и не меряет ничего; переименованный пункт или снятая
/// ступень оставляют такой файл за собой молча. Судится всё дерево разом:
/// порознь каждая половина считала бы чужие файлы своими сиротами.
fn no_orphan_files(used: &[String]) -> Result<(), String> {
    match FILES.iter().find(|(f, _)| !used.contains(&(*f).to_owned())) {
        Some((orphan, _)) => Err(format!("файл {orphan} не принадлежит ничему объявленному")),
        None => Ok(()),
    }
}

fn checked(gates: &[Gate], rules: &[Rule]) -> Result<(), String> {
    // ПУСТОЕ ОБЪЯВЛЕНИЕ НЕ РАСКЛАДЫВАЕТСЯ. Раскладка снимает всё, чего в нём
    // нет: дерево, потерянное сборкой, стёрло бы прибор целиком и молча.
    if rules.is_empty() {
        return Err("объявлено ноль пунктов гейта: похоже на потерянное дерево, а не на прибор".into());
    }
    let mut seen: Vec<&str> = Vec::new();
    for r in rules {
        let (phase, id) = (r.item.phase.as_str(), r.item.id.as_str());
        if phase.is_empty() || id.is_empty() || r.item.title.is_empty() {
            return Err(format!("пункт {phase} · {id} объявлен без фазы, имени или заголовка"));
        }
        // Имя АДРЕСУЕТ пункт: отметка, отмена и вина записаны на имя, а не на
        // пару с фазой. Одно имя в двух фазах развело бы правило надвое.
        if seen.contains(&id) {
            return Err(format!("имя пункта {id} объявлено дважды"));
        }
        seen.push(id);
        if !KINDS.contains(&r.item.kind.as_str()) {
            return Err(format!("у пункта {id} род «{}»; бывают {}", r.item.kind, KINDS.join(" · ")));
        }
        if (r.item.kind == "query") != !r.query.is_empty() {
            return Err(format!(
                "у пункта {id} род «{}» и {}запрос: запросный пункт обязан нести запрос, \
                 а незапросный — не нести",
                r.item.kind,
                if r.query.is_empty() { "не объявлен " } else { "объявлен " }
            ));
        }
        // ПРОБА ОБЯЗАТЕЛЬНА У ЗАПРОСНОГО. Пункт, который нечем уронить, самотест
        // не проверяет ни разу, и зелёное у него не значит ничего: на `myack`
        // так стояли семьдесят семь проб из ста десяти.
        if r.item.kind == "query" && r.probe.is_empty() {
            return Err(format!("у пункта {id} нет пробы: уронить его нечем, и самотест его не судит"));
        }
        if !r.item.owner.is_empty() && r.item.kind == "query" {
            return Err(format!("у запросного пункта {id} объявлен подписант: подпись бывает у ручного"));
        }
        if !gates.iter().any(|g| g.phase == phase) {
            return Err(format!("пункт {id} стоит в гейте {phase}, которого нет в объявлении"));
        }
        // ПРЕДМЕТ И ЕГО ОБЪЯСНЕНИЕ ХОДЯТ ПАРОЙ. Пустой предмет значит «пройдено
        // на отсутствии сущностей», и почему он бывает пуст — обязан сказать
        // `subjectWhy`. Потерянный файл предмета иначе прошёл бы молча: сироты
        // нет (файла нет), а пункт стал бы судить по всему набору.
        if r.subject.is_empty() != r.item.subject_why.is_empty() {
            return Err(format!(
                "у пункта {id} {}: предмет и объяснение пустого предмета объявляются вместе",
                if r.subject.is_empty() { "объяснение предмета без самого предмета" }
                else { "предмет без объяснения пустоты" }));
        }
        // ИМЯ ФАЙЛА ВЫВОДИТСЯ ИЗ ИМЕНИ ПУНКТА, и вывод обязан быть взаимно
        // однозначным: `a:b` и `a.b` дали бы ОДИН файл, оба пункта считали бы
        // его своим, и сироты бы не нашлось — два правила с общим запросом.
        if let Some(twin) = rules.iter().find(|o| {
            !std::ptr::eq(*o, r) && o.item.phase == r.item.phase && file_of(&o.item.id) == file_of(id)
        }) {
            return Err(format!(
                "имена пунктов {id} и {} дают один файл {}", twin.item.id, file_of(id)));
        }
    }
    Ok(())
}

/// Разложить объявленное по базе: завести, переписать изменённое, снять лишнее.
///
/// Всё одной транзакцией под общим замком: раскладывают двое — сервер и всякая
/// подкоманда, — и половина прибора хуже прежнего целиком.
pub async fn apply(pool: &Pool) -> Result<Value, String> {
    let Instrument { kinds, gates, rules, phases: steps, set, process, title, rungs } = declared()?;
    let mut client = crate::db::conn(pool).await.map_err(|e| crate::db::Says::says(&e))?;
    let tx = client.transaction().await.map_err(|e| e.to_string())?;
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('instrument'))", &[])
        .await
        .map_err(|e| e.to_string())?;

    // Настоящие наборы — те, что объявлены проектами. Копии примерки живут в
    // тех же таблицах, но проектами не числятся, и подсаживать в них нечего.
    let sets: Vec<String> = tx
        .query("SELECT id FROM projects ORDER BY id", &[])
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|r| r.get(0))
        .collect();

    // ВЕСЬ ПРИБОР РАЗБИРАЕТСЯ ДО ЕДИНОЙ ЗАПИСИ, и беды называются ВСЕ разом.
    //
    // Подготовка ловит и разбор, и снятую колонку, и несуществующую таблицу, и
    // не пишет ни строки. Прежде разбирались только изменённые пункты — а
    // правило, сломанное чужой правкой схемы, изменённым не числится: ступень
    // лестницы простояла так полдня. Прежде же разбор падал на первой беде, и
    // каждая следующая стоила ещё одного круга выкладки.
    //
    // Точка возврата на каждую попытку: ошибка в транзакции прерывает её целиком,
    // и без отката первая же беда сделала бы неразбираемым всё остальное.
    let mut broken: Vec<String> = Vec::new();
    for (what, whose, text) in rules
        .iter()
        .flat_map(|r| {
            [("запрос", format!("пункта {}", r.item.id), &r.query),
             ("проба", format!("пункта {}", r.item.id), &r.probe),
             ("предмет", format!("пункта {}", r.item.id), &r.subject)]
        })
        .chain(rungs.iter().flat_map(|r| {
            // Способ ступени бывает КОМАНДОЙ — её разбирает оболочка, а не база.
            // Проба, условие и предмет — запросы всегда.
            let method = if r.step.method_kind == "query" { &r.method } else { EMPTY };
            [("способ", format!("ступени {}", r.step.ord), method),
             ("проба", format!("ступени {}", r.step.ord), &r.probe),
             ("условие", format!("ступени {}", r.step.ord), &r.when_query),
             ("предмет", format!("ступени {}", r.step.ord), &r.subject)]
        }))
    {
        if text.is_empty() {
            continue;
        }
        tx.batch_execute("SAVEPOINT разбор").await.map_err(|e| e.to_string())?;
        if let Err(e) = tx.prepare(text).await {
            broken.push(format!("{what} {whose} не разбирается: {}", crate::db::Says::says(&e)));
        }
        tx.batch_execute("ROLLBACK TO SAVEPOINT разбор; RELEASE SAVEPOINT разбор")
            .await
            .map_err(|e| e.to_string())?;
    }
    if !broken.is_empty() {
        return Err(format!("объявленного не исполнить ({}):\n{}", broken.len(), broken.join("\n")));
    }

    let was = tx
        .query(
            "SELECT phase, id, item, kind, coalesce(query, ''), probe, coalesce(owner, ''), why,
                    subject_query, subject_why, since
               FROM gate_item",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut changed = Vec::new();
    for r in &rules {
        let old = was.iter().find(|w| {
            w.get::<_, &str>(0) == r.item.phase && w.get::<_, &str>(1) == r.item.id
        });
        // ТЕКСТ СРАВНИВАЕТСЯ ОБРЕЗАННЫМ С ОБЕИХ СТОРОН. Файл кончается переводом
        // строки, строка базы — нет, и без обрезки сорок один пункт из ста
        // шестидесяти числился переписанным на одном пустом знаке: раскладка
        // шумит, а вместе с ней слетает приговор пробе.
        let same = old.is_some_and(|w| {
            let db = |n: usize| w.get::<_, &str>(n).trim();
            db(2) == r.item.title
                && db(3) == r.item.kind
                && db(4) == r.query
                && db(5) == r.probe
                && db(6) == r.item.owner
                && db(7) == r.item.why
                && db(8) == r.subject
                && db(9) == r.item.subject_why
                && w.get::<_, i64>(10) == r.item.since
        });
        if same {
            continue;
        }
        changed.push(json!({ "id": r.item.id, "was": if old.is_some() { "изменён" } else { "заведён" } }));
        // ЗАПРОС И ПРОБА ОБЯЗАНЫ РАЗБИРАТЬСЯ. Подготовка ловит и разбор, и
        // несуществующую таблицу, и не пишет ни строки. Прежде это проверяла
        // дверь, и проверяла не зря: подстановка удваивала кавычку, пункт
        // ложился в объявление и молча отвечал «мерить нечем». Роняет — здесь:
        // прибор, который нельзя исполнить, не должен пережить выкладку.
        // ПРОБА ИСПОЛНЯЕТСЯ, А НЕ ТОЛЬКО РАЗБИРАЕТСЯ. Разбор пропускает пробу,
        // которая споткнётся о первое же правило таблицы: так уже было — проба
        // разобралась и упала на `project_requirements_kind_check`, то есть
        // объявление приняли, а уронить пункт ею было нельзя. Исполняется по
        // каждому настоящему набору под точкой возврата, и ни одна строка не
        // остаётся. Ноль подсаженных строк ВЕЗДЕ — тот же отказ: такой пробой не
        // роняется ничего. Ноль в одном наборе законен — у него может не быть
        // того, во что подсаживать.
        if !r.probe.is_empty() {
            let mut planted = 0u64;
            for project in &sets {
                tx.batch_execute("SAVEPOINT проба").await.map_err(|e| e.to_string())?;
                let ran = tx.execute(r.probe.as_str(), &[project]).await;
                tx.batch_execute("ROLLBACK TO SAVEPOINT проба; RELEASE SAVEPOINT проба")
                    .await
                    .map_err(|e| e.to_string())?;
                match ran {
                    Ok(n) => planted += n,
                    Err(e) => {
                        return Err(format!(
                            "проба пункта {} не исполнилась на наборе {project}: {}. \
                             Проба — запрос, ПОДСАЖИВАЮЩИЙ нарушение, и ею роняют правило",
                            r.item.id, crate::db::Says::says(&e)))
                    }
                }
            }
            if planted == 0 && !sets.is_empty() {
                return Err(format!(
                    "проба пункта {} не подсадила ни строки ни в одном наборе: уронить им правило нечем",
                    r.item.id));
            }
        }
        let query = (!r.query.is_empty()).then(|| r.query.clone());
        let owner = (!r.item.owner.is_empty()).then(|| r.item.owner.clone());
        tx.execute(
            "INSERT INTO gate_item (phase, id, item, kind, query, owner, probe, why,
                                    subject_query, subject_why, since)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (phase, id) DO UPDATE SET
               item = EXCLUDED.item, kind = EXCLUDED.kind, query = EXCLUDED.query,
               owner = EXCLUDED.owner, probe = EXCLUDED.probe, why = EXCLUDED.why,
               subject_query = EXCLUDED.subject_query, subject_why = EXCLUDED.subject_why,
               since = EXCLUDED.since",
            &[&r.item.phase, &r.item.id, &r.item.title, &r.item.kind, &query, &owner, &r.probe,
              &r.item.why, &r.subject, &r.item.subject_why, &r.item.since],
        )
        .await
        .map_err(|e| format!("пункт {} не записан: {}", r.item.id, crate::db::Says::says(&e)))?;
        // ПРИГОВОР ПРОБЕ СНИМАЕТСЯ ВМЕСТЕ С ПРАВКОЙ СУДЯЩЕГО ТЕКСТА. «Проба
        // роняет правило» сказано про ту пару «запрос — проба», которую прогнал
        // самотест; переписанная пара этого приговора не заслужила, а гейт
        // показывал бы её проверенной.
        let judged = old.is_some_and(|w| {
            w.get::<_, &str>(4).trim() != r.query
                || w.get::<_, &str>(5).trim() != r.probe
                || w.get::<_, &str>(8).trim() != r.subject
        });
        if judged {
            tx.execute("UPDATE project_gates SET probe_ok = NULL WHERE phase = $1 AND id = $2",
                       &[&r.item.phase, &r.item.id])
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // Снятый пункт уносит С СОБОЙ СВОИ ЗАМЕРЫ. Оставленный замер читается как
    // правило: доска показывает его вердикт, а меряться ему больше нечем.
    //
    // СНИМАЕТСЯ ПАРОЙ (ГЕЙТ, ИМЯ), а не именем. Ключ пункта — пара, и снятие по
    // одному имени не убрало бы пункт, ПЕРЕЕХАВШИЙ в другой гейт: он завёлся бы
    // на новом месте, а старый остался бы мерить, считаться в состояние гейта и
    // держать барьер — правило раздваивается молча. Ровно это однажды сделала
    // снятая дверь на `section-link-resolves`, и снять двойника теперь нечем.
    let phases: Vec<String> = rules.iter().map(|r| r.item.phase.clone()).collect();
    let ids: Vec<String> = rules.iter().map(|r| r.item.id.clone()).collect();
    let gone = tx
        .execute(&format!("DELETE FROM gate_item g WHERE {UNDECLARED}"), &[&phases, &ids])
        .await
        .map_err(|e| e.to_string())?;
    let measures = tx
        .execute(&format!("DELETE FROM project_gates g WHERE {UNDECLARED}"), &[&phases, &ids])
        .await
        .map_err(|e| e.to_string())?;

    // РАСКЛАДКА ВИДОВ — ТОТ ЖЕ ПРИБОР. Чем набор вправе быть, решали двери
    // `kind-add`, `kind-id-set` и ещё пять — на живом и для всех наборов сразу:
    // таблица у раскладки одна. Файл рядом (`MH_CORPUS_LAYOUT`) снят вместе с
    // ними: он заводил раскладку из чужого дерева, и что в ней лежит, зависело
    // от того, чей путь стоял в окружении.
    let kinds_was = tx
        .query("SELECT name, spec FROM kind_layout", &[])
        .await
        .map_err(|e| e.to_string())?;
    let mut kinds_changed = Vec::new();
    for (name, spec) in &kinds {
        let old = kinds_was.iter().find(|w| w.get::<_, &str>(0) == name);
        if old.is_some_and(|w| &w.get::<_, Value>(1) == spec) {
            continue;
        }
        kinds_changed.push(json!({ "вид": name,
                                   "было": if old.is_some() { "изменён" } else { "заведён" } }));
        tx.execute(
            "INSERT INTO kind_layout (name, spec, declared_at, declared_by)
             VALUES ($1,$2,$3,'instrument')
             ON CONFLICT (name) DO UPDATE SET spec = EXCLUDED.spec,
               declared_at = EXCLUDED.declared_at, declared_by = EXCLUDED.declared_by",
            &[name, spec, &crate::projector::now_ms()],
        )
        .await
        .map_err(|e| format!("вид {name} не записан: {}", crate::db::Says::says(&e)))?;
    }
    let kind_names: Vec<String> = kinds.keys().cloned().collect();
    let kinds_gone = tx
        .execute("DELETE FROM kind_layout WHERE NOT (name = ANY($1))", &[&kind_names])
        .await
        .map_err(|e| e.to_string())?;

    // ФАЗЫ — ТОТ ЖЕ ПРИБОР. Порядок работы, гейт фазы и вид задач, который она
    // пускает, решают, что набору выдадут следующим; до сих пор их правила
    // правили дверью на живом, и прочесть объявление было нечем.
    let phases_was = tx
        .query("SELECT id, ord, title, gate, plan_level, task_kind FROM phase", &[])
        .await
        .map_err(|e| e.to_string())?;
    let mut phases_changed = Vec::new();
    for p in &steps {
        let old = phases_was.iter().find(|w| w.get::<_, &str>(0) == p.id);
        let same = old.is_some_and(|w| {
            w.get::<_, i32>(1) == p.ord
                && w.get::<_, &str>(2) == p.title
                && w.get::<_, &str>(3) == p.gate
                && w.get::<_, &str>(4) == p.plan_level
                && w.get::<_, &str>(5) == p.task_kind
        });
        if !same {
            phases_changed.push(json!({ "id": p.id,
                                        "was": if old.is_some() { "изменена" } else { "заведена" } }));
        }
        tx.execute(
            "INSERT INTO phase (id, ord, title, gate, plan_level, task_kind)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (id) DO UPDATE SET ord = EXCLUDED.ord, title = EXCLUDED.title,
               gate = EXCLUDED.gate, plan_level = EXCLUDED.plan_level,
               task_kind = EXCLUDED.task_kind",
            &[&p.id, &p.ord, &p.title, &p.gate, &p.plan_level, &p.task_kind],
        )
        .await
        .map_err(|e| format!("фаза {} не записана: {}", p.id, crate::db::Says::says(&e)))?;
    }
    let phase_ids: Vec<String> = steps.iter().map(|p| p.id.clone()).collect();
    // Снятая фаза уносит свои замеры: строка `phase_state` без объявления
    // читается как фаза, которой больше нет, и держит барьер на пустоте.
    let phases_gone = tx
        .execute("DELETE FROM phase WHERE NOT (id = ANY($1))", &[&phase_ids])
        .await
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM phase_state WHERE NOT (phase = ANY($1))", &[&phase_ids])
        .await
        .map_err(|e| e.to_string())?;

    // ЛЕСТНИЦА — ТОТ ЖЕ ПРИБОР. Ступень отвечает «что делать дальше» всем наборам
    // сразу; правилась она шестью дверьми на живом, и способ при этом лежал в
    // ДВУХ таблицах: объявленный отдельно и его копия в строке ступени, которую
    // переносил особый проход. Объявление пришло из репозитория — копии и
    // проходу не осталось работы, и `harness_process_method` снята.
    tx.execute(
        "INSERT INTO harness_process (set_name, name, title) VALUES ($1,$2,$3)
         ON CONFLICT (set_name, name) DO UPDATE SET title = EXCLUDED.title",
        &[&set, &process, &title],
    )
    .await
    .map_err(|e| format!("процесс {process} не записан: {}", crate::db::Says::says(&e)))?;

    let ladder_was = tx
        .query(
            "SELECT ord, question, method_kind, method, when_query, when_why, owner_kind, owner,
                    touches, probe, work_run, subject_query, subject_why
               FROM harness_process_step WHERE set_name = $1 AND process = $2",
            &[&set, &process],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut ladder_changed = Vec::new();
    for r in &rungs {
        let old = ladder_was.iter().find(|w| w.get::<_, i32>(0) == r.step.ord);
        let same = old.is_some_and(|w| {
            w.get::<_, &str>(1) == r.step.question
                && w.get::<_, &str>(2) == r.step.method_kind
                && w.get::<_, &str>(3).trim() == r.method
                && w.get::<_, &str>(4).trim() == r.when_query
                && w.get::<_, &str>(5) == r.step.when_why
                && w.get::<_, &str>(6) == r.step.owner_kind
                && w.get::<_, &str>(7) == r.step.owner
                && w.get::<_, &str>(8) == r.step.touches
                && w.get::<_, &str>(9).trim() == r.probe
                && w.get::<_, &str>(10) == r.step.work_run
                && w.get::<_, &str>(11).trim() == r.subject
                && w.get::<_, &str>(12) == r.step.subject_why
        });
        if same {
            continue;
        }
        ladder_changed.push(json!({ "ord": r.step.ord,
                                    "was": if old.is_some() { "изменена" } else { "заведена" } }));
        tx.execute(
            "INSERT INTO harness_process_step (set_name, process, ord, question, method_kind, method,
                                               when_query, when_why, owner_kind, owner, touches,
                                               probe, work_run, subject_query, subject_why)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (set_name, process, ord) DO UPDATE SET
               question = EXCLUDED.question, method_kind = EXCLUDED.method_kind,
               method = EXCLUDED.method, when_query = EXCLUDED.when_query,
               when_why = EXCLUDED.when_why, owner_kind = EXCLUDED.owner_kind,
               owner = EXCLUDED.owner, touches = EXCLUDED.touches, probe = EXCLUDED.probe,
               work_run = EXCLUDED.work_run, subject_query = EXCLUDED.subject_query,
               subject_why = EXCLUDED.subject_why",
            &[&set, &process, &r.step.ord, &r.step.question, &r.step.method_kind, &r.method,
              &r.when_query, &r.step.when_why, &r.step.owner_kind, &r.step.owner, &r.step.touches,
              &r.probe, &r.step.work_run, &r.subject, &r.step.subject_why],
        )
        .await
        .map_err(|e| format!("ступень {} не записана: {}", r.step.ord, crate::db::Says::says(&e)))?;
    }
    let ords: Vec<i32> = rungs.iter().map(|r| r.step.ord).collect();
    let rungs_gone = tx
        .execute(
            "DELETE FROM harness_process_step
              WHERE set_name = $1 AND process = $2 AND NOT (ord = ANY($3))",
            &[&set, &process, &ords],
        )
        .await
        .map_err(|e| e.to_string())?;

    let heads: Vec<String> = gates.iter().map(|g| g.phase.clone()).collect();
    for g in &gates {
        tx.execute(
            "INSERT INTO gate_head (phase, title) VALUES ($1,$2)
             ON CONFLICT (phase) DO UPDATE SET title = EXCLUDED.title",
            &[&g.phase, &g.title],
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.execute("DELETE FROM gate_head WHERE NOT (phase = ANY($1))", &[&heads])
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    drop(client);
    // ПРАВКА ПРИБОРА — ПОВОД ПЕРЕМЕРИТЬ ВСЕХ. Отметку ставит всякая правка
    // набора, а правка ПРАВИЛА не ставила её никому: числа гейтов остались бы от
    // прежнего правила и выглядели бы настоящими до первой чужой правки. Новый
    // пункт при этом вовсе не имеет замера, и гейт над ним считался бы пройденным.
    let touched = !changed.is_empty() || gone > 0 || measures > 0
        || phases_gone > 0 || !phases_changed.is_empty()
        || !ladder_changed.is_empty() || rungs_gone > 0
        || !kinds_changed.is_empty() || kinds_gone > 0;
    if touched {
        crate::watch::touch_all(pool, "правка прибора").await.map_err(|e| crate::db::Says::says(&e))?;
    }
    Ok(json!({ "гейтов": gates.len(), "пунктов": rules.len(), "фаз": steps.len(),
               "видов": kinds.len(), "виды": kinds_changed, "снято видов": kinds_gone,
               "изменено": changed.len(), "что": changed,
               "снято пунктов": gone, "снято замеров": measures, "снято фаз": phases_gone,
               "фазы": phases_changed, "ступеней": rungs.len(),
               "ступени": ladder_changed, "снято ступеней": rungs_gone,
               "перемерить": touched }))
}

/// Снятие необъявленного — на живой базе, потому что ошибка тут в SQL, а не в Rust.
///
/// Ключ пункта — пара, и первая раскладка снимала по одному имени: пункт,
/// переехавший в другой гейт, оставался мерить на прежнем месте, а снять его
/// было уже нечем — дверь правки снята. Проверяется тем же текстом, которым
/// снимает раскладка.
#[cfg(test)]
mod removal {
    use super::UNDECLARED;

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_rule_moved_to_another_gate_leaves_no_twin() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let pool = crate::db::pool(&url, 1).expect("пул тестовой базы");
        let client = pool.get().await.expect("соединение с тестовой базой");
        client
            .batch_execute(
                "SET search_path TO pg_temp;
                 CREATE TEMP TABLE gate_item (phase text NOT NULL, id text NOT NULL);
                 INSERT INTO gate_item VALUES ('G2', 'direct-dep'), ('G2', 'кто-остаётся'),
                                              ('G3', 'снятый');",
            )
            .await
            .unwrap();
        // Объявление: `direct-dep` переехал в G3, `снятый` снят вовсе.
        let phases: Vec<String> = vec!["G3".into(), "G2".into()];
        let ids: Vec<String> = vec!["direct-dep".into(), "кто-остаётся".into()];
        let gone = client
            .execute(&format!("DELETE FROM gate_item g WHERE {UNDECLARED}"), &[&phases, &ids])
            .await
            .unwrap();
        let left: Vec<(String, String)> = client
            .query("SELECT phase, id FROM gate_item ORDER BY phase, id", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect();
        assert_eq!(gone, 2, "снимаются и переехавший с прежнего места, и снятый вовсе");
        assert_eq!(left, vec![("G2".to_owned(), "кто-остаётся".to_owned())]);
    }
}

#[cfg(test)]
mod declared {
    use super::FILES;

    #[test]
    fn instrument_of_the_repository_is_whole() {
        let i = super::declared().expect("объявление прибора цело");
        assert!(i.rules.len() > 100, "пунктов гейта {}: похоже на потерянное дерево", i.rules.len());
        assert!(i.phases.len() >= 5, "фаз объявлено {}: работа идёт не в двух шагах", i.phases.len());
        assert!(i.rungs.len() >= 10, "ступеней {}: лестница короче, чем была", i.rungs.len());
        assert!(i.kinds.len() >= 40, "видов {}: раскладка похудела", i.kinds.len());
        // Вид, на который ссылается другой видом-хозяином, обязан быть объявлен:
        // внутренний вид без хозяина адресовать нечем.
        for (name, spec) in &i.kinds {
            if let Some(home) = spec.get("in").and_then(|v| v.as_str()) {
                assert!(i.kinds.contains_key(home), "вид {name} живёт внутри {home}, которого нет");
            }
        }
        assert!(FILES.len() > i.rules.len(), "у пунктов нет ни запросов, ни проб");
        // Метка подстановки у ступени — та самая, которую подставляет сервер.
        // Объявление и код называют её порознь, и разойтись им нельзя: `next-step`
        // отдал бы команду с меткой внутри вместо имени находки.
        for r in &i.rungs {
            if r.step.work_run.contains('{') {
                assert!(r.step.work_run.contains(crate::projector::WORK_RUN_NAME),
                        "ступень {} подставляет метку, которой сервер не знает: {}",
                        r.step.ord, r.step.work_run);
            }
        }
    }

    // Файл, чьё имя не выводится из имени пункта, не нашёлся бы при раскладке и
    // молча остался бы файлом-сиротой — эту ошибку `read` ловит проверкой, а
    // проверку держит здесь: имена правил приходят с двоеточием.
    #[test]
    fn colon_in_a_name_is_not_a_directory() {
        assert_eq!(super::file_of("harness:articles-have-gates"), "harness.articles-have-gates");
        assert_eq!(super::file_of("direct-dep"), "direct-dep");
    }
}

/// Схема и прибор на ЧИСТОЙ базе — то, чего нельзя было проверить вовсе.
///
/// Двадцать семь таблиц сервер только правил, а заводил их когда-то донор: на
/// пустой базе схема не вставала, и потому ни один тест её не поднимал. Этот
/// поднимает — в своей схеме, чтобы не мешать соседям по базе, — и сразу
/// раскладывает прибор. Раскладка на пустом месте объявляет ВСЕ сто шестьдесят
/// пунктов, а значит готовит каждый их запрос и каждую пробу настоящей базой:
/// правило, зовущее снятую колонку, дальше этого теста не уезжает.
#[cfg(test)]
mod fresh {
    /// Каждая читающая дверь отвечает на пустом наборе — и отвечает по существу,
    /// а не «база не ответила».
    ///
    /// Класс, ради которого это стоит: запрос двери, зовущий снятую колонку.
    /// Он не ловится ни сборкой, ни разбором — только исполнением, и потому
    /// ловился до сих пор наборами. За один день так сломались `gate-measure`
    /// (колонка заголовка в замере) и две ступени лестницы, и обе нашлись
    /// случайно. Пустой набор для этого годится: «колонки нет» падает и на нуле
    /// строк.
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn every_reading_door_answers_on_an_empty_set() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Ddoors");
        let pool = crate::db::pool(&format!("{url}{apart}"), 4).expect("пул тестовой базы");
        {
            let client = pool.get().await.expect("соединение");
            client
                .batch_execute("DROP SCHEMA IF EXISTS doors CASCADE; CREATE SCHEMA doors;")
                .await
                .expect("своя схема");
        }
        crate::projector::ensure(&pool).await.expect("схема встаёт");
        super::apply(&pool).await.expect("прибор раскладывается");
        {
            let client = pool.get().await.expect("соединение");
            client
                .execute("INSERT INTO projects (id, json) VALUES ('проба', '{}'::jsonb)", &[])
                .await
                .expect("набор заводится");
        }
        let kinds = crate::kinds::Kinds::from_db(&pool).await.expect("раскладка видов");
        let door = crate::mcp::Mcp {
            pool: pool.clone(),
            kinds: std::sync::Arc::new(kinds),
            project: "проба".to_owned(),
            author: "проба".to_owned(),
        };
        // Спрашиваются ЧИТАЮЩИЕ двери: пишущая на пустом наборе отказывает по
        // существу, и это не то, что здесь проверяется.
        let writes = crate::mcp::Mcp::writes();
        let mut broken: Vec<String> = Vec::new();
        let names: Vec<String> = door
            .tools()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .filter(|n| !writes.contains(&n.as_str()))
            .collect();
        assert!(names.len() > 100, "читающих дверей {}: похоже на потерянный перечень", names.len());
        for name in &names {
            let said = door.call(name, &serde_json::json!({})).await;
            let text = said["content"][0]["text"].as_str().unwrap_or("").to_owned();
            // «База не ответила» — единственное, что здесь считается поломкой:
            // отказ по существу («вида нет», «документа нет») на пустом наборе
            // законен и ожидаем.
            if text.contains("база не ответила") || text.contains("не выполнился") {
                broken.push(format!("{name}: {}", text.chars().take(160).collect::<String>()));
            }
        }
        {
            let client = pool.get().await.expect("соединение");
            let _ = client.batch_execute("DROP SCHEMA IF EXISTS doors CASCADE").await;
        }
        assert!(broken.is_empty(), "двери не отвечают ({}):\n{}", broken.len(), broken.join("\n"));
    }

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_schema_and_the_instrument_rise_on_an_empty_database() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        // Своя схема, а не `public`: соседние тесты требуют, чтобы `public` была
        // пуста, и заведённый ими набор считают чужим.
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Dfresh");
        let pool = crate::db::pool(&format!("{url}{apart}"), 2).expect("пул тестовой базы");
        {
            let client = pool.get().await.expect("соединение с тестовой базой");
            client
                .batch_execute("DROP SCHEMA IF EXISTS fresh CASCADE; CREATE SCHEMA fresh;")
                .await
                .expect("своя схема заводится");
        }
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        let laid = super::apply(&pool).await.expect("прибор раскладывается");
        // Число берётся У ОБЪЯВЛЕНИЯ, а не вписывается сюда литералом. Литерал
        // приходилось двигать рукой при каждом новом пункте, и сегодня он
        // разошёлся: в дереве стало 168, в проверке осталось 165, и упало это
        // в CI, где база есть, — то есть после выкладки, а не при правке.
        let declared = super::declared().expect("объявление прибора цело").rules.len();
        assert_eq!(laid["пунктов"], serde_json::json!(declared as i64),
                   "разложено не всё, что объявлено");
        assert_eq!(laid["изменено"], laid["пунктов"], "на пустой базе объявляются все пункты");
        let client = pool.get().await.expect("соединение");
        let items: i64 = client
            .query_one("SELECT count(*) FROM gate_item", &[])
            .await
            .expect("пункты на месте")
            .get(0);
        assert_eq!(items as usize, declared, "в базе столько пунктов, сколько объявлено");
        client
            .batch_execute("DROP SCHEMA IF EXISTS fresh CASCADE")
            .await
            .expect("схема снимается");
    }
}

/// `G2 · task-dependency-points-back` на двух задачах одной вехи: зависимость от
/// задачи с бо́льшим номером — не находка, круг из них — находка.
///
/// Случай undassa/mh#124: `M5-T88` ждёт `M5-T94`, заведённую позже, и пункт
/// краснел, хотя порядок исполним.
#[cfg(test)]
mod dependency_order {
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_later_numbered_dependency_passes_and_a_ring_does_not() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Ddeporder");
        let pool = crate::db::pool(&format!("{url}{apart}"), 1).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS deporder CASCADE; CREATE SCHEMA deporder;")
            .await
            .expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт");
        let client = pool.get().await.expect("соединение");
        client
            .batch_execute(
                "INSERT INTO project_plan_versions (project_id, id) VALUES ('p', 'v1');
                 INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title)
                      VALUES ('p', 'M5', 'v1', 5, '');
                 INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state)
                      VALUES ('p', 'M5-T88', 'M5', 88, '', 'S', 'not_started'),
                             ('p', 'M5-T94', 'M5', 94, '', 'S', 'not_started');
                 INSERT INTO project_plan_task_deps (project_id, task_id, depends_on)
                      VALUES ('p', 'M5-T88', 'M5-T94');",
            )
            .await
            .expect("план заводится");
        let rule = include_str!("../../instrument/gate/G2/task-dependency-points-back.sql");
        let found = |rows: Vec<tokio_postgres::Row>| rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>();
        let forward = found(client.query(rule, &[&"p"]).await.expect("пункт исполняется"));
        assert!(forward.is_empty(), "зависимость от задачи с бо́льшим номером названа: {forward:?}");
        client
            .batch_execute("INSERT INTO project_plan_task_deps (project_id, task_id, depends_on) VALUES ('p', 'M5-T94', 'M5-T88')")
            .await
            .expect("круг заводится");
        let ring = found(client.query(rule, &[&"p"]).await.expect("пункт исполняется"));
        assert!(ring.iter().any(|d| d.contains("по кругу")), "круг не назван: {ring:?}");
        client.batch_execute("DROP SCHEMA IF EXISTS deporder CASCADE").await.expect("схема снимается");
    }
}

/// `corpus · decided-not-named` держателем решения принимает задачу либо
/// документ `process`. Решение о процессе исполняет обряд, а не задача
/// (undassa/mh#169): пункт краснел на нём, и закрыть находку честно было нечем.
///
/// Подсаживает сама проба пункта: перестанет она сажать вопрос, названный
/// только в process, — здесь пропадёт зелёная сторона, и тест покраснеет.
#[cfg(test)]
mod decided_holder {
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_decision_held_by_a_task_or_by_process_passes_and_an_unheld_one_does_not() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Ddecholder");
        let pool = crate::db::pool(&format!("{url}{apart}"), 1).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS decholder CASCADE; CREATE SCHEMA decholder;")
            .await
            .expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт");
        let client = pool.get().await.expect("соединение");
        client
            .batch_execute(
                "INSERT INTO project_questions (project_id, id, number, title, state, origin)
                      VALUES ('p', 'Q-1', 1, '', 'decided', 'declared'),
                             ('p', 'Q-2', 2, '', 'decided', 'declared');
                 INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                                bytes, revision, updated_at, updated_by)
                      VALUES ('p', 'task', 'M1-T1', 'Решения: Q-1', 'h', 0, 1, 1, 't'),
                             ('p', 'question', 'Q-2', 'Q-2 сам себя не держит', 'h', 0, 1, 1, 't');",
            )
            .await
            .expect("набор заводится");
        client
            .execute(include_str!("../../instrument/gate/corpus/decided-not-named.probe.sql"), &[&"p"])
            .await
            .expect("проба сажает");
        let rule = include_str!("../../instrument/gate/corpus/decided-not-named.sql");
        let found: Vec<String> = client.query(rule, &[&"p"]).await.expect("пункт исполняется")
            .iter().map(|r| r.get::<_, String>(0)).collect();
        let named: Vec<&str> = found.iter().map(|d| d.split(' ').next().unwrap_or("")).collect();
        assert_eq!(named, ["Q-2", "Q-9999"], "держат задача (Q-1) и process (Q-9998): {found:?}");
        assert!(found[0].contains("в задаче") && found[0].contains("в документе process"),
                "находка не называет обоих путей закрытия: {}", found[0]);
        client.batch_execute("DROP SCHEMA IF EXISTS decholder CASCADE").await.expect("схема снимается");
    }
}
