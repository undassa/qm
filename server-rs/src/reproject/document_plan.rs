//! Перечень документов: что план объявляет существующим и какими числами.
//!
//! Предметы, о которых план говорит числом, перечислены закрытым списком
//! нарочно: вытаскивать любое число рядом с любым словом значит выдумывать
//! утверждения за документ. И число бывает не переписью, а покрытием — «телом
//! названо 227 требований из 271»; такое инвентарём считать нельзя.

use super::rows::{at, cells, headings, title_of};
use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Документ, который эта проекция разбирает: вид и имя, а не адрес.
const KIND: &str = "document-plan";
const NAME: &str = "";
const HEADER: [&str; 3] = ["Документ", "Что содержит", "Состояние"];
/// Скобочное уточнение — часть подписи, а не имени: «sdd.md (IEEE 1016)».
static PARENTHETICAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\([^)]*\)\s*$").expect("образец скобок"));
static NAMEISH: Lazy<Regex> = Lazy::new(|| Regex::new(r"[./:]").expect("образец имени"));
/// `вид:имя` — та же запись, которой набор называет себя везде. У одиночки имя
/// пусто: `constitution:`. Всё, что под неё не подходит, — не документ набора.
static PLANNED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([a-z][a-z0-9-]*):([A-Za-z0-9][A-Za-z0-9/._-]*)?$").expect("образец вида и имени"));
static COVERAGE_BEFORE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(названо|назван|покрыт|описано|из)\s*$").expect("образец покрытия слева"));
static COVERAGE_AFTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*\S*\s*из\s+\d").expect("образец покрытия справа"));

static SUBJECTS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        ("articles", Regex::new(r"(\d+)\s+стат(?:ья|ьи|ей)").expect("статьи")),
        ("needs", Regex::new(r"(\d+)\s+ST-nn").expect("потребности")),
        ("requirements", Regex::new(r"(\d+)\s+требован").expect("требования")),
        ("areas", Regex::new(r"(\d+)\s+подсистем").expect("подсистемы")),
        ("nfr", Regex::new(r"\+\s*(\d+)\s+NFR").expect("нефункциональные")),
        ("screens", Regex::new(r"(\d+)\s+экран").expect("экраны")),
        ("checks", Regex::new(r"(\d+)\s+TC-nn").expect("проверки")),
        ("decisionFiles", Regex::new(r"(\d+)\s+файл").expect("файлы решений")),
    ]
});

fn claim_of(state: &str) -> &'static str {
    let head = state.trim().to_lowercase();
    if head.starts_with("этот файл") {
        return "self";
    }
    if head.starts_with("есть") {
        return "present";
    }
    if head.starts_with("нет") {
        "absent"
    } else {
        "unknown"
    }
}

/// Имя очищается от скобочного уточнения; несколько имён в ячейке — через запятую.
fn names_of(cell: &str) -> Vec<String> {
    PARENTHETICAL
        .replace(cell, "")
        .split(',')
        .map(|p| p.trim().to_owned())
        .filter(|p| NAMEISH.is_match(p))
        .collect()
}

/// Число внутри оборота о покрытии переписью не считается.
fn is_coverage(text: &str, at: usize, length: usize) -> bool {
    let start = text[..at].char_indices().rev().take(24).last().map(|(i, _)| i).unwrap_or(0);
    COVERAGE_BEFORE.is_match(&text[start..at]) || COVERAGE_AFTER.is_match(&text[at + length..])
}

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize), tokio_postgres::Error> {
    let blocks = cells(pool, project, KIND, NAME, false).await?;
    let heads = headings(pool, project, KIND, NAME).await?;
    let mut entries: Vec<(String, String, String, String, &str, String, String)> = Vec::new();
    let mut counts: Vec<(String, &str, i32, String, String)> = Vec::new();
    let mut seen = HashSet::new();

    for (block, rows) in &blocks {
        let Some(head) = rows.get(&0) else { continue };
        if !HEADER.iter().enumerate().all(|(i, name)| at(head, i).trim() == *name) {
            continue;
        }
        // Заголовок раздела над таблицей: «Уровень 0 — рамка» и далее.
        let level = title_of(&heads, *block);
        for (ord, row) in rows {
            if *ord == 0 {
                continue;
            }
            let state = at(row, 2).trim().to_owned();
            let claim = claim_of(&state);
            let contains = at(row, 1).trim().to_owned();
            for name in names_of(at(row, 0)) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                // Вид и имя вынимаются из самого названия: план теперь зовёт
                // документы так же, как зовёт их весь набор. Не подошло под
                // образец — колонки остаются пустыми, и строка честно говорит,
                // что документа набора за ней нет.
                let (kind, id) = match PLANNED.captures(&name) {
                    Some(m) => (m[1].to_owned(), m.get(2).map(|g| g.as_str().to_owned()).unwrap_or_default()),
                    None => (String::new(), String::new()),
                };
                entries.push((name.clone(), level.clone(), contains.clone(), state.clone(), claim,
                              kind.clone(), id.clone()));
                // Число живёт и в «Что содержит» («79 ST-nn»), и в «Состояние».
                for (subject, pattern) in SUBJECTS.iter() {
                    let mut taken = false;
                    for text in [at(row, 1), &state] {
                        if taken {
                            break;
                        }
                        for m in pattern.captures_iter(text) {
                            let whole = m.get(0).expect("совпадение целиком");
                            if is_coverage(text, whole.start(), whole.len()) {
                                continue;
                            }
                            // Вид и имя документа едут ВМЕСТЕ с числом: связь
                            // заявленного с фактом держалась на совпадении строк
                            // («constitution:» против «constitution.md»), и не
                            // совпадала ни разу — восемь чисел набора не сверял
                            // никто, а ответ выглядел как «сверено, молчит».
                            counts.push((name.clone(), subject, m[1].parse().unwrap_or(0),
                                         kind.clone(), id.clone()));
                            taken = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_document_plan_counts WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_document_plan WHERE project_id = $1", &[&project]).await?;
    for (name, level, contains, state, claim, planned_kind, planned_name) in &entries {
        tx.execute(
            "INSERT INTO project_document_plan(project_id, name, level, contains, state_text, claim,
                                               entity_kind, entity_name, planned_kind, planned_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[&project, name, level, contains, state, claim, &KIND, &NAME, planned_kind, planned_name],
        )
        .await?;
    }
    for (name, subject, claimed, kind, id) in &counts {
        tx.execute(
            "INSERT INTO project_document_plan_counts(project_id, name, subject, claimed,
                                                      planned_kind, planned_name)
             VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
            &[&project, name, subject, claimed, kind, id],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((entries.len(), counts.len()))
}
