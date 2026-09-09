//! Разбор подмножества YAML: ровно того, на котором написан контракт.
//!
//! Своего разборщика здесь не было бы, будь в дереве зависимостей чужой. Но
//! брать зависимость ради одного файла дороже, чем сто пятьдесят строк: у
//! чужой свои правила разбора, и расхождение с ними прочиталось бы как
//! расхождение контракта со схемой — находка на пустом месте.
//!
//! Проверяется он двенадцатью фикстурами, и каждая закрывает случай, на
//! котором прежний разбор ошибался. Самый дорогой из них — ключ с двоеточием
//! ВНУТРИ (`/signals:ingest`): на нём разбор терял всю ветку `paths` и отдавал
//! правилам пустой корпус, то есть показывал зелёное там, где не смотрел.

use serde_json::{Map, Value};

/// Скаляр: число, `true`, `null`, строка в кавычках или голая.
fn scalar(s: &str) -> Value {
    let s = s.trim();
    if s.is_empty() || s == "null" || s == "~" {
        return Value::Null;
    }
    if s == "true" {
        return Value::Bool(true);
    }
    if s == "false" {
        return Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return Value::from(n);
    }
    if s.contains('.') {
        if let Ok(n) = s.parse::<f64>() {
            return Value::from(n);
        }
    }
    let b: Vec<char> = s.chars().collect();
    if b.len() > 1
        && ((b[0] == '"' && b[b.len() - 1] == '"') || (b[0] == '\'' && b[b.len() - 1] == '\''))
    {
        return Value::String(b[1..b.len() - 1].iter().collect());
    }
    Value::String(s.to_owned())
}

/// Ключ и остаток строки. Ключ кончается на ПЕРВОМ `:` со следующим пробелом —
/// иначе `description: "FR-SIT-01: ровно три"` развалилось бы по второму.
fn split_kv(body: &str) -> Option<(String, String)> {
    let b: Vec<char> = body.chars().collect();
    if let Some(q) = b.first().filter(|c| **c == '"' || **c == '\'') {
        if let Some(end) = b.iter().skip(1).position(|c| c == q) {
            let close = end + 1;
            if b.get(close + 1) == Some(&':') {
                let rest: String = b[close + 2..].iter().collect();
                return Some((b[1..close].iter().collect(), rest.trim().to_owned()));
            }
        }
    }
    let mut at = None;
    for (i, c) in b.iter().enumerate() {
        if *c == ':' && b.get(i + 1).map(|n| n.is_whitespace()).unwrap_or(false) {
            at = Some(i);
            break;
        }
    }
    if let Some(i) = at {
        return Some((
            b[..i].iter().collect::<String>().trim().to_owned(),
            b[i + 1..].iter().collect::<String>().trim().to_owned(),
        ));
    }
    if body.ends_with(':') {
        return Some((body[..body.len() - 1].trim().to_owned(), String::new()));
    }
    None
}

/// Поточная запись: `{ a: 1 }` и `[a, b]`.
fn flow(s: &str) -> Value {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    fn ws(b: &[char], i: &mut usize) {
        while *i < b.len() && b[*i].is_whitespace() {
            *i += 1;
        }
    }
    fn quoted(b: &[char], i: &mut usize, q: char) -> String {
        *i += 1;
        let mut out = String::new();
        while *i < b.len() && b[*i] != q {
            out.push(b[*i]);
            *i += 1;
        }
        *i += 1;
        out
    }
    fn key(b: &[char], i: &mut usize) -> String {
        ws(b, i);
        if *i < b.len() && (b[*i] == '"' || b[*i] == '\'') {
            return quoted(b, i, b[*i]);
        }
        let mut out = String::new();
        while *i < b.len() && b[*i] != ':' && b[*i] != ',' && b[*i] != '}' && b[*i] != ']' {
            out.push(b[*i]);
            *i += 1;
        }
        out.trim().to_owned()
    }
    fn val(b: &[char], i: &mut usize) -> Value {
        ws(b, i);
        if *i >= b.len() {
            return Value::Null;
        }
        if b[*i] == '{' {
            *i += 1;
            let mut o = Map::new();
            ws(b, i);
            if b.get(*i) == Some(&'}') {
                *i += 1;
                return Value::Object(o);
            }
            loop {
                let k = key(b, i);
                ws(b, i);
                if b.get(*i) == Some(&':') {
                    *i += 1;
                }
                o.insert(k, val(b, i));
                ws(b, i);
                if b.get(*i) == Some(&',') {
                    *i += 1;
                    continue;
                }
                if b.get(*i) == Some(&'}') {
                    *i += 1;
                }
                break;
            }
            return Value::Object(o);
        }
        if b[*i] == '[' {
            *i += 1;
            let mut a = Vec::new();
            ws(b, i);
            if b.get(*i) == Some(&']') {
                *i += 1;
                return Value::Array(a);
            }
            loop {
                a.push(val(b, i));
                ws(b, i);
                if b.get(*i) == Some(&',') {
                    *i += 1;
                    continue;
                }
                if b.get(*i) == Some(&']') {
                    *i += 1;
                }
                break;
            }
            return Value::Array(a);
        }
        if b[*i] == '"' || b[*i] == '\'' {
            return Value::String(quoted(b, i, b[*i]));
        }
        let mut out = String::new();
        while *i < b.len() && b[*i] != ',' && b[*i] != ']' && b[*i] != '}' {
            out.push(b[*i]);
            *i += 1;
        }
        scalar(&out)
    }
    val(&b, &mut i)
}

/// Строка структуры: отступ, тело, номер в исходнике.
pub struct Line {
    pub n: usize,
    pub indent: usize,
    pub body: String,
}

/// Строки СТРУКТУРЫ, а не «текст без комментариев». Блочный скаляр
/// складывается в одну строку прямо здесь: иначе проза внутри `description`
/// читалась бы как разметка, и якорь в описании ронял бы разбор целиком.
pub fn structural_lines(text: &str) -> Vec<Line> {
    let src: Vec<&str> = text.split('\n').collect();
    let block = regex::Regex::new(r":(\s+[|>][-+0-9]*)\s*$").expect("образец блочного скаляра");
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < src.len() {
        let line = src[i];
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            i += 1;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let body = line.trim().to_owned();
        if block.is_match(&body) {
            let at = i + 1;
            let mut buf: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < src.len()
                && (src[j].trim().is_empty()
                    || src[j].len() - src[j].trim_start().len() > indent)
            {
                buf.push(src[j].trim().to_owned());
                j += 1;
            }
            i = j;
            let folded = buf.join(" ").trim().to_owned();
            let quoted = Value::String(folded).to_string();
            out.push(Line { n: at, indent, body: block.replace(&body, format!(": {quoted}")).into_owned() });
            continue;
        }
        out.push(Line { n: i + 1, indent, body });
        i += 1;
    }
    out
}

struct Frame {
    indent: usize,
    seq: bool,
    body: String,
}

/// Разбирает текст в дерево значений.
pub fn parse(text: &str) -> Value {
    let mut f: Vec<Frame> = structural_lines(text)
        .into_iter()
        .map(|l| {
            if l.body == "-" {
                Frame { indent: l.indent, seq: true, body: String::new() }
            } else if let Some(rest) = l.body.strip_prefix("- ") {
                Frame { indent: l.indent, seq: true, body: rest.to_owned() }
            } else {
                Frame { indent: l.indent, seq: false, body: l.body }
            }
        })
        .collect();
    let mut p = 0usize;
    at(&mut f, &mut p, 0)
}

fn at(f: &mut Vec<Frame>, p: &mut usize, indent: usize) -> Value {
    if *p >= f.len() || f[*p].indent < indent {
        return Value::Null;
    }
    if f[*p].seq && f[*p].indent == indent {
        let mut arr = Vec::new();
        while *p < f.len() && f[*p].indent == indent && f[*p].seq {
            let body = f[*p].body.clone();
            if body.is_empty() {
                *p += 1;
                arr.push(at(f, p, indent + 2));
                continue;
            }
            if body.starts_with('[') || body.starts_with('{') {
                arr.push(flow(&body));
                *p += 1;
                continue;
            }
            if split_kv(&body).is_none() {
                arr.push(scalar(&body));
                *p += 1;
                continue;
            }
            // Элемент списка, который сам отображение: строка переписывается
            // в обычную и читается тем же ходом уровнем глубже.
            f[*p] = Frame { indent: indent + 2, seq: false, body };
            arr.push(at(f, p, indent + 2));
        }
        return Value::Array(arr);
    }
    let mut o = Map::new();
    while *p < f.len() && f[*p].indent == indent && !f[*p].seq {
        let Some((k, rest)) = split_kv(&f[*p].body) else {
            *p += 1;
            continue;
        };
        *p += 1;
        let v = if rest.is_empty() {
            if *p < f.len() && f[*p].indent > indent {
                let deeper = f[*p].indent;
                at(f, p, deeper)
            } else {
                Value::Null
            }
        } else if rest.starts_with('[') || rest.starts_with('{') {
            flow(&rest)
        } else {
            scalar(&rest)
        };
        o.insert(k, v);
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn fixtures_hold() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            ("a: 1\nb: two\n", json!({"a": 1, "b": "two"})),
            ("m:\n  x: 1\n  y: 2\n", json!({"m": {"x": 1, "y": 2}})),
            ("s:\n  - one\n  - two\n", json!({"s": ["one", "two"]})),
            ("s:\n  - name: a\n    in: query\n  - name: b\n    in: path\n",
             json!({"s": [{"name": "a", "in": "query"}, {"name": "b", "in": "path"}]})),
            ("f: { type: string, example: \"x\" }\n",
             json!({"f": {"type": "string", "example": "x"}})),
            ("e: [a, b, c]\n", json!({"e": ["a", "b", "c"]})),
            // Ключ с двоеточием ВНУТРИ: на нём разбор терял всю ветку `paths`.
            ("paths:\n  /signals:ingest:\n    post:\n      tag: x\n",
             json!({"paths": {"/signals:ingest": {"post": {"tag": "x"}}}})),
            ("d: |\n  первая\n  вторая\nnext: 1\n", json!({"d": "первая вторая", "next": 1})),
            ("d: >-\n  текст\nnext: 1\n", json!({"d": "текст", "next": 1})),
            ("description: \"FR-SIT-01: ровно три\"\n",
             json!({"description": "FR-SIT-01: ровно три"})),
            ("e: [paged, null]\n", json!({"e": ["paged", null]})),
        ];
        let mut bad = Vec::new();
        for (input, want) in cases {
            let got = parse(input);
            if got != want {
                bad.push(format!("{input:?}\n  ждали: {want}\n  вышло: {got}"));
            }
        }
        assert!(bad.is_empty(), "разошлось {}:\n{}", bad.len(), bad.join("\n"));
    }
}

#[cfg(test)]
mod real {
    /// Разбор настоящего контракта: величины корпуса против ТЕКСТА.
    ///
    /// Отдельный тест, а не фикстура: сломанная ветка YAML уносит схему из
    /// корпуса, и обе охраны показывают ноль — правило зелено, потому что ему
    /// нечего сказать. Поэтому сверяется то, НА ЧЁМ СТОЯТ ПРАВИЛА: число схем,
    /// считанное обходом, против числа, считанного по отступам текста.
    #[test]
    fn contract_walk_matches_text() {
        let path = "/opt/src/github.com/myack-dev/myack/backend/api/openapi.yaml";
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("контракта нет на месте — сверять нечего");
            return;
        };
        let doc = super::parse(&text);
        let by_walk = doc
            .get("components")
            .and_then(|c| c.get("schemas"))
            .and_then(|s| s.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        let mut by_text = 0usize;
        let mut in_schemas = false;
        let head = regex::Regex::new(r"^[A-Za-z][A-Za-z0-9_]*:$").expect("образец имени схемы");
        for l in super::structural_lines(&text) {
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
        assert!(by_walk > 0, "обход не нашёл ни одной схемы");
        assert_eq!(by_walk, by_text, "схем обходом {by_walk}, по тексту {by_text}");
        eprintln!("схем в контракте: {by_walk}");
    }
}

#[cfg(test)]
mod marks {
    #[test]
    fn derived_mark_is_read() {
        let y = "components:\n  schemas:\n    A:\n      properties:\n        f:\n          type: string\n          x-derived: \"из b и c\"\n";
        let d = super::parse(y);
        let f = &d["components"]["schemas"]["A"]["properties"]["f"];
        assert_eq!(f["x-derived"].as_str(), Some("из b и c"), "вышло: {f}");
    }
}
