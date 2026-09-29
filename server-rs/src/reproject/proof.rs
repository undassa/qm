//! Требования и проверки — на них опирается почти всё остальное.
//!
//! Одно и то же имя стоит первой ячейкой и в наборе доказательства, и в задачах
//! плана, и в историях. Объявление сильнее цитаты: требование объявляется под
//! `10-intent/`, проверка — под `40-proof/`, и порядок прихода ячеек этого не
//! меняет.
//!
//! Родитель-потребность — связь, а не поле: у части требований в ячейке «← ST»
//! стоит больше одного имени, и колонка вместила бы первое, молча потеряв
//! остальные.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Ячейка, ОБЪЯВЛЯЮЩАЯ требование: имя, и дальше либо ничего, либо заголовок
/// через разделитель.
///
/// Хвост «через что угодно» делал объявлением любую строку любой таблицы,
/// начинающуюся с имени: строка «`NFR-10`, минимальные версии ОС и статус
/// Wayland | ждёт `M5-T14`» из таблицы открытых мест переписывала требованию и
/// текст, и заголовок, и способ измерения — на «ждёт матрицу CI».
static REQUIREMENT_CELL: Lazy<Regex> = Lazy::new(|| {
    // Хвост допустим двух родов: заголовок через разделитель и отметка
    // выполненности сразу за именем — `FR-EXT-01 ✓уд`. Проза, продолжающая
    // имя запятой, объявлением не считается.
    //
    // Без забегания вперёд: `regex` его не поддерживает, и образец с ним не
    // компилируется — а собирается он лениво, при первой пересборке, и роняет
    // не строку, а весь запрос. Поэтому знак отметки просто входит в хвост.
    Regex::new(r"^\s*`?((FR|NFR)-([A-Z0-9]+)(?:-\d+[a-z]?)?)`?\s*([·—–:✓✗][\s\S]*?)?\s*$")
        .expect("образец требования")
});
static CHECK_CELL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*`(TC-([A-Z0-9]+)-\d+[a-z]?)`\s*$").expect("образец проверки"));
static REQUIREMENT_BLOCK: Lazy<Regex> = Lazy::new(|| {
    // Заголовок — до закрывающих звёзд той же строки; без них блок остаётся
    // блоком, а заголовка у него нет.
    Regex::new(r"^\*\*((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)\s*·(?:\s*(.*?)\s*\*\*)?").expect("образец блока требования")
});

/// Виды документов, объявляющие требования БЛОКАМИ `**FR-NN · заголовок**`:
/// srs — функциональные и нефункциональные, ui-spec — `FR-UI-…` (у `tot-ade`
/// все 56 блоков `FR-UI` стоят в ui-spec, в srs ни одного). Остальные виды
/// блок только цитируют.
///
/// Перечень один на троих: пересборку заголовка, пересборку «Проверяется» и
/// отказ `requirement-add` (`written_in`). Пока их было два, дверь смотрела
/// заголовок только в srs, а способ доказательства — в любом документе, и одна
/// принимала бы поле, которое пересборка тут же перепишет.
pub(crate) const BLOCK_HOMES: &[&str] = &["srs", "ui-spec"];

/// Заголовки блоков `**FR-NN · заголовок**` документа: имя и заголовок, первый
/// по имени.
pub(crate) fn block_titles(content: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in content.lines() {
        let Some(m) = REQUIREMENT_BLOCK.captures(line.trim_start()) else { continue };
        let Some(title) = m.get(2).map(|t| t.as_str()).filter(|t| !t.is_empty()) else { continue };
        if !out.iter().any(|(id, _)| id == &m[1]) {
            out.push((m[1].to_owned(), title.to_owned()));
        }
    }
    out
}

/// Абзацы «Проверяется» документа: вид, имя, требование блока и текст после маркера.
///
/// Требование — то, чей блок `**FR-NN · …**` открыт выше; заголовок раздела
/// блок закрывает. Абзац кончается пустой строкой, разделителем или заголовком:
/// склейка через пустую строку и дала полю хвост «--- # Часть II».
fn paragraphs_checks(kind: String, name: String, content: &str, marker: &str) -> Vec<(String, String, Option<String>, String)> {
    let mut out = Vec::new();
    let mut block: Option<(String, bool)> = None;
    let mut paragraph: Option<(Option<String>, Vec<String>)> = None;
    let end = |line: &str| {
        let t = line.trim_start();
        t.is_empty() || t.starts_with('#') || t.starts_with("---") || t.starts_with('|') || REQUIREMENT_BLOCK.is_match(t)
    };
    let close_block = |block: Option<(String, bool)>, out: &mut Vec<(String, String, Option<String>, String)>| {
        if let Some((id, false)) = block {
            out.push((kind.clone(), name.clone(), Some(id), String::new()));
        }
    };
    for line in content.lines() {
        if paragraph.is_some() && end(line) {
            let (req, lines) = paragraph.take().expect("абзац открыт");
            out.push((kind.clone(), name.clone(), req, lines.join("\n")));
        }
        let t = line.trim_start();
        if let Some(m) = REQUIREMENT_BLOCK.captures(t) {
            close_block(block.take(), &mut out);
            block = Some((m[1].to_owned(), false));
        } else if t.starts_with('#') {
            close_block(block.take(), &mut out);
        }
        if let Some((_, lines)) = paragraph.as_mut() {
            lines.push(line.trim().to_owned());
        } else if let Some(at) = line.find(marker) {
            // Снимается только выделение САМОГО маркера — `*Проверяется:*`.
            // Все звёзды подряд срезали и начало жирного слова за ним:
            // «**компиляционный**» выходило «компиляционный**».
            let after = &line[at + marker.len()..];
            let after = ["**", "__", "*", "_"].iter().find_map(|m| after.strip_prefix(m)).unwrap_or(after);
            let tail = after.trim().to_owned();
            if let Some((_, was)) = block.as_mut() {
                *was = true;
            }
            paragraph = Some((block.as_ref().map(|b| b.0.clone()), vec![tail]));
        }
    }
    if let Some((req, lines)) = paragraph {
        out.push((kind.clone(), name.clone(), req, lines.join("\n")));
    }
    close_block(block, &mut out);
    out
}

static REFERENCED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"`((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)`").expect("образец ссылки"));
static NEED_REFERENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bST-(\d+)\b").expect("образец потребности"));
/// Приоритет записан одной буквой: обязательное · желательное · позже.
static PRIORITY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*`?([ОЖП])`?\s*$").expect("образец приоритета"));
struct Requirement {
    id: String,
    section: Option<i32>,
    kind: String,
    area: String,
    text: String,
    kind_of_document: String,
    name: String,
    satisfied: bool,
    priority: String,
    measured_by: String,
}

struct Check {
    id: String,
    section: Option<i32>,
    area: String,
    requirement: Option<String>,
    spec: String,
    kind_of_document: String,
    name: String,
}

/// Где ЖИВЁТ объявление: `test-cases` объявляет проверки, `srs` — требования,
/// остальные их лишь цитируют.
///
/// Прежде это было сравнение приставки пути (`40-proof/`, `10-intent/`) —
/// «объявляющий каталог». Каталог был способом сказать «эти документы»; вид
/// говорит это прямо и записан у каждого.
const DECLARES_CHECKS: &str = "test-cases";
const DECLARES_REQUIREMENTS: &str = "srs";

/// Доказательства по абзацу: где абзац — и что в нём названо.
type ProofBuckets = HashMap<(String, String, i32, i32), Vec<(String, String)>>;

/// Объявляющий документ сильнее цитирующего — и только он перебивает уже взятое.
fn declares_over(kind: &str, existing: Option<&String>, home: &str) -> bool {
    match existing {
        None => true,
        Some(had) => kind == home && had != home,
    }
}

/// Какие поля требования пересборка берёт из документа: документ, чей блок
/// даёт заголовок, и документ, чей абзац «Проверяется» даёт способ
/// доказательства. Читает тем же, чем пересборка, и в том же порядке: дверь,
/// спрашивающая иначе, приняла бы поле, которое пересборка перепишет.
pub(crate) async fn written_in(
    client: &impl deadpool_postgres::GenericClient,
    project: &str,
    id: &str,
) -> Result<(Option<String>, Option<String>), crate::db::Fail> {
    let marker = crate::scheme::Terms::load_at(client, project).await?.one("marker.verified-by").map(str::to_owned);
    let documents = client
        .query(
            "SELECT entity_kind, entity_name, content FROM project_documents
              WHERE project_id = $1 AND entity_kind = ANY($2) ORDER BY entity_kind, entity_name",
            &[&project, &BLOCK_HOMES],
        )
        .await?;
    let (mut title, mut measured) = (None, None);
    for r in &documents {
        let (kind, name, content): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let doc = format!("{kind} {name}");
        if title.is_none() && block_titles(&content).iter().any(|(had, _)| had == id) {
            title = Some(doc.clone());
        }
        if measured.is_none() {
            if let Some(m) = marker.as_deref().filter(|m| content.contains(m)) {
                if paragraphs_checks(kind, name, &content, m).iter()
                    .any(|p| p.2.as_deref() == Some(id) && !p.3.is_empty()) {
                    measured = Some(doc);
                }
            }
        }
    }
    Ok((title, measured))
}

pub(crate) async fn project(
    pool: &Pool,
    project: &str,
) -> Result<(usize, usize, usize, u64), crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    // Порядок строк решает, какое из двух объявлений одного имени взято первым,
    // и потому назван: вид, имя, блок, строка, колонка. Прежде первым шёл путь;
    // порядок сменился вместе с ключом, и старшинство объявляющего документа над
    // цитирующим держится теперь не порядком, а `declares_over` — как и держалось.
    let rows = client
        .query(
            "SELECT entity_kind, entity_name, block_ord, row_ord, col, raw, value
               FROM project_document_cells
              WHERE project_id = $1
                AND entity_kind IN ('srs', 'strs', 'test-cases', 'acceptance', 'tdd', 'test-plan',
                                    'brs', 'cjm', 'concept', 'traceability', 'ui-spec', 'risks',
                                    'coverage', 'feature', 'story', 'mockup', 'index')
              ORDER BY entity_kind, entity_name, block_ord, row_ord, col",
            &[&project],
        )
        .await?;

    // Секции документов — чтобы у строки сущности был не только документ, но и
    // МЕСТО в нём. Без места документ из таблицы не собрать: неизвестен ни
    // порядок, ни к какому разделу строка относится.
    let sec = client
        .query(
            "SELECT entity_kind, entity_name, ord FROM project_document_sections
              WHERE project_id = $1 ORDER BY entity_kind, entity_name, ord",
            &[&project],
        )
        .await?;
    let mut sections: HashMap<(String, String), Vec<i32>> = HashMap::new();
    for r in &sec {
        sections.entry((r.get(0), r.get(1))).or_default().push(r.get(2));
    }
    let section_for = |k: &str, n: &str, block: i32| -> Option<i32> {
        sections
            .get(&(k.to_owned(), n.to_owned()))
            .and_then(|v| v.iter().rev().find(|o| **o <= block).copied())
    };

    // Ячейки одной строки таблицы — вместе и в порядке появления.
    let mut order: Vec<(String, String, i32, i32)> = Vec::new();
    let mut buckets: ProofBuckets = HashMap::new();
    for r in &rows {
        let key = (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, i32>(2), r.get::<_, i32>(3));
        let bucket = buckets.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        bucket.push((r.get(5), r.get(6)));
    }

    let mut requirements: HashMap<String, Requirement> = HashMap::new();
    let mut checks: HashMap<String, Check> = HashMap::new();
    let mut needs: Vec<(String, String)> = Vec::new();
    let mut seen_need = HashSet::new();

    for key in &order {
        let (doc_kind, doc_name) = (&key.0, &key.1);
        let cells = &buckets[key];
        let Some((raw, _)) = cells.first() else { continue };
        let value_at = |i: usize| cells.get(i).map(|c| c.1.clone()).unwrap_or_default();

        if let Some(m) = CHECK_CELL.captures(raw) {
            if cells.len() < 2 {
                continue;
            }
            let id = m[1].to_owned();
            if !declares_over(doc_kind, checks.get(&id).map(|c| &c.kind_of_document), DECLARES_CHECKS) {
                continue;
            }
            let requirement = REFERENCED.captures(&cells[1].0).map(|r| r[1].to_owned());
            checks.insert(
                id.clone(),
                Check { id, area: m[2].to_owned(), requirement, spec: value_at(2),
                        kind_of_document: doc_kind.clone(), name: doc_name.clone(),
                        section: section_for(doc_kind, doc_name, key.2) },
            );
            continue;
        }

        let Some(m) = REQUIREMENT_CELL.captures(raw) else { continue };
        if cells.len() < 2 {
            continue;
        }
        let id = m[1].to_owned();
        if checks.contains_key(&id) {
            continue;
        }
        if !declares_over(doc_kind, requirements.get(&id).map(|r| &r.kind_of_document), DECLARES_REQUIREMENTS) {
            continue;
        }
        let kind = m[2].to_owned();
        // Третья колонка значит РАЗНОЕ у двух форм таблицы: у функционального
        // это родитель («← ST»), у нефункционального — способ измерения.
        let third = value_at(2);
        let is_fr = kind == "FR";
        requirements.insert(
            id.clone(),
            Requirement {
                id: id.clone(),
                section: section_for(doc_kind, doc_name, key.2),
                // У нефункционального области нет: `NFR-15` — номер, а не «область 15».
                area: if is_fr { m[3].to_owned() } else { String::new() },
                kind,
                text: value_at(1),
                kind_of_document: doc_kind.clone(),
                name: doc_name.clone(),
                // Хвост необязателен: ячейка бывает ровно именем. Обращение по
                // индексу к неучаствовавшей группе — паника, а не пустая строка.
                satisfied: m.get(4).is_some_and(|g| g.as_str().contains('✓')),
                priority: if is_fr {
                    PRIORITY.captures(&value_at(3)).map(|p| p[1].to_owned()).unwrap_or_default()
                } else {
                    String::new()
                },
                measured_by: if is_fr { String::new() } else { third.clone() },
            },
        );
        if is_fr {
            for n in NEED_REFERENCE.captures_iter(&third) {
                let need = format!("ST-{}", &n[1]);
                if seen_need.insert((id.clone(), need.clone())) {
                    needs.push((id.clone(), need));
                }
            }
        }
    }
    drop(client);

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_checks WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // ССЫЛКА ИЗ ТЕКСТА ТРЕБОВАНИЯ — отдельной строкой, а не растворённая в
    // связи документа. Имена берутся ОБЪЯВЛЕННЫМ раскрывателем `ids::plain`,
    // а не новым образцом: перечислить здесь `FR|NFR|ADR` значило бы зашить
    // слова, которыми владеет проект.
    //
    // Род связи — `cites`, и это всё, что разбор честно знает: зачем именно
    // требование сослалось, текст не говорит. Уточняется дверью.
    let mut links: Vec<(String, String)> = Vec::new();
    for r in requirements.values() {
        for name_said in super::ids::plain(&r.text) {
            if name_said != r.id {
                links.push((r.id.clone(), name_said));
            }
        }
    }
    tx.execute(
        "DELETE FROM project_requirement_sources WHERE project_id = $1 AND origin = 'projected'",
        &[&project],
    )
    .await?;
    for (from_where, where_to) in &links {
        tx.execute(
            "INSERT INTO project_requirement_sources (project_id, requirement_id, kind, target, origin)
             VALUES ($1,$2,'requirement',$3,'projected') ON CONFLICT DO NOTHING",
            &[&project, from_where, where_to],
        )
        .await?;
    }
    tx.execute("DELETE FROM project_requirement_needs WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_requirements WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    for r in requirements.values() {
        tx.execute(
            // Заголовок, версия и сквозной признак здесь не перечислены, и это
            // умышленно: таблица их не несёт, и переписать их пустотой значило
            // бы стереть прочитанное. Версию и сквозной признак кладёт
            // объявление, заголовок — блок srs ниже (`block_titles`).
            "INSERT INTO project_requirements(project_id, id, kind, area, text,
                                              entity_kind, entity_name, satisfied,
                                              priority, measured_by, section_ord)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (project_id, id) DO UPDATE SET kind = EXCLUDED.kind,
               area = EXCLUDED.area, text = EXCLUDED.text, entity_kind = EXCLUDED.entity_kind,
               entity_name = EXCLUDED.entity_name, satisfied = EXCLUDED.satisfied,
               priority = EXCLUDED.priority, measured_by = EXCLUDED.measured_by,
               section_ord = EXCLUDED.section_ord",
            &[&project, &r.id, &r.kind, &r.area, &r.text, &r.kind_of_document, &r.name,
              &r.satisfied, &r.priority, &r.measured_by, &r.section],
        )
        .await?;
    }
    for (requirement, need) in &needs {
        tx.execute(
            "INSERT INTO project_requirement_needs(project_id, requirement_id, need_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
            &[&project, requirement, need],
        )
        .await?;
    }
    for c in checks.values() {
        tx.execute(
            "INSERT INTO project_checks(project_id, id, area, requirement_id, spec,
                                        entity_kind, entity_name, section_ord)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (project_id, id) DO NOTHING",
            &[&project, &c.id, &c.area, &c.requirement, &c.spec, &c.kind_of_document, &c.name,
              &c.section],
        )
        .await?;
    }
    // ── Абзац «Проверяется» — из документа ──────────────────────────────────
    //
    // Набор пишет требование блоком `**FR-69 · …**` и способ доказательства —
    // абзацем после маркера. Разбор таких блоков не знал: требования жили
    // объявлениями, и `measured_by` держал текст, объявленный однажды. Правку
    // строки документ принимал, а проверки держались прежним полем — у 49
    // требований из 203, с хвостом следующего раздела у тридцати.
    //
    // Читается АБЗАЦ, а не строка с маркером: продолжение на следующей строке
    // («Плюс сценарий `TC-HARN-07`») терялось, и проверка, названная
    // документом, проверкой не становилась. Абзац кончается пустой строкой.
    let markers = crate::scheme::Terms::load_at(&tx, project).await?;
    let paragraphs = match markers.one("marker.verified-by") {
        Some(marker) => {
            let documents = tx
                .query(
                    "SELECT entity_kind, entity_name, content FROM project_documents
                      WHERE project_id = $1 AND content LIKE '%' || $2 || '%'
                      ORDER BY entity_kind, entity_name",
                    &[&project, &marker],
                )
                .await?;
            documents
                .iter()
                .flat_map(|r| paragraphs_checks(r.get(0), r.get(1), &r.get::<_, String>(2), marker))
                .collect()
        }
        None => Vec::new(),
    };
    let mut proven: HashMap<&str, &str> = HashMap::new();
    for (doc_kind, _, requirement_id, text_of) in &paragraphs {
        if !BLOCK_HOMES.contains(&doc_kind.as_str()) {
            continue;
        }
        if let Some(id) = requirement_id {
            let was = proven.entry(id.as_str()).or_insert("");
            if was.is_empty() {
                *was = text_of.as_str();
            }
        }
    }
    for (id, text_of) in &proven {
        tx.execute(
            "UPDATE project_requirements SET measured_by = $3
              WHERE project_id = $1 AND id = $2 AND measured_by IS DISTINCT FROM $3",
            &[&project, id, text_of],
        )
        .await?;
    }
    // ЗАГОЛОВОК БЛОКА — ИЗ ДОКУМЕНТА, как и его «Проверяется». Заголовок клала
    // только дверь `requirement-add`, а пересборка выводила из блока одно поле
    // способа доказательства. Замер 2026-09-28 на `tot-ade` (undassa/mh#133):
    // заголовок NFR-19 в srs переписан (r144), после `reproject` `measured_by`
    // новый, а `title` прежний — прежнего текста нет ни в srs, ни в поиске, а
    // дверь правки отказывает словами «записано в srs, правьте документ».
    //
    // Берутся только виды `BLOCK_HOMES`: блок, повторённый цитатой в другом
    // документе, не перебивает объявление.
    //
    // ПЕРВЫЙ ВЫВОД НЕ ДВИГАЕТ ОТМЕТКУ. У `tot-ade` все 148 заголовков srs
    // кончаются точкой, и ни один из 148 объявленных заголовков с ними не
    // совпадает (146 — только точкой). Заголовок входит в тело `entity_row`, и
    // первая же пересборка поставила бы новую отметку всем 148 требованиям —
    // ложно переоткрыв всё, что на них стоит, цепочкой до шести звеньев. Та же
    // волна, что описана у `paragraphs_with_which_time`: правка поля проекцией —
    // не правка записи. Пока отметка требования стоит на теле версии 3,
    // заголовок, ставший ЕДИНСТВЕННОЙ разницей с отпечатком, принимается в
    // отпечаток молча; другая разница рядом с ним — правка записи, и отметка
    // двигается. Переход на версию 4 — в этой же транзакции, и перепись, и
    // перевод остальных: пересборка, упавшая между ними, оставила бы окно, в
    // котором настоящая правка заголовка тоже принялась бы молча. Отметки ниже
    // версии 3 не трогаются: их доводит `stamp`, и шаг версии 3 обязан пройти
    // по ним раньше.
    let documents = tx
        .query(
            "SELECT content FROM project_documents WHERE project_id = $1 AND entity_kind = ANY($2)
              ORDER BY entity_kind, entity_name",
            &[&project, &BLOCK_HOMES],
        )
        .await?;
    let mut titled: HashSet<String> = HashSet::new();
    for r in &documents {
        for (id, title) in block_titles(&r.get::<_, String>(0)) {
            if !titled.insert(id.clone()) {
                continue;
            }
            let before: Option<String> = tx
                .query_opt(
                    "SELECT md5(body) FROM entity_row WHERE project_id = $1 AND kind = 'requirement' AND id = $2",
                    &[&project, &id],
                )
                .await?
                .map(|r| r.get(0));
            let moved = tx
                .execute(
                    "UPDATE project_requirements SET title = $3
                      WHERE project_id = $1 AND id = $2 AND title IS DISTINCT FROM $3",
                    &[&project, &id, &title],
                )
                .await?;
            if moved > 0 {
                tx.execute(
                    "UPDATE entity_stamp st SET text_hash = md5(e.body), body_version = 4
                       FROM entity_row e
                      WHERE st.project_id = $1 AND st.kind = 'requirement' AND st.id = $2
                        AND st.body_version = 3 AND st.text_hash = $3
                        AND e.project_id = st.project_id AND e.kind = st.kind AND e.id = st.id",
                    &[&project, &id, &before],
                )
                .await?;
            }
        }
    }
    tx.execute(
        "UPDATE entity_stamp SET body_version = 4 WHERE project_id = $1 AND kind = 'requirement' AND body_version = 3",
        &[&project],
    )
    .await?;

    // ── Проверка, названная самим требованием ────────────────────────────────
    //
    // Один набор объявляет проверки отдельной таблицей `TC-…`, второй называет
    // их прямо в требовании: «тест `fr_01_rebuild_is_byte_identical`», «гейт
    // `criterion:agent-write-denied`», «сценарий `S1-AC-1`». Второй способ
    // читался как отсутствие проверки — сто двадцать пять требований считались
    // непокрытыми при названном доказательстве у каждого.
    //
    // Берётся ТАБЛИЦА требований, а не разбор документа: требование бывает
    // объявлено ручкой, и тогда разбор о нём не знает вовсе.
    //
    // Проверкой считается имя, НАЗВАННОЕ словом: «тест `fr_01_…`», «бенч
    // `page_build`», «линт `await_holding_lock`», «гейт `criterion:…`»,
    // «сценарий `S1-AC-1`». Без этого условия проверкой становилось любое имя
    // в кавычках — и поля формулировки (`can_vouch_as_human`, `on_behalf_of`,
    // `proposed_by`) числились доказательствами наравне с тестами.
    let named = tx
        .execute(
            "INSERT INTO project_checks (project_id, id, area, requirement_id, spec,
                                         entity_kind, entity_name)
             SELECT DISTINCT ON (m[2]) r.project_id, m[2], r.area, r.id, r.measured_by,
                    r.entity_kind, r.entity_name
               FROM project_requirements r,
                    LATERAL regexp_matches(r.measured_by,
                      '(тест|бенч|линт|гейт|сценари|проверк|фикстур)[а-яё]*[^`]{0,24}`([^`]{2,80})`',
                      'g') m
              WHERE r.project_id = $1 AND r.measured_by <> ''
                -- ИМЯ РАЗБИРАЕТСЯ ОБРАЗЦОМ ВИДА, а не тремя формами, зашитыми
                -- здесь. Прежде сюда попадало всё подряд: и `TC-nn`, и имя
                -- правила в коде, и утверждение с пространством имён — 171
                -- запись жила проверкой, не будучи ею.
                AND m[2] ~ (SELECT spec->>'id' FROM kind_layout WHERE name = 'check')
             ON CONFLICT (project_id, id) DO NOTHING",
            &[&project],
        )
        .await?;
    let _ = named;
    // Связь проверки с требованием — отдельной строкой на каждую пару: одна
    // проверка доказывает несколько требований, и колонка в `project_checks`
    // вмещает только первое.
    tx.execute("DELETE FROM project_check_requirements WHERE project_id = $1", &[&project]).await?;
    tx.execute(
        "INSERT INTO project_check_requirements (project_id, check_id, requirement_id)
         SELECT r.project_id, m[2], r.id
           FROM project_requirements r,
                LATERAL regexp_matches(r.measured_by,
                  '(тест|бенч|линт|гейт|сценари|проверк|фикстур)[а-яё]*[^`]{0,24}`([^`]{2,80})`',
                  'g') m
          WHERE r.project_id = $1 AND r.measured_by <> ''
            AND m[2] ~ (SELECT spec->>'id' FROM kind_layout WHERE name = 'check')
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;
    tx.execute(
        "INSERT INTO project_check_requirements (project_id, check_id, requirement_id)
         SELECT project_id, id, requirement_id FROM project_checks
          WHERE project_id = $1 AND requirement_id IS NOT NULL AND requirement_id <> ''
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;
    // ПРОВЕРКИ ИЗ ПРОЗЫ. Проектор читал только ячейки таблиц и только форму
    // `TC-nn` в обратных кавычках — так пишет `myack`. У `tot-ade` то же
    // сказано СТРОКОЙ: «*Проверяется:* сценарий `TC-INDEX-01`, гейт
    // `runtime:single-async`, линты `await_holding_lock`», и проектор их не
    // видел: 227 записей существовали помимо документов.
    //
    // Маркер строки ОБЪЯВЛЕН проектом (`marker.verified-by`), а не зашит:
    // `myack` этой формы не знает вовсе, и навязывать её ему незачем.
    //
    // Имя разбирается по образцу ВИДА: что перед нами — проверка, правило кода
    // или утверждение, — говорит объявленный образец, а не догадка по форме.
    if markers.one("marker.verified-by").is_some() {
        let patterns = tx
            .query(
                "SELECT name, spec->>'id' FROM kind_layout
                  WHERE name IN ('check','lint-rule','assertion') AND spec->>'id' IS NOT NULL",
                &[],
            )
            .await?;
        let name_said = regex::Regex::new(r"`([^`]{2,80})`").expect("образец имени в кавычках");
        let mut found: Vec<(String, String, String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Абзац вне блока требования читается одной строкой маркера, как
        // прежде: целиком он тащил в правила кода слова историй и решений.
        for (dk, dn, requirement_id, paragraph) in &paragraphs {
            let line = if requirement_id.is_some() { paragraph.as_str() } else { paragraph.lines().next().unwrap_or("") };
            for c in name_said.captures_iter(line) {
                let id = c[1].to_owned();
                for pattern in &patterns {
                    let kind_name: String = pattern.get(0);
                    let pat: String = pattern.get(1);
                    let matches: bool = tx.query_one("SELECT $1 ~ $2", &[&id, &pat]).await?.get(0);
                    if matches && seen.insert(format!("{kind_name}\u{1}{id}")) {
                        found.push((kind_name, id.clone(), dk.clone(), dn.clone()));
                        break;
                    }
                }
            }
        }
        for (table, kind_name) in [("project_lint_rules", "lint-rule"), ("project_assertions", "assertion")] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE project_id = $1 AND origin = 'projected'"),
                &[&project],
            )
            .await?;
            for (at, id, dk, dn) in found.iter().filter(|x| x.0 == kind_name) {
                let _ = at;
                tx.execute(
                    &format!(
                        "INSERT INTO {table} (project_id, id, entity_kind, entity_name)
                         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING"
                    ),
                    &[&project, id, dk, dn],
                )
                .await?;
            }
        }
        for (at, id, dk, dn) in found.iter().filter(|x| x.0 == "check") {
            let _ = at;
            let area = id.split('-').nth(1).unwrap_or("").to_owned();
            tx.execute(
                "INSERT INTO project_checks (project_id, id, area, spec, entity_kind, entity_name)
                 VALUES ($1,$2,$3,'',$4,$5) ON CONFLICT (project_id, id) DO NOTHING",
                &[&project, id, &area, dk, dn],
            )
            .await?;
        }
    }

    // ЧЕМ ДОКАЗАНО ТРЕБОВАНИЕ. Имена берутся из той же колонки `measured_by`, но
    // род каждого определяется ОБРАЗЦОМ ВИДА, а не зашитой формой: проверка,
    // правило кода и утверждение попадают каждое к себе, и связь с требованием
    // записывается для всех трёх одинаково.
    tx.execute("DELETE FROM project_requirement_proof WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    tx.execute(
        "INSERT INTO project_requirement_proof (project_id, requirement_id, proof_kind, proof_id)
         SELECT r.project_id, r.id, k.name, m[2]
           FROM project_requirements r,
                LATERAL regexp_matches(r.measured_by,
                  '(тест|бенч|линт|гейт|сценари|проверк|фикстур)[а-яё]*[^`]{0,24}`([^`]{2,80})`',
                  'g') m,
                kind_layout k
          WHERE r.project_id = $1 AND r.measured_by <> ''
            AND k.name IN ('check','lint-rule','assertion')
            AND k.spec->>'id' IS NOT NULL AND m[2] ~ (k.spec->>'id')
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;

    // ИСТОЧНИК ПРОВЕРКИ — ДОКУМЕНТ, КОТОРЫЙ ЕЁ ОБЪЯВЛЯЕТ, а не тот, кто упомянул
    // первым.
    //
    // Три пути кладут сюда проверки, и все три — с `DO NOTHING`. Значит
    // источником записывался первый успевший, а путь из требования берёт
    // источник у ТРЕБОВАНИЯ. У `tot-ade` все пятьдесят шесть проверок живут в
    // `acceptance`, а числились написанными в `srs` — потому что их оттуда
    // назвали.
    //
    // Круг при этом замыкался: вид `named_id_role` зовёт вхождение
    // «объявляющим», когда документ совпал с ЗАПИСАННЫМ источником, — а
    // записанный источник и был неверен. Правило
    // `entity-written-in-its-source` честно говорило «документа, где она
    // написана, нет», и поправить это было нечем: двери снятия у проверок нет,
    // а чистка сносит выведенное и вставляет то же самое.
    //
    // Объявление опознаётся РАЗДЕЛОМ, названным именем: «TC-AGENT-01 · От
    // находки до дельты». Не любым упоминанием имени в заголовке — «TC-AGENT-01
    // считает переходы» это проза о ней, а не её заведение; разделяет их
    // разделитель после имени.
    //
    // ДВУСМЫСЛЕННОЕ РАЗБИРАЕТСЯ ОБЪЯВЛЕННЫМ, а не догадкой. Разделов, названных
    // именем проверки, бывает два: реестр, который её заводит, и задача, которая
    // о ней говорит. Отличает их не длина и не порядок, а раскладка: `single`
    // сказано у вида. Одиночка (`acceptance`, `srs`, `test-cases`) — корпусный
    // дом вида; документ задачи принадлежит своей задаче, и проверка ему не
    // принадлежит.
    //
    // Одиночек два или ни одного — не трогаем: тогда выбрать вправду нечем, и
    // красное правило честнее тихой догадки.
    let home = tx
        .execute(
            "UPDATE project_checks c
                SET entity_kind = d.kind, entity_name = d.name
               FROM (SELECT c2.id,
                            min(s.entity_kind) AS kind, min(s.entity_name) AS name,
                            count(DISTINCT s.entity_kind || E'\t' || s.entity_name) AS сколько
                       FROM project_checks c2
                       JOIN project_document_sections s
                         ON s.project_id = c2.project_id
                        -- Имя приходит из текста документа, и в выражение его не
                        -- подставляют: точка в имени совпала бы с чем угодно, а
                        -- незакрытая скобка уронила бы `UPDATE` — и с ним весь
                        -- круг пересборки этого набора, каждый раз.
                        AND (s.title = c2.id
                             OR (left(s.title, length(c2.id)) = c2.id
                                 AND substr(s.title, length(c2.id) + 1) ~ '^\\s*[·—:-]'))
                       JOIN kind_layout k
                         ON k.name = s.entity_kind AND k.spec->>'single' = 'true'
                      WHERE c2.project_id = $1
                      GROUP BY c2.id) d
              WHERE c.project_id = $1 AND c.id = d.id AND d.сколько = 1
                AND (c.entity_kind, c.entity_name) IS DISTINCT FROM (d.kind, d.name)",
            &[&project],
        )
        .await?;


    // ...И ПРОВЕРКИ, ПРИШЕДШИЕ ДРУГИМ ПУТЁМ. У `myack` доказательства не в
    // прозе `measured_by`, а в ячейках таблиц, и таблица доказательств
    // оставалась пустой — правило честно отвечало «судить нечем» при 363
    // живых проверках. Таблица собирает ВСЕ источники, иначе она не общая.
    tx.execute(
        "INSERT INTO project_requirement_proof (project_id, requirement_id, proof_kind, proof_id)
         SELECT project_id, requirement_id, 'check', id FROM project_checks
          WHERE project_id = $1 AND requirement_id IS NOT NULL AND requirement_id <> ''
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;
    tx.execute(
        "INSERT INTO project_requirement_proof (project_id, requirement_id, proof_kind, proof_id)
         SELECT project_id, requirement_id, 'check', check_id FROM project_check_requirements
          WHERE project_id = $1
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;

    // РАССУЖДЕНИЕ ИЗ ДОКУМЕНТОВ-КОНТЕЙНЕРОВ: проза раздела — всё, что не
    // таблица и не заголовок.
    //
    // Сперва я брал только разделы, которые НИЧЕГО не объявляют, и потерял
    // больше половины: из 27 прозаических разделов `srs` взялось 11, а 39851
    // байта — 13526. Остальное в разделах СМЕШАННЫХ: «3.16. STP» несёт и
    // таблицу требований, и шесть тысяч знаков о том, почему они такие.
    // Объявляет раздел сущности или нет — к его рассуждению отношения не имеет.
    //
    // Только контейнеры: у решения и вопроса проза — это сам документ, и она
    // уже лежит колонками `context`, `decision`, `consequences`.
    tx.execute("DELETE FROM project_rationale WHERE project_id = $1 AND origin = 'projected'",
               &[&project]).await?;
    tx.execute(
        "INSERT INTO project_rationale
                (project_id, id, entity_kind, entity_name, anchor, section_ord, title, body)
         SELECT * FROM (
         SELECT s.project_id,
                s.entity_kind || coalesce('/' || nullif(s.entity_name,''), '') || '#' || s.anchor,
                s.entity_kind, s.entity_name, s.anchor, s.ord, s.title,
                coalesce((SELECT string_agg(b.raw, E'\n' ORDER BY b.ord)
                   FROM project_document_blocks b
                  WHERE b.project_id = s.project_id AND b.entity_kind = s.entity_kind
                    AND b.entity_name = s.entity_name
                    AND b.ord > s.ord AND b.ord <= s.last_block
                    AND b.kind NOT IN ('heading','blank','table')
                    AND NOT EXISTS (SELECT 1 FROM project_document_sections c
                                     WHERE c.project_id = s.project_id AND c.entity_kind = s.entity_kind
                                       AND c.entity_name = s.entity_name
                                       AND c.ord > s.ord AND c.ord <= b.ord)), '') AS тело
           FROM project_document_sections s
           JOIN kind_layout k ON k.name = s.entity_kind AND k.spec->>'projection' = 'container'
          WHERE s.project_id = $1) z
          -- Раздел без собственного текста рассуждением не является: заголовок
          -- вернётся сам, когда соберутся его дети.
          WHERE z.тело <> ''
         ON CONFLICT DO NOTHING",
        &[&project],
    )
    .await?;

    tx.commit().await?;
    // ПЕРЕПИСАННЫЙ ИСТОЧНИК НАЗЫВАЕТСЯ ЧИСЛОМ. Проход молча правит, где написана
    // проверка; посчитать и выбросить значило бы прятать правку от того, кто её
    // потом ищет.
    Ok((requirements.len(), checks.len(), needs.len(), home))
}

/// ОТМЕТКА ИЗМЕНЕНИЯ. Отпечаток записи сверяется с прошлым: совпал — дата
/// держится, разошёлся — ставится новая.
///
/// ПЕРВАЯ отметка — ОДНА НА ВСЕХ, а не из документа-источника. Это ловилось
/// замером четырежды, и три раза я ошибался:
///     «сейчас»                  → 213 переоткрытых вопросов
///     текущая дата документа    → 210
///     первая ревизия документа  →  12, но 271 требование из 306
/// Последнее и показало ошибку: у разных документов разная первая ревизия,
/// и требование из `srs` выходило старше проверки из `test-cases`. Разница
/// была разницей ДОКУМЕНТОВ, а не правок записей — а то, что обновился
/// `srs`, о самой записи не говорит ничего.
///
/// Одна дата на всех значит «правок мы не видели ни одной». Это правда о
/// наборе, приехавшем переносом, и с неё каскад начинает считать честно.
pub(crate) async fn stamp(pool: &Pool, project: &str) -> Result<u64, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    let now = crate::projector::now_ms();
    tx.execute(STAMP_MIGRATION, &[&project, &now]).await?;
    tx.execute("UPDATE entity_stamp SET body_version = 2 WHERE project_id = $1 AND body_version < 2", &[&project])
        .await?;
    let changed = tx.execute(STAMP_CHANGED, &[&project, &now]).await?;
    paragraphs_with_which_time(&tx, project).await?;
    // Версия 4 — заголовок блока в теле (см. «первый вывод не двигает отметку»
    // в `project`). Отметки версии 3 переводит сама пересборка; здесь доходят
    // те, что были ниже и только что прошли шаг версии 3.
    tx.execute(
        "UPDATE entity_stamp SET body_version = 4 WHERE project_id = $1 AND kind = 'requirement' AND body_version < 4",
        &[&project],
    )
    .await?;
    tx.execute(STAMP_FRESH, &[&project, &now]).await?;
    tx.commit().await?;
    Ok(changed)
}

/// Отметка требования — с тех пор, как ДОКУМЕНТ говорит его абзац «Проверяется».
///
/// Разбор абзаца заменил объявленное поле, и у 186 требований tot-ade тело
/// сменилось без единой правки документа: отметка встала на «сейчас», и 43
/// закрытые записи, стоящие на них, переоткрылись ложно. Правка поля
/// проекцией — не правка записи.
///
/// Время берётся из истории ревизий: самая ранняя ревизия, начиная с которой
/// абзац непрерывно такой, как сейчас. Отметка только опускается к нему — и
/// один раз: шаг помечен версией тела 3.
async fn paragraphs_with_which_time(
    tx: &deadpool_postgres::Transaction<'_>,
    project: &str,
) -> Result<(), crate::db::Fail> {
    let waiting: i64 = tx
        .query_one(
            "SELECT count(*) FROM entity_stamp WHERE project_id = $1 AND kind = 'requirement' AND body_version < 3",
            &[&project],
        )
        .await?
        .get(0);
    if waiting == 0 {
        return Ok(());
    }
    if let Some(marker) = crate::scheme::Terms::load_at(tx, project).await?.one("marker.verified-by") {
        let revisions = tx
            .query(
                "SELECT entity_kind, entity_name, written_at, content FROM project_document_revisions
                  WHERE project_id = $1 AND content LIKE '%' || $2 || '%'
                  ORDER BY entity_kind, entity_name, written_at",
                &[&project, &marker],
            )
            .await?;
        let mut document: (String, String) = (String::new(), String::new());
        let mut was: HashMap<String, (String, i64)> = HashMap::new();
        let mut outcome: HashMap<String, Vec<(String, i64)>> = HashMap::new();
        let hand_over = |was: &mut HashMap<String, (String, i64)>, outcome: &mut HashMap<String, Vec<(String, i64)>>| {
            for (id, v) in was.drain() {
                outcome.entry(id).or_default().push(v);
            }
        };
        for r in &revisions {
            let (kind, name, at, content): (String, String, i64, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
            if (kind.as_str(), name.as_str()) != (document.0.as_str(), document.1.as_str()) {
                hand_over(&mut was, &mut outcome);
                document = (kind.clone(), name.clone());
            }
            let mut now: HashMap<String, (String, i64)> = HashMap::new();
            for (_, _, requirement_id, text_of) in paragraphs_checks(kind, name, &content, marker) {
                let Some(id) = requirement_id else { continue };
                if text_of.is_empty() || now.contains_key(&id) {
                    continue;
                }
                let start = match was.get(&id) {
                    Some((previous, long_ago)) if *previous == text_of => *long_ago,
                    _ => at,
                };
                now.insert(id, (text_of, start));
            }
            was = now;
        }
        hand_over(&mut was, &mut outcome);
        let fields = tx
            .query(
                "SELECT r.id, r.measured_by FROM project_requirements r
                   JOIN entity_stamp st ON st.project_id = r.project_id AND st.kind = 'requirement' AND st.id = r.id
                  WHERE r.project_id = $1 AND st.body_version < 3 AND r.measured_by <> ''",
                &[&project],
            )
            .await?;
        for r in &fields {
            let (id, field): (String, String) = (r.get(0), r.get(1));
            let Some(start) = outcome.get(&id).and_then(|v| v.iter().filter(|(t, _)| *t == field).map(|(_, long_ago)| *long_ago).min()) else {
                continue;
            };
            tx.execute(
                "UPDATE entity_stamp SET updated_at = $3
                  WHERE project_id = $1 AND kind = 'requirement' AND id = $2 AND updated_at > $3",
                &[&project, &id, &start],
            )
            .await?;
        }
    }
    tx.execute(
        "UPDATE entity_stamp SET body_version = 3 WHERE project_id = $1 AND kind = 'requirement' AND body_version < 3",
        &[&project],
    )
    .await?;
    Ok(())
}

const STAMP_MIGRATION: &str = "WITH old AS (SELECT 'requirement' AS kind, id, (to_jsonb(r.*) - 'project_id' - 'origin')::text AS body FROM project_requirements r WHERE project_id = $1
 UNION ALL SELECT 'check', id, (to_jsonb(c.*) - 'project_id' - 'origin')::text FROM project_checks c WHERE project_id = $1
 UNION ALL SELECT 'need', id, (to_jsonb(n.*) - 'project_id')::text FROM project_needs n WHERE project_id = $1
 UNION ALL SELECT 'decision', id, (to_jsonb(d.*) - 'project_id' - 'origin')::text FROM project_decisions d WHERE project_id = $1
 UNION ALL SELECT 'screen', id, (to_jsonb(s.*) - 'project_id' - 'origin')::text FROM project_screens s WHERE project_id = $1
 UNION ALL SELECT 'story', id, (to_jsonb(t.*) - 'project_id' - 'origin')::text FROM project_stories t WHERE project_id = $1
 UNION ALL SELECT 'task', id, (to_jsonb(p.*) - 'project_id' - 'origin')::text FROM project_plan_tasks p WHERE project_id = $1
 UNION ALL SELECT 'milestone', id, (to_jsonb(m.*) - 'project_id' - 'origin')::text FROM project_plan_milestones m WHERE project_id = $1
 UNION ALL SELECT 'question', id, (to_jsonb(q.*) - 'project_id' - 'origin' - 'created_at' - 'updated_at')::text FROM project_questions q WHERE project_id = $1
 UNION ALL SELECT 'rationale', id, (to_jsonb(a.*) - 'project_id' - 'origin')::text FROM project_rationale a WHERE project_id = $1)
UPDATE entity_stamp st
   SET text_hash = md5(e.body), body_version = 2,
       updated_at = CASE
         WHEN o.body IS NULL OR st.text_hash <> md5(o.body) THEN CASE WHEN e.origin <> 'declared' AND e.kind IN ('requirement', 'check', 'need', 'decision', 'screen', 'story', 'rationale')
                THEN coalesce((SELECT max(r.written_at) FROM project_document_revisions r
                  WHERE r.project_id = e.project_id AND r.entity_kind = e.entity_kind
                    AND r.entity_name = e.entity_name AND r.written_at <= $2), $2)
                ELSE $2 END
         WHEN e.origin <> 'declared' AND e.kind IN ('requirement', 'check', 'need', 'decision', 'screen', 'story', 'rationale')
           THEN coalesce((SELECT max(r.written_at) FROM project_document_revisions r
                  WHERE r.project_id = e.project_id AND r.entity_kind = e.entity_kind
                    AND r.entity_name = e.entity_name AND r.written_at <= st.updated_at), st.updated_at)
         ELSE st.updated_at END
  FROM entity_row e LEFT JOIN old o ON o.kind = e.kind AND o.id = e.id
 WHERE st.project_id = $1 AND st.body_version < 2
   AND e.project_id = st.project_id AND e.kind = st.kind AND e.id = st.id";

const STAMP_CHANGED: &str = "UPDATE entity_stamp st
   SET text_hash = md5(e.body), updated_at = $2
  FROM entity_row e
 WHERE st.project_id = $1 AND e.project_id = st.project_id AND e.kind = st.kind AND e.id = st.id
   AND st.text_hash <> md5(e.body)";

const STAMP_FRESH: &str = "INSERT INTO entity_stamp (project_id, kind, id, text_hash, created_at, updated_at, body_version)
SELECT e.project_id, e.kind, e.id, md5(e.body), нач.когда, нач.когда, 4
  FROM entity_row e
 CROSS JOIN (SELECT coalesce(min(written_at), $2) AS когда
               FROM project_document_revisions WHERE project_id = $1) нач
 WHERE e.project_id = $1
ON CONFLICT (project_id, kind, id) DO NOTHING";

#[cfg(test)]
mod tests {
    use super::{block_titles, paragraphs_checks};

    #[test]
    fn a_block_heading_is_the_title_up_to_its_closing_stars() {
        let doc = "**NFR-19 · Дополнение не задерживает кадр**\n*Проверяется:* бенч\n\n\
                   **FR-14 · Скоуп.** — пояснение\n\n**FR-09 · без закрытия\n\n**NFR-19 · повтор**";
        assert_eq!(block_titles(doc), vec![
            ("NFR-19".to_owned(), "Дополнение не задерживает кадр".to_owned()),
            ("FR-14".to_owned(), "Скоуп.".to_owned()),
        ], "заголовок — до закрывающих звёзд, первый по имени; блок без них заголовка не даёт");
    }

    /// Случай undassa/mh#133: заголовок блока переписан, пересборка обязана
    /// переписать и `title` требования, объявленного дверью. Блоки живут в srs
    /// (NFR-19) и в ui-spec (`FR-UI-…`); цитаты — в `feature` и `story`, по обе
    /// стороны от `srs` по алфавиту, и не побеждают ни в каком порядке чтения.
    /// Первый вывод (у `tot-ade` заголовки отличались одной точкой) не двигает
    /// отметку записи, если заголовок — единственная разница; правка заголовка
    /// после него и заголовок вместе с другим полем — двигают. Отметка,
    /// заведённая после перехода, и отметка ниже версии 3 тоже кончают версией
    /// 4. Абзац «Проверяется» цитаты не становится способом доказательства.
    /// Пустая строка, поданная двери, поле очищает — это договор `requirement-add`.
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_rewritten_heading_rewrites_the_title() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Dheading_title");
        let pool = crate::db::pool(&format!("{url}{apart}"), 2).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS heading_title CASCADE; CREATE SCHEMA heading_title;")
            .await.expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        let declare = |id: &'static str, title: Option<&'static str>, text: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                crate::projector::declare_requirement(&pool, "p", crate::projector::Requirement {
                    id, kind: None, area: None, title, text, measured_by: None, priority: None,
                }, false).await.expect("ответ двери")
            }
        };
        for (id, title) in [("NFR-19", "Дополнение отвечает в бюджете кадра"), ("FR-UI-01", "Кнопка видна"),
                            ("FR-UI-02", "Поле ввода"), ("FR-UI-03", "Метка")] {
            let v = declare(id, Some(title), Some("тело")).await;
            assert_eq!(v["status"], "declared", "{v}");
        }
        // Способ доказательства и область объявлены до документов: иначе первая
        // пересборка меняла бы и их, и отметка двигалась бы по законной причине.
        for (id, measured_by, area) in [("NFR-19", Some("бенч `a`"), None), ("FR-UI-01", None, Some("экран"))] {
            let v = crate::projector::declare_requirement(&pool, "p", crate::projector::Requirement {
                id, kind: None, area, title: None, text: Some("тело"), measured_by, priority: None,
            }, false).await.expect("ответ двери");
            assert_eq!(v["status"], "declared", "{v}");
        }
        // Отметки — как у набора до правки: сняты с прежнего тела, версия 3.
        // FR-UI-02 после отметки поправлен и по другому полю: его новая отметка
        // законна, и молчаливая перепись её бы спрятала.
        pool.get().await.expect("соединение")
            .batch_execute(
                "INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                                bytes, revision, updated_at, updated_by)
                 VALUES ('p', 'srs', '', E'# Требования\\n\\n**NFR-19 · Дополнение отвечает в бюджете кадра.**\\nТело.\\n*Проверяется:* бенч `a`\\n', '', 0, 1, 0, 't'),
                        ('p', 'ui-spec', '', E'# Экраны\\n\\n**FR-UI-01 · Кнопка видна.**\\n\\n**FR-UI-02 · Поле ввода.**\\n\\n**FR-UI-03 · Метка**\\n', '', 0, 1, 0, 't'),
                        ('p', 'feature', 'F-1', E'**NFR-19 · цитата в фиче.**\\n*Проверяется:* цитата-проверка\\n\\n**FR-UI-01 · цитата в фиче.**\\n', '', 0, 1, 0, 't'),
                        ('p', 'story', 'S-1', E'**NFR-19 · цитата в истории.**\\n\\n**FR-UI-01 · цитата в истории.**\\n', '', 0, 1, 0, 't');
                 INSERT INTO entity_stamp (project_id, kind, id, text_hash, created_at, updated_at, body_version)
                 SELECT project_id, kind, id, md5(body), 1, 1, 3 FROM entity_row
                  WHERE project_id = 'p' AND kind = 'requirement';
                 UPDATE project_requirements SET priority = 'О' WHERE project_id = 'p' AND id = 'FR-UI-02';
                 UPDATE entity_stamp SET body_version = 2 WHERE project_id = 'p' AND id = 'FR-UI-03';
                 INSERT INTO scheme_term (project_id, role, value) VALUES ('p', 'marker.verified-by', 'Проверяется:');")
            .await.expect("документы и отметки заводятся");
        let read = |id: &'static str| {
            let pool = pool.clone();
            async move {
                let client = pool.get().await.expect("соединение");
                let title: String = client
                    .query_one("SELECT title FROM project_requirements WHERE project_id = 'p' AND id = $1", &[&id])
                    .await.expect("требование на месте").get(0);
                let at: i64 = client
                    .query_one("SELECT updated_at FROM entity_stamp WHERE project_id = 'p' AND kind = 'requirement'
                                   AND id = $1", &[&id])
                    .await.expect("отметка на месте").get(0);
                (title, at)
            }
        };
        let field = |id: &'static str, sql: &'static str| {
            let pool = pool.clone();
            async move {
                pool.get().await.expect("соединение").query_one(sql, &[&id]).await.expect("поле читается")
                    .get::<_, String>(0)
            }
        };
        let version = |id: &'static str| {
            let pool = pool.clone();
            async move {
                pool.get().await.expect("соединение")
                    .query_one("SELECT body_version FROM entity_stamp WHERE project_id = 'p' AND kind = 'requirement'
                                   AND id = $1", &[&id])
                    .await.expect("отметка на месте").get::<_, i32>(0)
            }
        };
        let rebuild = || {
            let pool = pool.clone();
            async move {
                super::project(&pool, "p").await.expect("пересборка проходит");
                super::stamp(&pool, "p").await.expect("отметки ставятся");
            }
        };

        rebuild().await;
        assert_eq!(read("NFR-19").await, ("Дополнение отвечает в бюджете кадра.".to_owned(), 1),
                   "заголовок взят не из srs, или первый вывод сдвинул отметку записи");
        assert_eq!(read("FR-UI-01").await, ("Кнопка видна.".to_owned(), 1),
                   "заголовок взят не из ui-spec, или первый вывод сдвинул отметку записи");
        let (title, at) = read("FR-UI-02").await;
        assert_eq!(title, "Поле ввода.");
        assert!(at > 1, "правка другого поля рядом с заголовком принята в отпечаток молча");
        assert_eq!(field("NFR-19", "SELECT measured_by FROM project_requirements WHERE project_id = 'p' AND id = $1").await,
                   "бенч `a`", "способом доказательства стал абзац цитаты");
        assert_eq!(version("FR-UI-03").await, 4, "отметка ниже версии 3 не дошла до версии 4");

        // Требование, заведённое после перехода: его отметка заводится сразу
        // версией 4, и правка одного заголовка её двигает.
        declare("NFR-20", Some("Новое"), Some("тело")).await;
        pool.get().await.expect("соединение")
            .batch_execute("UPDATE project_documents SET content = content || E'\\n**NFR-20 · Новое**\\n'
                             WHERE project_id = 'p' AND entity_kind = 'srs'")
            .await.expect("блок дописывается");
        rebuild().await;
        pool.get().await.expect("соединение")
            .batch_execute("UPDATE entity_stamp SET updated_at = 1 WHERE project_id = 'p' AND id = 'NFR-20';
                            UPDATE project_documents SET content = replace(content, 'NFR-20 · Новое', 'NFR-20 · Новое имя')
                             WHERE project_id = 'p' AND entity_kind = 'srs'")
            .await.expect("заголовок нового переписывается");
        rebuild().await;
        let (title, at) = read("NFR-20").await;
        assert_eq!(title, "Новое имя");
        assert!(at > 1, "правка заголовка требования, заведённого после перехода, принята молча");

        pool.get().await.expect("соединение")
            .batch_execute("UPDATE project_documents SET content = replace(content, 'отвечает в бюджете кадра', 'не задерживает кадр')
                             WHERE project_id = 'p' AND entity_kind = 'srs'")
            .await.expect("заголовок переписывается");
        rebuild().await;
        let (title, at) = read("NFR-19").await;
        assert_eq!(title, "Дополнение не задерживает кадр.", "заголовок требования не взят из документа");
        assert!(at > 1, "правка заголовка после первого вывода не сдвинула отметку");

        // Заголовок поверх блока прожил бы до следующей пересборки — в любом
        // объявляющем виде.
        for id in ["NFR-19", "FR-UI-01"] {
            let over = declare(id, Some("поверх документа"), Some("тело")).await;
            assert_eq!(over["status"], "written_in_document", "заголовок поверх блока {id} принят: {over}");
        }
        // Способ доказательства поверх абзаца «Проверяется» — тот же случай.
        let over = crate::projector::declare_requirement(&pool, "p", crate::projector::Requirement {
            id: "NFR-19", kind: None, area: None, title: None, text: Some("тело"), measured_by: Some("поверх"), priority: None,
        }, false).await.expect("ответ двери");
        assert_eq!(over["status"], "written_in_document", "способ доказательства поверх абзаца принят: {over}");
        // Объявление без заголовка, как советует отказ, не затирает выведенный.
        let without = declare("NFR-19", None, Some("новое тело")).await;
        assert_eq!(without["status"], "declared", "{without}");
        assert_eq!(read("NFR-19").await.0, "Дополнение не задерживает кадр.", "объявление без title стёрло заголовок");
        // Поданная пустая строка — «очистить», а не «не подано».
        let cleared = crate::projector::declare_requirement(&pool, "p", crate::projector::Requirement {
            id: "FR-UI-01", kind: None, area: Some(""), title: None, text: Some("тело"), measured_by: None, priority: None,
        }, false).await.expect("ответ двери");
        assert_eq!(cleared["status"], "declared", "{cleared}");
        assert_eq!(field("FR-UI-01", "SELECT area FROM project_requirements WHERE project_id = 'p' AND id = $1").await,
                   "", "пустая строка не очистила поле");

        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS heading_title CASCADE").await.expect("схема снимается");
    }

    #[test]
    fn paragraph_checks_whole_and_without_tail() {
        let doc = "**FR-14 · Скоуп.**\n*Проверяется:* **компиляционный** род — `mirror_no_budget`\n\n\
                   **FR-08 · Раскладка якорная.**\nПорядок наследуется.\n*Проверяется:* тест `fr_08_keep` — снимок двух\nальтернатив.\n\n---\n\n# Часть II\n\n\
                   **FR-UI-40 · Почему.**\nТекст.\n*Проверяется:* тест `ui_40` — одно действие.\nПлюс сценарий `TC-HARN-07` — форма.\n\n\
                   **FR-09 · Без проверки.**\nТекст.\n\n# Часть III\n*Проверяется:* гейт `x:y`";
        let got = paragraphs_checks("srs".into(), "".into(), doc, "Проверяется:");
        let text = |id: &str| got.iter().find(|a| a.2.as_deref() == Some(id)).map(|a| a.3.as_str());
        assert_eq!(text("FR-14"), Some("**компиляционный** род — `mirror_no_budget`"), "жирное слово после маркера цело");
        assert_eq!(text("FR-08"), Some("тест `fr_08_keep` — снимок двух\nальтернатив."));
        assert_eq!(text("FR-UI-40"), Some("тест `ui_40` — одно действие.\nПлюс сценарий `TC-HARN-07` — форма."));
        assert_eq!(text("FR-09"), Some(""), "блок без абзаца говорит, что доказательство не названо");
        assert!(got.iter().any(|a| a.2.is_none() && a.3 == "гейт `x:y`"), "абзац после заголовка раздела ничьим требованием не становится");
    }
}
