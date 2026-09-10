//! Вопросы. Состояние объявлено полем «Состояние», а не выводится из наличия
//! раздела «Ответ»: «решено» и «закрыт» — разные вещи, и признак закрытия у
//! части вопросов — исполнение, а не согласие.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

/// Имя вопроса — `Q-123`; номер вынимается из него же.
static QUESTION_NAME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(Q-(\d+))$").expect("образец вопроса"));
static DATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4}-\d{2}-\d{2})").expect("образец даты"));
/// Заголовок ответа несёт дату и суть: «Ответ, 2026-08-14 — …», «Ответ 2».
///
/// Заголовок — ЗАПАСНОЙ путь. Признак ответа объявляется полем `Ответ` либо
/// `Ответа нет` в шапке вопроса: новейшие вопросы пишутся без шаблона вовсе, и
/// у них разделы зовутся «Что исполнено», «Исход», «Почему это подпись». Флаг,
/// выведенный из заголовка, врал на них — и врал в опасную сторону, объявляя
/// «ответа нет» там, где ответ есть.
static ANSWER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^Ответ(\s*[,:—-]|\s+\d|$)").expect("образец ответа"));

/// Образцы заголовков, которыми набор помечает «решение за владельцем».
///
/// Пусто — роль не объявлена, и состояние `owner` в этом наборе не возникает.
/// Умолчания в коде здесь нет намеренно: слово принадлежит набору.
async fn owner_sections(pool: &Pool, project: &str) -> Result<Vec<Regex>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    Ok(client
        .query("SELECT value FROM scheme($1) WHERE role = 'section.owner-decides'", &[&project])
        .await?
        .iter()
        .filter_map(|r| Regex::new(&r.get::<_, String>(0)).ok())
        .collect())
}

/// Первое слово поля решает; неизвестное считаем открытым, а не закрытым.
fn state_of(field: &str) -> (&'static str, String) {
    let text = field.trim().to_owned();
    let head: String = text
        .split(|c: char| c.is_whitespace() || c == '·')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if head.starts_with("закрыт") {
        return ("closed", text);
    }
    if head.starts_with("решено") || head.starts_with("решён") {
        return ("decided", text);
    }
    ("open", text)
}

/// Поля документов: имя → значение (не сырое: у вопроса важен текст, не разметка).
async fn fields(pool: &Pool, project: &str) -> Result<HashMap<String, HashMap<String, String>>, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    let rows = client
        .query(
            "SELECT entity_name, name, value FROM project_document_fields
              WHERE project_id = $1 AND entity_kind = 'question'
              ORDER BY entity_name, section_ord, ord",
            &[&project],
        )
        .await?;
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    for r in &rows {
        out.entry(r.get(0)).or_default().insert(r.get(1), r.get(2));
    }
    Ok(out)
}

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    // «Решение за владельцем» — ЧЕТВЁРТОЕ состояние ответа, и оно не выводится
    // из молчания. Вопрос, разобранный до конца и упирающийся в слово владельца,
    // и вопрос, которого никто не открывал, стояли одним `unsaid`: ступень
    // считала оба нарушением, и проход, честно записавший десять находок,
    // выглядел откатившим проект назад. Лестница, на которой записанная находка
    // читается как регресс, учит находки не записывать.
    //
    // Слово даёт НАБОР, а не код: роль `section.owner-decides` — образец
    // заголовка. Роли нет — состояние не возникает, и это видно, а не
    // подразумевается.
    let owner_marks = owner_sections(pool, project).await?;
    let named = super::runs::named_of_kinds(pool, project, &["question"]).await?;
    let titles = super::runs::section_titles(pool, project, &["question"]).await?;
    let fields = fields(pool, project).await?;
    let empty_fields = HashMap::new();
    let empty_titles: Vec<String> = Vec::new();

    let mut out: Vec<(String, i32, String, String, String, &str, String, String, String, String, bool,
                      (&str, String))> = Vec::new();
    for e in &named {
        let Some(m) = QUESTION_NAME.captures(&e.1) else { continue };
        let f = fields.get(&e.1).unwrap_or(&empty_fields);
        let got = |name: &str| f.get(name).map(String::as_str).unwrap_or("");
        let (state, text) = state_of(got("Состояние"));
        let list = titles.get(e).unwrap_or(&empty_titles);
        let date = |v: &str| DATE.captures(v).map(|d| d[1].to_owned()).unwrap_or_default();
        out.push((
            m[1].to_owned(),
            m[2].parse().unwrap_or(0),
            super::title_without_name(&list.first().cloned().unwrap_or_else(|| e.1.clone()), &m[1]),
            e.0.clone(),
            e.1.clone(),
            state,
            text,
            got("Гейт").to_owned(),
            date(got("Заведён")),
            date(got("Закрыт")),
            list.iter().any(|t| ANSWER.is_match(t.trim())),
            // Три состояния, и третье обязательно: отвечен · искали и не нашли ·
            // не сказано ничего.
            //
            // Ответ объявляется ДВУМЯ способами, и набор пользуется обоими:
            // полем `Ответ` в шапке (17 вопросов) и разделом «Ответ» в теле
            // (342). Проекция читала только поле — и 342 отвеченных вопроса
            // числились молчащими. «Не сказано ничего» стояло там, где ответ
            // написан целым разделом.
            //
            // Заголовок берётся только в ПОЛОЖИТЕЛЬНУЮ сторону, и это существенно:
            // вывести из его отсутствия «ответа нет» нельзя — новейшие вопросы
            // пишутся без шаблона, и разделы у них зовутся «Что исполнено»,
            // «Исход», «Почему это подпись». Раз заголовок «Ответ» есть — ответ
            // есть; нет заголовка — судит поле, а не догадка.
            {
                let said = got("Ответ");
                let none = got("Ответа нет");
                let section = list.iter().any(|t| ANSWER.is_match(t.trim()));
                // Отвеченный вопрос отвечен, даже если раздел про владельца в нём
                // остался: ответ сильнее ожидания ответа.
                let held = list.iter().find(|t| owner_marks.iter().any(|r| r.is_match(t.trim())));
                if !said.trim().is_empty() {
                    ("answered", said.to_owned())
                } else if section {
                    ("answered", "объявлен разделом «Ответ»".to_owned())
                } else if !none.trim().is_empty() {
                    ("searched", none.to_owned())
                } else if let Some(t) = held {
                    ("owner", format!("решение за владельцем: раздел «{}»", t.trim()))
                } else {
                    ("unsaid", String::new())
                }
            },
        ));
    }
    out.sort_by_key(|q| q.1);

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_questions WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // ОБЪЯВЛЕННОЕ ТЕМ ЖЕ ИМЕНЕМ ПОГЛОЩАЕТСЯ ДОКУМЕНТОМ. Чистка снимала только
    // строки происхождения `projected`, а объявленная дверью оставалась — и
    // вставка документа с тем же именем падала на первичном ключе. Пересборка
    // обрывалась целиком, гейт продолжал отдавать числа по недособранным
    // проекциям, и час уходил на ложные находки.
    //
    // Документ — полнее объявления: все колонки, которые заполняет дверь, он
    // считает сам. Побеждает он.
    let ids: Vec<String> = out.iter().map(|q| q.0.clone()).collect();
    tx.execute("DELETE FROM project_questions WHERE project_id = $1 AND id = ANY($2)",
               &[&project, &ids]).await?;
    for (id, number, title, kind, name, state, text, gate, opened, closed, has_answer, answer) in &out {
        // `has_answer` остаётся: он о ЗАГОЛОВКЕ и нужен, чтобы видеть расхождение
        // поля с текстом. Судит же `answer_state` — объявленное поле.
        tx.execute(
            "INSERT INTO project_questions(project_id, id, number, title, entity_kind, entity_name,
                                           state, state_text, gate, opened_at, closed_at, has_answer,
                                           answer_state, answer)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            &[&project, id, number, title, kind, name, state, text, gate, opened, closed, has_answer,
              &answer.0, &answer.1],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(out.len())
}
