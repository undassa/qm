//! Статьи конституции и ссылки на них.
//!
//! Статья — сущность, а не заголовок в тексте: на статьи ссылаются из сотен
//! документов, и без таблицы такую ссылку нечем проверить.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;

/// Заголовок статьи. Два способа назвать её и три разных тире.
///
/// `## Article 1 — Tenant isolation` и `## ART-01 · Фундамент не зависит ни от
/// чего` — одна и та же вещь, названная по-разному в двух наборах. Образец знал
/// только первый, и второй набор получал ноль статей при двадцати пяти
/// написанных: документ лежал целиком, а сущностей из него не выходило.
static HEADING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^##\s+(?:Article\s+(\d+)|ART-(\d+))\s*[—–·-]\s*(.+?)\s*$").expect("образец заголовка")
});
/// Ссылка на статью — где угодно в тексте любого документа, обеими записями.
static REFERENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:Article\s+|ART-)(\d+)").expect("образец ссылки"));
/// Тематический разрыв: `---`, `***`, `___`.
static BREAK: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(-{3,}|\*{3,}|_{3,})\s*$").expect("образец разрыва"));
static NOT_WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\p{L}\p{N}\s-]").expect("образец мусора"));
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("образец пробелов"));

/// Якорь статьи совпадает с якорем раздела документа: ссылка ведёт в нужное место.
pub fn anchor(number: i32, title: &str) -> String {
    let base = format!("article-{number}-{title}").to_lowercase();
    let clean = NOT_WORD.replace_all(&base, "");
    SPACES.replace_all(clean.trim(), "-").into_owned()
}

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize), tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let docs = client
        .query(
            "SELECT entity_kind, entity_name, content FROM project_documents
              WHERE project_id = $1 ORDER BY entity_kind, entity_name",
            &[&project],
        )
        .await?;

    // Статьи — только из конституции, и она зовётся своим видом: одиночка, у
    // которой имени нет, потому что вид его заменяет.
    let constitution = docs.iter().find(|r| r.get::<_, String>(0) == "constitution");
    let mut articles: Vec<(i32, String, String, String, String, String)> = Vec::new();
    if let Some(row) = constitution {
        let (kind, name): (String, String) = (row.get(0), row.get(1));
        let content: String = row.get(2);
        let mut current: Option<(i32, String, Vec<String>)> = None;
        for line in content.split('\n') {
            if let Some(m) = HEADING.captures(line) {
                if let Some((n, title, body)) = current.take() {
                    articles.push((n, title.clone(), kind.clone(), name.clone(), anchor(n, &title), body.join("\n").trim().to_owned()));
                }
                // Номер приходит первой либо второй скобкой — смотря какой
                // записью названа статья; заголовок всегда третьей.
                let number = m.get(1).or_else(|| m.get(2))
                    .and_then(|g| g.as_str().parse().ok()).unwrap_or(0);
                current = Some((number, m[3].to_owned(), Vec::new()));
                continue;
            }
            // Тело статьи кончается следующим разделом ИЛИ тематическим разрывом:
            // без разрыва статья 16 забирала бы всю историю версий документа.
            if current.is_some() && (line.starts_with("## ") || line.starts_with("##\t") || BREAK.is_match(line)) {
                if let Some((n, title, body)) = current.take() {
                    articles.push((n, title.clone(), kind.clone(), name.clone(), anchor(n, &title), body.join("\n").trim().to_owned()));
                }
            } else if let Some((_, _, body)) = current.as_mut() {
                body.push(line.to_owned());
            }
        }
        if let Some((n, title, body)) = current.take() {
            articles.push((n, title.clone(), kind.clone(), name.clone(), anchor(n, &title), body.join("\n").trim().to_owned()));
        }
    }
    articles.sort_by_key(|a| a.0);

    // Ссылка считается по документу один раз: пара «документ · статья».
    let mut refs: Vec<(String, String, i32)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in &docs {
        let (kind, name): (String, String) = (row.get(0), row.get(1));
        let content: String = row.get(2);
        for m in REFERENCE.captures_iter(&content) {
            let n: i32 = m[1].parse().unwrap_or(0);
            if seen.insert((kind.clone(), name.clone(), n)) {
                refs.push((kind.clone(), name.clone(), n));
            }
        }
    }

    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_article_references WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_articles WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // Статья, которую документ ГОВОРИТ, обновляется из документа — даже если её
    // однажды объявили ручкой. Вставка без разрешения конфликта роняла всю
    // пересборку о ключ: ровно это и случилось с 25 статьями, объявленными,
    // пока разбор их не видел.
    //
    // Пометка происхождения при этом НЕ МЕНЯЕТСЯ, и это стоило одного
    // требования. Перекрасив объявленную строку в выведенную, следующая
    // пересборка удаляла её (стираются только выведенные) и вставляла заново —
    // без колонок, которых разбор не выводит вовсе. У `NFR-10` так пропали
    // заголовок, область и способ измерения. Объявленное остаётся объявленным;
    // документ обновляет в нём то, что говорит, и молчит об остальном.
    for (n, title, kind, name, anchor, body) in &articles {
        tx.execute(
            "INSERT INTO project_articles(project_id, number, title, entity_kind, entity_name, anchor, body)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (project_id, number) DO UPDATE SET title = EXCLUDED.title,
               entity_kind = EXCLUDED.entity_kind, entity_name = EXCLUDED.entity_name,
               anchor = EXCLUDED.anchor, body = EXCLUDED.body",
            &[&project, n, title, kind, name, anchor, body],
        )
        .await?;
    }
    for (kind, name, n) in &refs {
        tx.execute(
            "INSERT INTO project_article_references(project_id, entity_kind, entity_name, number)
             VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&project, kind, name, n],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((articles.len(), refs.len()))
}
