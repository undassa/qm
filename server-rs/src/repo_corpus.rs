//! Три корпуса репозитория: домен, схема, контракт.
//!
//! Одно и то же множество описано в двух местах, и они расходятся молча.
//! Цена измерена, а не предположена: множество `escalation_executions.state`
//! знало `cancelled`, домен давал `ExecState::Stopped`, и первая же остановка
//! исполнения упала бы на `CHECK`.
//!
//! **Сопоставление и расхождение — разные находки.** Не всякое перечисление
//! домена отображается в множество схемы: часть живёт в `jsonb`, часть
//! вычисляется, часть — лазейка разбора чужого ввода под `#[serde(other)]`.
//! Правило, молча пропускающее несопоставленное, вырождается в зелёное: пар
//! не нашлось — сверять нечего — всё хорошо. Поэтому несопоставленное здесь
//! не пропускается, а НАЗЫВАЕТСЯ.

/// Перечисление домена: имя и хранимые значения в змеином регистре.
pub(crate) struct Enum {
    pub name: String,
    pub file: String,
    /// Значения, которые доходят до базы. Ветка под `#[serde(other)]` сюда не
    /// входит: она ловит чужой ввод, а не хранится.
    pub stored: Vec<String>,
    pub unstored: Vec<String>,
    pub fallback: Option<String>,
    /// Годно к хранению: либо все ветки без полей, либо помечено `tag =`.
    pub storable: bool,
    pub mark: Option<(String, String)>,
}

/// Множество `CHECK (col IN (…))` схемы.
pub(crate) struct CheckSet {
    pub at: String,
    pub values: Vec<String>,
    pub file: String,
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = s.chars().collect();
    for (i, c) in b.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_lower = i > 0 && (b[i - 1].is_ascii_lowercase() || b[i - 1].is_ascii_digit());
            if prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(*c);
        }
    }
    out
}

pub(crate) fn mark<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let head = format!("x-{key}:");
    text.lines()
        .filter_map(|l| l.trim_start().trim_start_matches('/').trim_start().strip_prefix(head.as_str()))
        .map(str::trim)
        .find(|rest| !rest.is_empty())
}

fn up_only(text: &str) -> String {
    let mut up = true;
    text.lines()
        .filter(|line| {
            if let Some(said) = line.trim_start().strip_prefix("--") {
                let opts = |rest: &[&str]| rest.iter().all(|w| w.contains(':') || *w == "notransaction");
                match said.to_ascii_lowercase().split_whitespace().collect::<Vec<&str>>().as_slice() {
                    ["+goose" | "+migrate", "down", rest @ ..] | ["migrate:down", rest @ ..] if opts(rest) => up = false,
                    ["+goose" | "+migrate", "up", rest @ ..] | ["migrate:up", rest @ ..] if opts(rest) => up = true,
                    _ => {}
                }
            }
            up
        })
        .collect::<Vec<&str>>()
        .join("\n")
}

pub(crate) fn migration_up(name: &str) -> bool {
    name.ends_with(".sql") && !name.ends_with(".down.sql") && name.rsplit('/').next() != Some("down.sql")
}

fn uncommented(text: &str) -> String {
    let text = up_only(text);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let (mut quoted, mut block) = (false, 0usize);
    while let Some(c) = chars.next() {
        let next = chars.peek().copied();
        if block > 0 {
            match (c, next) {
                ('*', Some('/')) => {
                    chars.next();
                    block -= 1;
                }
                ('/', Some('*')) => {
                    chars.next();
                    block += 1;
                }
                ('\n', _) => out.push('\n'),
                _ => {}
            }
            continue;
        }
        match (c, next) {
            ('\'', _) => {
                quoted = !quoted;
                out.push(c);
            }
            ('-', Some('-')) if !quoted => while chars.next_if(|n| *n != '\n').is_some() {},
            ('/', Some('*')) if !quoted => {
                chars.next();
                block = 1;
            }
            _ => out.push(c),
        }
    }
    out
}

/// Перечисления домена из текста одного файла.
pub(crate) fn enums_of(text: &str, file: &str) -> Vec<Enum> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let t = lines[i].trim();
        // Видимость снимается, а не перечисляется: `pub(crate) enum` — тот же
        // перечень, и пропущенный, он молча выпадает из сверки с множеством.
        let head = crate::client::without_visibility(t);
        if !(head.starts_with("enum ") && t.ends_with('{')) {
            i += 1;
            continue;
        }
        let name = head["enum ".len()..].trim_end_matches('{').trim().to_owned();
        // Атрибуты стоят НАД объявлением: `#[derive(...)]`, `#[serde(tag = ...)]`.
        let mut attrs = String::new();
        let mut a = i;
        while a > 0 {
            let prev = lines[a - 1].trim();
            if prev.starts_with("#[") || prev.starts_with("//") {
                attrs.push_str(prev);
                attrs.push('\n');
                a -= 1;
            } else {
                break;
            }
        }
        let tagged = attrs.contains("tag =") || attrs.contains("tag=");
        let marked = mark(&attrs, "derived")
            .map(|m| ("x-derived".to_owned(), m.to_owned()))
            .or_else(|| mark(&attrs, "stored-in").map(|m| ("x-stored-in".to_owned(), m.to_owned())));
        let mut stored = Vec::new();
        let mut unstored = Vec::new();
        let mut fallback = None;
        let mut all_unit = true;
        let mut pending = String::new();
        let mut docs = String::new();
        let mut depth = 0i32;
        let mut j = i + 1;
        while j < lines.len() {
            let raw = lines[j];
            let t = raw.trim();
            if depth == 0 && t == "}" {
                break;
            }
            if depth == 0 {
                if t.starts_with("#[") {
                    pending.push_str(t);
                    pending.push(' ');
                } else if t.starts_with("///") {
                    docs.push_str(t);
                    docs.push('\n');
                } else if !t.starts_with("//") && !t.is_empty() {
                    let head: String =
                        t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if head.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                        if pending.contains("other)") || pending.contains("other )") {
                            fallback = Some(snake(&head));
                        } else if mark(&docs, "derived").is_some() {
                            unstored.push(snake(&head));
                        } else {
                            stored.push(snake(&head));
                        }
                        let rest = t[head.len()..].trim_start();
                        if rest.starts_with('(') || rest.starts_with('{') {
                            all_unit = false;
                        }
                        pending.clear();
                        docs.clear();
                    }
                }
            }
            for c in raw.chars() {
                match c {
                    '{' | '(' => depth += 1,
                    '}' | ')' => depth -= 1,
                    _ => {}
                }
            }
            j += 1;
        }
        out.push(Enum {
            name,
            file: file.to_owned(),
            stored,
            unstored,
            fallback,
            storable: tagged || all_unit,
            mark: marked,
        });
        i = j.max(i + 1);
    }
    out
}

/// Множества `CHECK` из текста одной миграции. Комментарии снимаются: `--` и
/// `/* */` вне строки в кавычках съели бы половину определения.
pub(crate) fn checks_of(text: &str, file: &str) -> Vec<CheckSet> {
    let no_comments = uncommented(text);
    let table = regex::Regex::new(r"(?i)CREATE TABLE (?:IF NOT EXISTS )?([a-z][a-z0-9_]*)\s*\(")
        .expect("образец таблицы");
    let check = regex::Regex::new(r"(?i)CHECK\s*\(\s*([a-z_][a-z0-9_]*)\s+IN\s*\(([^)]*)\)")
        .expect("образец множества");
    let value = regex::Regex::new(r"'([^']*)'|(\d+)").expect("образец значения");
    let b: Vec<char> = no_comments.chars().collect();
    let mut out = Vec::new();
    for m in table.captures_iter(&no_comments) {
        let whole = m.get(0).expect("совпадение целиком");
        let name = m[1].to_owned();
        // Тело таблицы — до закрывающей скобки на своём уровне.
        let start = no_comments[..whole.end()].chars().count();
        let mut i = start;
        let mut depth = 1i32;
        while i < b.len() && depth > 0 {
            match b[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let body: String = b[start..i.saturating_sub(1)].iter().collect();
        for c in check.captures_iter(&body) {
            let values: Vec<String> = value
                .captures_iter(&c[2])
                .filter_map(|v| v.get(1).or_else(|| v.get(2)).map(|x| x.as_str().to_owned()))
                .collect();
            if values.is_empty() {
                continue;
            }
            out.push(CheckSet {
                at: format!("{name}.{}", &c[1]),
                values,
                file: file.to_owned(),
            });
        }
    }
    out
}

/// Пара «перечисление ↔ множество». Совпадение считается по пересечению:
/// два общих значения — уже повод, а большинство с любой стороны — уже пара.
/// Порог не «похоже на глаз»: он назван числом и проверяем.
pub(crate) struct Pair {
    pub name: String,
    pub detail: String,
}

/// Закрыто ли множество `CHECK` СЛОВАРЁМ КОНТРАКТА.
///
/// Тот же довод, которым пользуется `check_values_without_path`: множество,
/// пересекающееся со словарём по значениям, и есть этот словарь. Вынесено в одно
/// место, чтобы два правила одного датчика не расходились в том, что считают
/// источником, — а они расходились, и цена была восемь исключений.
pub(crate) fn covered_by_contract(s: &CheckSet, contract: &[(String, Vec<String>)]) -> Option<String> {
    for (name, vals) in contract {
        let common = vals.iter().filter(|v| s.values.contains(v)).count();
        if common >= 2 || (common == 1 && s.values.len() == 1) {
            return Some(name.clone());
        }
    }
    None
}

pub(crate) fn pair(
    enums: &[Enum],
    checks: &[CheckSet],
    contract: &[(String, Vec<String>)],
    forced: &[(&str, &str)],
    tables: &[Table],
) -> Vec<Pair> {
    let inter =|a: &[String], b: &[String]| -> Vec<String> {
        a.iter().filter(|x| b.contains(x)).cloned().collect()
    };
    struct Cand<'a> {
        e: &'a Enum,
        s: &'a CheckSet,
        n: usize,
    }
    let mut cand: Vec<Cand> = Vec::new();
    for e in enums {
        for s in checks {
            if forced.iter().any(|(en, at)| *en == e.name && *at == s.at) {
                cand.push(Cand { e, s, n: usize::MAX });
                continue;
            }
            let i = inter(&e.stored, &s.values);
            if i.len() < 2 {
                continue;
            }
            let half = e.stored.len().max(s.values.len()).div_ceil(2);
            if i.len() == e.stored.len() || i.len() == s.values.len() || i.len() >= half {
                cand.push(Cand { e, s, n: i.len() });
            }
        }
    }
    let best_for_enum = |name: &str, c: &[Cand]| {
        c.iter().filter(|x| x.e.name == name).map(|x| x.n).max().unwrap_or(0)
    };
    let best_for_set = |at: &str, c: &[Cand]| {
        c.iter().filter(|x| x.s.at == at).map(|x| x.n).max().unwrap_or(0)
    };
    let outside = |e: &Enum| {
        if e.unstored.is_empty() {
            String::new()
        } else {
            format!("; вне хранения (x-derived): {}", e.unstored.join(", "))
        }
    };
    let mut out = Vec::new();
    let mut paired_enum: Vec<&str> = Vec::new();
    let mut paired_set: Vec<&str> = Vec::new();
    for c in &cand {
        if c.n != best_for_enum(&c.e.name, &cand) || c.n != best_for_set(&c.s.at, &cand) {
            continue;
        }
        paired_enum.push(&c.e.name);
        paired_set.push(&c.s.at);
        let only_domain: Vec<String> =
            c.e.stored.iter().filter(|v| !c.s.values.contains(v)).cloned().collect();
        let only_schema: Vec<String> =
            c.s.values.iter().filter(|v| !c.e.stored.contains(v)).cloned().collect();
        let mut detail = if only_domain.is_empty() && only_schema.is_empty() {
            format!("сходятся, значений {}", c.e.stored.len())
        } else {
            let mut why = String::from("разошлись:");
            if !only_domain.is_empty() {
                why.push_str(&format!(" только в домене {}", only_domain.join(", ")));
            }
            if !only_schema.is_empty() {
                why.push_str(&format!(" только в схеме {}", only_schema.join(", ")));
            }
            why
        };
        detail.push_str(&outside(c.e));
        out.push(Pair { name: format!("{} ↔ {}", c.e.name, c.s.at), detail });
    }
    // Лазейка разбора в множестве CHECK. Ветка под `#[serde(other)]` ловит
    // ЧУЖОЙ ввод — значение, которого мы не знаем. Попав в множество, она
    // становится хранимым значением: неизвестное записывается как известное, и
    // отличить их потом нечем.
    for e in enums {
        let Some(fallback) = &e.fallback else { continue };
        for s in checks {
            if s.values.iter().any(|v| v == fallback) {
                out.push(Pair {
                    name: format!("{} · {}", s.at, fallback),
                    detail: format!("лазейка разбора в множестве CHECK: ветка {}::{} стоит под \
                                     #[serde(other)] и ловит чужой ввод, а множество {} её хранит",
                                    e.name, fallback, s.at),
                });
            }
        }
    }

    // Несопоставленное НАЗЫВАЕТСЯ. Молчание тут читается как «сверили и сошлось».
    for e in enums {
        if !e.storable || paired_enum.contains(&e.name.as_str()) || e.stored.len() + e.unstored.len() < 2 {
            continue;
        }
        let finding = format!("перечисление без множества CHECK ({}), значений {}", e.file, e.stored.len());
        let detail = match &e.mark {
            Some((key, text)) if key == "x-derived" => format!("помечено x-derived: {text}"),
            Some((_, target)) => {
                let json = target
                    .split_once('.')
                    .and_then(|(tn, cn)| tables.iter().find(|t| t.name == tn)?.cols.iter().find(|c| c.name == cn))
                    .is_some_and(|c| c.ty == "jsonb" || c.ty == "json");
                if json {
                    format!("помечено x-stored-in: {target}")
                } else {
                    format!("{finding}; x-stored-in называет {target} — jsonb-колонки нет")
                }
            }
            None => finding,
        } + &outside(e);
        out.push(Pair { name: e.name.clone(), detail });
    }
    for s in checks {
        if paired_set.contains(&s.at.as_str()) {
            continue;
        }
        // ИСТОЧНИКОМ БЫВАЕТ И КОНТРАКТ. Правило принимало один источник —
        // перечисление домена, — а сосед по тому же датчику принимал два. На
        // восьми находках из девяти словарь стоял в контракте ДОСЛОВНО тем же
        // составом, и врезки миграций называли контракт источником своими
        // словами. Завести перечисление в домене ради правила значило бы
        // поставить второе место для одного слова и мёртвый тип.
        if let Some(schema) = covered_by_contract(s, contract) {
            out.push(Pair {
                name: format!("{} ↔ {}", schema, s.at),
                detail: format!("сходятся со словарём контракта, значений {}", s.values.len()),
            });
            continue;
        }
        out.push(Pair {
            name: s.at.clone(),
            detail: format!("множество CHECK без перечисления домена и без словаря контракта ({})",
                            s.file),
        });
    }
    out
}

#[cfg(test)]
mod source {
    use super::{covered_by_contract, CheckSet};

    fn set_of(values: &[&str]) -> CheckSet {
        CheckSet {
            at: "signal_sources.kind".into(),
            values: values.iter().map(|v| (*v).to_owned()).collect(),
            file: "0003.sql".into(),
        }
    }

    #[test]
    fn glossary_contract_closes_set_of() {
        let c = vec![("Source.kind".to_owned(),
                      vec!["push".to_owned(), "pull".to_owned(), "manual".to_owned()])];
        assert_eq!(covered_by_contract(&set_of(&["push", "pull", "manual"]), &c).as_deref(),
                   Some("Source.kind"));
    }

    #[test]
    fn foreign_glossary_not_closes() {
        let c = vec![("Role.kind".to_owned(), vec!["owner".to_owned(), "member".to_owned()])];
        assert!(covered_by_contract(&set_of(&["push", "pull", "manual"]), &c).is_none());
    }

    #[test]
    fn one_shared_value_is_a_coincidence_not_a_glossary() {
        let c = vec![("Source.kind".to_owned(), vec!["push".to_owned(), "x".to_owned()])];
        // Одно общее значение из трёх — совпадение, а не словарь.
        assert!(covered_by_contract(&set_of(&["push", "pull", "manual"]), &c).is_none());
        assert!(covered_by_contract(&set_of(&["push"]), &c).is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RS: &str = r#"
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecState {
    Pending,
    Running,
    Cancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug)]
pub enum Payload {
    Text(String),
    Blob { bytes: Vec<u8> },
}
"#;

    const SQL: &str = r#"
CREATE TABLE IF NOT EXISTS escalation_executions (
  id bigserial PRIMARY KEY,
  -- прежде здесь стояло CHECK (note IN ('нет')) — оговорка ВНУТРИ тела:
  -- снаружи её не читают и без снятия комментариев, и проверка проходила
  -- бы по случайности, а не по правилу
  state text NOT NULL CHECK (state IN ('pending', 'running', 'cancelled', 'stopped')),
  note text
);
"#;

    #[test]
    fn enum_variants_are_snake_and_fallback_is_apart() {
        let e = enums_of(RS, "x.rs");
        assert_eq!(e.len(), 2, "перечислений");
        assert_eq!(e[0].stored, vec!["pending", "running", "cancelled"]);
        assert_eq!(e[0].fallback.as_deref(), Some("unknown"));
        assert!(e[0].storable, "все ветки без полей — годно к хранению");
        assert!(!e[1].storable, "ветки с полями хранению не годятся");
    }

    #[test]
    fn checks_ignore_comments() {
        let c = checks_of(SQL, "0001.sql");
        assert_eq!(c.len(), 1, "множеств: комментарий не считается");
        assert_eq!(c[0].at, "escalation_executions.state");
        assert_eq!(c[0].values, vec!["pending", "running", "cancelled", "stopped"]);
    }

    #[test]
    fn divergence_is_named_in_both_directions() {
        let e = enums_of(RS, "x.rs");
        let c = checks_of(SQL, "0001.sql");
        let p = pair(&e, &c, &[], &[], &[]);
        let drift = p.iter().find(|x| x.name.contains('↔')).expect("пара нашлась");
        assert!(drift.detail.contains("только в схеме stopped"), "вышло: {}", drift.detail);
    }
}

// ── Контракт против схемы ────────────────────────────────────────────────
//
// Контракт и схема описывают одно и то же множество полей, и расходятся они
// молча: поле, которого нет колонкой, читается в контракте как обещание;
// колонка, у которой нет входа, читается в схеме как хранимое — а положить в
// неё нечем.

use serde_json::Value;

/// Колонка таблицы: имя и то, что о ней сказано определением.
pub(crate) struct Column {
    pub name: String,
    pub ty: String,
    pub not_null: bool,
    pub has_default: bool,
    /// Первичный ключ пуст не бывает и без `NOT NULL`: обещание контракта он
    /// держит устройством, а не словом.
    pub primary_key: bool,
    pub references: Option<String>,
    pub source: Option<String>,
}

pub(crate) struct Table {
    pub name: String,
    pub cols: Vec<Column>,
    pub source: Option<String>,
}

/// Поле схемы контракта: где стоит, как зовётся, чем помечено.
pub(crate) struct Field {
    /// Точечный адрес: `PolicyTestRun.window.from`.
    pub at: String,
    pub schema: String,
    pub name: String,
    pub marks: Vec<String>,
    pub required: bool,
    /// Обнуляемое по контракту: `type: [string, "null"]` или `nullable: true`.
    pub nullable: bool,
    /// Составное поле: вложенный объект, список, `allOf`/`oneOf`/`$ref`.
    /// Колонкой оно не бывает по устройству, и требовать её значит требовать
    /// невозможного — правило захлебнулось бы своими же находками.
    pub composite: bool,
    pub stored_in: Option<String>,
}

impl Field {
    fn column(&self, table: &str) -> &str {
        match self.stored_in.as_deref().and_then(|s| s.split_once('.')) {
            Some((t, c)) if t == table => c,
            _ => &self.name,
        }
    }
}

fn column_def(c: &regex::Captures) -> Column {
    let rest = c[3].to_uppercase();
    Column {
        name: c[1].to_owned(),
        ty: c[2].to_lowercase(),
        not_null: rest.contains("NOT NULL"),
        has_default: rest.contains("DEFAULT"),
        primary_key: rest.contains("PRIMARY KEY"),
        references: regex::Regex::new(r"(?i)\bREFERENCES\s+(?:[a-z][a-z0-9_]*\.)?([a-z][a-z0-9_]*)")
            .expect("образец ссылки")
            .captures(&c[3])
            .map(|r| r[1].to_owned()),
        source: None,
    }
}

/// Колонки из текста миграции. Тот же разбор тела, что у множеств `CHECK`.
fn tables_of(no_comments: &str) -> Vec<(usize, Table)> {
    let table = regex::Regex::new(r"(?i)CREATE TABLE (?:IF NOT EXISTS )?([a-z][a-z0-9_]*)\s*\(")
        .expect("образец таблицы");
    let foreign = regex::Regex::new(
        r"(?i)^(?:CONSTRAINT \S+ )?FOREIGN KEY\s*\(([^)]*)\)\s*REFERENCES\s+(?:[a-z][a-z0-9_]*\.)?([a-z][a-z0-9_]*)",
    )
    .expect("образец внешнего ключа");
    let col = regex::Regex::new(
        r"(?i)^([a-z][a-z0-9_]*) (text|integer|bigint|bigserial|serial|smallint|boolean|jsonb|json|uuid|date|numeric|timestamptz|timestamp|timetz|time|interval|inet|bytea|real|double)\b(.*)$",
    )
    .expect("образец колонки");
    let b: Vec<char> = no_comments.chars().collect();
    let mut out = Vec::new();
    for m in table.captures_iter(no_comments) {
        let whole = m.get(0).expect("совпадение целиком");
        let start = no_comments[..whole.end()].chars().count();
        let mut i = start;
        let mut depth = 1i32;
        while i < b.len() && depth > 0 {
            match b[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let body: String = b[start..i.saturating_sub(1)].iter().collect();
        // Определения режутся запятой ВЕРХНЕГО уровня: внутри `CHECK (...)`
        // запятая своя и колонку бы разорвала.
        let mut defs = Vec::new();
        let mut cur = String::new();
        let mut d = 0i32;
        for ch in body.chars() {
            match ch {
                '(' => d += 1,
                ')' => d -= 1,
                _ => {}
            }
            if ch == ',' && d == 0 {
                defs.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
        }
        defs.push(cur);
        let mut cols: Vec<Column> = Vec::new();
        let mut keys = Vec::new();
        for def in &defs {
            let t: String = def.split_whitespace().collect::<Vec<&str>>().join(" ");
            if let Some(c) = col.captures(&t) {
                cols.push(column_def(&c));
            } else if let Some(k) = foreign.captures(&t) {
                keys.push((k[1].to_owned(), k[2].to_owned()));
            }
        }
        for (list, target) in keys {
            for name in list.split(',') {
                if let Some(c) = cols.iter_mut().find(|c| c.name == name.trim() && c.references.is_none()) {
                    c.references = Some(target.clone());
                }
            }
        }
        out.push((whole.start(), Table { name: m[1].to_owned(), cols, source: None }));
    }
    out
}

enum Ddl {
    Create(Table),
    DropTable(String),
    Add(String, Column),
    DropColumn(String, String),
    Retype(String, String, String),
    Required(String, String, bool),
    Defaulted(String, String, bool),
    Comment(String, Option<String>),
}

fn ddl_of(text: &str) -> Vec<Ddl> {
    let clean = uncommented(text);
    let mut steps: Vec<(usize, Ddl)> =
        tables_of(&clean).into_iter().map(|(at, t)| (at, Ddl::Create(t))).collect();
    steps.extend(alters_of(&clean));
    let drop = regex::Regex::new(r"(?is)DROP TABLE\s+(?:IF EXISTS\s+)?([^;]*)").expect("образец снятой таблицы");
    for m in drop.captures_iter(&clean) {
        let at = m.get(0).expect("совпадение целиком").start();
        for name in m[1].split(',').filter_map(|n| n.split_whitespace().next()) {
            steps.push((at, Ddl::DropTable(name.rsplit('.').next().unwrap_or(name).to_owned())));
        }
    }
    let comment = regex::Regex::new(
        r"(?is)COMMENT\s+ON\s+(?:TABLE|COLUMN)\s+([a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)?)\s+IS\s+(?:NULL|'((?:[^']|'')*)')",
    )
    .expect("образец комментария");
    for c in comment.captures_iter(&clean) {
        let said = c.get(2).map(|s| s.as_str().replace("''", "'")).unwrap_or_default();
        let source = mark(said.lines().next().unwrap_or(""), "source").map(str::to_owned);
        steps.push((c.get(0).expect("совпадение целиком").start(), Ddl::Comment(c[1].to_owned(), source)));
    }
    steps.sort_by_key(|(at, _)| *at);
    steps.into_iter().map(|(_, step)| step).collect()
}

/// Схема — это `CREATE TABLE` ПЛЮС всё, что доехало `ALTER`-ами.
///
/// Датчик читал только `CREATE TABLE`, и колонка, приехавшая
/// `ALTER TABLE … ADD COLUMN`, для него не существовала. Прямое следствие:
/// аддитивная миграция не могла закрыть НИ ОДНУ находку состава — а «миграции
/// аддитивны» у myack записано правилом (`data-model.md` §1bis). Колонку
/// `signals.service_id` пришлось вписывать в исходный `CREATE TABLE` вместо
/// новой миграции, иначе обещание контракта осталось бы без машинной опоры.
///
/// Снятая `DROP COLUMN` колонка уходит: считать её присутствующей значит
/// обещать вход, которого нет.
///
/// Тексты берутся В ПОРЯДКЕ миграций — иначе `ALTER` мог бы прийти прежде
/// таблицы, которую правит.
pub(crate) fn schema_of(texts: &[String]) -> Vec<Table> {
    let mut out: Vec<Table> = Vec::new();
    for step in texts.iter().flat_map(|text| ddl_of(text)) {
        match step {
            Ddl::Create(t) => match out.iter_mut().find(|x| x.name == t.name) {
                Some(had) => {
                    for c in t.cols {
                        if !had.cols.iter().any(|x| x.name == c.name) {
                            had.cols.push(c);
                        }
                    }
                }
                None => out.push(t),
            },
            Ddl::DropTable(name) => out.retain(|x| x.name != name),
            Ddl::Add(table, c) => {
                if let Some(had) = out.iter_mut().find(|x| x.name == table) {
                    match had.cols.iter_mut().find(|x| x.name == c.name) {
                        Some(x) => *x = c,
                        None => had.cols.push(c),
                    }
                }
            }
            Ddl::DropColumn(table, name) => {
                if let Some(had) = out.iter_mut().find(|x| x.name == table) {
                    had.cols.retain(|x| x.name != name);
                }
            }
            Ddl::Retype(table, name, ty) => {
                if let Some(c) = column_mut(&mut out, &table, &name) {
                    c.ty = ty;
                }
            }
            Ddl::Required(table, name, on) => {
                if let Some(c) = column_mut(&mut out, &table, &name) {
                    c.not_null = on;
                }
            }
            Ddl::Defaulted(table, name, on) => {
                if let Some(c) = column_mut(&mut out, &table, &name) {
                    c.has_default = on;
                }
            }
            Ddl::Comment(at, source) => match at.split_once('.') {
                Some((table, name)) => {
                    let col = out
                        .iter_mut()
                        .find(|x| x.name == table)
                        .and_then(|t| t.cols.iter_mut().find(|c| c.name == name));
                    if let Some(c) = col {
                        c.source = source;
                    }
                }
                None => {
                    if let Some(t) = out.iter_mut().find(|x| x.name == at) {
                        t.source = source;
                    }
                }
            },
        }
    }
    out
}

fn column_mut<'a>(tables: &'a mut [Table], table: &str, name: &str) -> Option<&'a mut Column> {
    tables.iter_mut().find(|x| x.name == table).and_then(|t| t.cols.iter_mut().find(|c| c.name == name))
}

/// Правки таблицы с местом в тексте: добавленная и снятая колонка, смена типа,
/// обязательность и умолчание.
///
/// Одна команда `ALTER` несёт несколько действий через запятую, и каждое —
/// своя строка ответа.
fn alters_of(no_comments: &str) -> Vec<(usize, Ddl)> {
    let head = regex::Regex::new(r"(?is)ALTER TABLE\s+(?:IF EXISTS\s+)?(?:ONLY\s+)?([a-z][a-z0-9_]*)\s+([^;]*);")
        .expect("образец правки таблицы");
    let add = regex::Regex::new(
        r"(?i)^ADD (?:COLUMN )?(?:IF NOT EXISTS )?([a-z][a-z0-9_]*) (text|integer|bigint|bigserial|serial|smallint|boolean|jsonb|json|uuid|date|numeric|timestamptz|timestamp|timetz|time|interval|inet|bytea|real|double)\b(.*)$",
    )
    .expect("образец добавленной колонки");
    let drop = regex::Regex::new(r"(?i)^DROP (?:COLUMN )?(?:IF EXISTS )?([a-z][a-z0-9_]*)")
        .expect("образец снятой колонки");
    let retype = regex::Regex::new(r#"(?i)^ALTER (?:COLUMN )?"?([a-z][a-z0-9_]*)"? (?:SET DATA )?TYPE "?([a-z][a-z0-9_]*)"#)
        .expect("образец смены типа");
    let flag = regex::Regex::new(r#"(?i)^ALTER (?:COLUMN )?"?([a-z][a-z0-9_]*)"? (SET|DROP) (NOT NULL|DEFAULT)\b"#)
        .expect("образец обязательности и умолчания");
    let mut out = Vec::new();
    for m in head.captures_iter(no_comments) {
        let at = m.get(0).expect("совпадение целиком").start();
        let table = m[1].to_owned();
        // Действия режутся запятой ВЕРХНЕГО уровня: внутри `CHECK (…)` и
        // `DEFAULT (…)` запятая своя и действие бы разорвала.
        let mut acts = Vec::new();
        let mut cur = String::new();
        let mut d = 0i32;
        for ch in m[2].chars() {
            match ch {
                '(' => d += 1,
                ')' => d -= 1,
                _ => {}
            }
            if ch == ',' && d == 0 {
                acts.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
        }
        acts.push(cur);
        for a in &acts {
            let t: String = a.split_whitespace().collect::<Vec<&str>>().join(" ");
            if let Some(c) = add.captures(&t) {
                out.push((at, Ddl::Add(table.clone(), column_def(&c))));
            } else if let Some(c) = drop.captures(&t) {
                // `DROP CONSTRAINT` — не колонка, и снимать по нему нечего.
                if !t.to_uppercase().starts_with("DROP CONSTRAINT") {
                    out.push((at, Ddl::DropColumn(table.clone(), c[1].to_owned())));
                }
            } else if let Some(c) = flag.captures(&t) {
                let (name, on) = (c[1].to_owned(), c[2].eq_ignore_ascii_case("SET"));
                out.push((at, if c[3].eq_ignore_ascii_case("DEFAULT") {
                    Ddl::Defaulted(table.clone(), name, on)
                } else {
                    Ddl::Required(table.clone(), name, on)
                }));
            } else if let Some(c) = retype.captures(&t) {
                out.push((at, Ddl::Retype(table.clone(), c[1].to_owned(), c[2].to_lowercase())));
            }
        }
    }
    out
}

#[cfg(test)]
mod edits {
    use super::schema_of;

    const CREATE: &str = "CREATE TABLE signals (id text NOT NULL, at timestamptz);";

    #[test]
    fn a_column_added_by_alter_exists() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE signals ADD COLUMN service_id text NOT NULL;".into()]);
        let cols: Vec<&str> = t[0].cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, vec!["id", "at", "service_id"], "{cols:?}");
        assert!(t[0].cols[2].not_null, "NOT NULL потерян");
    }

    #[test]
    fn a_drop_without_the_word_column_is_still_a_drop() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE IF EXISTS signals ADD IF NOT EXISTS note text;".into()]);
        assert!(t[0].cols.iter().any(|c| c.name == "note"), "{:?}",
                t[0].cols.iter().map(|c| &c.name).collect::<Vec<_>>());
    }

    #[test]
    fn dropped_column_leaves() {
        let t = schema_of(&[CREATE.into(), "ALTER TABLE signals DROP COLUMN at;".into()]);
        assert!(!t[0].cols.iter().any(|c| c.name == "at"), "снятая колонка осталась");
    }

    #[test]
    fn several_actions_one_command() {
        let t = schema_of(&[CREATE.into(),
            "ALTER TABLE signals ADD COLUMN a text DEFAULT 'x', ADD COLUMN b integer, DROP COLUMN at;".into()]);
        let cols: Vec<&str> = t[0].cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, vec!["id", "a", "b"], "{cols:?}");
    }

    #[test]
    fn constraint_removed_alter_alter_column_not_touches() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE signals DROP CONSTRAINT signals_at_check;".into()]);
        assert!(t[0].cols.iter().any(|c| c.name == "at"), "ограничение съело колонку");
    }

    #[test]
    fn requiredness_and_default_edited_by_alter() {
        let create = "CREATE TABLE signals (id text NOT NULL, team_id text, tier text NOT NULL DEFAULT 'free');";
        let col = |alter: &str, name: &str| {
            let t = schema_of(&[create.into(), alter.into()]);
            t[0].cols.iter().find(|c| c.name == name).map(|c| (c.not_null, c.has_default)).expect("колонка")
        };
        let tighten = "ALTER TABLE signals ALTER COLUMN team_id SET NOT NULL, ALTER tier DROP DEFAULT;";
        assert_eq!((col(tighten, "team_id"), col(tighten, "tier")), ((true, false), (true, false)));
        let loosen = "ALTER TABLE signals ALTER COLUMN tier DROP NOT NULL, ALTER COLUMN team_id SET DEFAULT 'x';";
        assert_eq!((col(loosen, "team_id"), col(loosen, "tier")), ((false, true), (false, true)));
    }

    #[test]
    fn section_down_not_read() {
        for (up, down) in [("-- +goose Up", "-- +goose Down"), ("-- migrate:up", "-- migrate:down"),
                           ("--+goose up", "-- +goose    Down"), ("-- migrate:up", "-- migrate:down transaction:false"),
                           ("-- +migrate Up", "-- +migrate Down notransaction")] {
            let text = format!("{up}\nCREATE TABLE api_tokens (id text PRIMARY KEY, owner_id text NOT NULL CHECK (owner_id IN ('a')));\n{down}\nDROP TABLE api_tokens;\nCREATE TABLE api_tokens (id text PRIMARY KEY, owner_id text CHECK (owner_id IN ('b')));\n");
            let t = schema_of(std::slice::from_ref(&text));
            assert!(t.iter().any(|x| x.name == "api_tokens" && x.cols.iter().any(|c| c.name == "owner_id" && c.not_null)), "{down}");
            let sets: Vec<Vec<String>> = super::checks_of(&text, "0001.sql").into_iter().map(|c| c.values).collect();
            assert_eq!(sets, vec![vec!["a".to_owned()]], "{down}");
        }
        for prose in ["-- migrate:down не пишем: откат только вперёд", "-- +goose Down не используется"] {
            let t = schema_of(&[format!("{prose}\nCREATE TABLE api_tokens (id text PRIMARY KEY);")]);
            assert_eq!(t.len(), 1, "{prose}");
        }
        assert!(schema_of(&["CREATE TABLE api_tokens (id text PRIMARY KEY);".into(), "DROP TABLE api_tokens;".into()]).is_empty(),
                "снятая миграцией таблица осталась");
        assert!(super::migration_up("0001_x.up.sql") && super::migration_up("0001_x.sql") && !super::migration_up("0001_x.down.sql"));
        assert!(!super::migration_up("migrations/2024_x/down.sql") && super::migration_up("migrations/2024_x/up.sql"));
    }

    #[test]
    fn an_edit_of_an_unknown_table_creates_nothing() {
        let t = schema_of(&[CREATE.into(), "ALTER TABLE чужая ADD COLUMN x text;".into()]);
        assert_eq!(t.len(), 1, "завелась таблица, которой не создавали");
    }
}

/// Достаётся ли параметр отрезка пути этой таблице.
///
/// Отрезок сверялся С ИМЕНЕМ таблицы, и параметр доставался только ей. У
/// дочерней коллекции родительский ключ так не доставался НИКОМУ:
/// `/postmortems/{postmortem_id}/actions` отдавал `postmortem_id` таблице
/// `postmortems`, а нужен он `postmortem_actions.postmortem_id` — колонке
/// `NOT NULL`, которая иначе выглядит не заполняемой ничем.
///
/// Дочерняя — та, чьё имя начинается с ОСНОВЫ отрезка и подчёркивания:
/// `postmortems` → `postmortem_actions`, `postmortem_blocks`. Подчёркивание
/// обязательно, иначе `posts` затянул бы `postmortems`.
pub(crate) fn segment_owns(segment: &str, table: &str) -> bool {
    if segment == table {
        return true;
    }
    // Множественное число по-английски снимается тремя способами, и какой из
    // них верен, знает только слово: `postmortems` → `postmortem`,
    // `policies` → `policy`, `boxes` → `box`. Годится ЛЮБАЯ основа, потому что
    // подчёркивание всё равно требуется: `policie_` не совпадёт ни с чем, а
    // `policy_versions` совпадёт с `policy_`.
    let mut stems = vec![segment.to_owned()];
    if let Some(base) = segment.strip_suffix("ies") {
        stems.push(format!("{base}y"));
    }
    if let Some(base) = segment.strip_suffix("es") {
        stems.push(base.to_owned());
    }
    if let Some(base) = segment.strip_suffix('s') {
        stems.push(base.to_owned());
    }
    // Основа короче трёх букв ничего не различает и раздала бы входы наугад.
    stems
        .iter()
        .any(|st| st.chars().count() >= 3 && table.starts_with(&format!("{st}_")))
}

#[cfg(test)]
mod segment {
    use super::segment_owns;

    #[test]
    fn own_table() {
        assert!(segment_owns("postmortems", "postmortems"));
    }

    #[test]
    fn child_collection_gets_parent_key() {
        assert!(segment_owns("postmortems", "postmortem_actions"));
        assert!(segment_owns("postmortems", "postmortem_blocks"));
    }

    #[test]
    fn foreign_table_with_shared_prefix_not_gets() {
        // `posts` и `postmortems` — разные вещи, и подчёркивание их разводит.
        assert!(!segment_owns("posts", "postmortems"));
    }

    #[test]
    fn neighbour_table_not_gets() {
        assert!(!segment_owns("postmortems", "monitors"));
    }

    #[test]
    fn plural_on_ies_gives_base_on_y() {
        // `/policies/{policy_id}/versions` — ключ нужен `policy_versions`.
        assert!(segment_owns("policies", "policy_versions"));
        assert!(segment_owns("policies", "policy_test_runs"));
    }

    #[test]
    fn short_base_not_hands_out() {
        assert!(!segment_owns("as", "a_b"));
    }
}

fn snake_name(s: &str) -> String {
    snake(s)
}

/// Какой таблице отвечает схема контракта: имя в змеином регистре, оно же во
/// множественном числе. Пара, которую так не угадать, называется явно.
pub(crate) fn schema_table(schemas: &[String], tables: &[Table], forced: &[(&str, &str)]) -> Vec<(String, String)> {
    let known: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    let mut out = Vec::new();
    for name in schemas {
        if let Some((_, t)) = forced.iter().find(|(s, _)| *s == name) {
            if !t.is_empty() && known.contains(t) {
                out.push((name.clone(), (*t).to_owned()));
            }
            continue;
        }
        let b = snake_name(name);
        let plural: Vec<String> = if b.ends_with('y') {
            vec![format!("{}ies", &b[..b.len() - 1]), format!("{b}s")]
        } else {
            vec![format!("{b}s"), format!("{b}es")]
        };
        let mut cands = vec![b.clone()];
        cands.extend(plural);
        if let Some(t) = cands.into_iter().find(|x| known.contains(&x.as_str())) {
            out.push((name.clone(), t));
        }
    }
    out
}

/// Поля схем контракта вместе с пометками и обязательностью.
pub(crate) fn contract_fields(doc: &Value) -> Vec<Field> {
    let mut out = Vec::new();
    let Some(schemas) = doc.get("components").and_then(|c| c.get("schemas")).and_then(|s| s.as_object())
    else {
        return out;
    };
    // Обход РЕКУРСИВНЫЙ: `PolicyTestRun.window.from` — такое же поле, как поле
    // верхнего уровня, и колонку оно требует так же. Обход по одному уровню
    // терял их и заодно раздувал вторую находку: колонка, названная вложенным
    // полем, выглядела бы колонкой без входа.
    fn walk(node: &Value, at: &str, out: &mut Vec<Field>) {
        let Some(o) = node.as_object() else { return };
        let req: Vec<String> = o
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        if let Some(props) = o.get("properties").and_then(|p| p.as_object()) {
            for (name, p) in props {
                let here = format!("{at}.{name}");
                let marks: Vec<String> = ["x-derived", "x-derived-unknown", "x-input-only"]
                    .iter()
                    .filter(|m| p.get(**m).is_some())
                    .map(|m| (*m).to_owned())
                    .collect();
                let composite = p.is_object()
                    && (["properties", "items", "allOf", "oneOf", "anyOf", "additionalProperties"]
                        .iter()
                        .any(|k| p.get(*k).is_some())
                        || p.get("type").and_then(|t| t.as_str()) == Some("object")
                        || p.get("type").and_then(|t| t.as_str()) == Some("array"));
                let nullable = p
                    .get("type")
                    .and_then(|t| t.as_array())
                    .map(|a| a.iter().any(|x| x.as_str() == Some("null")))
                    .unwrap_or(false)
                    || p.get("nullable").and_then(|n| n.as_bool()).unwrap_or(false);
                out.push(Field {
                    at: here.clone(),
                    nullable,
                    schema: at.split('.').next().unwrap_or(at).trim_end_matches("[]").to_owned(),
                    name: name.clone(),
                    marks,
                    required: req.iter().any(|r| r == name),
                    composite,
                    stored_in: p
                        .get("x-stored-in")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                });
                walk(p, &here, out);
            }
        }
        if let Some(items) = o.get("items") {
            walk(items, &format!("{at}[]"), out);
        }
        if let Some(all) = o.get("allOf").and_then(|a| a.as_array()) {
            for a in all {
                walk(a, at, out);
            }
        }
    }
    for (name, body) in schemas {
        walk(body, name, &mut out);
    }
    out
}

/// Имена всех свойств и параметров контракта: вход, откуда бы он ни пришёл.
pub(crate) fn contract_names(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(node: &Value, out: &mut Vec<String>) {
        match node {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => {
                if let Some(p) = o.get("properties").and_then(|p| p.as_object()) {
                    out.extend(p.keys().cloned());
                }
                if o.get("in").is_some() {
                    if let Some(n) = o.get("name").and_then(|n| n.as_str()) {
                        out.push(n.to_owned());
                    }
                }
                o.values().for_each(|v| walk(v, out));
            }
            _ => {}
        }
    }
    walk(doc, &mut out);
    out.sort();
    out.dedup();
    out
}

/// Три сверки контракта со схемой. Возвращает те же пары «имя · пояснение»,
/// что и остальные датчики.
pub(crate) fn contract_vs_schema(
    doc: &Value,
    tables: &[Table],
    forced: &[(&str, &str)],
) -> Vec<Pair> {
    let inputs = inputs_of_table(doc);
    const SYSTEM: [&str; 5] = ["id", "account_id", "created_at", "updated_at", "rev"];
    // Способ сказать «колонку пишет не контракт» был, а находка о нём молчала:
    // набор искал его в дверях и в спецификации датчика и подал заявку.
    const X_SOURCE: &str = ". Если колонку пишет не контракт, а сервер или воркер, — \
                            источник называется в миграции: COMMENT ON COLUMN таблица.колонка IS 'x-source: чем'";
    let schemas: Vec<String> = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let map = schema_table(&schemas, tables, forced);
    let fields = contract_fields(doc);
    let names = contract_names(doc);
    // Схема ответа сводится к САМОМУ ДЛИННОМУ известному имени-приставке:
    // `MonitorRunStep` принадлежит `MonitorRun`, а не `Monitor`. Короткая
    // приставка затянула бы чужую таблицу, и колонка выглядела бы с входом.
    let owner_of = |name: &str| -> Option<&String> {
        map.iter()
            .filter(|(sc, _)| name == sc || name.starts_with(sc.as_str()))
            .max_by_key(|(sc, _)| sc.len())
            .map(|(_, t)| t)
    };
    let exact = |name: &str| map.iter().find(|(sc, _)| sc == name).map(|(_, t)| t);
    // Скобка адреса, разложенная по таблице: `/monitors/{monitor_id}` кладёт
    // `monitor_id` таблице `monitors` и её дочерним. Колонка, названная скобкой,
    // вход имеет — просто пришла она не телом, а адресом.
    let mut params_of: Vec<(String, String)> = Vec::new();
    if let Some(paths) = doc.get("paths").and_then(|p| p.as_object()) {
        for path in paths.keys() {
            let segs: Vec<&str> = path.split('/').collect();
            for w in segs.windows(2) {
                let Some(n) = w[1].strip_prefix('{').and_then(|b| b.strip_suffix('}')) else { continue };
                // Отрезок обязан НАЗЫВАТЬ таблицу: иначе всякое слово в адресе
                // раздавало бы входы по совпадению приставки.
                if !tables.iter().any(|x| x.name == w[0]) {
                    continue;
                }
                for tab in tables.iter().filter(|x| segment_owns(w[0], &x.name)) {
                    params_of.push((tab.name.clone(), n.to_owned()));
                }
            }
        }
    }
    for (created, _, path) in &inputs {
        let Some(table) = exact(created) else { continue };
        for p in braces(path) {
            params_of.push((table.clone(), p.to_owned()));
        }
    }
    let mut out = Vec::new();
    let mut stored: Vec<(&str, &str)> = Vec::new();

    for f in &fields {
        if !f.marks.is_empty() || f.composite {
            continue;
        }
        if let Some(target) = &f.stored_in {
            let Some(home) = owner_of(&f.schema) else { continue };
            let col = target.split_once('.').filter(|(tn, cn)| {
                tn == home && tables.iter().any(|x| x.name == *tn && x.cols.iter().any(|c| c.name == *cn))
            });
            out.push(Pair {
                name: f.at.clone(),
                detail: match col {
                    Some(at) => {
                        stored.push(at);
                        format!("помечено x-stored-in: {target}")
                    }
                    None => format!(
                        "поле контракта без колонки: x-stored-in называет {target}, а в таблице {home} такой колонки нет"
                    ),
                },
            });
            continue;
        }
        let Some((_, table)) = map.iter().find(|(s, _)| *s == f.schema) else { continue };
        let Some(t) = tables.iter().find(|t| &t.name == table) else { continue };
        if t.cols.iter().any(|c| c.name == f.name) {
            continue;
        }
        out.push(Pair {
            name: f.at.clone(),
            detail: format!("поле контракта без колонки: в таблице {table} колонки {} нет, \
                             и пометки у поля нет", f.name),
        });
    }

    // Семья схем одной таблицы: `Monitor`, `MonitorCreate`, `MonitorPatch` пишут
    // в одну и ту же. Колонка, названная любой из них, названа.
    let family = |table: &str| -> Vec<String> {
        let owners: Vec<&String> = map.iter().filter(|(_, t)| t == table).map(|(s, _)| s).collect();
        fields
            .iter()
            .filter(|f| owners.iter().any(|o| f.schema == **o || f.schema.starts_with(o.as_str())))
            .map(|f| f.name.clone())
            .chain(stored.iter().filter(|(t, _)| *t == table).map(|(_, c)| (*c).to_owned()))
            .collect()
    };

    for (_, table) in map.iter().collect::<std::collections::BTreeSet<_>>() {
        let Some(t) = tables.iter().find(|t| &t.name == table) else { continue };
        let known = family(table);
        for c in &t.cols {
            if SYSTEM.contains(&c.name.as_str()) || known.contains(&c.name) {
                continue;
            }
            if params_of.iter().any(|(t2, n)| t2 == &t.name && n == &c.name) {
                continue;
            }
            let elsewhere = names.contains(&c.name);
            out.push(Pair {
                name: format!("{}.{}", t.name, c.name),
                detail: if let Some(s) = &c.source {
                    format!("помечено x-source: {s}")
                } else if elsewhere {
                    format!("колонка без входа: имя есть в контракте, но в ЧУЖОЙ схеме — \
                             положить в {} нечем{X_SOURCE}", t.name)
                } else {
                    format!("колонка без входа: ни свойства, ни параметра с именем {}{X_SOURCE}", c.name)
                },
            });
        }
    }

    // Обязательность сверяется В ОДНУ сторону: контракт обещал значение, а
    // схема разрешает пустое. Обратное — колонка строже контракта — не ложь
    // клиенту, а запас, и находкой не является: сверять его значит топить
    // настоящее обещание в шуме.
    let mut seen = std::collections::HashSet::new();
    // Колонка обязательна, а обязательного входа нет: `NOT NULL` без умолчания
    // значит, что без значения строку не завести, — и хоть одна схема семьи
    // обязана требовать это поле. Иначе создание падает на первом же запросе, и
    // узнают об этом не гейтом, а отказом базы.
    for (_, table) in map.iter().collect::<std::collections::BTreeSet<_>>() {
        let Some(t) = tables.iter().find(|t| &t.name == table) else { continue };
        // Схемы записи ЭТОЙ таблицы: те, что пишут (тело запроса) И сами
        // отображаются на неё. Адрес сюда не годится: `POST /forecasts/{id}/tasks`
        // трогает два отрезка пути, а пишет в один.
        // Входы ЭТОЙ таблицы: тела запросов у операций, чей ответ называет
        // вещь этой таблицы. Имя входной схемы таблицей не зовётся, и искать
        // по нему — то же, что искать по фамилии соседа.
        let here: Vec<(&String, &String)> = inputs
            .iter()
            .filter(|(created, input, _)| {
                (if input.is_empty() { exact(created) } else { owner_of(created) }) == Some(table)
            })
            .map(|(_, input, path)| (input, path))
            .collect();
        let paths: Vec<&str> = inputs
            .iter()
            .filter(|(created, _, _)| exact(created) == Some(table))
            .map(|(_, _, path)| path.as_str())
            .collect();
        if here.is_empty() {
            // Таблица, в которую контракт ничем не пишет: половина Б её не
            // смотрит. Строки в ней заводит что-то мимо контракта — миграция,
            // рука, чужой процесс, — и обещания контракта её не касаются.
            // Считаются только ОБЯЗАТЕЛЬНЫЕ колонки. Таблица, все колонки которой
            // с умолчанием, заводится пустой вставкой, и отсутствие операции в
            // контракте про неё ничего не говорит.
            let cols: Vec<&str> = t
                .cols
                .iter()
                .filter(|c| {
                    !SYSTEM.contains(&c.name.as_str())
                        && c.not_null
                        && !c.has_default
                        && !c.primary_key
                })
                .map(|c| c.name.as_str())
                .collect();
            if !cols.is_empty() {
                out.push(Pair {
                    name: format!("{} · без записи", t.name),
                    detail: match &t.source {
                        Some(s) => format!("помечено x-source: {s}"),
                        None => format!("таблица есть, а контракт в неё ничем не пишет: {} колонок", cols.len()),
                    },
                });
            }
            continue;
        }
        let required_here: Vec<&str> = fields
            .iter()
            .filter(|f| f.required && here.iter().any(|(s, _)| **s == f.schema))
            .map(|f| f.column(&t.name))
            .chain(paths.iter().flat_map(|path| braces(path)))
            .collect();
        let parents: Vec<&str> = paths
            .iter()
            .flat_map(|path| {
                let segs: Vec<&str> = path.split('/').collect();
                segs.windows(2)
                    .filter(|w| {
                        w[1].strip_prefix('{')
                            .and_then(|b| b.strip_suffix('}'))
                            .is_some_and(|b| !t.cols.iter().any(|c| c.name == b && !c.primary_key))
                    })
                    .map(|w| w[0])
                    .collect::<Vec<&str>>()
            })
            .collect();
        for c in &t.cols {
            if SYSTEM.contains(&c.name.as_str()) || !c.not_null || c.has_default || c.primary_key {
                continue;
            }
            if required_here.contains(&c.name.as_str()) {
                continue;
            }
            let by_key = c.references.as_deref().is_some_and(|r| {
                parents.contains(&r)
                    && t.cols
                        .iter()
                        .filter(|x| x.not_null && x.references.as_deref() == Some(r))
                        .count()
                        == 1
            });
            if by_key {
                continue;
            }
            // Имя несёт РОД находки: у факта ключ — имя, и две разные находки
            // об одной колонке вытесняли одна другую молча. Счёт «колонка без
            // входа» упал с шестидесяти пяти до двадцати семи, и выглядело это
            // починкой.
            out.push(Pair {
                name: format!("{}.{} · обязательна", t.name, c.name),
                detail: match &c.source {
                    Some(s) => format!("помечено x-source: {s}"),
                    None => format!("колонка обязательна, а обязательного входа нет: NOT NULL без умолчания, \
                             и ни одна схема семьи её не требует{X_SOURCE}"),
                },
            });
        }
    }

    for f in &fields {
        if !f.required || f.composite || f.nullable {
            continue;
        }
        if f.marks.iter().any(|m| m == "x-derived" || m == "x-input-only") {
            continue;
        }
        let Some((_, table)) = map.iter().find(|(s, _)| *s == f.schema) else { continue };
        let Some(t) = tables.iter().find(|t| &t.name == table) else { continue };
        let Some(c) = t.cols.iter().find(|c| c.name == f.column(&t.name)) else { continue };
        if c.not_null || c.primary_key {
            continue;
        }
        if !seen.insert(format!("{} :: {}", f.at, t.name)) {
            continue;
        }
        out.push(Pair {
            name: f.at.clone(),
            detail: format!("обязательность разошлась: поле в required и не обнуляемо, \
                             а колонка {}.{} допускает пустое", t.name, c.name),
        });
    }
    out
}

fn braces(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter_map(|s| s.strip_prefix('{')?.strip_suffix('}'))
}

/// Требование → операции контракта. Разметка живёт в `description` операции,
/// и читается она построчно по отступу, а не разбором дерева: ветка
/// `components` называет те же `FR` в описаниях СХЕМ, и они не операции.
///
/// Строка-комментарий не принадлежит операции. Заголовок вида
/// `# ── срез статуса (FR-STP-01…14) ──` стоит ПОСЛЕ последней операции блока,
/// и без этого правила разметка соседнего среза приписывалась предыдущей
/// операции: `DELETE /devices/{deviceId}` оказывался операцией статус-страницы.
pub(crate) fn requirement_ops(text: &str) -> Vec<Pair> {
    let Some(after) = text.split("\npaths:").nth(1) else { return Vec::new() };
    let block = after.split("\ncomponents:").next().unwrap_or("");
    let path_re = regex::Regex::new(r"^ {2}(/\S+):").expect("образец пути");
    let op_re = regex::Regex::new(r"^ {4}(get|post|patch|put|delete):").expect("образец операции");
    let mut path: Option<String> = None;
    let mut op: Option<String> = None;
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in block.split('\n') {
        if let Some(p) = path_re.captures(line) {
            path = Some(p[1].to_owned());
            op = None;
            continue;
        }
        if let Some(o) = op_re.captures(line) {
            op = Some(o[1].to_uppercase());
            continue;
        }
        let (Some(p), Some(o)) = (&path, &op) else { continue };
        if line.trim_start().starts_with('#') {
            continue;
        }
        // Перечень раскрывается тем же раскрывателем, что и везде: `FR-STP-01…14`
        // в описании операции значит четырнадцать требований, а не одно.
        for name in crate::reproject::ids::expand(line) {
            if !name.starts_with("FR-") {
                continue;
            }
            let key = format!("{name} · {o} {p}");
            if seen.insert(key.clone()) {
                pairs.push((key, format!("{o} {p}")));
            }
        }
    }
    pairs.into_iter().map(|(name, detail)| Pair { name, detail }).collect()
}

/// Требования, названные ТЕЛОМ контракта — тем, что идёт после `info:`.
///
/// Шапка контракта называет требования прозой описания: перечисляет, что он
/// покрывает. Считать это «названо телом» значит считать оглавление за текст —
/// разница в сорок четыре требования из двухсот семидесяти двух.
pub(crate) fn requirements_in_body(text: &str) -> Vec<Pair> {
    let body = match text.find("\ninfo:") {
        Some(i) => &text[i..],
        None => text,
    };
    let re = regex::Regex::new(r"FR-[A-Z]+-[0-9]+").expect("образец требования");
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in re.find_iter(body) {
        let name = m.as_str().to_owned();
        if seen.insert(name.clone()) {
            out.push(Pair { name, detail: "названо телом контракта".to_owned() });
        }
    }
    out
}

/// Пометки полей контракта: чем поле объявлено выводимым и записано ли правило.
pub(crate) fn contract_marks(doc: &Value) -> Vec<Pair> {
    let mut out = Vec::new();
    let Some(schemas) = doc.get("components").and_then(|c| c.get("schemas")).and_then(|s| s.as_object())
    else {
        return out;
    };
    fn walk(node: &Value, at: &str, out: &mut Vec<Pair>) {
        let Some(o) = node.as_object() else { return };
        if let Some(props) = o.get("properties").and_then(|p| p.as_object()) {
            for (name, p) in props {
                let here = format!("{at}.{name}");
                // `x-derived-unknown` — пометка «выводится, а как — не записано».
                // Она законна ровно как временная: правило обязано приехать.
                if let Some(v) = p.get("x-derived-unknown") {
                    let why = v.as_str().unwrap_or("").trim();
                    out.push(Pair {
                        name: here.clone(),
                        detail: format!("правило вывода не записано: {}",
                                        if why.is_empty() { "пометка пуста" } else { why }),
                    });
                }
                if let Some(v) = p.get("x-derived") {
                    let why = v.as_str().unwrap_or("").trim();
                    if why.is_empty() {
                        out.push(Pair {
                            name: here.clone(),
                            detail: "пометка «выводится» ничего не называет".to_owned(),
                        });
                    }
                }
                walk(p, &here, out);
            }
        }
        if let Some(items) = o.get("items") {
            walk(items, &format!("{at}[]"), out);
        }
        if let Some(all) = o.get("allOf").and_then(|a| a.as_array()) {
            for a in all {
                walk(a, at, out);
            }
        }
    }
    for (name, body) in schemas {
        walk(body, name, &mut out);
    }
    out
}

/// Множества `enum` контракта с их адресом.
pub(crate) fn contract_enums(doc: &Value) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    fn walk(node: &Value, at: &str, out: &mut Vec<(String, Vec<String>)>) {
        match node {
            Value::Array(a) => {
                for x in a {
                    walk(x, at, out);
                }
            }
            Value::Object(o) => {
                if let Some(e) = o.get("enum").and_then(|e| e.as_array()) {
                    let vals: Vec<String> = e
                        .iter()
                        .filter(|v| !v.is_null())
                        .map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))
                        .collect();
                    if !vals.is_empty() {
                        out.push((at.to_owned(), vals));
                    }
                }
                for (k, v) in o {
                    let here = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                    walk(v, &here, out);
                }
            }
            _ => {}
        }
    }
    walk(doc, "", &mut out);
    out
}

/// Значение множества `CHECK`, у которого нет пути внутрь: ни ветки домена, ни
/// значения словаря контракта. Такое значение хранится, а положить его нечем.
pub(crate) fn check_values_without_path(
    enums: &[Enum],
    checks: &[CheckSet],
    contract: &[(String, Vec<String>)],
    forced: &[(&str, &str)],
    tables: &[Table],
) -> Vec<Pair> {
    // Пара «перечисление ↔ множество» уже считается: берём её же, чтобы два
    // правила не расходились в том, что с чем спарено.
    let paired = pair(enums, checks, contract, forced, tables);
    let mut out = Vec::new();
    for s in checks {
        let mut paths: std::collections::HashSet<&str> = Default::default();
        // Ветки парного перечисления.
        if let Some(p) = paired.iter().find(|p| p.name.ends_with(&format!("↔ {}", s.at))) {
            let enum_name = p.name.split(" ↔ ").next().unwrap_or("");
            if let Some(e) = enums.iter().find(|e| e.name == enum_name) {
                for v in &e.stored {
                    paths.insert(v.as_str());
                }
            }
        }
        // Значения словаря контракта: множество, пересекающееся с этим по
        // значению, и есть словарь этой колонки.
        for (_, vals) in contract {
            let common = vals.iter().filter(|v| s.values.contains(v)).count();
            if common >= 2 || (common == 1 && s.values.len() == 1) {
                for v in vals {
                    paths.insert(v.as_str());
                }
            }
        }
        for v in &s.values {
            if !paths.contains(v.as_str()) {
                out.push(Pair {
                    name: format!("{}='{}'", s.at, v),
                    detail: format!("значение хранится, а пути внутрь нет: \
                                     ни ветки домена, ни значения словаря контракта ({})", s.file),
                });
            }
        }
    }
    out
}

/// Схема записи и АДРЕС, по которому пишут: `POST /monitors` → `MonitorCreate`.
///
/// Без адреса схему записи приходится угадывать по имени, и семья `Monitor*`
/// затягивает читающие схемы: они ничего не требуют по природе, и колонка
/// выглядит без входа. Адрес называет таблицу прямо.
/// Вход таблицы: схема ТЕЛА ЗАПРОСА у операции, чей ответ 201 называет саму
/// вещь. Таблицу называет ответ, а вход — запрос: искать таблицу по имени
/// входной схемы бесполезно, `MonitorInput` таблицей не зовётся.
pub(crate) fn inputs_of_table(doc: &Value) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let Some(paths) = doc.get("paths").and_then(|p| p.as_object()) else { return out };
    fn refs(node: &Value, out: &mut Vec<String>) {
        match node {
            Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
            Value::Object(o) => {
                if let Some(r) = o.get("$ref").and_then(|r| r.as_str()) {
                    if let Some(name) = r.rsplit('/').next() {
                        out.push(name.to_owned());
                    }
                }
                o.values().for_each(|v| refs(v, out));
            }
            _ => {}
        }
    }
    for (path, ops) in paths {
        let Some(o) = ops.as_object() else { continue };
        for (method, op) in o {
            if !matches!(method.as_str(), "post" | "put" | "patch") {
                continue;
            }
            // Только `201` и только ПРЯМАЯ ссылка схемы ответа. Ответ `200`
            // означает «вот тебе результат», а не «строка заведена»: у
            // `POST /correlation/rules/reorder` он есть, вставки нет, и по нему
            // таблица выглядела бы пишущейся. Обход вглубь тоже лишний — он
            // цепляет `items` перечня и вложенные части, и создание чужой вещи
            // читалось бы созданием этой.
            let mut created = Vec::new();
            if let Some(c) = op
                .get("responses")
                .and_then(|r| r.get("201"))
                .and_then(|r| r.get("content"))
                .and_then(|c| c.as_object())
            {
                for mm in c.values() {
                    if let Some(r) = mm.get("schema").and_then(|s| s.get("$ref")).and_then(|r| r.as_str()) {
                        if let Some(name) = r.rsplit('/').next() {
                            created.push(name.to_owned());
                        }
                    }
                }
            }
            let mut input = Vec::new();
            if let Some(b) = op.get("requestBody") {
                refs(b, &mut input);
            }
            if input.is_empty() {
                input.push(String::new());
            }
            for c in &created {
                for i in &input {
                    out.push((c.clone(), i.clone(), path.clone()));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Величины контракта: сколько в нём путей, операций и схем.
///
/// Шапка контракта называет их прозой, и правят её реже, чем сам контракт.
/// Число в шапке читают первым, и разошедшееся врёт убедительнее всего.
pub(crate) fn contract_sizes(doc: &Value) -> Vec<(String, usize)> {
    const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
    let paths = doc.get("paths").and_then(|p| p.as_object());
    let n_paths = paths.map(|p| p.len()).unwrap_or(0);
    let n_ops = paths
        .map(|p| {
            p.values()
                .filter_map(|ops| ops.as_object())
                .map(|o| o.keys().filter(|k| METHODS.contains(&k.as_str())).count())
                .sum()
        })
        .unwrap_or(0);
    let n_schemas = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .map(|o| o.len())
        .unwrap_or(0);
    vec![
        ("путей".to_owned(), n_paths),
        ("операций".to_owned(), n_ops),
        ("схем".to_owned(), n_schemas),
    ]
}

/// Шапка контракта против него самого: число, названное прозой заголовка,
/// сверяется с обходом.
/// Требования, названные телом контракта, и те, что объявлены только границей.
///
/// Граница — это имена, стоящие в шапке и НЕ стоящие в теле: шапка перечисляет
/// то, чему в контракте места не нашлось. Считать её отдельным перечнем нельзя,
/// иначе она разойдётся с телом при первом же переносе имени.
pub(crate) fn contract_requirement_split(text: &str) -> (usize, usize) {
    let Ok(re) = regex::Regex::new(r"FR-[A-Z]+-[0-9]+") else { return (0, 0) };
    let cut = match text.find("\ninfo:") {
        Some(i) => i,
        None => return (0, 0),
    };
    let head: std::collections::HashSet<&str> = re.find_iter(&text[..cut]).map(|m| m.as_str()).collect();
    let body: std::collections::HashSet<&str> = re.find_iter(&text[cut..]).map(|m| m.as_str()).collect();
    (body.len(), head.difference(&body).count())
}

/// Заявленное шапкой общее число требований — фактом, а не сверкой.
///
/// Сверять его с набором требований в коде было бы пересказом базы: число
/// требований живёт в базе, и связь с ним — соединение по колонке, а не
/// вторая копия счёта здесь.
pub(crate) fn contract_head_total(text: &str) -> Vec<Pair> {
    let mut out = Vec::new();
    let Some(cut) = text.find("\ninfo:") else { return out };
    let head = unquote(&text[..cut]);
    let Ok(re) = regex::Regex::new(r"не названо ни одного:\s*([0-9]{1,5})\s*\+\s*([0-9]{1,5})\s*=\s*([0-9]{1,5})") else {
        return out;
    };
    match re.captures(&head) {
        Some(c) => out.push(Pair {
            name: format!("шапка · всего требований · {}", &c[3]),
            detail: format!("шапка закрывает себя равенством и называет всего {}", &c[3]),
        }),
        None => out.push(Pair {
            name: "шапка · всего требований · не названо".to_owned(),
            detail: "строки «не названо ни одого: A + B = N» в шапке нет: равенство, \
                     которым шапка сама себя закрывает, сверять не с чем"
                .to_owned(),
        }),
    }
    out
}

/// Гасит цитаты: «ёлочки» и обратные кавычки.
///
/// Число в кавычках — УПОМИНАНИЕ прошлого состояния, а не сегодняшнее
/// утверждение. Шапка рассказывает, как правило нашло расхождение `225 / 36`
/// против `222 / 39`, — потребовать сдвинуть эти числа значит потребовать
/// стереть улику. Та же граница, что у правила снятого термина: в кавычках —
/// назвали, без кавычек — сказали.
fn unquote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut tick = false;
    for ch in text.chars() {
        match ch {
            '«' => { depth += 1; out.push(' '); }
            '»' => { depth = depth.saturating_sub(1); out.push(' '); }
            '`' => { tick = !tick; out.push(' '); }
            // Перевод строки кавычку закрывает: незакрытая обратная кавычка
            // иначе гасила бы весь остаток шапки, и правило молчало бы,
            // выглядя зелёным.
            '\n' => { tick = false; out.push('\n'); }
            _ => out.push(if depth > 0 || tick { ' ' } else { ch }),
        }
    }
    out
}

/// Число, перед которым не буква, не цифра и не дефис.
///
/// «`FR-MON-19` названо ТЕЛОМ» — это про одно требование, а не «девятнадцать
/// названо телом». Хвост имени, принятый за величину, дал бы находку на ровном
/// месте.
fn free_number(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '-')
}

/// Числа, которые шапка контракта называет о самой себе.
///
/// Шапка держит те же величины, что и тело, и сдвигается отдельно от них.
/// Правило спрашивает КАЖДОЕ вхождение имени величины, а не первую строку
/// заданной формы: та же величина, сказанная в абзаце другими словами, иначе
/// не читается никем — и это доказано делом, а не опасением.
pub(crate) fn contract_head_echo(text: &str, in_body: usize, border: usize, sizes: &[(String, usize)]) -> Vec<Pair> {
    let mut out = Vec::new();
    let Some(cut) = text.find("\ninfo:") else {
        out.push(Pair {
            name: "шапка · нет строки info:".to_owned(),
            detail: "шапка от тела не отделяется, и обе мерки шапки считать не по чему".to_owned(),
        });
        return out;
    };
    let head = unquote(&text[..cut]);
    let mut nth: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut say = |place: &str, got: &str, what: &str, fact: String| {
        let key = format!("{place} {got}");
        let k = nth.entry(key).or_insert(0);
        *k += 1;
        // Одна и та же пара стоит в шапке дважды, в разных абзацах. Без
        // порядкового номера второй список вытеснил бы первый молча, и рост
        // долга прошёл бы при зелёном.
        out.push(Pair {
            name: format!("шапка · {place} · {got} #{k}"),
            detail: format!("{place}: «{got}», а по контракту {what} {fact}"),
        });
    };
    let named = [
        // `(?i)` — не вкус: шапка пишет «названо ТЕЛОМ» прописными, и образец,
        // знающий один регистр, на ней молчит. Проверено подсадкой: без этого
        // сдвинутое число прошло мимо.
        (r"(?i)([0-9]{1,5})\s*(?:—\s*)?назван[оы]\s+телом", in_body, "названо телом"),
        (r"(?i)([0-9]{1,5})\s*(?:—\s*)?объявлен[оы](?:\s+только)?\s+границ", border, "объявлено границей"),
    ];
    for (form, fact, what) in named {
        let Ok(re) = regex::Regex::new(form) else { continue };
        for c in re.captures_iter(&head) {
            let m = c.get(1).expect("группа числа");
            if !free_number(&head, m.start()) {
                continue;
            }
            let said: usize = m.as_str().parse().unwrap_or(0);
            if said != fact {
                say("имя величины", c.get(0).expect("совпадение").as_str().trim(), what, fact.to_string());
            }
        }
    }
    // Разбиение. Шапка сама объявляет, что величины делятся надвое без
    // остатка, — значит всякое равенство в ней есть это разбиение, другого
    // она не пишет.
    if let Ok(re) = regex::Regex::new(r"([0-9]{2,4})\s*\+\s*([0-9]{1,4})\s*=\s*([0-9]{2,4})") {
        for c in re.captures_iter(&head) {
            let a: usize = c[1].parse().unwrap_or(0);
            let b: usize = c[2].parse().unwrap_or(0);
            let n: usize = c[3].parse().unwrap_or(0);
            if a != in_body || b != border {
                say("равенство", c.get(0).expect("совпадение").as_str().trim(), "разбиение",
                    format!("{in_body} + {border} = {}", in_body + border));
            } else if a + b != n {
                say("равенство", c.get(0).expect("совпадение").as_str().trim(), "равенство не сходится само с собой",
                    format!("{} ≠ {n}", a + b));
            }
        }
    }
    // Тройки. Их у шапки ровно две — «путей / операций / схем» и «всего /
    // телом / границей»; третьей она не пишет, и всякая иная протухла.
    let by = |k: &str| sizes.iter().find(|(w, _)| w == k).map(|(_, n)| *n);
    let mut allowed: Vec<String> = Vec::new();
    if let (Some(p), Some(o), Some(s)) = (by("путей"), by("операций"), by("схем")) {
        allowed.push(format!("{p} / {o} / {s}"));
    }
    allowed.push(format!("{} / {in_body} / {border}", in_body + border));
    if let Ok(re) = regex::Regex::new(r"([0-9]{2,4})\s*/\s*([0-9]{1,4})\s*/\s*([0-9]{1,4})") {
        for c in re.captures_iter(&head) {
            let got = format!("{} / {} / {}", &c[1], &c[2], &c[3]);
            if !allowed.contains(&got) {
                say("тройка", &got, "тройки", allowed.join("  либо  "));
            }
        }
    }
    // Пара раскрывателя. Абзац — единица: слово и пара разъезжаются переносом
    // строки, и пара, найденная в чужом абзаце, — не та пара.
    if let Ok(re) = regex::Regex::new(r"([0-9]{2,4})\s*/\s*([0-9]{1,4})") {
        let pair = format!("{in_body} / {border}");
        for para in head.split("\n#\n") {
            if !para.contains("раскрыват") {
                continue;
            }
            for c in re.captures_iter(para) {
                let m = c.get(0).expect("совпадение");
                if !free_number(para, m.start()) {
                    continue;
                }
                // Тройка — не пара: у неё сзади ещё одна дробь.
                if para[m.end()..].starts_with('/') || para[m.end()..].starts_with(" /") {
                    continue;
                }
                let got = format!("{} / {}", &c[1], &c[2]);
                if got != pair {
                    say("пара раскрывателя", &got, "пары", pair.clone());
                }
            }
        }
    }
    out
}

pub(crate) fn contract_head(text: &str, doc: &Value) -> Vec<Pair> {
    let sizes = contract_sizes(doc);
    let mut out = Vec::new();
    for (what, n) in &sizes {
        // Величину называют двумя порядками слов: «192 схем» и «схем: 192».
        // Правило, знающее один, пропускает ту же величину, сказанную иначе, —
        // и шапка врёт половиной, которую никто не сверяет.
        let stem: String = what.chars().take(what.chars().count().saturating_sub(2)).collect();
        let stem = if stem.is_empty() { what.clone() } else { stem };
        for form in [
            format!(r"(?m)^#.*?([0-9]{{1,5}})\s+{stem}"),
            // Второй порядок слов — «схем: 192» — берётся только вплотную.
            // Попытка допустить слова между ними притянула двенадцать чужих
            // чисел той же строки: правило стало находить величину там, где её
            // не называли.
            format!(r"(?m)^#.*?{stem}[а-яё]*\s*[:—–-]\s*([0-9]{{1,5}})"),
        ] {
            let Ok(re) = regex::Regex::new(&form) else { continue };
            for c in re.captures_iter(text) {
                let said: usize = c[1].parse().unwrap_or(0);
                if said != *n {
                    let key = format!("шапка · {what} · {said}");
                    if !out.iter().any(|p: &Pair| p.name == key) {
                        out.push(Pair {
                            name: key,
                            detail: format!("шапка говорит {said}, а обход даёт {n}"),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Разобран ли корпус ЦЕЛИКОМ: величина обходом против величины по тексту.
///
/// Сломанная ветка уносит из корпуса целый раздел, и обе охраны показывают
/// ноль — правило зелено ровно потому, что ему нечего сказать. Поэтому
/// сверяется то, НА ЧЁМ СТОЯТ ПРАВИЛА: число схем, число таблиц, число
/// перечислений — каждое двумя способами.
pub(crate) fn corpus_reach(
    contract_text: &str,
    doc: &Value,
    enums: &[Enum],
    tables: &[Table],
) -> Vec<Pair> {
    let mut out = Vec::new();
    let by_walk = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .map(|o| o.len())
        .unwrap_or(0);
    let head = regex::Regex::new(r"^[A-Za-z][A-Za-z0-9_]*:$").expect("образец имени схемы");
    let mut by_text = 0usize;
    let mut in_schemas = false;
    for l in crate::yaml::structural_lines(contract_text) {
        if l.indent == 2 && l.body == "schemas:" {
            in_schemas = true;
            continue;
        }
        if in_schemas && l.indent <= 2 {
            in_schemas = false;
        }
        if in_schemas && l.indent == 4 && head.is_match(&l.body) {
            by_text += 1;
        }
    }
    if by_walk != by_text {
        out.push(Pair {
            name: "контракт · схемы".to_owned(),
            detail: format!("обходом {by_walk}, по тексту {by_text}: разбор потерял ветку"),
        });
    }
    if by_walk == 0 {
        out.push(Pair {
            name: "контракт".to_owned(),
            detail: "не разобрано ни одной схемы: корпус пуст, сверять нечем".to_owned(),
        });
    }
    if enums.is_empty() {
        out.push(Pair {
            name: "домен".to_owned(),
            detail: "не разобрано ни одного перечисления: корпус пуст, сверять нечем".to_owned(),
        });
    }
    if tables.is_empty() {
        out.push(Pair {
            name: "схема".to_owned(),
            detail: "не разобрано ни одной таблицы: корпус пуст, сверять нечем".to_owned(),
        });
    }
    out
}

#[cfg(test)]
mod marks {
    use super::*;
    use serde_json::json;

    const SQL: &str = r#"
CREATE TABLE absences (
  id text PRIMARY KEY,
  account_id text NOT NULL,
  starts_at timestamptz NOT NULL,
  synced_at timestamptz,
  source text NOT NULL
);
CREATE TABLE gap_dismissals (id text PRIMARY KEY, gap_key text NOT NULL);
CREATE TABLE tenants (id text PRIMARY KEY, name text NOT NULL);
CREATE TABLE monitors (id text PRIMARY KEY, name text NOT NULL);
CREATE TABLE shifts (id text PRIMARY KEY, account_id text NOT NULL);
CREATE TABLE handovers (
  id text PRIMARY KEY,
  account_id text NOT NULL REFERENCES accounts(id),
  from_shift_id text NOT NULL,
  to_shift_id text,
  FOREIGN KEY (from_shift_id, account_id) REFERENCES shifts (id, account_id),
  FOREIGN KEY (to_shift_id, account_id) REFERENCES shifts (id, account_id)
);
"#;

    const MARKS: &str = "
COMMENT ON COLUMN absences.synced_at IS 'x-source: синхронизация календаря';
COMMENT ON COLUMN absences.source IS 'x-source: POST /absences пишет ''manual''';
COMMENT ON COLUMN absences.starts_at IS 'x-source: ';
COMMENT ON TABLE tenants IS 'x-source: импорт конфигурации';
";

    fn contract() -> Value {
        let body = |s: &str| {
            json!({ "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{s}") } } } })
        };
        json!({
            "paths": {
                "/absences": { "post": { "requestBody": body("AbsenceInput"), "responses": { "201": body("Absence") } } },
                "/gaps/{gap_key}/dismiss": { "post": { "responses": { "201": body("GapDismissal") } } },
                "/shifts/{shift_id}/handover": {
                    "put": { "requestBody": body("HandoffInput"), "responses": { "201": body("Handoff") } }
                }
            },
            "components": { "schemas": {
                "Absence": { "properties": { "from": { "type": "string", "x-stored-in": "absences.starts_at" } } },
                "AbsenceInput": { "required": ["from"],
                                  "properties": { "from": { "type": "string", "x-stored-in": " absences.starts_at " } } },
                "GapDismissal": { "properties": { "at": { "type": "string", "x-stored-in": "gap_dismissals.dismissed_at" } } },
                "Tenant": { "properties": { "name": { "type": "string" } } },
                "Monitor": { "properties": { "name": { "type": "string" } } },
                "Handoff": { "properties": { "to_shift_id": { "type": "string" } } },
                "HandoffInput": { "properties": { "note": { "type": "string" } } }
            } }
        })
    }

    fn compare(sql: &str) -> Vec<Pair> {
        contract_vs_schema(&contract(), &schema_of(&[sql.into(), MARKS.into()]), &[("Handoff", "handovers")])
    }

    fn note<'a>(p: &'a [Pair], name: &str) -> Option<&'a str> {
        p.iter().find(|x| x.name == name).map(|x| x.detail.as_str())
    }

    #[test]
    fn comment_and_table_foreign_key_reach_before_schema() {
        let t = schema_of(&[SQL.into(), MARKS.into()]);
        let col = |table: &str, name: &str| {
            t.iter().find(|x| x.name == table).and_then(|x| x.cols.iter().find(|c| c.name == name)).expect("колонка")
        };
        assert_eq!(col("absences", "source").source.as_deref(), Some("POST /absences пишет 'manual'"));
        assert!(col("absences", "starts_at").source.is_none(), "пустая метка меткой не считается");
        assert_eq!(t.iter().find(|x| x.name == "tenants").and_then(|x| x.source.as_deref()),
                   Some("импорт конфигурации"));
        assert_eq!(col("handovers", "from_shift_id").references.as_deref(), Some("shifts"));
        assert_eq!(col("handovers", "to_shift_id").references.as_deref(), Some("shifts"));
        assert_eq!(col("handovers", "account_id").references.as_deref(), Some("accounts"));
    }

    #[test]
    fn a_field_carries_x_stored_in_truncated() {
        let f = contract_fields(&contract());
        let at = |a: &str| f.iter().find(|x| x.at == a).and_then(|x| x.stored_in.as_deref());
        assert_eq!(at("AbsenceInput.from"), Some("absences.starts_at"));
        assert_eq!(at("Tenant.name"), None);
    }

    #[test]
    fn creation_without_body_too_input_and_carries_path() {
        let i = inputs_of_table(&contract());
        assert!(i.contains(&("GapDismissal".into(), String::new(), "/gaps/{gap_key}/dismiss".into())), "{i:?}");
        assert!(i.contains(&("Absence".into(), "AbsenceInput".into(), "/absences".into())), "{i:?}");
    }

    #[test]
    fn x_source_marks_both_a_column_and_a_table() {
        let p = compare(SQL);
        assert_eq!(note(&p, "absences.synced_at"), Some("помечено x-source: синхронизация календаря"));
        assert_eq!(note(&p, "absences.source · обязательна"),
                   Some("помечено x-source: POST /absences пишет 'manual'"));
        assert_eq!(note(&p, "tenants · без записи"), Some("помечено x-source: импорт конфигурации"));
        assert!(note(&p, "monitors · без записи").is_some_and(|d| d.starts_with("таблица есть")));
    }

    #[test]
    fn x_stored_in_moves_the_comparison_to_the_named_column() {
        let p = compare(SQL);
        assert_eq!(note(&p, "Absence.from"), Some("помечено x-stored-in: absences.starts_at"));
        assert_eq!(note(&p, "absences.starts_at"), None, "колонка не засчитана семье");
        assert_eq!(note(&p, "absences.starts_at · обязательна"), None, "колонка не засчитана входу");
    }

    #[test]
    fn broken_goal_leaves_finding() {
        let p = compare(SQL);
        assert!(note(&p, "GapDismissal.at").is_some_and(|d| d.starts_with(
            "поле контракта без колонки: x-stored-in называет gap_dismissals.dismissed_at")));
    }

    #[test]
    fn parameter_path_and_201_without_body_counted() {
        let p = compare(SQL);
        assert_eq!(note(&p, "gap_dismissals.gap_key"), None);
        assert_eq!(note(&p, "gap_dismissals.gap_key · обязательна"), None);
        assert_eq!(note(&p, "gap_dismissals · без записи"), None);
    }

    #[test]
    fn foreign_key_to_segment_path_counted_only_only() {
        assert_eq!(note(&compare(SQL), "handovers.from_shift_id · обязательна"), None);
        let two = compare(&SQL.replace("to_shift_id text,", "to_shift_id text NOT NULL,"));
        assert!(note(&two, "handovers.from_shift_id · обязательна").is_some());
        assert!(note(&two, "handovers.to_shift_id · обязательна").is_some());
    }

    const RS: &str = r#"
/// Состояние агента.
/// x-derived: из last_seen_at
#[derive(Debug)]
pub enum AgentState {
    Online,
    Offline,
}

/// x-stored-in: monitors.condition
#[derive(Debug)]
pub enum Operator {
    Gt,
    Lt,
}

/// x-stored-in: monitors.name
#[derive(Debug)]
pub enum Combinator {
    All,
    Any,
}

#[derive(Debug)]
pub enum SyncState {
    Clean,
    Drifted,
    /// x-derived: строки нет
    NotVersioned,
}
"#;

    const CHECKS: &str = "
CREATE TABLE monitors (id text PRIMARY KEY, name text NOT NULL, condition jsonb NOT NULL);
CREATE TABLE config_repos (id text PRIMARY KEY, sync_state text NOT NULL CHECK (sync_state IN ('clean', 'drifted')));
";

    #[test]
    fn mark_above_enum_and_above_branch() {
        let e = enums_of(RS, "x.rs");
        assert_eq!(e[0].mark, Some(("x-derived".to_owned(), "из last_seen_at".to_owned())));
        assert_eq!(e[1].mark, Some(("x-stored-in".to_owned(), "monitors.condition".to_owned())));
        assert_eq!(e[3].stored, vec!["clean", "drifted"]);
        assert_eq!(e[3].unstored, vec!["not_versioned"]);
    }

    #[test]
    fn pair_and_unpaired_read_marks() {
        let p = pair(&enums_of(RS, "x.rs"), &checks_of(CHECKS, "0001.sql"), &[], &[], &schema_of(&[CHECKS.into()]));
        assert_eq!(note(&p, "AgentState"), Some("помечено x-derived: из last_seen_at"));
        assert_eq!(note(&p, "Operator"), Some("помечено x-stored-in: monitors.condition"));
        assert!(note(&p, "Combinator").is_some_and(|d| d.starts_with("перечисление без множества CHECK")
            && d.ends_with("x-stored-in называет monitors.name — jsonb-колонки нет")));
        assert_eq!(note(&p, "SyncState ↔ config_repos.sync_state"),
                   Some("сходятся, значений 2; вне хранения (x-derived): not_versioned"));
    }

    fn link(schema: &str) -> Value {
        json!({ "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{schema}") } } } })
    }

    #[test]
    fn creation_foreign_thing_table_not_writes_and_path_not_enters() {
        let sql = "
CREATE TABLE monitors (id text PRIMARY KEY, account_id text NOT NULL, name text NOT NULL, team_id text NOT NULL);
CREATE TABLE teams (id text PRIMARY KEY, name text NOT NULL);
CREATE TABLE webhooks (id text PRIMARY KEY, account_id text NOT NULL, user_id text NOT NULL);
";
        let doc = json!({
            "paths": {
                "/monitors": { "post": { "requestBody": link("MonitorInput"), "responses": { "201": link("Monitor") } } },
                "/teams/{team_id}/monitors/test-run": { "post": { "responses": { "201": link("MonitorTestRun") } } },
                "/users/{user_id}/webhook-test": { "post": { "responses": { "201": link("WebhookTestResult") } } }
            },
            "components": { "schemas": {
                "Monitor": { "properties": { "name": { "type": "string" } } },
                "MonitorInput": { "required": ["name"], "properties": { "name": { "type": "string" } } },
                "MonitorTestRun": { "properties": { "ok": { "type": "boolean" } } },
                "Team": { "properties": { "name": { "type": "string" } } },
                "Webhook": { "properties": { "user_id": { "type": "string" } } },
                "WebhookTestResult": { "properties": { "ok": { "type": "boolean" } } }
            } }
        });
        let p = contract_vs_schema(&doc, &schema_of(&[sql.into()]), &[]);
        assert!(note(&p, "monitors.team_id").is_some_and(|d| d.starts_with("колонка без входа")), "параметр чужого пути");
        assert!(note(&p, "monitors.team_id · обязательна").is_some(), "путь чужого создания");
        assert!(note(&p, "webhooks · без записи").is_some_and(|d| d.starts_with("таблица есть")), "201 чужой вещи без тела");
    }

    #[test]
    fn foreign_key_to_parent_not_counted_when_brace_parent_own_column() {
        let sql = "
CREATE TABLE accounts (id text PRIMARY KEY);
CREATE TABLE services (id text PRIMARY KEY, account_id text NOT NULL);
CREATE TABLE billing_profiles (
  id text PRIMARY KEY,
  account_id text NOT NULL REFERENCES accounts(id),
  payer_account_id text NOT NULL REFERENCES accounts(id),
  service_id text NOT NULL REFERENCES services(id),
  plan text NOT NULL
);
";
        let doc = json!({
            "paths": { "/accounts/{account_id}/billing-profiles": {
                "post": { "requestBody": link("BillingProfileInput"), "responses": { "201": link("BillingProfile") } }
            } },
            "components": { "schemas": {
                "BillingProfile": { "properties": {
                    "plan": { "type": "string" }, "payer_account_id": { "type": "string" }, "service_id": { "type": "string" }
                } },
                "BillingProfileInput": { "required": ["plan"], "properties": { "plan": { "type": "string" } } }
            } }
        });
        let p = contract_vs_schema(&doc, &schema_of(&[sql.into()]), &[]);
        assert!(note(&p, "billing_profiles.payer_account_id · обязательна").is_some(), "скобка родителя названа своей колонкой");
        assert!(note(&p, "billing_profiles.service_id · обязательна").is_some(), "таблицы ключа нет в пути");
        let camel = serde_json::to_string(&doc).expect("контракт").replace("{account_id}", "{accountId}");
        let p = contract_vs_schema(&serde_json::from_str(&camel).expect("контракт"), &schema_of(&[sql.into()]), &[]);
        assert!(note(&p, "billing_profiles.payer_account_id · обязательна").is_some(), "второй ключ к родителю при скобке не колонкой");
        let runs = "
CREATE TABLE monitors (id text PRIMARY KEY, name text NOT NULL);
CREATE TABLE monitor_runs (id text PRIMARY KEY, monitor_id text NOT NULL REFERENCES monitors(id), note text);
";
        let doc = json!({
            "paths": { "/monitors/{id}/runs": {
                "post": { "requestBody": link("MonitorRunInput"), "responses": { "201": link("MonitorRun") } }
            } },
            "components": { "schemas": {
                "MonitorRun": { "properties": { "note": { "type": "string" } } },
                "MonitorRunInput": { "properties": { "note": { "type": "string" } } }
            } }
        });
        let p = contract_vs_schema(&doc, &schema_of(&[runs.into()]), &[]);
        assert_eq!(note(&p, "monitor_runs.monitor_id · обязательна"), None, "скобка {{id}} — первичный ключ, а не колонка-ключ");
    }

    #[test]
    fn input_gives_only_brace_immediately_behind_segment_table() {
        let sql = "
CREATE TABLE changes (id text PRIMARY KEY, environment_id text NOT NULL);
CREATE TABLE teams (id text PRIMARY KEY, name text NOT NULL);
CREATE TABLE monitors (id text PRIMARY KEY, name text NOT NULL, team_id text NOT NULL);
CREATE TABLE postmortems (id text PRIMARY KEY, title text NOT NULL);
CREATE TABLE postmortem_actions (id text PRIMARY KEY, postmortem_id text NOT NULL, note text NOT NULL);
";
        let get = |schema: &str, params: Value| json!({ "get": { "parameters": params, "responses": { "200": link(schema) } } });
        let doc = json!({
            "paths": {
                "/changes": get("Change", json!([{ "name": "environment_id", "in": "query" }])),
                "/teams/{team_id}/monitors/test-run": get("Monitor", json!([{ "name": "team_id", "in": "path" }])),
                "/postmortems/{postmortem_id}/actions": get("PostmortemAction", json!([{ "name": "postmortem_id", "in": "path" }]))
            },
            "components": { "schemas": {
                "Change": { "properties": { "id": { "type": "string" } } },
                "Monitor": { "properties": { "name": { "type": "string" } } },
                "PostmortemAction": { "properties": { "note": { "type": "string" } } }
            } }
        });
        let p = contract_vs_schema(&doc, &schema_of(&[sql.into()]), &[]);
        assert!(note(&p, "changes.environment_id").is_some_and(|d| d.starts_with("колонка без входа")), "параметр запроса");
        assert!(note(&p, "monitors.team_id").is_some_and(|d| d.starts_with("колонка без входа")), "скобка чужого отрезка");
        assert_eq!(note(&p, "postmortem_actions.postmortem_id"), None, "скобка родителя дочерней таблице");
    }

    #[test]
    fn plural_from_y_happens_and_on_s() {
        let t = schema_of(&["CREATE TABLE api_keys (id text PRIMARY KEY);\nCREATE TABLE policies (id text PRIMARY KEY);".into()]);
        assert_eq!(schema_table(&["ApiKey".into(), "Policy".into()], &t, &[]),
                   vec![("ApiKey".to_owned(), "api_keys".to_owned()), ("Policy".to_owned(), "policies".to_owned())]);
    }

    #[test]
    fn mark_columns_and_table_lives_before_removal() {
        let base = "CREATE TABLE sessions (id text PRIMARY KEY, jti text NOT NULL, state jsonb NOT NULL);";
        let marked = "COMMENT ON TABLE sessions IS 'x-source: подписант';\nCOMMENT ON COLUMN sessions.jti IS 'x-source: подписант';";
        let source = |texts: &[&str]| {
            let t = schema_of(&texts.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>());
            let s = t.iter().find(|x| x.name == "sessions").expect("таблица");
            (s.source.clone(), s.cols.iter().find(|c| c.name == "jti").and_then(|c| c.source.clone()))
        };
        let both = (Some("подписант".to_owned()), Some("подписант".to_owned()));
        assert_eq!(source(&[base, marked]), both);
        assert_eq!(source(&[base, marked, "COMMENT ON COLUMN sessions.jti IS NULL;"]).1, None, "IS NULL");
        let readded = format!("{marked}\nALTER TABLE sessions DROP COLUMN jti;\nALTER TABLE sessions ADD COLUMN jti text NOT NULL;");
        assert_eq!(source(&[base, &readded]).1, None, "колонка снята и заведена заново");
        assert_eq!(source(&[base, marked, &format!("DROP TABLE IF EXISTS sessions CASCADE;\n{base}")]), (None, None),
                   "таблица снята и заведена заново");
        let commented = "/*\nCOMMENT ON COLUMN sessions.jti IS 'x-source: подписант';\n*/\n-- COMMENT ON TABLE sessions IS 'x-source: подписант';";
        assert_eq!(source(&[base, commented]), (None, None), "закомментированное");
        let dashes = "COMMENT ON COLUMN sessions.state IS 'см. --help';\nCOMMENT ON TABLE sessions IS 'x-source: подписант';\nCOMMENT ON COLUMN sessions.jti IS 'x-source: подписант';";
        assert_eq!(source(&[base, dashes]), both, "-- внутри строки");
    }

    #[test]
    fn change_type_moves_out_column_from_jsonb() {
        let rs = "/// x-stored-in: sessions.state\n#[derive(Debug)]\npub enum Kind {\n    A,\n    B,\n}\n";
        let create = "CREATE TABLE sessions (id text PRIMARY KEY, state jsonb NOT NULL);";
        let kind = |texts: &[&str]| {
            let t = schema_of(&texts.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>());
            pair(&enums_of(rs, "k.rs"), &[], &[], &[], &t).into_iter().find(|p| p.name == "Kind").map(|p| p.detail)
        };
        assert_eq!(kind(&[create]).as_deref(), Some("помечено x-stored-in: sessions.state"));
        for alter in ["ALTER TABLE sessions ALTER COLUMN state TYPE text USING state::text;",
                      "ALTER TABLE sessions ALTER state SET DATA TYPE text;"] {
            assert!(kind(&[create, alter]).is_some_and(|d| d.ends_with("jsonb-колонки нет")), "{alter}");
        }
    }

    #[test]
    fn mention_marks_at_prose_mark_not_becomes() {
        let rs = "
/// Состояние. В отличие от поля контракта с `x-derived: …`, хранится в jobs.state.
#[derive(Debug)]
pub enum JobState {
    Queued,
    Done,
}

#[derive(Debug)]
pub enum Other {
    /// Не как `x-derived: foo` — эта хранится
    A,
    B,
}
";
        let e = enums_of(rs, "j.rs");
        assert_eq!(e[0].mark, None);
        assert_eq!((e[1].stored.len(), e[1].unstored.len()), (2, 0));
        let t = schema_of(&["CREATE TABLE sessions (id text PRIMARY KEY, jti text NOT NULL);".into(),
                            "COMMENT ON COLUMN sessions.jti IS 'заполняет x-source: подписант';".into()]);
        assert_eq!(t[0].cols[1].source, None);
    }

    #[test]
    fn x_stored_in_counts_only_in_the_schema_of_its_own_table() {
        let sql = "
CREATE TABLE monitors (id text PRIMARY KEY, account_id text NOT NULL, name text NOT NULL, secret_hash text);
CREATE TABLE accounts (id text PRIMARY KEY, slug text NOT NULL);
";
        let doc = json!({
            "paths": { "/monitors": { "post": { "requestBody": link("MonitorInput"), "responses": { "201": link("Monitor") } } } },
            "components": { "schemas": {
                "Monitor": { "properties": { "name": { "type": "string" },
                                             "runbook": { "type": "string", "x-stored-in": "accounts.slug" } } },
                "MonitorInput": { "required": ["name"], "properties": { "name": { "type": "string" } } },
                "LegacyExportRow": { "properties": { "whatever": { "type": "string", "x-stored-in": "monitors.secret_hash" } } }
            } }
        });
        let p = contract_vs_schema(&doc, &schema_of(&[sql.into()]), &[]);
        assert!(note(&p, "Monitor.runbook").is_some_and(|d| d.starts_with("поле контракта без колонки")), "чужая таблица");
        assert!(note(&p, "monitors.secret_hash").is_some_and(|d| d.starts_with("колонка без входа")), "схема без таблицы");
        assert_eq!(note(&compare(SQL), "AbsenceInput.from"), Some("помечено x-stored-in: absences.starts_at"));
    }

    #[test]
    fn branch_outside_storage_not_moves_out_enum_from_rules() {
        let rs = "#[derive(Debug)]\npub enum Plan {\n    Free,\n    /// x-derived: считается\n    Paid,\n}\n";
        assert_eq!(note(&pair(&enums_of(rs, "p.rs"), &[], &[], &[], &[]), "Plan"),
                   Some("перечисление без множества CHECK (p.rs), значений 1; вне хранения (x-derived): paid"));
    }

    #[test]
    fn foreign_key_with_schema_and_named_constraint() {
        let sql = "
CREATE TABLE swaps (
  id text PRIMARY KEY,
  offered_shift_id text NOT NULL REFERENCES public.shifts (id),
  taken_shift_id text,
  CONSTRAINT \"swaps_taken_fk\" FOREIGN KEY (taken_shift_id) REFERENCES public.shifts (id)
);
";
        let t = schema_of(&[sql.into()]);
        let refs: Vec<(&str, Option<&str>)> = t[0].cols.iter().map(|c| (c.name.as_str(), c.references.as_deref())).collect();
        assert_eq!(refs, vec![("id", None), ("offered_shift_id", Some("shifts")), ("taken_shift_id", Some("shifts"))]);
    }
}
