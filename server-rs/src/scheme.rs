//! Словарь схемы: роль, которую знает код, и слово, которым её зовёт набор.
//!
//! Бинарник обязан работать с любым проектом. Слово, зашитое в него, делает
//! его знающим один: набор, назвавший раздел иначе, получил бы не отказ, а
//! молчаливый ноль — правило нашло бы пусто и позеленело.
//!
//! Отсюда правило: **умолчаний нет**. Роль, для которой значение не объявлено,
//! называется вслух, и правило, на неё опирающееся, отвечает «не знаю», а не
//! «сошлось».

use deadpool_postgres::Pool;
use std::collections::HashMap;

pub struct Terms {
    by_role: HashMap<String, Vec<String>>,
}


/// РОЛИ, КОТОРЫЕ СПРАШИВАЮТ ПРОЕКЦИИ, и чем платит набор за молчание каждой.
///
/// Роль, которую набор не объявил, не «находит ноль нарушений» — она ВЫКЛЮЧАЕТ
/// правило: считать нечем, и правило честно молчит. Снаружи это неотличимо от
/// зелёного, и узнать, чего не хватает, можно было только чтением исходника.
/// Один набор объявил за день семь таких ролей, каждую — после того, как нашёл
/// её в коде.
///
/// Перечень держится РЯДОМ С ЧИТАТЕЛЕМ и сверяется с ним пробой ниже: список,
/// который разошёлся бы с местами вызова, врал бы ровно там, где нужен.
///
/// Второе поле — КАК роль спрашивают. Роль, спрошенную одним словом, два
/// объявленных значения выключают так же насмерть, как ноль: `one` про
/// двусмысленную честно отвечает `None`. А роль-список двумя значениями не
/// ломается — она для того и список, и звать это двусмысленностью значит
/// поднимать тревогу на здоровом.
/// ОБРАЗЕЦ ИМЕНИ ВИДА — ОДНИМ ЧИТАТЕЛЕМ НА ВСЕХ.
///
/// Как выглядит имя задачи, требования, проверки — записано в раскладке вида
/// (`kind_layout.spec->>'id'`), а набор, которому общий образец не подходит,
/// поправляет его ролью `id.<вид>`. Эти двое — власть, и правило «имя следует
/// образцу своего вида» судит по ним.
///
/// Всё остальное было КОПИЯМИ, и они разошлись. Цена померена: `relations.rs`
/// искал задачи образцом `M[0-9]+-T…` — у `tot-ade` 82 задачи из 164 названы на
/// `V`, и для этого модуля их не существовало. `repo_corpus.rs` требовал букв в
/// середине имени требования — подходило 55 имён из 203. Разошлись они молча:
/// сравнивать копии между собой было некому.
pub(crate) async fn id_pattern(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    kind: &str,
) -> Result<Vec<String>, crate::db::Fail> {
    let own = client
        .query(
            "SELECT value FROM scheme($1) WHERE role = 'id.' || $2 ORDER BY ord, value",
            &[&project, &kind],
        )
        .await?;
    // ПУСТОЙ ОБРАЗЕЦ — ЭТО ОТСУТСТВИЕ ОБРАЗЦА, А НЕ ОБРАЗЕЦ «ЧТО УГОДНО».
    //
    // Дверь `kind-add` кладёт `"id": ""`, когда образца не назвали, — и это
    // правда о видах-одиночках: у `srs`, `glossary`, `feature` имя свободное.
    // Но пустая строка, дошедшая до `Regex::new(r"\b(?:)\b")`, разбирается и
    // совпадает с нулевой длиной в любом месте любого значения. В `relations`
    // это стоило бы всех красных задач разом: родителем каждой стала бы пустая
    // строка, а `red_task` отбирает по `parent_task_id <> ''`.
    let live = |r: &tokio_postgres::Row| {
        let v: Option<String> = r.get(0);
        v.filter(|v| !v.trim().is_empty())
    };
    let own: Vec<String> = own.iter().filter_map(live).collect();
    if !own.is_empty() {
        return Ok(own);
    }
    // Раскладка общая на все наборы; набора, поправившего образец, здесь уже нет.
    Ok(client
        .query_opt("SELECT spec->>'id' FROM kind_layout WHERE name = $1", &[&kind])
        .await?
        .as_ref()
        .and_then(live)
        .into_iter()
        .collect())
}

pub(crate) const ROLES: &[(&str, &str, &str)] = &[
    ("field.red-checks", "all", "перечень проверок красной задачи: без него `red-checks-match-parent` не с чем сверять"),
    ("field.red-parent", "all", "пара красной задачи: без неё у красных нет родителя, и `red_task` роняет их все"),
    ("field.requirements", "one", "требования экрана: без них связь экран→требование не выводится вовсе"),
    ("field.task-contract-ops", "all", "операции контракта у задачи: без них `task-names-contract-ops` слеп"),
    ("id.check", "all", "образец имени проверки: без него доказательством считается только `TC-`"),
    ("id.requirement", "all", "образец имени требования: без него не разбирается ни одна строка держателя инварианта"),
    ("marker.verified-by", "one", "строка «Проверяется:» у требования: без неё доказательство, названное прозой, не читается"),
    ("marker.milestone-requirements", "one", "строка требований в вехе: без неё требования не попадают ни в один этап"),
    ("marker.surface-list", "one", "перечень поверхностей: без него `surface-set-closed` считает по пустому"),
    ("path.crate-home", "all", "где живут крейты: без этого `crate-declared` не находит дерева"),
    ("section.proof", "all", "раздел доказательства задачи: без него связь задача→проверка пуста"),
    ("section.tree", "one", "раздел дерева задачи: без него листья дерева не разбираются"),
    ("word.caveat", "all", "слово оговорки: без него снятое имя считается живым"),
    ("word.elsewhere", "all",
     "слово отказа: «здесь не закрывается» — строка называет имя, ничего им не обещая"),
    ("word.not-a-subject", "all", "слова, не являющиеся предметом: без них предметом становится что попало"),
    ("word.total", "all", "слово итога таблицы: без него итог читается нулём"),
];

impl Terms {
    /// Словарь ЭТОГО набора: своё, если роль объявлена, иначе общее.
    pub async fn load(pool: &Pool, project: &str) -> Result<Self, crate::db::Fail> {
        let client = crate::db::conn(pool).await?;
        Self::load_at(&*client, project).await
    }

    /// То же на ГОТОВОМ соединении: словарь читается тем же соединением, что
    /// уже держит вызывающий. Второе соединение при открытой транзакции —
    /// способ запереть пул на себе же.
    pub(crate) async fn load_at(
        client: &impl deadpool_postgres::GenericClient,
        project: &str,
    ) -> Result<Self, crate::db::Fail> {
        let rows = client
            .query("SELECT role, value FROM scheme($1) ORDER BY role, ord, value", &[&project])
            .await?;
        let mut by_role: HashMap<String, Vec<String>> = HashMap::new();
        for r in &rows {
            by_role.entry(r.get(0)).or_default().push(r.get(1));
        }
        Ok(Terms { by_role })
    }

    /// Единственное слово роли. `None` — роль не объявлена, и это НЕ пустая
    /// строка: пустая строка совпала бы с пустым значением в базе и правило
    /// нашло бы «всё», а не «ничего».
    ///
    /// `None` и при ДВУСМЫСЛЕННОСТИ: у роли два слова, а спрашивают одно.
    /// Прежде брался первый по порядку и о втором не говорилось. На `myack` у
    /// роли `field.red-parent` стояли «Пара» и «Родительская задача», обе с
    /// нулевым порядком; взялось не то, и восемьдесят две красные задачи разом
    /// потеряли родителя — два пункта покраснели на сто шестьдесят четыре
    /// нарушения, а выглядело это порчей набора.
    ///
    /// Молча выбирать нельзя. Правило без слова не считается и говорит об этом;
    /// правило с двумя словами не считается тем более — оно не знает, каким.
    pub(crate) fn one(&self, role: &str) -> Option<&str> {
        match self.by_role.get(role) {
            Some(v) if v.len() == 1 => v.first().map(String::as_str),
            _ => None,
        }
    }


    /// Все слова роли: список стоп-слов, синонимы заголовка.
    pub(crate) fn all(&self, role: &str) -> &[String] {
        static EMPTY: Vec<String> = Vec::new();
        self.by_role.get(role).unwrap_or(&EMPTY)
    }

    /// Роли, которых нет. Их называют вслух: правило без слова не считается.
    pub(crate) fn missing(&self, roles: &[&str]) -> Vec<String> {
        roles
            .iter()
            .filter(|r| !self.by_role.contains_key(**r))
            .map(|r| (*r).to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(pairs: &[(&str, &str)]) -> Terms {
        let mut by_role: HashMap<String, Vec<String>> = HashMap::new();
        for (r, v) in pairs {
            by_role.entry((*r).to_owned()).or_default().push((*v).to_owned());
        }
        Terms { by_role }
    }

    #[test]
    fn one_word_roles_returned() {
        assert_eq!(terms(&[("field.parent", "Родитель")]).one("field.parent"), Some("Родитель"));
    }

    /// Ровно тот слом, что стоил восьмидесяти двух красных задач: у роли два
    /// слова, бралось первое по порядку, и о втором никто не узнавал.
    #[test]
    fn two_words_for_one_role_are_not_chosen_silently() {
        let t = terms(&[("field.red-parent", "Пара"), ("field.red-parent", "Родительская задача")]);
        assert_eq!(t.one("field.red-parent"), None);
        assert_eq!(t.all("field.red-parent").len(), 2, "оба слова видны перечнем, а не выбираются молча");
    }

    #[test]
    fn an_undeclared_role_and_one_declared_twice_differ() {
        let t = terms(&[("a", "x"), ("a", "y")]);
        assert_eq!(t.missing(&["b"]), vec!["b".to_owned()], "необъявленная роль называется");
        assert!(t.missing(&["a"]).is_empty(), "объявленная дважды — объявлена");
        assert_eq!(t.one("a"), None, "из двух слов роли не выбирается ни одно");
    }
}

#[cfg(test)]
mod roles {
    /// Перечень ролей сверяется С МЕСТАМИ ВЫЗОВА, а не с памятью правившего.
    ///
    /// Список, живущий отдельно от читателей, расходится с ними молча — и врёт
    /// ровно там, где нужен: при вопросе «чего набору не хватает».
    #[test]
    fn list_matches_with_that_that_asked() {
        // ЧИТАТЕЛЕЙ ИЩЕМ НА ДИСКЕ, А НЕ В СПИСКЕ.
        //
        // Список стоял руками и отстал ровно так, как и должен был:
        // `traceability_said.rs` спрашивал `word.total` набором, реестр объявлял
        // её одиночной, а обход этого файла не видел и молчал. Список, который
        // надо помнить пополнять, — тот же дрейф, от которого этот тест заведён.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut queue = vec![root];
        let mut sources: Vec<(String, String)> = Vec::new();
        while let Some(dir) = queue.pop() {
            for entry in std::fs::read_dir(&dir).expect("каталог исходников") {
                let path_of = entry.expect("запись каталога").path();
                if path_of.is_dir() {
                    queue.push(path_of);
                } else if path_of.extension().is_some_and(|e| e == "rs") {
                    // Спрошенное под `#[cfg(test)]` — заглушка, а не читатель:
                    // сам этот файл зовёт `.one("field.parent")` на выдуманном
                    // наборе, и считать его потребителем роли значит требовать
                    // объявления того, чего в работе никто не спрашивает.
                    let text_of = std::fs::read_to_string(&path_of).expect("исходник");
                    let working = text_of
                        .split_once("#[cfg(test)]")
                        .map_or(text_of.as_str(), |(before, _)| before)
                        .to_owned();
                    sources.push((path_of.display().to_string(), working));
                }
            }
        }
        assert!(sources.len() > 10, "исходников не нашлось: {}", sources.len());
        // Точка и вызов бывают на РАЗНЫХ строках: `.all("id.check")` стоит под
        // своим `terms`, и обход, требовавший их рядом, прошёл мимо двух ролей.
        let pattern = regex::Regex::new(r#"\.\s*(one|all)\("([a-z.\-]+)"\)"#).expect("образец роли");
        let declared: std::collections::HashMap<&str, &str> =
            super::ROLES.iter().map(|(r, how, _)| (*r, *how)).collect();
        let mut not_named: Vec<String> = Vec::new();
        for (_, text_of) in &sources {
            for c in pattern.captures_iter(text_of) {
                let (how, role_name) = (c[1].to_owned(), c[2].to_owned());
                match declared.get(role_name.as_str()) {
                    None => {
                        if !not_named.contains(&role_name) {
                            not_named.push(role_name);
                        }
                    }
                    // Арность тоже сверяется: список, записанный как «одним
                    // словом», поднял бы тревогу на здоровом — два значения у
                    // роли-списка законны.
                    Some(was) if *was != how => {
                        let word = format!("{role_name}: спрашивают `{how}`, записано `{was}`");
                        if !not_named.contains(&word) {
                            not_named.push(word);
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        assert!(
            not_named.is_empty(),
            "роли спрашиваются и не названы в `ROLES`: {}. \
             Набор о них не узнает, и правило будет молчать без объяснения",
            not_named.join(" · ")
        );
    }
}

#[cfg(test)]
mod patterns_names {
    /// ОДИН ВИД — ОДНО ПРАВИЛО ИМЕНОВАНИЯ, и здесь считаются копии.
    ///
    /// Как выглядит имя задачи или требования, записано в раскладке вида
    /// (`kind_layout.spec->>'id'`) и поправляется набором ролью `id.<вид>`.
    /// Это власть, и по ней судит правило «имя следует образцу своего вида».
    ///
    /// Но то же самое было переписано регулярками в десятке модулей, и они
    /// разошлись: `relations.rs` знал задачи только на `M`, а раскладка говорит
    /// `[MmVv]`; `repo_corpus.rs` требовал букв в середине имени требования, а
    /// под это не подходит 148 имён из 203 у `tot-ade`; разбор имён держал свой
    /// список приставок, где `M` и `V` нет вовсе. Расходились молча: сравнивать
    /// копии было некому.
    ///
    /// Проба считает копии и держит их число. Она НЕ запрещает зашитый образец —
    /// у клиента, читающего репозиторий, базы под рукой нет, и там он законен.
    /// Она запрещает завести НОВЫЙ, не сказав об этом: линия двигается правкой
    /// этого числа, и правка видна в разборе.
    #[test]
    fn copies_pattern_name_not_grew() {
        let sources: &[(&str, &str)] = &[
            ("reproject/relations.rs", include_str!("reproject/relations.rs")),
            ("reproject/surface.rs", include_str!("reproject/surface.rs")),
            ("reproject/runs.rs", include_str!("reproject/runs.rs")),
            ("reproject/plan.rs", include_str!("reproject/plan.rs")),
            ("reproject/plan_status.rs", include_str!("reproject/plan_status.rs")),
            ("reproject/proof.rs", include_str!("reproject/proof.rs")),
            ("reproject/needs.rs", include_str!("reproject/needs.rs")),
            ("reproject/decisions.rs", include_str!("reproject/decisions.rs")),
            ("reproject/ids.rs", include_str!("reproject/ids.rs")),
            ("repo_corpus.rs", include_str!("repo_corpus.rs")),
        ];
        // Имя вида в регулярке: `FR-`, `TC-`, `US-`, `SCR-`, `ST-`, `[MV]\d-T`.
        let pattern = regex::Regex::new(
            r#"Regex::new\(r"[^"]*(?:\(\?:)?(?:FR|NFR|TC|US|SCR|ST)\b|Regex::new\(r"[^"]*\[MmVv\]|Regex::new\(r"[^"]*\[MV\]"#,
        )
        .expect("образец копии");
        let mut found: Vec<String> = Vec::new();
        for (name_said, text_of) in sources {
            let n = pattern.find_iter(text_of).count();
            if n > 0 {
                found.push(format!("{name_said}: {n}"));
            }
        }
        let total: usize = found
            .iter()
            .filter_map(|s| s.rsplit(": ").next()?.parse::<usize>().ok())
            .sum();
        // Линия: столько копий было, когда её провели. Меньше — опустите число.
        const LINE: usize = 22;
        assert!(
            total <= LINE,
            "образцов имени, зашитых мимо раскладки, стало больше: {total} против {LINE}. \
             Берите образец из `kind_layout`/`scheme` — иначе он разойдётся с раскладкой молча. \
             Где: {}",
            found.join(" · ")
        );
        if total < LINE {
            eprintln!("копий стало меньше ({total} из {LINE}) — опустите линию");
        }
    }
}

#[cfg(test)]
mod latin_only {
    /// В КОДЕ — ТОЛЬКО ЛАТИНИЦА. Правило держалось одной большой уборкой, и
    /// первая же правка после неё вернула кириллическое имя: `команда` в
    /// проде и два имени в тесте. Правило, которое некому проверить, живёт до
    /// первой спешки — этот обход и есть его проверка.
    ///
    /// Комментарии и строки НЕ трогаются: сообщения людям и разбор ошибок
    /// написаны по-русски, и это отдельное решение.
    #[test]
    fn no_cyrillic_identifiers_in_code() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut queue = vec![root];
        let mut found: Vec<String> = Vec::new();
        while let Some(dir) = queue.pop() {
            for entry in std::fs::read_dir(&dir).expect("каталог исходников") {
                let path = entry.expect("запись каталога").path();
                if path.is_dir() {
                    queue.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("файл исходника");
                for (n, line) in code_only(&text).lines().enumerate() {
                    if let Some(name) = cyrillic_identifier(line) {
                        found.push(format!("{}:{}: {name}", path.display(), n + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "кириллица в именах:\n{}", found.join("\n"));
    }

    /// Строки и комментарии вырезаются: в них кириллица законна. Разбор идёт
    /// по всему файлу, а не построчно: в коде есть многострочные строки — SQL
    /// пачками, — и построчный разбор принял бы их за код.
    fn code_only(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            let next = bytes.get(i + 1).copied();
            if c == '/' && next == Some('/') {
                while i < bytes.len() && bytes[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if c == '/' && next == Some('*') {
                let mut depth = 1;
                i += 2;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == '/' && bytes.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == '*' && bytes.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        if bytes[i] == '\n' {
                            out.push('\n');
                        }
                        i += 1;
                    }
                }
                continue;
            }
            if c == 'r' && matches!(next, Some('#') | Some('"')) {
                let mut hashes = 0;
                let mut j = i + 1;
                while bytes.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if bytes.get(j) == Some(&'"') {
                    j += 1;
                    loop {
                        if j >= bytes.len() {
                            break;
                        }
                        if bytes[j] == '\n' {
                            out.push('\n');
                        }
                        if bytes[j] == '"' && (1..=hashes).all(|k| bytes.get(j + k) == Some(&'#')) {
                            j += hashes + 1;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
            }
            // Знаковый литерал `'"'` — не начало строки, а буква. Пока он им
            // считался, разбор терял кавычку и читал остаток файла наизнанку.
            if c == '\'' {
                let escaped = next == Some('\\');
                let end = if escaped { i + 3 } else { i + 2 };
                if bytes.get(end) == Some(&'\'') {
                    i = end + 1;
                    continue;
                }
                // Иначе это имя времени жизни: `'a`, `'static` — обычный код.
                out.push(c);
                i += 1;
                continue;
            }
            if c == '"' {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == '\n' {
                        out.push('\n');
                    }
                    if bytes[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }

    fn cyrillic_identifier(line: &str) -> Option<String> {
        let mut name = String::new();
        for c in line.chars() {
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
                continue;
            }
            if name.chars().any(|c| ('а'..='я').contains(&c) || ('А'..='Я').contains(&c) || c == 'ё' || c == 'Ё') {
                return Some(name);
            }
            name.clear();
        }
        None
    }
}
