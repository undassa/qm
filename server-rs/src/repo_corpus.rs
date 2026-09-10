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
pub struct Enum {
    pub name: String,
    pub file: String,
    /// Значения, которые доходят до базы. Ветка под `#[serde(other)]` сюда не
    /// входит: она ловит чужой ввод, а не хранится.
    pub stored: Vec<String>,
    pub fallback: Option<String>,
    /// Годно к хранению: либо все ветки без полей, либо помечено `tag =`.
    pub storable: bool,
}

/// Множество `CHECK (col IN (…))` схемы.
pub struct CheckSet {
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

/// Перечисления домена из текста одного файла.
pub fn enums_of(text: &str, file: &str) -> Vec<Enum> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let t = lines[i].trim();
        if !(t.starts_with("pub enum ") && t.ends_with('{')) {
            i += 1;
            continue;
        }
        let name = t["pub enum ".len()..].trim_end_matches('{').trim().to_owned();
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
        let mut stored = Vec::new();
        let mut fallback = None;
        let mut all_unit = true;
        let mut pending = String::new();
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
                } else if !t.starts_with("//") && !t.is_empty() {
                    let head: String =
                        t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if head.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                        if pending.contains("other)") || pending.contains("other )") {
                            fallback = Some(snake(&head));
                        } else {
                            stored.push(snake(&head));
                        }
                        let rest = t[head.len()..].trim_start();
                        if rest.starts_with('(') || rest.starts_with('{') {
                            all_unit = false;
                        }
                        pending.clear();
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
            fallback,
            storable: tagged || all_unit,
        });
        i = j.max(i + 1);
    }
    out
}

/// Множества `CHECK` из текста одной миграции. Комментарии снимаются: `--`
/// внутри строки в кавычках здесь не встречается, а вне её съел бы половину
/// определения.
pub fn checks_of(text: &str, file: &str) -> Vec<CheckSet> {
    let no_comments: String = text
        .split('\n')
        .map(|l| match l.find("--") {
            Some(p) => &l[..p],
            None => l,
        })
        .collect::<Vec<&str>>()
        .join("\n");
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
pub struct Pair {
    pub name: String,
    pub detail: String,
}

pub fn pair(enums: &[Enum], checks: &[CheckSet], forced: &[(&str, &str)]) -> Vec<Pair> {
    let inter = |a: &[String], b: &[String]| -> Vec<String> {
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
            let half = (e.stored.len().max(s.values.len()) + 1) / 2;
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
        let detail = if only_domain.is_empty() && only_schema.is_empty() {
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
        if !e.storable || paired_enum.contains(&e.name.as_str()) || e.stored.len() < 2 {
            continue;
        }
        out.push(Pair {
            name: e.name.clone(),
            detail: format!("перечисление без множества CHECK ({}), значений {}",
                            e.file, e.stored.len()),
        });
    }
    for s in checks {
        if paired_set.contains(&s.at.as_str()) {
            continue;
        }
        out.push(Pair {
            name: s.at.clone(),
            detail: format!("множество CHECK без перечисления домена ({})", s.file),
        });
    }
    out
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
        let p = pair(&e, &c, &[]);
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
pub struct Column {
    pub name: String,
    pub not_null: bool,
    pub has_default: bool,
    /// Первичный ключ пуст не бывает и без `NOT NULL`: обещание контракта он
    /// держит устройством, а не словом.
    pub primary_key: bool,
}

pub struct Table {
    pub name: String,
    pub cols: Vec<Column>,
}

/// Поле схемы контракта: где стоит, как зовётся, чем помечено.
pub struct Field {
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
}

/// Колонки из текста миграции. Тот же разбор тела, что у множеств `CHECK`.
pub fn tables_of(text: &str) -> Vec<Table> {
    let no_comments: String = text
        .split('\n')
        .map(|l| match l.find("--") {
            Some(p) => &l[..p],
            None => l,
        })
        .collect::<Vec<&str>>()
        .join("\n");
    let table = regex::Regex::new(r"(?i)CREATE TABLE (?:IF NOT EXISTS )?([a-z][a-z0-9_]*)\s*\(")
        .expect("образец таблицы");
    let col = regex::Regex::new(
        r"(?i)^([a-z][a-z0-9_]*) (text|integer|bigint|bigserial|serial|smallint|boolean|jsonb|json|uuid|date|numeric|timestamptz|timestamp|timetz|time|interval|inet|bytea|real|double)\b(.*)$",
    )
    .expect("образец колонки");
    let b: Vec<char> = no_comments.chars().collect();
    let mut out = Vec::new();
    for m in table.captures_iter(&no_comments) {
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
        let mut cols = Vec::new();
        for def in &defs {
            let t: String = def.split_whitespace().collect::<Vec<&str>>().join(" ");
            if let Some(c) = col.captures(&t) {
                cols.push(Column {
                    name: c[1].to_owned(),
                    not_null: c[3].to_uppercase().contains("NOT NULL"),
                    has_default: c[3].to_uppercase().contains("DEFAULT"),
                    primary_key: c[3].to_uppercase().contains("PRIMARY KEY"),
                });
            }
        }
        out.push(Table { name: m[1].to_owned(), cols });
    }
    out
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
pub fn schema_of(texts: &[String]) -> Vec<Table> {
    let mut out: Vec<Table> = Vec::new();
    for text in texts {
        for t in tables_of(text) {
            match out.iter_mut().find(|x| x.name == t.name) {
                Some(had) => {
                    for c in t.cols {
                        if !had.cols.iter().any(|x| x.name == c.name) {
                            had.cols.push(c);
                        }
                    }
                }
                None => out.push(t),
            }
        }
        for (table, add, drop) in alters_of(text) {
            let Some(had) = out.iter_mut().find(|x| x.name == table) else { continue };
            if let Some(c) = add {
                match had.cols.iter_mut().find(|x| x.name == c.name) {
                    Some(x) => *x = c,
                    None => had.cols.push(c),
                }
            }
            if let Some(name) = drop {
                had.cols.retain(|x| x.name != name);
            }
        }
    }
    out
}

/// Правки таблицы: `(таблица, добавленная колонка, снятое имя)`.
///
/// Одна команда `ALTER` несёт несколько действий через запятую, и каждое —
/// своя строка ответа.
pub fn alters_of(text: &str) -> Vec<(String, Option<Column>, Option<String>)> {
    let no_comments: String = text
        .split('\n')
        .map(|l| match l.find("--") {
            Some(p) => &l[..p],
            None => l,
        })
        .collect::<Vec<&str>>()
        .join("\n");
    let head = regex::Regex::new(r"(?is)ALTER TABLE\s+(?:IF EXISTS\s+)?(?:ONLY\s+)?([a-z][a-z0-9_]*)\s+([^;]*);")
        .expect("образец правки таблицы");
    let add = regex::Regex::new(
        r"(?i)^ADD (?:COLUMN )?(?:IF NOT EXISTS )?([a-z][a-z0-9_]*) (text|integer|bigint|bigserial|serial|smallint|boolean|jsonb|json|uuid|date|numeric|timestamptz|timestamp|timetz|time|interval|inet|bytea|real|double)\b(.*)$",
    )
    .expect("образец добавленной колонки");
    let drop = regex::Regex::new(r"(?i)^DROP (?:COLUMN )?(?:IF EXISTS )?([a-z][a-z0-9_]*)")
        .expect("образец снятой колонки");
    let mut out = Vec::new();
    for m in head.captures_iter(&no_comments) {
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
                out.push((
                    table.clone(),
                    Some(Column {
                        name: c[1].to_owned(),
                        not_null: c[3].to_uppercase().contains("NOT NULL"),
                        has_default: c[3].to_uppercase().contains("DEFAULT"),
                        primary_key: c[3].to_uppercase().contains("PRIMARY KEY"),
                    }),
                    None,
                ));
            } else if let Some(c) = drop.captures(&t) {
                // `DROP CONSTRAINT` — не колонка, и снимать по нему нечего.
                if !t.to_uppercase().starts_with("DROP CONSTRAINT") {
                    out.push((table.clone(), None, Some(c[1].to_owned())));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod правки {
    use super::schema_of;

    const CREATE: &str = "CREATE TABLE signals (id text NOT NULL, at timestamptz);";

    #[test]
    fn колонка_приехавшая_alter_ом_существует() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE signals ADD COLUMN service_id text NOT NULL;".into()]);
        let cols: Vec<&str> = t[0].cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, vec!["id", "at", "service_id"], "{cols:?}");
        assert!(t[0].cols[2].not_null, "NOT NULL потерян");
    }

    #[test]
    fn if_not_exists_и_без_слова_column() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE IF EXISTS signals ADD IF NOT EXISTS note text;".into()]);
        assert!(t[0].cols.iter().any(|c| c.name == "note"), "{:?}",
                t[0].cols.iter().map(|c| &c.name).collect::<Vec<_>>());
    }

    #[test]
    fn снятая_колонка_уходит() {
        let t = schema_of(&[CREATE.into(), "ALTER TABLE signals DROP COLUMN at;".into()]);
        assert!(!t[0].cols.iter().any(|c| c.name == "at"), "снятая колонка осталась");
    }

    #[test]
    fn несколько_действий_одной_командой() {
        let t = schema_of(&[CREATE.into(),
            "ALTER TABLE signals ADD COLUMN a text DEFAULT 'x', ADD COLUMN b integer, DROP COLUMN at;".into()]);
        let cols: Vec<&str> = t[0].cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, vec!["id", "a", "b"], "{cols:?}");
    }

    #[test]
    fn ограничение_снятое_alter_ом_колонку_не_трогает() {
        let t = schema_of(&[CREATE.into(),
                            "ALTER TABLE signals DROP CONSTRAINT signals_at_check;".into()]);
        assert!(t[0].cols.iter().any(|c| c.name == "at"), "ограничение съело колонку");
    }

    #[test]
    fn правка_неизвестной_таблицы_ничего_не_заводит() {
        let t = schema_of(&[CREATE.into(), "ALTER TABLE чужая ADD COLUMN x text;".into()]);
        assert_eq!(t.len(), 1, "завелась таблица, которой не создавали");
    }
}

/// Имена параметров адреса — и объявленных на месте, и объявленных ССЫЛКОЙ.
///
/// Обходчик брал только узлы с полем `in`, а `$ref` его не имеет:
/// `- $ref: "#/components/parameters/PostmortemId"` — тот же параметр, но для
/// датчика его не существовало. Отсюда `postmortem_actions.postmortem_id` и ещё
/// четыре родительских ключа выглядели колонками `NOT NULL`, которые нечем
/// заполнить, — хотя контракт объявляет их прямо.
///
/// Ссылка на СХЕМУ сюда не попадает: у неё нет двойника в
/// `components/parameters`, и поиск возвращает пусто.
pub fn path_params(item: &Value, doc: &Value) -> Vec<String> {
    fn walk(node: &Value, doc: &Value, out: &mut Vec<String>) {
        match node {
            Value::Array(a) => a.iter().for_each(|x| walk(x, doc, out)),
            Value::Object(o) => {
                if o.get("in").is_some() {
                    if let Some(n) = o.get("name").and_then(|n| n.as_str()) {
                        out.push(n.to_owned());
                    }
                }
                if let Some(r) = o.get("$ref").and_then(|r| r.as_str()) {
                    if let Some(key) = r.rsplit('/').next() {
                        if let Some(n) = doc
                            .get("components")
                            .and_then(|c| c.get("parameters"))
                            .and_then(|p| p.get(key))
                            .and_then(|p| p.get("name"))
                            .and_then(|n| n.as_str())
                        {
                            out.push(n.to_owned());
                        }
                    }
                }
                o.values().for_each(|v| walk(v, doc, out));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(item, doc, &mut out);
    out
}

#[cfg(test)]
mod параметры {
    use super::path_params;
    use serde_json::json;

    fn контракт() -> serde_json::Value {
        json!({ "components": { "parameters": {
            "PostmortemId": { "name": "postmortem_id", "in": "path" } } } })
    }

    #[test]
    fn объявленный_на_месте_виден() {
        let item = json!({ "parameters": [{ "name": "block_id", "in": "path" }] });
        assert_eq!(path_params(&item, &контракт()), vec!["block_id"]);
    }

    #[test]
    fn объявленный_ссылкой_виден() {
        let item = json!({ "parameters": [{ "$ref": "#/components/parameters/PostmortemId" }] });
        assert_eq!(path_params(&item, &контракт()), vec!["postmortem_id"]);
    }

    #[test]
    fn ссылка_на_схему_параметром_не_становится() {
        let item = json!({ "post": { "requestBody": { "content": { "application/json": {
            "schema": { "$ref": "#/components/schemas/PostmortemBlockInput" } } } } } });
        assert!(path_params(&item, &контракт()).is_empty());
    }

    #[test]
    fn неизвестная_ссылка_ничего_не_даёт() {
        let item = json!({ "parameters": [{ "$ref": "#/components/parameters/НетТакого" }] });
        assert!(path_params(&item, &контракт()).is_empty());
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
pub fn segment_owns(segment: &str, table: &str) -> bool {
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
mod отрезок {
    use super::segment_owns;

    #[test]
    fn своя_таблица() {
        assert!(segment_owns("postmortems", "postmortems"));
    }

    #[test]
    fn дочерняя_коллекция_получает_родительский_ключ() {
        assert!(segment_owns("postmortems", "postmortem_actions"));
        assert!(segment_owns("postmortems", "postmortem_blocks"));
    }

    #[test]
    fn чужая_таблица_с_общей_приставкой_не_получает() {
        // `posts` и `postmortems` — разные вещи, и подчёркивание их разводит.
        assert!(!segment_owns("posts", "postmortems"));
    }

    #[test]
    fn соседняя_таблица_не_получает() {
        assert!(!segment_owns("postmortems", "monitors"));
    }

    #[test]
    fn множественное_на_ies_даёт_основу_на_y() {
        // `/policies/{policy_id}/versions` — ключ нужен `policy_versions`.
        assert!(segment_owns("policies", "policy_versions"));
        assert!(segment_owns("policies", "policy_test_runs"));
    }

    #[test]
    fn короткая_основа_не_раздаёт() {
        assert!(!segment_owns("as", "a_b"));
    }
}

fn snake_name(s: &str) -> String {
    snake(s)
}

/// Какой таблице отвечает схема контракта: имя в змеином регистре, оно же во
/// множественном числе. Пара, которую так не угадать, называется явно.
pub fn schema_table(schemas: &[String], tables: &[Table], forced: &[(&str, &str)]) -> Vec<(String, String)> {
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
            vec![format!("{}ies", &b[..b.len() - 1])]
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
pub fn contract_fields(doc: &Value) -> Vec<Field> {
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
pub fn contract_names(doc: &Value) -> Vec<String> {
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
pub fn contract_vs_schema(
    doc: &Value,
    tables: &[Table],
    forced: &[(&str, &str)],
) -> Vec<Pair> {
    let writes = write_schemas(doc);
    let targets = write_targets(doc);
    let inputs = inputs_of_table(doc);
    let _ = &targets;
    const SYSTEM: [&str; 5] = ["id", "account_id", "created_at", "updated_at", "rev"];
    let schemas: Vec<String> = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let map = schema_table(&schemas, tables, forced);
    let fields = contract_fields(doc);
    let names = contract_names(doc);
    // Параметры операций, разложенные по таблице: адрес `/monitors/{id}` кладёт
    // свои параметры таблице `monitors`. Колонка, названная параметром, вход
    // имеет — просто пришла она не телом, а адресом.
    let mut params_of: Vec<(String, String)> = Vec::new();
    if let Some(paths) = doc.get("paths").and_then(|p| p.as_object()) {
        for (path, ops) in paths {
            let here = path_params(ops, doc);
            for seg in path.split('/') {
                let t = seg.trim_matches(|c| c == '{' || c == '}');
                // Отрезок обязан НАЗЫВАТЬ таблицу: иначе всякое слово в адресе
                // раздавало бы входы по совпадению приставки.
                if !tables.iter().any(|x| x.name == t) {
                    continue;
                }
                for tab in tables.iter().filter(|x| segment_owns(t, &x.name)) {
                    for n in &here {
                        params_of.push((tab.name.clone(), n.clone()));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();

    // Семья схем одной таблицы: `Monitor`, `MonitorCreate`, `MonitorPatch` пишут
    // в одну и ту же. Колонка, названная любой из них, названа.
    let family = |table: &str| -> Vec<String> {
        let owners: Vec<&String> = map.iter().filter(|(_, t)| t == table).map(|(s, _)| s).collect();
        fields
            .iter()
            .filter(|f| owners.iter().any(|o| f.schema == **o || f.schema.starts_with(o.as_str())))
            .map(|f| f.name.clone())
            .collect()
    };

    for f in &fields {
        let Some((_, table)) = map.iter().find(|(s, _)| *s == f.schema) else { continue };
        if !f.marks.is_empty() || f.composite {
            continue;
        }
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
                detail: if elsewhere {
                    format!("колонка без входа: имя есть в контракте, но в ЧУЖОЙ схеме — \
                             положить в {} нечем", t.name)
                } else {
                    format!("колонка без входа: ни свойства, ни параметра с именем {}", c.name)
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
        let owners: Vec<&String> = map.iter().filter(|(_, tt)| tt == table).map(|(s, _)| s).collect();
        // Схемы записи ЭТОЙ таблицы: те, что пишут (тело запроса) И сами
        // отображаются на неё. Адрес сюда не годится: `POST /forecasts/{id}/tasks`
        // трогает два отрезка пути, а пишет в один.
        // Входы ЭТОЙ таблицы: тела запросов у операций, чей ответ называет
        // вещь этой таблицы. Имя входной схемы таблицей не зовётся, и искать
        // по нему — то же, что искать по фамилии соседа.
        // Схема ответа сводится к САМОМУ ДЛИННОМУ известному имени-приставке:
        // `MonitorRunStep` принадлежит `MonitorRun`, а не `Monitor`. Короткая
        // приставка затянула бы чужую таблицу, и колонка выглядела бы с входом.
        let owner_of = |name: &str| -> Option<&String> {
            map.iter()
                .filter(|(sc, _)| name == sc || name.starts_with(sc.as_str()))
                .max_by_key(|(sc, _)| sc.len())
                .map(|(_, t)| t)
        };
        let here: Vec<&String> = inputs
            .iter()
            .filter(|(created, _)| owner_of(created) == Some(table))
            .map(|(_, input)| input)
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
                    detail: format!("таблица есть, а контракт в неё ничем не пишет: {} колонок",
                                    cols.len()),
                });
            }
            continue;
        }
        let required_here: Vec<&str> = fields
            .iter()
            .filter(|f| f.required && here.iter().any(|s| &&f.schema == s))
            .map(|f| f.name.as_str())
            .collect();
        for c in &t.cols {
            if SYSTEM.contains(&c.name.as_str()) || !c.not_null || c.has_default || c.primary_key {
                continue;
            }
            if required_here.contains(&c.name.as_str()) {
                continue;
            }
            // Имя несёт РОД находки: у факта ключ — имя, и две разные находки
            // об одной колонке вытесняли одна другую молча. Счёт «колонка без
            // входа» упал с шестидесяти пяти до двадцати семи, и выглядело это
            // починкой.
            out.push(Pair {
                name: format!("{}.{} · обязательна", t.name, c.name),
                detail: "колонка обязательна, а обязательного входа нет: NOT NULL без умолчания, \
                         и ни одна схема семьи её не требует"
                    .to_owned(),
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
        let Some(c) = t.cols.iter().find(|c| c.name == f.name) else { continue };
        if c.not_null || c.primary_key {
            continue;
        }
        if !seen.insert(format!("{} :: {}", f.at, t.name)) {
            continue;
        }
        out.push(Pair {
            name: f.at.clone(),
            detail: format!("обязательность разошлась: поле в required и не обнуляемо,                              а колонка {}.{} допускает пустое", t.name, c.name),
        });
    }
    out
}

/// Требование → операции контракта. Разметка живёт в `description` операции,
/// и читается она построчно по отступу, а не разбором дерева: ветка
/// `components` называет те же `FR` в описаниях СХЕМ, и они не операции.
///
/// Строка-комментарий не принадлежит операции. Заголовок вида
/// `# ── срез статуса (FR-STP-01…14) ──` стоит ПОСЛЕ последней операции блока,
/// и без этого правила разметка соседнего среза приписывалась предыдущей
/// операции: `DELETE /devices/{deviceId}` оказывался операцией статус-страницы.
pub fn requirement_ops(text: &str) -> Vec<Pair> {
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
        for name in crate::ids::expand(line) {
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
pub fn requirements_in_body(text: &str) -> Vec<Pair> {
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
pub fn contract_marks(doc: &Value) -> Vec<Pair> {
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
pub fn contract_enums(doc: &Value) -> Vec<(String, Vec<String>)> {
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
pub fn check_values_without_path(
    enums: &[Enum],
    checks: &[CheckSet],
    contract: &[(String, Vec<String>)],
    forced: &[(&str, &str)],
) -> Vec<Pair> {
    // Пара «перечисление ↔ множество» уже считается: берём её же, чтобы два
    // правила не расходились в том, что с чем спарено.
    let paired = pair(enums, checks, forced);
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
pub fn inputs_of_table(doc: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
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
    for (_, ops) in paths {
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
            for c in &created {
                for i in &input {
                    out.push((c.clone(), i.clone()));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn write_targets(doc: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
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
            let Some(body) = op.get("requestBody") else { continue };
            let mut names = Vec::new();
            refs(body, &mut names);
            for seg in path.split('/') {
                let t = seg.trim_matches(|c| c == '{' || c == '}');
                if t.is_empty() {
                    continue;
                }
                for n in &names {
                    out.push((t.to_owned(), n.clone()));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Схемы, которыми в таблицу ПИШУТ: тело запроса у `post`, `put`, `patch`.
/// Читающая схема ничего не требует по природе, и считать её входом значит
/// требовать обязательности от того, что только показывают.
pub fn write_schemas(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
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
    for (_, ops) in paths {
        let Some(o) = ops.as_object() else { continue };
        for (method, op) in o {
            if !matches!(method.as_str(), "post" | "put" | "patch") {
                continue;
            }
            if let Some(body) = op.get("requestBody") {
                refs(body, &mut out);
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
pub fn contract_sizes(doc: &Value) -> Vec<(String, usize)> {
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
pub fn contract_requirement_split(text: &str) -> (usize, usize) {
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
pub fn contract_head_total(text: &str) -> Vec<Pair> {
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
pub fn contract_head_echo(text: &str, in_body: usize, border: usize, sizes: &[(String, usize)]) -> Vec<Pair> {
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

pub fn contract_head(text: &str, doc: &Value) -> Vec<Pair> {
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
pub fn corpus_reach(
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
