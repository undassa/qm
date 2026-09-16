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
    Regex::new(r"^\*\*((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)\s*·").expect("образец блока требования")
});

/// Абзацы «Проверяется» документа: вид, имя, требование блока и текст после маркера.
///
/// Требование — то, чей блок `**FR-NN · …**` открыт выше; заголовок раздела
/// блок закрывает. Абзац кончается пустой строкой, разделителем или заголовком:
/// склейка через пустую строку и дала полю хвост «--- # Часть II».
fn абзацы_проверки(kind: String, name: String, content: &str, маркер: &str) -> Vec<(String, String, Option<String>, String)> {
    let mut out = Vec::new();
    let mut блок: Option<(String, bool)> = None;
    let mut абзац: Option<(Option<String>, Vec<String>)> = None;
    let конец = |line: &str| {
        let t = line.trim_start();
        t.is_empty() || t.starts_with('#') || t.starts_with("---") || t.starts_with('|') || REQUIREMENT_BLOCK.is_match(t)
    };
    let закрыть_блок = |блок: Option<(String, bool)>, out: &mut Vec<(String, String, Option<String>, String)>| {
        if let Some((id, false)) = блок {
            out.push((kind.clone(), name.clone(), Some(id), String::new()));
        }
    };
    for line in content.lines() {
        if абзац.is_some() && конец(line) {
            let (req, lines) = абзац.take().expect("абзац открыт");
            out.push((kind.clone(), name.clone(), req, lines.join("\n")));
        }
        let t = line.trim_start();
        if let Some(m) = REQUIREMENT_BLOCK.captures(t) {
            закрыть_блок(блок.take(), &mut out);
            блок = Some((m[1].to_owned(), false));
        } else if t.starts_with('#') {
            закрыть_блок(блок.take(), &mut out);
        }
        if let Some((_, lines)) = абзац.as_mut() {
            lines.push(line.trim().to_owned());
        } else if let Some(at) = line.find(маркер) {
            // Снимается только выделение САМОГО маркера — `*Проверяется:*`.
            // Все звёзды подряд срезали и начало жирного слова за ним:
            // «**компиляционный**» выходило «компиляционный**».
            let после = &line[at + маркер.len()..];
            let после = ["**", "__", "*", "_"].iter().find_map(|m| после.strip_prefix(m)).unwrap_or(после);
            let хвост = после.trim().to_owned();
            if let Some((_, был)) = блок.as_mut() {
                *был = true;
            }
            абзац = Some((блок.as_ref().map(|b| b.0.clone()), vec![хвост]));
        }
    }
    if let Some((req, lines)) = абзац {
        out.push((kind.clone(), name.clone(), req, lines.join("\n")));
    }
    закрыть_блок(блок, &mut out);
    out
}

static REFERENCED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"`((?:FR|NFR)-[A-Z0-9]+(?:-\d+[a-z]?)?)`").expect("образец ссылки"));
static NEED_REFERENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bST-(\d+)\b").expect("образец потребности"));
/// Приоритет записан одной буквой: обязательное · желательное · позже.
static PRIORITY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*`?([ОЖП])`?\s*$").expect("образец приоритета"));
struct Requirement {
    id: String,
    секция: Option<i32>,
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
    секция: Option<i32>,
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

/// Объявляющий документ сильнее цитирующего — и только он перебивает уже взятое.
fn declares_over(kind: &str, existing: Option<&String>, home: &str) -> bool {
    match existing {
        None => true,
        Some(had) => kind == home && had != home,
    }
}

pub async fn project(
    pool: &Pool,
    project: &str,
) -> Result<(usize, usize, usize, u64), tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
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
    let mut секции: HashMap<(String, String), Vec<i32>> = HashMap::new();
    for r in &sec {
        секции.entry((r.get(0), r.get(1))).or_default().push(r.get(2));
    }
    let секция_для = |k: &str, n: &str, block: i32| -> Option<i32> {
        секции
            .get(&(k.to_owned(), n.to_owned()))
            .and_then(|v| v.iter().rev().find(|o| **o <= block).copied())
    };

    // Ячейки одной строки таблицы — вместе и в порядке появления.
    let mut order: Vec<(String, String, i32, i32)> = Vec::new();
    let mut buckets: HashMap<(String, String, i32, i32), Vec<(String, String)>> = HashMap::new();
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
                        секция: секция_для(doc_kind, doc_name, key.2) },
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
                секция: секция_для(doc_kind, doc_name, key.2),
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

    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    tx.execute("DELETE FROM project_checks WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    // ССЫЛКА ИЗ ТЕКСТА ТРЕБОВАНИЯ — отдельной строкой, а не растворённая в
    // связи документа. Имена берутся ОБЪЯВЛЕННЫМ раскрывателем `ids::plain`,
    // а не новым образцом: перечислить здесь `FR|NFR|ADR` значило бы зашить
    // слова, которыми владеет проект.
    //
    // Род связи — `cites`, и это всё, что разбор честно знает: зачем именно
    // требование сослалось, текст не говорит. Уточняется дверью.
    let mut ссылки: Vec<(String, String)> = Vec::new();
    for r in requirements.values() {
        for имя in super::ids::plain(&r.text) {
            if имя != r.id {
                ссылки.push((r.id.clone(), имя));
            }
        }
    }
    tx.execute(
        "DELETE FROM project_requirement_sources WHERE project_id = $1 AND origin = 'projected'",
        &[&project],
    )
    .await?;
    for (откуда, куда) in &ссылки {
        tx.execute(
            "INSERT INTO project_requirement_sources (project_id, requirement_id, kind, target, origin)
             VALUES ($1,$2,'requirement',$3,'projected') ON CONFLICT DO NOTHING",
            &[&project, откуда, куда],
        )
        .await?;
    }
    tx.execute("DELETE FROM project_requirement_needs WHERE project_id = $1", &[&project]).await?;
    tx.execute("DELETE FROM project_requirements WHERE project_id = $1 AND origin = 'projected'", &[&project]).await?;
    for r in requirements.values() {
        tx.execute(
            // Заголовок, версия и сквозной признак здесь не перечислены, и это
            // умышленно: их кладёт объявление, разбор их не выводит, и
            // переписать их пустотой значило бы стереть прочитанное.
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
              &r.satisfied, &r.priority, &r.measured_by, &r.секция],
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
              &c.секция],
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
    let маркеры = crate::scheme::Terms::load(pool, project).await?;
    let абзацы = match маркеры.one("marker.verified-by") {
        Some(маркер) => {
            let документы = tx
                .query(
                    "SELECT entity_kind, entity_name, content FROM project_documents
                      WHERE project_id = $1 AND content LIKE '%' || $2 || '%'
                      ORDER BY entity_kind, entity_name",
                    &[&project, &маркер],
                )
                .await?;
            документы
                .iter()
                .flat_map(|r| абзацы_проверки(r.get(0), r.get(1), &r.get::<_, String>(2), маркер))
                .collect()
        }
        None => Vec::new(),
    };
    let mut доказано: HashMap<&str, &str> = HashMap::new();
    for (_, _, требование, текст) in &абзацы {
        if let Some(id) = требование {
            let было = доказано.entry(id.as_str()).or_insert("");
            if было.is_empty() {
                *было = текст.as_str();
            }
        }
    }
    for (id, текст) in &доказано {
        tx.execute(
            "UPDATE project_requirements SET measured_by = $3
              WHERE project_id = $1 AND id = $2 AND measured_by IS DISTINCT FROM $3",
            &[&project, id, текст],
        )
        .await?;
    }

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
    if маркеры.one("marker.verified-by").is_some() {
        let образцы = tx
            .query(
                "SELECT name, spec->>'id' FROM kind_layout
                  WHERE name IN ('check','lint-rule','assertion') AND spec->>'id' IS NOT NULL",
                &[],
            )
            .await?;
        let имя = regex::Regex::new(r"`([^`]{2,80})`").expect("образец имени в кавычках");
        let mut найдено: Vec<(String, String, String, String)> = Vec::new();
        let mut видели = std::collections::HashSet::new();
        // Абзац вне блока требования читается одной строкой маркера, как
        // прежде: целиком он тащил в правила кода слова историй и решений.
        for (dk, dn, требование, абзац) in &абзацы {
            let line = if требование.is_some() { абзац.as_str() } else { абзац.lines().next().unwrap_or("") };
            for c in имя.captures_iter(line) {
                let id = c[1].to_owned();
                for обр in &образцы {
                    let вид: String = обр.get(0);
                    let pat: String = обр.get(1);
                    let подходит: bool = tx.query_one("SELECT $1 ~ $2", &[&id, &pat]).await?.get(0);
                    if подходит && видели.insert(format!("{вид}\u{1}{id}")) {
                        найдено.push((вид, id.clone(), dk.clone(), dn.clone()));
                        break;
                    }
                }
            }
        }
        for (таблица, вид) in [("project_lint_rules", "lint-rule"), ("project_assertions", "assertion")] {
            tx.execute(
                &format!("DELETE FROM {таблица} WHERE project_id = $1 AND origin = 'projected'"),
                &[&project],
            )
            .await?;
            for (в, id, dk, dn) in найдено.iter().filter(|x| x.0 == вид) {
                let _ = в;
                tx.execute(
                    &format!(
                        "INSERT INTO {таблица} (project_id, id, entity_kind, entity_name)
                         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING"
                    ),
                    &[&project, id, dk, dn],
                )
                .await?;
            }
        }
        for (в, id, dk, dn) in найдено.iter().filter(|x| x.0 == "check") {
            let _ = в;
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
    let дом = tx
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
    Ok((requirements.len(), checks.len(), needs.len(), дом))
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
pub async fn stamp(pool: &Pool, project: &str) -> Result<u64, tokio_postgres::Error> {
    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    let now = crate::projector::now_ms();
    tx.execute(STAMP_MIGRATION, &[&project, &now]).await?;
    tx.execute("UPDATE entity_stamp SET body_version = 2 WHERE project_id = $1 AND body_version < 2", &[&project])
        .await?;
    let changed = tx.execute(STAMP_CHANGED, &[&project, &now]).await?;
    абзацы_с_какого_времени(pool, &tx, project).await?;
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
async fn абзацы_с_какого_времени(
    pool: &Pool,
    tx: &deadpool_postgres::Transaction<'_>,
    project: &str,
) -> Result<(), tokio_postgres::Error> {
    let ждут: i64 = tx
        .query_one(
            "SELECT count(*) FROM entity_stamp WHERE project_id = $1 AND kind = 'requirement' AND body_version < 3",
            &[&project],
        )
        .await?
        .get(0);
    if ждут == 0 {
        return Ok(());
    }
    if let Some(маркер) = crate::scheme::Terms::load(pool, project).await?.one("marker.verified-by") {
        let ревизии = tx
            .query(
                "SELECT entity_kind, entity_name, written_at, content FROM project_document_revisions
                  WHERE project_id = $1 AND content LIKE '%' || $2 || '%'
                  ORDER BY entity_kind, entity_name, written_at",
                &[&project, &маркер],
            )
            .await?;
        let mut документ: (String, String) = (String::new(), String::new());
        let mut было: HashMap<String, (String, i64)> = HashMap::new();
        let mut итог: HashMap<String, Vec<(String, i64)>> = HashMap::new();
        let mut сдать = |было: &mut HashMap<String, (String, i64)>, итог: &mut HashMap<String, Vec<(String, i64)>>| {
            for (id, v) in было.drain() {
                итог.entry(id).or_default().push(v);
            }
        };
        for r in &ревизии {
            let (kind, name, at, content): (String, String, i64, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
            if (kind.as_str(), name.as_str()) != (документ.0.as_str(), документ.1.as_str()) {
                сдать(&mut было, &mut итог);
                документ = (kind.clone(), name.clone());
            }
            let mut сейчас: HashMap<String, (String, i64)> = HashMap::new();
            for (_, _, требование, текст) in абзацы_проверки(kind, name, &content, маркер) {
                let Some(id) = требование else { continue };
                if текст.is_empty() || сейчас.contains_key(&id) {
                    continue;
                }
                let с = match было.get(&id) {
                    Some((прежний, с)) if *прежний == текст => *с,
                    _ => at,
                };
                сейчас.insert(id, (текст, с));
            }
            было = сейчас;
        }
        сдать(&mut было, &mut итог);
        let поля = tx
            .query(
                "SELECT r.id, r.measured_by FROM project_requirements r
                   JOIN entity_stamp st ON st.project_id = r.project_id AND st.kind = 'requirement' AND st.id = r.id
                  WHERE r.project_id = $1 AND st.body_version < 3 AND r.measured_by <> ''",
                &[&project],
            )
            .await?;
        for r in &поля {
            let (id, поле): (String, String) = (r.get(0), r.get(1));
            let Some(с) = итог.get(&id).and_then(|v| v.iter().filter(|(t, _)| *t == поле).map(|(_, с)| *с).min()) else {
                continue;
            };
            tx.execute(
                "UPDATE entity_stamp SET updated_at = $3
                  WHERE project_id = $1 AND kind = 'requirement' AND id = $2 AND updated_at > $3",
                &[&project, &id, &с],
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
SELECT e.project_id, e.kind, e.id, md5(e.body), нач.когда, нач.когда, 3
  FROM entity_row e
 CROSS JOIN (SELECT coalesce(min(written_at), $2) AS когда
               FROM project_document_revisions WHERE project_id = $1) нач
 WHERE e.project_id = $1
ON CONFLICT (project_id, kind, id) DO NOTHING";

#[cfg(test)]
mod tests {
    use super::абзацы_проверки;

    #[test]
    fn абзац_проверки_целиком_и_без_хвоста() {
        let doc = "**FR-14 · Скоуп.**\n*Проверяется:* **компиляционный** род — `mirror_no_budget`\n\n\
                   **FR-08 · Раскладка якорная.**\nПорядок наследуется.\n*Проверяется:* тест `fr_08_keep` — снимок двух\nальтернатив.\n\n---\n\n# Часть II\n\n\
                   **FR-UI-40 · Почему.**\nТекст.\n*Проверяется:* тест `ui_40` — одно действие.\nПлюс сценарий `TC-HARN-07` — форма.\n\n\
                   **FR-09 · Без проверки.**\nТекст.\n\n# Часть III\n*Проверяется:* гейт `x:y`";
        let got = абзацы_проверки("srs".into(), "".into(), doc, "Проверяется:");
        let text = |id: &str| got.iter().find(|a| a.2.as_deref() == Some(id)).map(|a| a.3.as_str());
        assert_eq!(text("FR-14"), Some("**компиляционный** род — `mirror_no_budget`"), "жирное слово после маркера цело");
        assert_eq!(text("FR-08"), Some("тест `fr_08_keep` — снимок двух\nальтернатив."));
        assert_eq!(text("FR-UI-40"), Some("тест `ui_40` — одно действие.\nПлюс сценарий `TC-HARN-07` — форма."));
        assert_eq!(text("FR-09"), Some(""), "блок без абзаца говорит, что доказательство не названо");
        assert!(got.iter().any(|a| a.2.is_none() && a.3 == "гейт `x:y`"), "абзац после заголовка раздела ничьим требованием не становится");
    }
}
