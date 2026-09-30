//! Имена, названные документами набора.
//!
//! Имя, которого нет, стоит в живом документе и утверждает его существование.
//! Читатель идёт по нему и не находит ничего — и решает, что не понял, а не
//! что документ врёт.
//!
//! Раскрывается перечень тем же раскрывателем, что и везде: `FR-SIT-04, 07, 09`
//! — три имени, а не одно. Строка с оговоркой («удалено», «прежнее имя»)
//! называет имя ради истории, и это законно; оговорка помечается колонкой.
//!
//! ГДЕ имя названо — тоже колонка. Имя первой ячейкой строки таблицы документ
//! объявляет СВОИМ: так перечисляют состав. То же имя в прозе («тот же принцип,
//! что у `FR-SIG-10`») — ссылка на чужое, и считать их одним значит спрашивать с
//! документа за чужой состав. Различать это по тексту в запросе нельзя: строка
//! документа до запроса не доезжает.

/// Строка упоминания имени: вид, имя, где сказано и чем помечено.
type NamedIdRow = (String, String, String, bool, bool, bool, Option<i32>);

use deadpool_postgres::Pool;
use regex::Regex;

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let caveats: Vec<String> = client
        .query("SELECT value FROM scheme($1) WHERE role = 'word.caveat'", &[&project])
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    let docs = client
        .query(
            "SELECT entity_kind, entity_name, content FROM project_documents WHERE project_id = $1",
            &[&project],
        )
        .await?;

    let low_caveats: Vec<String> = caveats.iter().map(|c| c.to_lowercase()).collect();
    let says_caveat = |text: &str| -> bool {
        let low = text.to_lowercase();
        low_caveats.iter().any(|c| low.contains(c.as_str()))
    };
    let mut rows: Vec<NamedIdRow> = Vec::new();
    let mut hidden: Vec<(String, String, String, String)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut seen_hidden = std::collections::HashSet::new();
    for d in &docs {
        let kind: String = d.get(0);
        let name: String = d.get(1);
        let content: String = d.get(2);
        // Оговорка бывает трёх видов, и все три законны: слово в самой строке,
        // зачёркивание, и оговорённый ЗАГОЛОВОК — он накрывает свой раздел до
        // следующего заголовка того же уровня. Читать только строку значит
        // требовать оговорки в каждой строке раздела «Что удалено».
        // Обход по БЛОКАМ, а не по строкам: блок знает свой `ord`, а ord
        // блока-заголовка и есть ord его секции — тот же, каким она лежит в
        // `project_document_sections`. Без этого связь остаётся на уровне
        // документа: «ui-spec называет SCR-SHELL-01» при сорока килобайтах и
        // двадцати шести секциях.
        let parsed = crate::parse::parse_document(&content);
        let mut section_caveat = false;
        let mut section: Option<i32> = None;
        for block in &parsed.blocks {
            if block.kind == "heading" {
                section = Some(block.ord);
                section_caveat = says_caveat(&block.raw);
            }
            for line in block.raw.lines() {
            let caveated = section_caveat || line.contains("~~") || says_caveat(line);
            // Голое число, оставшееся в строке-перечне: запись прячет имя
            // формой, которой раскрыватель не знает. Строка обязана СОДЕРЖАТЬ
            // хотя бы одно имя — иначе это не перечень, а проза с числом, и
            // «§8.1» или «п.3» стали бы находками.
            let exists_name = !super::ids::plain(line).is_empty();
            for n in super::ids::hidden_numbers(line).into_iter().filter(|_| exists_name) {
                let key = format!("{kind}\u{1}{name}\u{1}голое {n}");
                if seen_hidden.insert(key) {
                    hidden.push((kind.clone(), name.clone(), n, line.trim().chars().take(90).collect::<String>()));
                }
            }
            // Две формы одной строки: написанное целиком и раскрытое из
            // перечня. Правило указателя читает первую, правило покрытия — обе.
            let written = super::ids::plain(line);
            let heading = super::ids::heading_cell(line);
            for id in super::ids::expand(line) {
                let from_range = !written.contains(&id);
                let heads_row = heading.contains(&id);
                let key = format!("{kind}\u{1}{name}\u{1}{id}");
                match seen.get(&key) {
                    // Имя, названное документом дважды — прозой и таблицей, —
                    // названо и таблицей. Брать первое попавшееся значило бы
                    // терять состав из-за порядка строк в файле.
                    Some(&i) => { if heads_row { rows[i].5 = true } }
                    None => {
                        seen.insert(key, rows.len());
                        rows.push((kind.clone(), name.clone(), id, caveated, from_range, heads_row, section));
                    }
                }
            }
            }
        }
    }

    // Адреса «файл:строка» и якорь за ними. Якорь — первый кусок в бэктиках
    // или в кавычках-ёлочках сразу после адреса; без него адрес проверяется
    // только существованием строки.
    let addr = Regex::new(r"`([A-Za-z0-9_./-]+\.(?:rs|sql|ts|tsx|yaml|toml)):([0-9]+)`(.{0,4}?[`«])([^`»]{2,120})[`»]")
        .expect("образец адреса с якорем");
    let addr_plain = Regex::new(r"`([A-Za-z0-9_./-]+\.(?:rs|sql|ts|tsx|yaml|toml)):([0-9]+)`")
        .expect("образец адреса");
    // ПРЕДМЕТ, НАЗВАННЫЙ ФРАЗОЙ У АДРЕСА. Берётся последний токен в обратных
    // кавычках ПЕРЕД адресом и в пределах той же фразы: ячейки таблицы,
    // предложения или скобки. По всей строке брать нельзя — у `Q-123` строка
    // таблицы длинная, в её начале `writes`, и два адреса рядом с разными
    // именами получили бы одно чужое.
    let elem = Regex::new(r"`([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)`")
        .expect("образец имени предмета");
    let mut addresses: Vec<(String, String, String, i32, String, String)> = Vec::new();
    let mut seen_addr = std::collections::HashSet::new();
    // АДРЕС ВНУТРИ СПРАВКИ — АДРЕС ЧУЖОГО ДЕРЕВА. Виды с проекцией
    // `provenance` («ценность в том, откуда взято») держат материал соседнего
    // продукта: у `tot-ade` из 112 адресов 40 указывали в `packages/`, `cmd/`,
    // `routes/` — каталогов, которых в этом репозитории нет вовсе. Правило
    // мерило наш код чужими адресами и давало 39 нарушений из 39.
    //
    // Отбор идёт по ОБЪЯВЛЕННОЙ проекции, а не по имени вида: перечислить
    // здесь `reference` значило бы зашить слово, которым владеет проект.
    let foreign: std::collections::HashSet<String> = client
        .query(
            "SELECT name FROM kind_layout WHERE spec->>'projection' = 'provenance'",
            &[],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    for d in &docs {
        if foreign.contains(&d.get::<_, String>(0)) {
            continue;
        }
        let kind: String = d.get(0);
        let name: String = d.get(1);
        let content: String = d.get(2);
        for line in content.lines() {
            for c in addr.captures_iter(line) {
                let n: i32 = c[2].parse().unwrap_or(0);
                // Якорь берётся, только если он ЦЕЛЫЙ и самостоятельный:
                // сокращённый многоточием не сравнить буквально, а обрывок,
                // начинающийся с двоеточия, — хвост соседнего адреса, а не
                // цитата строки. Оба давали ложную находку.
                let anchor = c[4].trim().to_owned();
                let ok = anchor.chars().count() >= 3
                    && !anchor.contains('…')
                    && !anchor.starts_with(':');
                if seen_addr.insert(format!("{kind}\u{1}{name}\u{1}{}\u{1}{n}", &c[1])) {
                    let named = named_before(&elem, line, c.get(0).map_or(0, |m| m.start()));
                    addresses.push((kind.clone(), name.clone(), c[1].to_owned(), n,
                                    if ok { anchor } else { String::new() }, named));
                }
            }
            for c in addr_plain.captures_iter(line) {
                let n: i32 = c[2].parse().unwrap_or(0);
                if seen_addr.insert(format!("{kind}\u{1}{name}\u{1}{}\u{1}{n}", &c[1])) {
                    let named = named_before(&elem, line, c.get(0).map_or(0, |m| m.start()));
                    addresses.push((kind.clone(), name.clone(), c[1].to_owned(), n, String::new(), named));
                }
            }
        }
    }

    // Читающее соединение отпущено: писать и читать одновременно здесь нечем,
    // а два соединения на одну проекцию запирают пул на себе же.
    drop(client);
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    // Вставка ПАЧКАМИ, а не по строке. Здесь тринадцать тысяч имён, и запрос на
    // каждое — тринадцать тысяч обращений к базе: сорок две секунды из сорока
    // трёх, что занимала вся пересборка набора. Работа та же, ожидание — нет.
    //
    // Предел Postgres — 65535 параметров на запрос; при семи колонках это
    // девять тысяч строк, и тысяча в пачке оставляет запас на любую колонку,
    // которую сюда допишут.
    const BATCH: usize = 1000;
    tx.execute("DELETE FROM project_code_address WHERE project_id = $1", &[&project]).await?;
    for chunk in addresses.chunks(BATCH) {
        let mut sql = String::from(
            "INSERT INTO project_code_address(project_id, entity_kind, entity_name, path, line, anchor, named) VALUES ",
        );
        let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&project];
        for (i, (k, n, path, line, anchor, named)) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 6 + 2;
            sql.push_str(&format!("($1,${},${},${},${},${},${})", b, b + 1, b + 2, b + 3, b + 4, b + 5));
            args.extend([k as &(dyn tokio_postgres::types::ToSql + Sync), n, path, line, anchor, named]);
        }
        sql.push_str(" ON CONFLICT DO NOTHING");
        tx.execute(sql.as_str(), &args).await?;
    }
    tx.execute("DELETE FROM project_hidden_name WHERE project_id = $1", &[&project]).await?;
    for chunk in hidden.chunks(BATCH) {
        let mut sql = String::from(
            "INSERT INTO project_hidden_name(project_id, entity_kind, entity_name, number, line) VALUES ",
        );
        let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&project];
        for (i, (kind, name, num, line)) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 4 + 2;
            sql.push_str(&format!("($1,${},${},${},${})", b, b + 1, b + 2, b + 3));
            args.extend([kind as &(dyn tokio_postgres::types::ToSql + Sync), name, num, line]);
        }
        sql.push_str(" ON CONFLICT DO NOTHING");
        tx.execute(sql.as_str(), &args).await?;
    }
    tx.execute("DELETE FROM project_named_id WHERE project_id = $1", &[&project]).await?;
    for chunk in rows.chunks(BATCH) {
        let mut sql = String::from(
            "INSERT INTO project_named_id(project_id, entity_kind, entity_name, said_id, caveated, from_range, heads_row, section_ord) VALUES ",
        );
        let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&project];
        for (i, (kind, name, id, caveated, from_range, heads_row, section)) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 7 + 2;
            sql.push_str(&format!("($1,${},${},${},${},${},${},${})", b, b + 1, b + 2, b + 3, b + 4, b + 5, b + 6));
            args.extend([kind as &(dyn tokio_postgres::types::ToSql + Sync), name, id, caveated,
                         from_range, heads_row, section]);
        }
        sql.push_str(" ON CONFLICT DO NOTHING");
        tx.execute(sql.as_str(), &args).await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

/// Предмет, названный фразой ПЕРЕД адресом, — последний сегмент его имени.
///
/// **Граница фразы, а не строки.** У `Q-123` строка таблицы держит два адреса,
/// у каждого своё имя рядом, а в начале строки стоит третье. Разбор по строке
/// приписал бы начальное имя обоим. Границей служат знаки, которыми фраза
/// кончается: разделитель ячейки, точка, точка с запятой, скобка, тире.
///
/// **Последний сегмент, а не любой.** Сверка по объемлющему имени прячет съезд
/// внутри него: у адреса, съехавшего с `EventMsg::Reverted` на
/// `EventMsg::Applied`, объемлющее `EventMsg` совпадает, и правило сказало бы
/// «верно». Прочитано на четырнадцати живых адресах: по последнему сегменту
/// восемь съехавших видны все восемь.
///
/// **Имя файла не предмет.** `scope.rs` и `error.rs` называют файл, а адрес и
/// так его называет; сверять имя файла с самим собой — проверка ни о чём.
fn named_before(elem: &Regex, line: &str, at: usize) -> String {
    let head = &line[..at];
    // ТИРЕ НЕ ГРАНИЦА: в живых фразах оно СОЕДИНЯЕТ имя с адресом — «событие
    // `EventMsg::Reverted` — `event.rs:406`», — и, посчитанное границей,
    // отрезало имя у каждой такой фразы. Скобка — то же самое: она вводит
    // адрес сразу за именем («`delegation_narrows` (`policy.rs:167`)»), и как
    // граница отрезала имя. Границы только там, где фраза и правда кончается:
    // ячейка таблицы, точка с запятой, точка. Обе ошибки нашла проверка ниже —
    // сперва на тире, потом на скобке, и обе на живых фразах набора.
    //
    // Считается по СИМВОЛАМ, а не по байтам: тире трёхбайтовое, и `rfind + 1`
    // резал бы строку внутри него — срез по такой границе паникует, то есть
    // отказывает в проде. Обе ошибки поймала проверка ниже, до выкладки.
    let start = head
        .char_indices()
        .rfind(|(_, c)| matches!(c, '|' | ';'))
        .map_or(0, |(i, c)| i + c.len_utf8())
        .max(head.rfind(". ").map_or(0, |i| i + 2));
    elem.captures_iter(&head[start..])
        .filter_map(|c| {
            let whole = c[1].to_owned();
            let last = whole.rsplit("::").next().unwrap_or(&whole).to_owned();
            (!last.is_empty() && !whole.contains('.')).then_some(last)
        })
        .last()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::named_before;
    use regex::Regex;

    /// Фикстура — живые фразы четырнадцати адресов, прочитанных глазами.
    ///
    /// Три из них правило оценивало неверно, пока имя брали из всей строки, а
    /// не из фразы у адреса. Самый злой вход — строка таблицы `Q-123`: в её
    /// начале стоит `writes`, а рядом с двумя адресами — свои имена. Разбор по
    /// строке приписывал начальное имя обоим и давал две ложные находки.
    #[test]
    fn a_name_is_taken_from_the_phrase_not_the_line() {
        let elem = Regex::new(r"`([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)`").unwrap();
        let cases: &[(&str, &str, &str)] = &[
            ("событие `EventMsg::Reverted` — `crates/tot-protocol/src/event.rs:406`",
             "crates/tot-protocol/src/event.rs:406", "Reverted"),
            ("состояние `DeltaState::Reverted` в `crates/tot-protocol/src/delta.rs:192`",
             "crates/tot-protocol/src/delta.rs:192", "Reverted"),
            ("| `writes` | пункты 1 и 2 в коде (`crates/tot-harness/src/policy.rs:98`) | `delegation_narrows` (`crates/tot-harness/src/policy.rs:167`) |",
             "crates/tot-harness/src/policy.rs:167", "delegation_narrows"),
            ("в `crates/tot-tui/src/frame/tabs.rs:34` и `scope.rs:21`",
             "scope.rs:21", ""),
        ];
        for (line, addr, want) in cases {
            let at = line.find(&format!("`{addr}`")).expect("адрес во фразе");
            let got = named_before(&elem, line, at);
            assert_eq!(
                &got, want,
                "фраза «{line}»: у адреса {addr} названо «{got}», а фраза называет «{want}»"
            );
        }
    }
}
