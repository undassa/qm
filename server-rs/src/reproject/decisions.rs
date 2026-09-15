//! Решения (ADR): состояние по каталогу, разделы колонками, отвергнутые
//! варианты строками.
//!
//! Набор двуязычен: ранние решения писаны английскими полями, поздние русскими,
//! и одноимённые поля значат разное. Русское «Связано» перечисляет требования,
//! английское `Related` — другие решения. Свалить их в один вид связи значило бы
//! объявить `ADR-0001` несуществующим требованием.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Имя решения — его собственный идентификатор: `ADR-0157`.
static DECISION_NAME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^ADR-(\d{3,4})$").expect("образец решения"));

/// Состояние решения объявляет ПОЛЕ «Статус», а не каталог, в котором лежал файл.
///
/// Первое слово решает: «принято» · «accepted» — принято, «отменено» ·
/// «superseded» — отменено. Поле непусто у всех 171, и на нём же стоял каталог:
/// два отменённых говорят «Superseded by ADR-0014» и «Отменено ADR-0087».
fn status_of(text: &str) -> &'static str {
    let head = text
        .trim()
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if head.starts_with("отменен") || head.starts_with("отменён") || head.starts_with("superseded") {
        "superseded"
    } else if head.starts_with("принят") || head.starts_with("accepted") {
        "accepted"
    } else {
        "template"
    }
}
static DATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4}-\d{2}-\d{2})").expect("образец даты"));
static ADR_REFERENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"ADR-(\d{3,4})").expect("образец ссылки на решение"));
static REQUIREMENT_REFERENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b((?:FR|NFR|TC)-[A-Z0-9]+(?:-\d+[a-z]?)?)\b").expect("образец требования"));
static QUESTION_REFERENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(Q-\d+)\b").expect("образец вопроса"));
/// «Amends = Constitution Article 4» — правка статьи, а не другого решения.
static ARTICLE_REFERENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"Article\s+(\d+)").expect("образец статьи"));
/// Жирный зачин абзаца; точка-в-любой-символ обязательна — зачин бывает длиннее строки.
static LEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)^\*\*(.+?)\*\*").expect("образец зачина"));
static BULLET: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*[-*+]\s+").expect("образец пункта"));
static BULLET_BOLD: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*[-*+]\s+\*\*").expect("образец жирного пункта"));
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("образец пробелов"));
static TAIL_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.:;,]\s*$").expect("образец хвоста"));
static HEAD_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[.:—-]\s*").expect("образец зачина тела"));
static PARAGRAPH: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n\s*\n").expect("образец абзаца"));

/// Пара имён одного раздела: ранние решения писаны по-английски, поздние по-русски.
const CONTEXT: [&str; 2] = ["Контекст", "Context"];
const DECISION: [&str; 2] = ["Решение", "Decision"];
const CONSEQUENCES: [&str; 2] = ["Последствия", "Consequences"];
const ALTERNATIVES: [&str; 4] = [
    "Отвергнутые варианты",
    "Отвергнутые варианты и их цена",
    "Alternatives considered",
    "Alternatives",
];

pub struct Alternative {
    pub ord: i32,
    pub title: String,
    pub body: String,
}

/// Разбор раздела отвергнутых на варианты.
///
/// Зачин варианта жирный, и форм у него ДВЕ, обе живые: ранние решения пишут
/// списком, поздние абзацем. Единица — АБЗАЦ, а не строка: у части решений зачин
/// растянут на две строки, и построчный разбор терял вариант целиком. Абзац без
/// зачина принадлежит предыдущему варианту — это его продолжение.
pub fn split_alternatives(body: &str) -> Vec<Alternative> {
    let mut out: Vec<Alternative> = Vec::new();
    let open = |out: &mut Vec<Alternative>, title: &str, rest: &str| {
        let clean = SPACES.replace_all(title, " ");
        let clean = TAIL_PUNCT.replace(&clean, "").trim().to_owned();
        if clean.is_empty() {
            return;
        }
        out.push(Alternative {
            ord: out.len() as i32,
            title: clean,
            body: HEAD_PUNCT.replace(rest, "").trim().to_owned(),
        });
    };
    let append = |out: &mut Vec<Alternative>, text: &str| {
        if let Some(last) = out.last_mut() {
            last.body = format!("{}\n\n{}", last.body, text).trim().to_owned();
        }
    };
    let lead = |text: &str| -> Option<(String, String)> {
        let t = text.trim();
        LEAD.captures(t).map(|m| (m[1].to_owned(), t[m[0].len()..].to_owned()))
    };

    for chunk in PARAGRAPH.split(body) {
        let text = chunk.trim();
        if text.is_empty() {
            continue;
        }

        // Форма списком — ранние решения: «- **Stay on Rust.** Fully viable…».
        if BULLET_BOLD.is_match(text) {
            let mut item = String::new();
            let mut flush = |item: &mut String, out: &mut Vec<Alternative>| {
                let piece = item.trim().to_owned();
                item.clear();
                if piece.is_empty() {
                    return;
                }
                match lead(&BULLET.replace(&piece, "")) {
                    Some((head, rest)) => open(out, &head, &rest),
                    None => append(out, &piece),
                }
            };
            for line in text.split('\n') {
                if BULLET.is_match(line) {
                    flush(&mut item, &mut out);
                }
                item.push_str(line);
                item.push('\n');
            }
            flush(&mut item, &mut out);
            continue;
        }

        // Форма абзацем — поздние решения: «**Оставить как есть.** Ноль правок…».
        match lead(text) {
            Some((head, rest)) => open(&mut out, &head, &rest),
            None => append(&mut out, text),
        }
    }
    out
}

/// Тело раздела по любому из имён; имя ищется НАЧАЛОМ заголовка: набор пишет
/// «Отвергнутые варианты и их цена», «Последствия, измеренные» — заголовок несёт уточнение.
fn body_of(titles: &[String], bodies: &HashMap<String, String>, names: &[&str]) -> String {
    for name in names {
        let found = titles
            .iter()
            .find(|t| t.as_str() == *name)
            .or_else(|| titles.iter().find(|t| t.starts_with(name)));
        if let Some(t) = found {
            return bodies.get(t).map(|b| b.trim().to_owned()).unwrap_or_default();
        }
    }
    String::new()
}

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize, usize), tokio_postgres::Error> {
    let named = super::runs::named_of_kinds(pool, project, &["decision"]).await?;
    let fields = {
        let client = pool.get().await.expect("пул отдал соединение");
        let rows = client
            .query(
                "SELECT entity_name, name, value FROM project_document_fields
                  WHERE project_id = $1 AND entity_kind = 'decision'
                  ORDER BY entity_name, section_ord, ord",
                &[&project],
            )
            .await?;
        let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
        for r in &rows {
            out.entry(r.get(0)).or_default().insert(r.get(1), r.get(2));
        }
        out
    };
    // Заголовки и тела разделов: при одинаковых заголовках побеждает последний,
    // как у донора.
    let (titles, bodies) = {
        let client = pool.get().await.expect("пул отдал соединение");
        let rows = client
            .query(
                "SELECT s.entity_name, s.title,
                        coalesce((SELECT string_agg(b.raw, '' ORDER BY b.ord)
                                    FROM project_document_blocks b
                                   WHERE b.project_id = s.project_id AND b.entity_kind = s.entity_kind AND b.entity_name = s.entity_name
                                     AND b.ord > s.first_block AND b.ord <= s.last_block), '')
                   FROM project_document_sections s
                  WHERE s.project_id = $1 AND s.entity_kind = 'decision'
                  ORDER BY s.entity_name, s.ord",
                &[&project],
            )
            .await?;
        let mut titles: HashMap<String, Vec<String>> = HashMap::new();
        let mut bodies: HashMap<String, HashMap<String, String>> = HashMap::new();
        for r in &rows {
            let name: String = r.get(0);
            let title: String = r.get(1);
            titles.entry(name.clone()).or_default().push(title.clone());
            bodies.entry(name).or_default().insert(title, r.get(2));
        }
        (titles, bodies)
    };

    let empty_fields = HashMap::new();
    let empty_titles: Vec<String> = Vec::new();
    let empty_bodies = HashMap::new();
    // Статус и решающие, объявленные СТРОКОЙ В ШАПКЕ документа — «Статус:
    // принято · 2026-08-06», «Решают: владелец продукта». Полем это не
    // считалось: полем звалась только строка таблицы или пункт списка, и все
    // 103 решения набора tot выходили без даты и без ответственных при
    // написанных дате и ответственном.
    let head: std::collections::HashMap<String, (String, String)> = {
        let client = pool.get().await.expect("пул отдал соединение");
        client
            .query(
                "SELECT entity_name,
                        coalesce(substring(content from '(?n)^Статус:\\s*(.*)$'), ''),
                        coalesce(substring(content from '(?n)^Реша[юе]т:\\s*(.*)$'), '')
                   FROM project_documents WHERE project_id = $1 AND entity_kind = 'decision'",
                &[&project],
            )
            .await?
            .iter()
            .map(|r| (r.get::<_, String>(0), (r.get::<_, String>(1), r.get::<_, String>(2))))
            .collect()
    };
    let no_head = (String::new(), String::new());
    let mut decisions: Vec<(String, i32, String, String, String, &str, String, String, String, String, String, String)> =
        Vec::new();
    let mut alternatives: Vec<(String, Alternative)> = Vec::new();
    let mut links: Vec<(String, &str, String)> = Vec::new();
    let mut seen = HashSet::new();

    for e in &named {
        let path = &e.1;
        let Some(m) = DECISION_NAME.captures(path) else { continue };
        let f = fields.get(path).unwrap_or(&empty_fields);
        let id = e.1.clone();
        let got = |name: &str| f.get(name).map(String::as_str).unwrap_or("");
        let (head_status, head_deciders) = head.get(path).unwrap_or(&no_head);
        let status_text = if !got("Статус").is_empty() {
            got("Статус").trim().to_owned()
        } else if !got("Status").is_empty() {
            got("Status").trim().to_owned()
        } else {
            head_status.trim().to_owned()
        };
        let list = titles.get(path).unwrap_or(&empty_titles);
        let body = bodies.get(path).unwrap_or(&empty_bodies);

        let date_field = if !got("Дата").is_empty() { got("Дата") } else { got("Date") };
        // Дата ищется и в самом «Статусе»: часть решений пишет «принято 2026-09-05»
        // и отдельного поля даты не имеет.
        let date = DATE
            .captures(date_field)
            .or_else(|| DATE.captures(&status_text))
            .map(|d| d[1].to_owned())
            .unwrap_or_default();
        // Тем же правилом решающие: у части решений поле «Решает», а не «Решают».
        let deciders = ["Решают", "Решает", "Deciders"]
            .iter()
            .map(|n| got(n))
            .find(|v| !v.is_empty())
            .unwrap_or(head_deciders.as_str())
            .trim()
            .to_owned();

        decisions.push((
            id.clone(),
            m[1].parse().unwrap_or(0),
            super::title_without_name(&list.first().cloned().unwrap_or_else(|| path.clone()), &id),
            e.0.clone(),
            e.1.clone(),
            status_of(&status_text),
            status_text,
            date,
            deciders,
            body_of(list, body, &CONTEXT),
            body_of(list, body, &DECISION),
            body_of(list, body, &CONSEQUENCES),
        ));
        for a in split_alternatives(&body_of(list, body, &ALTERNATIVES)) {
            alternatives.push((id.clone(), a));
        }

        let mut add = |kind: &'static str, target: String| {
            if target == id || !seen.insert(format!("{id} {kind} {target}")) {
                return;
            }
            links.push((id.clone(), kind, target));
        };
        let amends = got("Amends").to_owned();
        for c in ADR_REFERENCE.captures_iter(got("Уточняет")) {
            add("refines", format!("ADR-{}", &c[1]));
        }
        for c in ADR_REFERENCE.captures_iter(&amends) {
            add("refines", format!("ADR-{}", &c[1]));
        }
        for c in ARTICLE_REFERENCE.captures_iter(&amends) {
            add("amends-article", c[1].to_owned());
        }
        for c in REQUIREMENT_REFERENCE.captures_iter(got("Связано")) {
            add("related", c[1].to_owned());
        }
        for c in ADR_REFERENCE.captures_iter(got("Related")) {
            add("relates-to-decision", format!("ADR-{}", &c[1]));
        }
        for c in ADR_REFERENCE.captures_iter(got("Supersedes")) {
            add("supersedes", format!("ADR-{}", &c[1]));
        }
        for c in QUESTION_REFERENCE.captures_iter(got("Закрывает")) {
            add("closes", c[1].to_owned());
        }
    }
    decisions.sort_by_key(|d| d.1);

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    // Своё — стирается, объявленное — нет: дверь `decision-link-add` пишет
    // `origin='declared'`, и снос целиком стирал её запись каждой пересборкой.
    tx.execute("DELETE FROM project_decision_links WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    tx.execute("DELETE FROM project_decision_alternatives WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // Альтернативы адресуются решением и порядком, и объявленные тем же решением
    // сталкиваются так же. Уходят они ЦЕЛИКОМ по своему решению: половина
    // объявленная, половина из документа — это перечень, которого никто не писал.
    {
        let ids: Vec<String> = alternatives.iter().map(|(id, _)| id.clone()).collect();
        tx.execute("DELETE FROM project_decision_alternatives WHERE project_id = $1 AND decision_id = ANY($2)",
                   &[&project, &ids]).await?;
    }
    tx.execute("DELETE FROM project_decisions WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // ОБЪЯВЛЕННОЕ ТЕМ ЖЕ ИМЕНЕМ ПОГЛОЩАЕТСЯ ДОКУМЕНТОМ. Чистка снимала только
    // строки происхождения `projected`, а объявленная дверью оставалась — и
    // вставка документа с тем же именем падала на первичном ключе. Пересборка
    // обрывалась целиком, гейт продолжал отдавать числа по недособранным
    // проекциям, и час уходил на ложные находки.
    //
    // Документ — полнее объявления: все колонки, которые заполняет дверь, он
    // считает сам. Побеждает он.
    let ids: Vec<String> = decisions.iter().map(|d| d.0.clone()).collect();
    tx.execute("DELETE FROM project_decisions WHERE project_id = $1 AND id = ANY($2)",
               &[&project, &ids]).await?;
    for (id, number, title, kind, name, status, status_text, date, deciders, context, decision, consequences) in &decisions {
        tx.execute(
            "INSERT INTO project_decisions(project_id, id, number, title, entity_kind, entity_name,
                                           status, status_text,
                                           date, deciders, context, decision, consequences)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
            &[&project, id, number, title, kind, name, status, status_text, date, deciders, context, decision, consequences],
        )
        .await?;
    }
    for (id, a) in &alternatives {
        tx.execute(
            "INSERT INTO project_decision_alternatives(project_id, decision_id, ord, title, body)
             VALUES ($1,$2,$3,$4,$5)",
            &[&project, id, &a.ord, &a.title, &a.body],
        )
        .await?;
    }
    for (id, kind, target) in &links {
        tx.execute(
            "INSERT INTO project_decision_links(project_id, decision_id, kind, target) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING",
            &[&project, id, kind, target],
        )
        .await?;
    }
    // СВЯЗЬ, ПЕРЕЖИВШАЯ СВОЁ РЕШЕНИЕ, — не запись, а ложная зелень.
    //
    // Дверь `decision-link-add` существования решения не проверяет, а
    // `decision-add drop` снимает только строку решения. Прежний снос целиком
    // такую связь подбирал; снос «только своего» оставил бы её навсегда — и
    // `closed-question-names-closer` держал бы вопрос ЗЕЛЁНЫМ через
    // `NOT EXISTS (… kind = 'closes' AND target = q.id)`: вопрос закрыт без
    // ответа и назвал закрывателя, которого нет.
    //
    // Снимается ПОСЛЕ вставки решений — до неё их нет вовсе, и то же условие
    // снесло бы объявленное целиком. Тот же приём, что у фич в `runs`.
    tx.execute(
        "DELETE FROM project_decision_links l
          WHERE l.project_id = $1
            AND NOT EXISTS (SELECT 1 FROM project_decisions d
                             WHERE d.project_id = l.project_id AND d.id = l.decision_id)",
        &[&project],
    )
    .await?;
    tx.commit().await?;
    Ok((decisions.len(), links.len(), alternatives.len()))
}
