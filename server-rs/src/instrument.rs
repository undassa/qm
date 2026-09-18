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

fn read() -> Result<(Vec<Gate>, Vec<Rule>), String> {
    let raw = text("gates.json").ok_or("объявления гейтов нет: instrument/gates.json")?;
    let declared: Declared =
        serde_json::from_str(raw).map_err(|e| format!("instrument/gates.json не разбирается: {e}"))?;
    let mut rules = Vec::new();
    let mut used = vec!["gates.json".to_owned()];
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
    // ФАЙЛ БЕЗ ПУНКТА — НЕ УКРАШЕНИЕ. Запрос, который никто не объявил, выглядит
    // работающим правилом и не меряет ничего; переименованный пункт оставляет
    // такой файл за собой молча.
    if let Some((orphan, _)) = FILES.iter().find(|(f, _)| !used.contains(&(*f).to_owned())) {
        return Err(format!("файл {orphan} не принадлежит ни одному объявленному пункту"));
    }
    Ok((declared.gates, rules))
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
    let (gates, rules) = read()?;
    checked(&gates, &rules)?;
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
        for (what, text) in [("запрос", &r.query), ("проба", &r.probe)] {
            if text.is_empty() {
                continue;
            }
            tx.prepare(text)
                .await
                .map_err(|e| format!("{what} пункта {} не разбирается: {e}", r.item.id))?;
        }
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
                            "проба пункта {} не исполнилась на наборе {project}: {e}. \
                             Проба — запрос, ПОДСАЖИВАЮЩИЙ нарушение, и ею роняют правило",
                            r.item.id))
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
        .map_err(|e| format!("пункт {} не записан: {e}", r.item.id))?;
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
    let touched = !changed.is_empty() || gone > 0 || measures > 0;
    if touched {
        crate::watch::touch_all(pool, "правка прибора").await.map_err(|e| crate::db::Says::says(&e))?;
    }
    Ok(json!({ "гейтов": gates.len(), "пунктов": rules.len(),
               "изменено": changed.len(), "что": changed,
               "снято пунктов": gone, "снято замеров": measures,
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
    use super::{checked, read, FILES};

    #[test]
    fn instrument_of_the_repository_is_whole() {
        let (gates, rules) = read().expect("объявление прибора разбирается");
        checked(&gates, &rules).expect("объявление прибора цело");
        assert!(rules.len() > 100, "пунктов гейта {}: похоже на потерянное дерево", rules.len());
        assert!(FILES.len() > rules.len(), "у пунктов нет ни запросов, ни проб");
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
