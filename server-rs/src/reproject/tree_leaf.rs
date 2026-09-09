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
static QUESTION: Lazy<Regex> = Lazy::new(|| Regex::new(r"Q-\d+").expect("образец вопроса"));

/// Лист записи дерева. `None` — строка записью не является: продолжение
/// комментария, заголовок, проза.
pub fn leaf_of(raw_body: &str) -> Option<String> {
    if !CONNECTOR.is_match(raw_body) {
        return None;
    }
    let entry = CONNECTOR.replace(raw_body, "");
    let entry = DECOR_HEAD.replace(&entry, "");
    let head = entry.split_whitespace().next().unwrap_or("");
    Some(DECOR_TAIL.replace(head, "").into_owned())
}

/// Похоже ли на путь. Пустой лист путём считается: это уже не наша находка.
pub fn looks_like_path(leaf: &str) -> bool {
    leaf.is_empty() || (PATH_CHARS.is_match(leaf) && (leaf.ends_with('/') || TREE_EXT.is_match(leaf)))
}

pub struct Leaf {
    pub dir: String,
    pub leaf: String,
    pub is_path: bool,
    pub exempt: bool,
}

/// Разбирает блок задачи в перечень листьев.
pub fn leaves_of(content: &str, section: &str) -> Vec<Leaf> {
    // Имя раздела приходит из словаря схемы: зашитое, оно сделало бы разбор
    // знающим один набор, а другой отдал бы пустоту вместо отказа.
    if section.is_empty() {
        return Vec::new();
    }
    let head = format!("## {section}");
    let Some(after) = content.split(head.as_str()).nth(1) else { return Vec::new() };
    let block = after.split("```diff").nth(1).unwrap_or(after);
    let block = block.split("```").next().unwrap_or("");
    let mut out = Vec::new();
    let mut dir = String::new();
    let mut exempt = false;
    for raw in block.split('\n') {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let mark = line.chars().next().unwrap_or(' ');
        let raw_body: String = line.chars().skip(1).collect();
        // Строка без пометки — заголовок каталога: он задаёт место следующим
        // записям и может нести оговорку про нерешённый вопрос.
        if mark == ' ' {
            let body = CONNECTOR.replace(&raw_body, "");
            let body = body.trim_start();
            let head = body.split("  ").next().unwrap_or("").trim();
            if head.contains('/') {
                dir = head.to_owned();
                exempt = QUESTION.is_match(body);
            }
            continue;
        }
        if mark != '+' && mark != '!' {
            continue;
        }
        let Some(leaf) = leaf_of(&raw_body) else { continue };
        let is_path = looks_like_path(&leaf);
        out.push(Leaf { dir: dir.clone(), leaf, is_path, exempt });
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

/// Каталог, в который ляжет лист. Разбор пути — дело проекции: запрос гейта
/// соединяет каталоги РАВЕНСТВОМ, а не считает подстроки на лету.
pub fn target_dir(dir: &str, leaf: &str) -> String {
    let full = if leaf.contains('/') && dir.is_empty() {
        leaf.to_owned()
    } else {
        format!("{}{}", dir, leaf)
    };
    let dir = match full.rfind('/') {
        Some(i) => full[..i].to_owned(),
        None => return String::new(),
    };
    normalize(&dir)
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
