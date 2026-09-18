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

#[derive(Deserialize)]
struct Declared {
    gates: Vec<Gate>,
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Gate {
    phase: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
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
        let same = old.is_some_and(|w| {
            w.get::<_, &str>(2) == r.item.title
                && w.get::<_, &str>(3) == r.item.kind
                && w.get::<_, &str>(4) == r.query
                && w.get::<_, &str>(5) == r.probe
                && w.get::<_, &str>(6) == r.item.owner
                && w.get::<_, &str>(7) == r.item.why
                && w.get::<_, &str>(8) == r.subject
                && w.get::<_, &str>(9) == r.item.subject_why
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
            w.get::<_, &str>(4) != r.query
                || w.get::<_, &str>(5) != r.probe
                || w.get::<_, &str>(8) != r.subject
        });
        if judged {
            tx.execute("UPDATE project_gates SET probe_ok = NULL WHERE id = $1", &[&r.item.id])
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // Снятый пункт уносит С СОБОЙ СВОИ ЗАМЕРЫ. Оставленный замер читается как
    // правило: доска показывает его вердикт, а меряться ему больше нечем.
    let ids: Vec<String> = rules.iter().map(|r| r.item.id.clone()).collect();
    let gone = tx
        .execute("DELETE FROM gate_item WHERE NOT (id = ANY($1))", &[&ids])
        .await
        .map_err(|e| e.to_string())?;
    let measures = tx
        .execute("DELETE FROM project_gates WHERE NOT (id = ANY($1))", &[&ids])
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
    Ok(json!({ "гейтов": gates.len(), "пунктов": rules.len(),
               "изменено": changed.len(), "что": changed,
               "снято пунктов": gone, "снято замеров": measures }))
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
