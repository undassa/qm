//! Разбор документа на блоки, разделы, ячейки, ссылки и поля.
//!
//! **Порт донорского `document-structure.ts`, строка в строку.** Это не «своя
//! версия»: таблицы уже наполнены его разбором, и любое расхождение сдвинуло бы
//! существующие 51043 блока и 26740 ячеек. Каждая тонкость здесь замерена на
//! наборе донором, и переносится вместе с доводом, а не с одним поведением.

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub ord: i32,
    pub kind: &'static str,
    pub level: Option<i32>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub ord: i32,
    pub level: i32,
    pub title: String,
    pub anchor: String,
    pub parent_ord: Option<i32>,
    pub first_block: i32,
    pub last_block: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub block_ord: i32,
    pub row: i32,
    pub col: i32,
    pub raw: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub block_ord: i32,
    pub ord: i32,
    pub text: String,
    pub target_path: String,
    pub target_anchor: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub section_ord: i32,
    pub ord: i32,
    pub name: String,
    pub shape: &'static str,
    pub value_raw: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct Structure {
    pub blocks: Vec<Block>,
    pub sections: Vec<Section>,
    pub cells: Vec<Cell>,
    pub links: Vec<Link>,
    pub fields: Vec<Field>,
}

static FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\s*)(`{3,}|~{3,})").unwrap());
static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})\s+(.*)$").unwrap());
static TABLE_ROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\|").unwrap());
static TABLE_SEPARATOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\|[\s:|-]+\|\s*$").unwrap());
static LIST_ITEM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s+").unwrap());
static HTML_OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*<[a-zA-Z!/]").unwrap());
/// Метка ссылки не переходит на другую строку, и запрет не косметический: с
/// `[^\]]*` она перескакивала переводы строк, и открывающая скобка без
/// закрывающей дотягивалась до `]` следующей настоящей ссылки, съедая её вместе
/// с прозой между ними.
static LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]\n]*)\]\(([^)\s]+)\)").unwrap());
static BULLET_FIELD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*[-*+]\s+\*\*([^*]+?):?\*\*:?\s*(.*)$").unwrap());
static ROW_FIELD: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\*\*([^*]+?)\*\*\s*$").unwrap());
static CODE_SPAN: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`]*)`").unwrap());
static BOLD: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*([^*]*)\*\*").unwrap());
static ITALIC: Lazy<Regex> = Lazy::new(|| Regex::new(r"(^|[^*])\*([^*]+)\*").unwrap());
static STRIKE: Lazy<Regex> = Lazy::new(|| Regex::new(r"~~([^~]*)~~").unwrap());
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

pub fn strip_markup(raw: &str) -> String {
    let s = LINK.replace_all(raw, "$1");
    let s = CODE_SPAN.replace_all(&s, "$1");
    let s = BOLD.replace_all(&s, "$1");
    let s = ITALIC.replace_all(&s, "$1$2");
    let s = STRIKE.replace_all(&s, "$1");
    s.trim().to_owned()
}

pub fn anchor_of(title: &str) -> String {
    let stripped = strip_markup(title).to_lowercase();
    let kept: String = stripped
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
        .collect();
    SPACES.replace_all(kept.trim(), "-").into_owned()
}


fn bare(line: &str) -> &str {
    line.strip_suffix('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).unwrap_or(line)
}

/// Строки с сохранённым переводом: сборка документа идёт склейкой `raw`, и
/// потерянный здесь перевод строки потерялся бы в самом документе.
fn split_lines(content: &str) -> Vec<String> {
    let parts: Vec<&str> = content.split('\n').collect();
    let mut out: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, l)| if i + 1 == parts.len() { (*l).to_owned() } else { format!("{l}\n") })
        .collect();
    if out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

fn kind_of_line(raw: &str, next: Option<&String>) -> &'static str {
    let line = bare(raw);
    if line.trim().is_empty() {
        return "blank";
    }
    if HEADING.is_match(line) {
        return "heading";
    }
    if TABLE_ROW.is_match(line) && next.map(|n| TABLE_SEPARATOR.is_match(bare(n))).unwrap_or(false) {
        return "table";
    }
    if LIST_ITEM.is_match(line) {
        return "list";
    }
    if HTML_OPEN.is_match(line) {
        return "html";
    }
    "prose"
}

/// Строка таблицы на ячейки.
///
/// `split('|')` неверен дважды, и оба случая замерены на наборе. **Экранированная
/// черта**: автор пишет `\|`, когда черта — часть значения; деление по всем
/// чертам резало ячейку надвое, а проекции читают ячейки ПО НОМЕРУ КОЛОНКИ и
/// получали сдвинутое. **Черта внутри кодовой вставки** — текст, а не граница.
/// Экранирование снимается: в значении живёт черта, а `\|` — способ её записать.
pub fn split_row(inner: &str) -> Vec<String> {
    let chars: Vec<char> = inner.chars().collect();
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && chars.get(i + 1) == Some(&'|') {
            current.push('|');
            i += 2;
            continue;
        }
        if ch == '`' {
            in_code = !in_code;
        }
        if ch == '|' && !in_code {
            cells.push(std::mem::take(&mut current));
            i += 1;
            continue;
        }
        current.push(ch);
        i += 1;
    }
    cells.push(current);
    cells
}

fn cells_of_table(block_ord: i32, raw: &str) -> Vec<Cell> {
    let mut out = Vec::new();
    let mut row = 0;
    for line in raw.split('\n') {
        if !TABLE_ROW.is_match(line) || TABLE_SEPARATOR.is_match(line) {
            continue;
        }
        let trimmed = bare(line).trim();
        let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
        let inner = inner.strip_suffix('|').unwrap_or(inner);
        for (col, cell) in split_row(inner).into_iter().enumerate() {
            let value = strip_markup(&cell);
            out.push(Cell { block_ord, row, col: col as i32, raw: cell, value });
        }
        row += 1;
    }
    out
}

/// Похоже ли это на адрес вообще.
///
/// Образец `[текст](цель)` встречается не только в разметке: в js и css такие
/// скобки идут подряд, и в набор попадали «ссылки» с целью `x-`, `s`, `\w+` —
/// шесть штук из трёх макетов. Адрес несёт хотя бы одно из трёх: косую, точку
/// или двоеточие. Без них это не цель, а совпадение скобок.
fn looks_like_target(target: &str) -> bool {
    let head = target.split('#').next().unwrap_or("");
    !head.is_empty() && (head.contains('/') || head.contains('.') || head.contains(':'))
}

fn links_of(block_ord: i32, raw: &str) -> Vec<Link> {
    LINK.captures_iter(raw)
        .filter(|m| looks_like_target(m.get(2).map(|g| g.as_str()).unwrap_or("")))
        .enumerate()
        .map(|(ord, m)| {
            let target = m.get(2).map(|g| g.as_str()).unwrap_or("");
            let (path, anchor) = match target.find('#') {
                Some(at) => (&target[..at], &target[at + 1..]),
                None => (target, ""),
            };
            Link {
                block_ord,
                ord: ord as i32,
                text: m.get(1).map(|g| g.as_str()).unwrap_or("").to_owned(),
                target_path: path.to_owned(),
                target_anchor: anchor.to_owned(),
            }
        })
        .collect()
}

fn fields_of_table(section_ord: i32, cells: &[Cell], block_ord: i32, raw: &str) -> Vec<Field> {
    // Пустая шапка над двумя колонками — `| | |` — значит «здесь пары».
    // Проверяется по СЫРОМУ тексту: разбор такую шапку выбрасывает, и к ячейкам
    // она уже не приходит. Так написаны выжимки прогонов: этап, диапазон,
    // закрыта, ревью, раундов починки — и все восемнадцать отдавали вместо
    // полей заголовки соседней прозы.
    let headless = raw
        .lines()
        .next()
        .map(|l| {
            let cols: Vec<&str> = l.trim().trim_matches('|').split('|').collect();
            cols.len() == 2 && cols.iter().all(|c| c.trim().is_empty())
        })
        .unwrap_or(false);
    let mut rows: std::collections::BTreeMap<i32, Vec<&Cell>> = std::collections::BTreeMap::new();
    for cell in cells.iter().filter(|c| c.block_ord == block_ord) {
        rows.entry(cell.row).or_default().push(cell);
    }
    // Таблица САМА говорит, что она — поля: шапка «Поле · Значение». Тогда имя
    // поля не обязано быть жирным.
    //
    // Жирность была единственным признаком, и набор, пишущий имена полей
    // обычным текстом, не отдавал НИ ОДНОГО поля: 154 задачи tot назвали веху,
    // размер, вид и зависимости — и все 154 читались пустыми. Отсюда же был и
    // один вал в плане вместо двадцати двух: зависимости берутся полем.
    // Таблица пар опознаётся тремя способами, и третий — ПУСТАЯ шапка.
    //
    // `| | |` над двумя колонками значит ровно «здесь пары, а не колонки
    // данных»: так написаны все выжимки прогонов — этап, диапазон, закрыта,
    // ревью, раундов починки. Ни жирное имя, ни шапка «Поле · Значение» их не
    // опознавали, и восемнадцать выжимок отдавали вместо полей заголовки
    // соседней прозы.
    let titled = rows
        .values()
        .next()
        .map(|head| {
            head.len() == 2
                && {
                    let (a, b) = (head[0].value.trim().to_lowercase(), head[1].value.trim().to_lowercase());
                    // Не `eq_ignore_ascii_case`: для неё «П» и «п» — разные
                    // буквы, и шапка на русском не совпадала ни с чем.
                    a == "поле" && b == "значение"
                }
        })
        .unwrap_or(false);
    // У таблицы без шапки первая строка — уже пара, а не заголовок.
    let (titled, skip_head) = if headless { (true, false) } else { (titled, titled) };
    let mut out = Vec::new();
    for (row, bucket) in rows.iter() {
        if bucket.len() != 2 || (skip_head && *row == 0) {
            continue;
        }
        let Some(name) = ROW_FIELD
            .captures(&bucket[0].raw)
            .and_then(|c| c.get(1).map(|g| g.as_str().trim().to_owned()))
            .or_else(|| titled.then(|| bucket[0].value.trim().to_owned()))
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let ord = out.len() as i32;
        out.push(Field {
            section_ord,
            ord,
            name,
            shape: "row",
            value_raw: bucket[1].raw.clone(),
            value: bucket[1].value.clone(),
        });
    }
    out
}

/// Пункт списка может переноситься: продолжение идёт с отступом и без маркера.
/// Читая по одной строке, значение обрывалось посреди фразы и уносило с собой
/// незакрытую разметку.
fn join_wrapped_bullets(raw: &str) -> Vec<String> {
    let mut joined: Vec<String> = Vec::new();
    for line in raw.split('\n') {
        let indented = line.starts_with(char::is_whitespace) && !line.trim().is_empty();
        let is_continuation = indented && !LIST_ITEM.is_match(line) && line.trim_start().chars().next().is_some();
        let bullet_start = Regex::new(r"^\s*[-*+]\s").unwrap().is_match(line);
        if indented && !bullet_start && !joined.is_empty() {
            let last = joined.last_mut().unwrap();
            last.push(' ');
            last.push_str(line.trim());
        } else {
            let _ = is_continuation;
            joined.push(line.to_owned());
        }
    }
    joined
}

fn fields_of_list(section_ord: i32, raw: &str) -> Vec<Field> {
    let mut out = Vec::new();
    for line in join_wrapped_bullets(raw) {
        let Some(m) = BULLET_FIELD.captures(bare(&line)) else { continue };
        let ord = out.len() as i32;
        let value_raw = m.get(2).map(|g| g.as_str()).unwrap_or("").to_owned();
        out.push(Field {
            section_ord,
            ord,
            name: m.get(1).map(|g| g.as_str().trim().to_owned()).unwrap_or_default(),
            shape: "bullet",
            value: strip_markup(&value_raw),
            value_raw,
        });
    }
    out
}

pub fn parse_document(content: &str) -> Structure {
    let lines = split_lines(content);
    let mut blocks: Vec<Block> = Vec::new();
    let mut buffer: Vec<String> = Vec::new();
    let mut buffer_kind: Option<&'static str> = None;

    macro_rules! flush {
        () => {
            if let Some(kind) = buffer_kind {
                if !buffer.is_empty() {
                    blocks.push(Block {
                        ord: blocks.len() as i32,
                        kind,
                        level: None,
                        raw: buffer.join(""),
                    });
                }
                buffer.clear();
                buffer_kind = None;
            }
        };
    }

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].clone();
        if let Some(fence) = FENCE.captures(&line) {
            flush!();
            let marker = fence.get(2).map(|g| g.as_str().to_owned()).unwrap_or_default();
            let mut fenced = vec![line];
            i += 1;
            while i < lines.len() {
                fenced.push(lines[i].clone());
                if lines[i].trim_start().starts_with(&marker) {
                    break;
                }
                i += 1;
            }
            blocks.push(Block { ord: blocks.len() as i32, kind: "code", level: None, raw: fenced.join("") });
            i += 1;
            continue;
        }

        let kind = kind_of_line(&line, lines.get(i + 1));
        if kind == "heading" {
            flush!();
            let level = HEADING.captures(bare(&line)).map(|c| c[1].len() as i32).unwrap_or(1);
            blocks.push(Block { ord: blocks.len() as i32, kind: "heading", level: Some(level), raw: line });
            i += 1;
            continue;
        }
        if kind == "table" || buffer_kind == Some("table") {
            let still_table = TABLE_ROW.is_match(&line);
            if buffer_kind == Some("table") && !still_table {
                flush!();
            } else if buffer_kind != Some("table") && buffer_kind.is_some() {
                flush!();
            }
            if still_table {
                buffer_kind = Some("table");
                buffer.push(line);
                i += 1;
                continue;
            }
        }
        // Продолжение пункта списка идёт с отступом и без маркера; считая его
        // прозой, значение поля обрывалось на конце строки вместе с разметкой.
        let continues_list = buffer_kind == Some("list")
            && kind == "prose"
            && line.starts_with(char::is_whitespace)
            && !line.trim().is_empty();
        if buffer_kind.is_some() && buffer_kind != Some(kind) && !continues_list {
            flush!();
        }
        if !continues_list {
            buffer_kind = Some(kind);
        }
        buffer.push(line);
        i += 1;
    }
    flush!();

    let mut sections: Vec<Section> = Vec::new();
    let mut stack: Vec<(i32, i32)> = Vec::new(); // (ord, level)
    for block in &blocks {
        if block.kind != "heading" {
            continue;
        }
        let title = HEADING
            .captures(bare(&block.raw))
            .and_then(|c| c.get(2).map(|g| g.as_str().trim_end().to_owned()))
            .unwrap_or_default();
        let level = block.level.unwrap_or(1);
        while stack.last().map(|(_, l)| *l >= level).unwrap_or(false) {
            stack.pop();
        }
        sections.push(Section {
            ord: block.ord,
            level,
            title: strip_markup(&title),
            anchor: anchor_of(&title),
            parent_ord: stack.last().map(|(o, _)| *o),
            first_block: block.ord,
            last_block: blocks.len() as i32 - 1,
        });
        stack.push((block.ord, level));
    }
    for i in 0..sections.len() {
        for j in i + 1..sections.len() {
            if sections[j].level <= sections[i].level {
                sections[i].last_block = sections[j].ord - 1;
                break;
            }
        }
    }

    let mut cells: Vec<Cell> = Vec::new();
    let mut links: Vec<Link> = Vec::new();
    let mut fields: Vec<Field> = Vec::new();
    for block in &blocks {
        if block.kind == "table" {
            cells.extend(cells_of_table(block.ord, &block.raw));
        }
        links.extend(links_of(block.ord, &block.raw));
        let section_ord = sections.iter().filter(|s| s.ord <= block.ord).map(|s| s.ord).next_back();
        let Some(section_ord) = section_ord else { continue };
        let found = if block.kind == "table" {
            fields_of_table(section_ord, &cells, block.ord, &block.raw)
        } else if block.kind == "list" {
            fields_of_list(section_ord, &block.raw)
        } else {
            Vec::new()
        };
        for mut field in found {
            field.ord = fields.len() as i32;
            fields.push(field);
        }
    }

    Structure { blocks, sections, cells, links, fields }
}

/// Документ обратно из блоков — склейкой `raw` без разделителя.
pub fn render_document(blocks: &[Block]) -> String {
    let mut sorted: Vec<&Block> = blocks.iter().collect();
    sorted.sort_by_key(|b| b.ord);
    sorted.iter().map(|b| b.raw.as_str()).collect()
}

/// Сверка порта с тем, что в базе оставил донор.
///
/// Слабее этого проверять нельзя: таблицы уже наполнены его разбором, и порт,
/// сошедшийся «в основном», сдвинул бы ячейки по номеру колонки — молча.
pub async fn check_against_donor(pool: &deadpool_postgres::Pool, project: &str) -> Result<serde_json::Value, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let docs = client
        .query(
            "SELECT entity_kind, entity_name, content FROM project_documents
              WHERE project_id = $1 ORDER BY entity_kind, entity_name",
            &[&project],
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut same = 0usize;
    let mut differ: Vec<serde_json::Value> = Vec::new();
    let (mut blocks, mut sections, mut cells, mut links, mut fields) = (0usize, 0usize, 0usize, 0usize, 0usize);

    for d in &docs {
        let (kind, name): (String, String) = (d.get(0), d.get(1));
        let content: String = d.get(2);
        let mine = parse_document(&content);
        blocks += mine.blocks.len();
        sections += mine.sections.len();
        cells += mine.cells.len();
        links += mine.links.len();
        fields += mine.fields.len();

        let theirs_blocks = client
            .query(
                "SELECT ord, kind, raw FROM project_document_blocks
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 ORDER BY ord",
                &[&project, &kind, &name],
            )
            .await
            .map_err(|e| e.to_string())?;
        let theirs_cells = client
            .query(
                "SELECT block_ord, row_ord, col, raw, value FROM project_document_cells
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                  ORDER BY block_ord, row_ord, col",
                &[&project, &kind, &name],
            )
            .await
            .map_err(|e| e.to_string())?;
        let theirs_sections = client
            .query(
                "SELECT ord, level, title, anchor, first_block, last_block FROM project_document_sections
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 ORDER BY ord",
                &[&project, &kind, &name],
            )
            .await
            .map_err(|e| e.to_string())?;

        let mut why: Vec<String> = Vec::new();
        if theirs_blocks.len() != mine.blocks.len() {
            why.push(format!("блоков {} против {}", mine.blocks.len(), theirs_blocks.len()));
        } else {
            for (m, t) in mine.blocks.iter().zip(theirs_blocks.iter()) {
                if m.kind != t.get::<_, String>(1) || m.raw != t.get::<_, String>(2) {
                    why.push(format!("блок {} расходится", m.ord));
                    break;
                }
            }
        }
        if theirs_cells.len() != mine.cells.len() {
            why.push(format!("ячеек {} против {}", mine.cells.len(), theirs_cells.len()));
        } else {
            for (m, t) in mine.cells.iter().zip(theirs_cells.iter()) {
                if m.raw != t.get::<_, String>(3) || m.value != t.get::<_, String>(4) {
                    why.push(format!("ячейка {}:{}:{} расходится", m.block_ord, m.row, m.col));
                    break;
                }
            }
        }
        if theirs_sections.len() != mine.sections.len() {
            why.push(format!("разделов {} против {}", mine.sections.len(), theirs_sections.len()));
        } else {
            for (m, t) in mine.sections.iter().zip(theirs_sections.iter()) {
                if m.anchor != t.get::<_, String>(3)
                    || m.title != t.get::<_, String>(2)
                    || m.last_block != t.get::<_, i32>(5)
                {
                    why.push(format!("раздел {} расходится: {}", m.ord, m.anchor));
                    break;
                }
            }
        }

        if why.is_empty() {
            same += 1;
        } else if differ.len() < 12 {
            differ.push(serde_json::json!({
                "entity": if name.is_empty() { kind.clone() } else { format!("{kind} {name}") },
                "why": why }));
        }
    }

    Ok(serde_json::json!({
        "documents": docs.len(), "same": same, "differ": docs.len() - same,
        "mine": { "blocks": blocks, "sections": sections, "cells": cells, "links": links, "fields": fields },
        "examples": differ,
    }))
}
