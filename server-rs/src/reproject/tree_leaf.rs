//! Листья блока «Что меняется в дереве» задачи.
//!
//! Блок — не украшение, а место для кода. Не назвал его путём — выберет
//! исполнитель, и выберет иначе, чем сосед. Прежняя проверка искала только
//! Rust-крейты в бэктиках, поэтому фронтенд и мобильное проходили мимо: имена
//! React-компонентов без каталога и расширения считались за путь.
//!
//! Законное исключение объявляется в самом документе: строка каталога,
//! называющая вопрос реестра (`Q-nn`), означает «место ещё не решено», а не
//! «место не нужно». Решится вопрос — строка станет путём, и исключение
//! исчезнет само.

use once_cell::sync::Lazy;
use regex::Regex;

/// Соединитель дерева: `├──`, `└─`. Строка без него записью не является.
static CONNECTOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[\s│]*[├└]─+\s*").expect("образец связки"));
static DECOR_HEAD: Lazy<Regex> = Lazy::new(|| Regex::new("^[\\s`«»\"'“”·—–-]+").expect("образец украшений слева"));
static DECOR_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new("[\\s`«»\"'“”,.;:]+$").expect("образец украшений справа"));
/// Из чего состоит путь — ЗАМЕРЕНО НА ДИСКЕ, а не выбрано на вкус: среди имён
/// пятнадцати тысяч файлов репозитория нет ни одного не-ASCII символа и ни
/// одного пробела.
static PATH_CHARS: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9_@./+-]+$").expect("образец знаков пути"));
static TREE_EXT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\.(rs|ts|tsx|js|mjs|cjs|sql|ya?ml|toml|json|md|css|sh|swift|kt)$").expect("образец расширения"));
static QUESTION: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bQ-\d+").expect("образец вопроса"));

/// Лист записи дерева. `None` — строка записью не является: продолжение
/// комментария, заголовок, проза.
pub(crate) fn leaf_of(raw_body: &str) -> Option<String> {
    if !CONNECTOR.is_match(raw_body) {
        return None;
    }
    let entry = CONNECTOR.replace(raw_body, "");
    let entry = DECOR_HEAD.replace(&entry, "");
    let head = entry.split_whitespace().next().unwrap_or("");
    Some(DECOR_TAIL.replace(head, "").into_owned())
}

/// Лист ПЛОСКОЙ записи — `+ crates/tot-page/src/lib.rs`, без рисунка дерева.
///
/// Один набор рисует дерево, второй пишет пути списком, и список разбор не
/// видел вовсе: у tot-ade ни одного листа, а значит, ни одной проверки места.
/// Строка без рисунка бывает и продолжением комментария («!  транзакцией…»),
/// поэтому листом она становится, только если первое слово — путь: с `/` или
/// файл с расширением из перечня, как `docker-compose.yml` в корне.
fn flat_leaf(raw_body: &str) -> Option<String> {
    let entry = DECOR_HEAD.replace(raw_body, "");
    let head = entry.split_whitespace().next().unwrap_or("");
    let leaf = DECOR_TAIL.replace(head, "").into_owned();
    (PATH_CHARS.is_match(&leaf) && (leaf.contains('/') || TREE_EXT.is_match(&leaf))).then_some(leaf)
}

/// Похоже ли на путь. Пустой лист путём считается: это уже не наша находка.
pub(crate) fn looks_like_path(leaf: &str) -> bool {
    leaf.is_empty() || (PATH_CHARS.is_match(leaf) && (leaf.ends_with('/') || TREE_EXT.is_match(leaf)))
}

pub(crate) struct Leaf {
    pub dir: String,
    pub leaf: String,
    pub is_path: bool,
    pub exempt: bool,
    /// `+` — файла нет, задача его создаёт; `!` — есть и правится; `-` — снимается.
    pub op: char,
    /// Полный путь листа, приведённый (`full_path`).
    pub path: String,
    /// Каталог, который должен уже быть, чтобы лист лёг: родитель листа, а если
    /// родителя создаёт эта же задача (`+ каталог/`) — первый предок, которого
    /// она не создаёт.
    pub target: String,
}

/// Разбирает блок задачи в перечень листьев.
pub(crate) fn leaves_of(content: &str, section: &str) -> Vec<Leaf> {
    // Имя раздела приходит из словаря схемы: зашитое, оно сделало бы разбор
    // знающим один набор, а другой отдал бы пустоту вместо отказа.
    if section.is_empty() {
        return Vec::new();
    }
    let head = format!("## {section}");
    let Some(after) = content.split(head.as_str()).nth(1) else { return Vec::new() };
    // Блок не выходит за свой раздел: без ограды `diff` разбор шёл до ближайшей
    // ограды документа, и пункт списка ниже («- `crates/x.rs` …») становился
    // листом со снятием.
    let section = after.split("\n## ").next().unwrap_or(after);
    let block = section.split("```diff").nth(1).unwrap_or(section);
    let block = block.split("```").next().unwrap_or("");
    let mut out = Vec::new();
    let mut dir = String::new();
    let mut unsettled: Option<String> = None;
    // Каталоги рисунка, под которыми стоит следующая запись: колонка соединителя,
    // имя и место, которое строка каталога объявила нерешённым. Запись глубже
    // каталога лежит в нём — чем бы ни был нарисован отступ, `│` или пробелами.
    // Без этого `seed/account.rs` читался как `account.rs` рядом с `seed/`, и на
    // диске его не находили.
    let mut nest: Vec<(usize, String, Option<String>)> = Vec::new();
    for raw in block.split('\n') {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let mark = line.chars().next().unwrap_or(' ');
        let raw_body: String = line.chars().skip(1).collect();
        if !matches!(mark, ' ' | '+' | '!' | '-') {
            continue;
        }
        let Some(leaf) = leaf_of(&raw_body) else {
            // Строка без рисунка — путь целиком. Каталогом она задаёт место следующим
            // записям рисунка; без пометки — это заголовок, и только он может нести
            // оговорку про нерешённый вопрос. С пометкой она ещё и лист.
            if raw_body.chars().take_while(|c| c.is_whitespace()).count() > 3 {
                continue;
            }
            if mark == ' ' {
                let head = raw_body.trim_start().split("  ").next().unwrap_or("").trim();
                if head.contains('/') {
                    dir = if head.ends_with('/') { head.to_owned() } else { format!("{head}/") };
                    unsettled = (QUESTION.is_match(&raw_body) && PATH_CHARS.is_match(head))
                        .then(|| normalize(head))
                        .filter(|place| !place.is_empty());
                    nest.clear();
                }
                continue;
            }
            let Some(path) = flat_leaf(&raw_body) else { continue };
            if path.ends_with('/') {
                dir = path.clone();
                nest.clear();
            }
            let exempt = lies_in_any(&path, unsettled.iter().chain(nest.iter().filter_map(|(_, _, place)| place.as_ref())));
            out.push(Leaf { dir: String::new(), leaf: path, is_path: true, exempt, op: mark,
                            path: String::new(), target: String::new() });
            continue;
        };
        let column = raw_body.chars().position(|c| c == '├' || c == '└').unwrap_or(0);
        while nest.last().is_some_and(|(c, _, _)| *c >= column) {
            nest.pop();
        }
        let under = format!("{dir}{}", nest.iter().map(|(_, name, _)| name.as_str()).collect::<String>());
        let at = format!("{under}/{leaf}");
        let exempt = lies_in_any(&at, unsettled.iter().chain(nest.iter().filter_map(|(_, _, place)| place.as_ref())));
        // Оговорка про нерешённый вопрос на строке каталога рисунка без пометки значит
        // то же, что на заголовке: место под этим каталогом ещё не решено. Пометка
        // говорит обратное — место решено.
        if leaf.ends_with('/') {
            let place = (mark == ' ' && QUESTION.is_match(&raw_body))
                .then(|| normalize(&at))
                .filter(|place| !place.is_empty());
            nest.push((column, leaf.clone(), place));
        }
        // Запись без пометки — каталог, который не меняется: он только ведёт к
        // тому, что меняется глубже.
        if mark == ' ' {
            continue;
        }
        let is_path = looks_like_path(&leaf);
        out.push(Leaf { dir: under, leaf, is_path, exempt, op: mark,
                        path: String::new(), target: String::new() });
    }
    let created: Vec<String> = out
        .iter()
        .filter(|l| l.op == '+' && l.leaf.ends_with('/'))
        .map(|l| (normalize(&format!("{}/{}", l.dir, l.leaf)), normalize(&l.dir)))
        .filter(|(made, at)| !within(at, made))
        .map(|(made, _)| made)
        .collect();
    for l in &mut out {
        l.path = full_path(&l.dir, &l.leaf);
        let mut target = parent(&l.path);
        while !target.is_empty() && created.iter().any(|made| within(&target, made)) {
            target = parent(&target);
        }
        l.target = target;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{leaf_of, looks_like_path};

    /// Семнадцать записей: у `docs-lint` этот класс держался образцами, у
    /// `tasks-check` не держался ничем — правку класса можно было внести, и
    /// гейт остался бы зелёным.
    #[test]
    fn leaf_fixtures_hold() {
        let cases: [(&str, bool); 17] = [
            (" ├── mod.rs                    объёмы одним местом", false),
            (" └── seed/", false),
            ("     ├── bootstrap.js           вход по ключу", false),
            (" ├── constants                 объёмы одним местом", true),
            (" ├── объёмы                     объёмы одним местом", true),
            (" ├── `объёмы`                   объёмы одним местом", true),
            (" ├── «объёмы»                   объёмы одним местом", true),
            (" ├── объёмы посева              объёмы одним местом", true),
            (" ├── — объёмы                   объёмы одним местом", true),
            (" ├── объёмы одним местом в mod.rs", true),
            (" ├── agent.rs (переименован)", false),
            (" ├── объёмы/account.rs", true),
            (" ├── объёмы,mod.rs", true),
            (" ├─ объёмы                      комментарий", true),
            (" ├── mobile/Sources/Offline/ActionQueue.swift", false),
            ("     29 сценариев изменения конфигурации  проверка права", false),
            (" 29 сценариев изменения конфигурации  проверка права", false),
        ];
        let mut bad = Vec::new();
        for (body, catches) in cases {
            let got = match leaf_of(body) {
                // Строка не запись — поймать её нечем, и это не находка.
                None => false,
                Some(leaf) => !looks_like_path(&leaf),
            };
            if got != catches {
                bad.push(format!("{body:?}\n  ждали ловит={catches}, вышло {got}"));
            }
        }
        assert!(bad.is_empty(), "разошлось {}:\n{}", bad.len(), bad.join("\n"));
    }
}

/// Родитель приведённого пути: `a/b/c.rs` и `a/b/c/` лежат в `a/b`. Разбор пути —
/// дело проекции: запрос гейта соединяет каталоги РАВЕНСТВОМ, а не считает
/// подстроки на лету.
fn parent(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.rfind('/').map(|i| path[..i].to_owned()).unwrap_or_default()
}

fn lies_in_any<'a>(path: &str, places: impl IntoIterator<Item = &'a String>) -> bool {
    let path = normalize(path);
    places.into_iter().any(|place| within(&path, place))
}

fn within(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// Полный путь листа, приведённый: каталог записи и сам лист. Каталог
/// оставляет косую черту в конце — по ней видно, что лист назван каталогом.
pub(crate) fn full_path(dir: &str, leaf: &str) -> String {
    let tail = if leaf.ends_with('/') || (leaf.is_empty() && dir.ends_with('/')) { "/" } else { "" };
    format!("{}{tail}", normalize(&format!("{dir}/{leaf}")))
}

/// Приводит путь к нормальному виду: `a/b/../c` — это `a/c`. Без этого каталог
/// сравнивался бы буквой и не находился, хотя он есть: `..` в записи дерева —
/// обычный способ назвать соседа.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    out.join("/")
}

#[cfg(test)]
mod norm_tests {
    #[test]
    fn dots_collapse() {
        assert_eq!(super::normalize("a/b/../c"), "a/c");
        assert_eq!(super::normalize("backend/crates/myack-http/src/.."), "backend/crates/myack-http");
        assert_eq!(super::normalize("a/./b"), "a/b");
    }
}

#[cfg(test)]
mod flat_tests {
    use super::leaves_of;

    #[test]
    fn flat_and_drawn_leaves_carry_op_and_path() {
        let doc = "## Что меняется в дереве\n\n```diff\n+ crates/tot-page/src/lib.rs\n! backend/Cargo.lock\n! .github/workflows/ci.yml\n- crates/tot-ui/src/spike/file_view.rs\n!    транзакцией, что правка правил\n  backend/crates/app/src/\n! └── evaluator.rs   оператор\n+     └── seed/\n```\n";
        let leaves = leaves_of(doc, "Что меняется в дереве");
        assert!(leaves.iter().all(|l| l.is_path), "плоская строка со `/` — путь и без расширения из перечня");
        let got: Vec<(char, String)> = leaves.iter().map(|l| (l.op, l.path.clone())).collect();
        assert_eq!(got, vec![
            ('+', "crates/tot-page/src/lib.rs".to_owned()),
            ('!', "backend/Cargo.lock".to_owned()),
            ('!', ".github/workflows/ci.yml".to_owned()),
            ('-', "crates/tot-ui/src/spike/file_view.rs".to_owned()),
            ('!', "backend/crates/app/src/evaluator.rs".to_owned()),
            ('+', "backend/crates/app/src/seed/".to_owned()),
        ]);
    }
}

#[cfg(test)]
mod nest_tests {
    use super::leaves_of;

    #[test]
    fn a_leaf_deeper_than_a_directory_lies_in_it() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/crates/myack-postgres/\n  ├── src/\n! │   └── repo/\n! │       ├── mod.rs\n+ │       └── identity.rs\n! ├── reference_account.rs      слом правила\n! └── tests/\n!     └── seed/\n!         ├── mod.rs\n!         └── account.rs      сценарий приёма\n+ migrations/0001.sql\n  backend/\n+ └── build.rs\n```\n";
        let got: Vec<(char, String)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.op, l.path.clone())).collect();
        assert_eq!(got, vec![
            ('!', "backend/crates/myack-postgres/src/repo/".to_owned()),
            ('!', "backend/crates/myack-postgres/src/repo/mod.rs".to_owned()),
            ('+', "backend/crates/myack-postgres/src/repo/identity.rs".to_owned()),
            ('!', "backend/crates/myack-postgres/reference_account.rs".to_owned()),
            ('!', "backend/crates/myack-postgres/tests/".to_owned()),
            ('!', "backend/crates/myack-postgres/tests/seed/".to_owned()),
            ('!', "backend/crates/myack-postgres/tests/seed/mod.rs".to_owned()),
            ('!', "backend/crates/myack-postgres/tests/seed/account.rs".to_owned()),
            ('+', "migrations/0001.sql".to_owned()),
            ('+', "backend/build.rs".to_owned()),
        ]);
    }

    #[test]
    fn a_leaf_must_find_the_place_its_task_does_not_create() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/crates/app/\n+ ├── tests/\n+ │   └── seed/\n+ │       └── account.rs\n! └── src/\n!     └── lib.rs\n+ backend/crates/new/\n```\n";
        let got: Vec<(String, String)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.path.clone(), l.target.clone())).collect();
        assert_eq!(got, vec![
            ("backend/crates/app/tests/".to_owned(), "backend/crates/app".to_owned()),
            ("backend/crates/app/tests/seed/".to_owned(), "backend/crates/app".to_owned()),
            ("backend/crates/app/tests/seed/account.rs".to_owned(), "backend/crates/app".to_owned()),
            ("backend/crates/app/src/".to_owned(), "backend/crates/app".to_owned()),
            ("backend/crates/app/src/lib.rs".to_owned(), "backend/crates/app/src".to_owned()),
            ("backend/crates/new/".to_owned(), "backend/crates".to_owned()),
        ]);
    }

    #[test]
    fn a_header_starts_a_new_tree() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/\n  └── deep/\n+     └── a.rs\n  web/\n+     └── b.ts\n```\n";
        let got: Vec<String> = leaves_of(doc, "Что меняется в дереве").iter().map(|l| l.path.clone()).collect();
        assert_eq!(got, vec!["backend/deep/a.rs".to_owned(), "web/b.ts".to_owned()]);
    }

    #[test]
    fn a_leaf_at_the_root_ends_the_walk() {
        for doc in [
            "## Что меняется в дереве\n\n```diff\n  backend/\n+ └── ../\n```\n",
            "## Что меняется в дереве\n\n```diff\n+ ./\n```\n",
            "## Что меняется в дереве\n\n```diff\n  backend/\n+ ├── ../\n! └── x.rs\n```\n",
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let targets: Vec<String> = leaves_of(doc, "Что меняется в дереве").into_iter().map(|l| l.target).collect();
                let _ = tx.send(targets);
            });
            let targets = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or_else(|_| panic!("разбор не кончился: {doc}"));
            assert!(targets.iter().all(|t| t.is_empty() || t == "backend"), "{doc}: {targets:?}");
        }
    }

    #[test]
    fn an_unsettled_directory_exempts_what_lies_under_it() {
        let doc = "## Что меняется в дереве\n\n```diff\n  mobile/   место решает Q-12\n+       └── deep.swift\n  backend/\n  ├── later/   Q-30\n+ │   └── a.rs\n+ backend/later/flat.rs\n+ ├── note.rs   Q-31\n+ └── b.rs\n```\n";
        let got: Vec<(String, bool)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.path.clone(), l.exempt)).collect();
        assert_eq!(got, vec![
            ("mobile/deep.swift".to_owned(), true),
            ("backend/later/a.rs".to_owned(), true),
            ("backend/later/flat.rs".to_owned(), true),
            ("backend/note.rs".to_owned(), false),
            ("backend/b.rs".to_owned(), false),
        ]);
    }

    #[test]
    fn a_marked_directory_is_a_settled_place_whatever_it_mentions() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/src/\n! ├── handler/     место решено в Q-40\n! │   └── typo.rs\n+ │   └── deep/x.rs\n  ├── check/       TC-UNIQ-01\n+ │   └── a.rs\n  mobile/   Q-12\n+ backend/flat.rs\n+ mobile/src/x.ts\n+ backend/new/\n+ mobile/src/\n+\u{a0}mobile/app.json\n  web/ (Q-13)\n+ └── deep.ts\n  ./  Q-14\n+ backend/typo.rs\n  mobile/  Q-12\n+ └── ../backend/crates/typo.rs\n  backend/crates/app  Q-15\n  └── src/\n+     └── x.rs\n  frontend/src/features/{monitor,inventory}/   Q-7\n+ frontend/y.ts\n```\n";
        let got: Vec<(String, bool)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.path.clone(), l.exempt)).collect();
        assert_eq!(got, vec![
            ("backend/src/handler/".to_owned(), false),
            ("backend/src/handler/typo.rs".to_owned(), false),
            ("backend/src/handler/deep/x.rs".to_owned(), false),
            ("backend/src/check/a.rs".to_owned(), false),
            ("backend/flat.rs".to_owned(), false),
            ("mobile/src/x.ts".to_owned(), true),
            ("backend/new/".to_owned(), false),
            ("mobile/src/".to_owned(), true),
            ("mobile/app.json".to_owned(), true),
            ("web/ (Q-13)/deep.ts".to_owned(), false),
            ("backend/typo.rs".to_owned(), false),
            ("backend/crates/typo.rs".to_owned(), false),
            ("backend/crates/app/src/x.rs".to_owned(), true),
            ("frontend/y.ts".to_owned(), false),
        ]);
    }

    #[test]
    fn a_flat_line_is_a_whole_path() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/crates/a/\n+ backend/crates/new/\n+ ├── Cargo.toml\n+ └── src/\n+     └── lib.rs\n! docker-compose.yml     вторым стеком\n!    транзакцией, что правка правил\n!                  lib.rs и mod.rs держат одно\n  см. docs/adr и k6/\n  backend/crates/app\n! └── x.rs\n  → backend/crates/b/src/\n! └── lib.rs\n```\n";
        let got: Vec<(String, String)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.path.clone(), l.target.clone())).collect();
        assert_eq!(got, vec![
            ("backend/crates/new/".to_owned(), "backend/crates".to_owned()),
            ("backend/crates/new/Cargo.toml".to_owned(), "backend/crates".to_owned()),
            ("backend/crates/new/src/".to_owned(), "backend/crates".to_owned()),
            ("backend/crates/new/src/lib.rs".to_owned(), "backend/crates".to_owned()),
            ("docker-compose.yml".to_owned(), "".to_owned()),
            ("backend/crates/app/x.rs".to_owned(), "backend/crates/app".to_owned()),
            ("→ backend/crates/b/src/lib.rs".to_owned(), "→ backend/crates/b/src".to_owned()),
        ]);
    }

    #[test]
    fn only_a_new_name_is_created() {
        let doc = "## Что меняется в дереве\n\n```diff\n  backend/\n+ ├── ./\n+ ├── nonexistent.rs\n+ └── newcrate/\n      └── src/\n+         └── lib.rs\n```\n";
        let got: Vec<(String, String)> =
            leaves_of(doc, "Что меняется в дереве").iter().map(|l| (l.path.clone(), l.target.clone())).collect();
        assert_eq!(got, vec![
            ("backend/".to_owned(), "".to_owned()),
            ("backend/nonexistent.rs".to_owned(), "backend".to_owned()),
            ("backend/newcrate/".to_owned(), "backend".to_owned()),
            ("backend/newcrate/src/lib.rs".to_owned(), "backend".to_owned()),
        ]);
    }
}

#[cfg(test)]
mod section_tests {
    #[test]
    fn fenceless_block_stays_in_its_section() {
        let doc = "## Что меняется в дереве\n\n+ crates/a/src/lib.rs\n\n## Где обычно ошибаются\n\n- `crates/b/src/x.rs` правят без теста\n\n```rust\nfn main() {}\n```\n";
        let got: Vec<String> = super::leaves_of(doc, "Что меняется в дереве").iter().map(|l| l.leaf.clone()).collect();
        assert_eq!(got, vec!["crates/a/src/lib.rs".to_owned()]);
    }
}
