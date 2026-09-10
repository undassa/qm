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

use deadpool_postgres::Pool;
use regex::Regex;

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
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
    let mut rows: Vec<(String, String, String, bool, bool, bool, Option<i32>)> = Vec::new();
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
        let разбор = crate::parse::parse_document(&content);
        let mut section_caveat = false;
        let mut секция: Option<i32> = None;
        for блок in &разбор.blocks {
            if блок.kind == "heading" {
                секция = Some(блок.ord);
                section_caveat = says_caveat(&блок.raw);
            }
            for line in блок.raw.lines() {
            let caveated = section_caveat || line.contains("~~") || says_caveat(line);
            // Голое число, оставшееся в строке-перечне: запись прячет имя
            // формой, которой раскрыватель не знает. Строка обязана СОДЕРЖАТЬ
            // хотя бы одно имя — иначе это не перечень, а проза с числом, и
            // «§8.1» или «п.3» стали бы находками.
            let есть_имя = !super::ids::plain(line).is_empty();
            for n in super::ids::hidden_numbers(line).into_iter().filter(|_| есть_имя) {
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
                        rows.push((kind.clone(), name.clone(), id, caveated, from_range, heads_row, секция));
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
    let mut addresses: Vec<(String, String, String, i32, String)> = Vec::new();
    let mut seen_addr = std::collections::HashSet::new();
    for d in &docs {
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
                    addresses.push((kind.clone(), name.clone(), c[1].to_owned(), n,
                                    if ok { anchor } else { String::new() }));
                }
            }
            for c in addr_plain.captures_iter(line) {
                let n: i32 = c[2].parse().unwrap_or(0);
                if seen_addr.insert(format!("{kind}\u{1}{name}\u{1}{}\u{1}{n}", &c[1])) {
                    addresses.push((kind.clone(), name.clone(), c[1].to_owned(), n, String::new()));
                }
            }
        }
    }

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    // Вставка ПАЧКАМИ, а не по строке. Здесь тринадцать тысяч имён, и запрос на
    // каждое — тринадцать тысяч обращений к базе: сорок две секунды из сорока
    // трёх, что занимала вся пересборка набора. Работа та же, ожидание — нет.
    //
    // Предел Postgres — 65535 параметров на запрос; при шести колонках это
    // десять тысяч строк, и тысяча в пачке оставляет запас на любую колонку,
    // которую сюда допишут.
    const BATCH: usize = 1000;
    tx.execute("DELETE FROM project_code_address WHERE project_id = $1", &[&project]).await?;
    for chunk in addresses.chunks(BATCH) {
        let mut sql = String::from(
            "INSERT INTO project_code_address(project_id, entity_kind, entity_name, path, line, anchor) VALUES ",
        );
        let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&project];
        for (i, (k, n, path, line, anchor)) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 5 + 2;
            sql.push_str(&format!("($1,${},${},${},${},${})", b, b + 1, b + 2, b + 3, b + 4));
            args.extend([k as &(dyn tokio_postgres::types::ToSql + Sync), n, path, line, anchor]);
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
        for (i, (kind, name, id, caveated, from_range, heads_row, секция)) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 7 + 2;
            sql.push_str(&format!("($1,${},${},${},${},${},${},${})", b, b + 1, b + 2, b + 3, b + 4, b + 5, b + 6));
            args.extend([kind as &(dyn tokio_postgres::types::ToSql + Sync), name, id, caveated,
                         from_range, heads_row, секция]);
        }
        sql.push_str(" ON CONFLICT DO NOTHING");
        tx.execute(sql.as_str(), &args).await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}
