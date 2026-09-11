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

pub async fn project(pool: &Pool, project: &str) -> Result<(usize, usize, usize), tokio_postgres::Error> {
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
    let маркеры = crate::scheme::Terms::load(pool, project).await?;
    if let Some(маркер) = маркеры.one("marker.verified-by") {
        let образцы = tx
            .query(
                "SELECT name, spec->>'id' FROM kind_layout
                  WHERE name IN ('check','lint-rule','assertion') AND spec->>'id' IS NOT NULL",
                &[],
            )
            .await?;
        let строки = tx
            .query(
                "SELECT d.entity_kind, d.entity_name, l.line
                   FROM project_documents d
                   CROSS JOIN LATERAL regexp_split_to_table(d.content, E'\n') l(line)
                  WHERE d.project_id = $1 AND l.line LIKE '%' || $2 || '%'",
                &[&project, &маркер],
            )
            .await?;
        let имя = regex::Regex::new(r"`([^`]{2,80})`").expect("образец имени в кавычках");
        let mut найдено: Vec<(String, String, String, String)> = Vec::new();
        let mut видели = std::collections::HashSet::new();
        for r in &строки {
            let (dk, dn): (String, String) = (r.get(0), r.get(1));
            let line: String = r.get(2);
            for c in имя.captures_iter(&line) {
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

    // ОТМЕТКА ИЗМЕНЕНИЯ. Отпечаток записи сверяется с прошлым: совпал — дата
    // держится, разошёлся — ставится новая.
    //
    // ПЕРВАЯ отметка — ОДНА НА ВСЕХ, а не из документа-источника. Это ловилось
    // замером четырежды, и три раза я ошибался:
    //     «сейчас»                  → 213 переоткрытых вопросов
    //     текущая дата документа    → 210
    //     первая ревизия документа  →  12, но 271 требование из 306
    // Последнее и показало ошибку: у разных документов разная первая ревизия,
    // и требование из `srs` выходило старше проверки из `test-cases`. Разница
    // была разницей ДОКУМЕНТОВ, а не правок записей — а то, что обновился
    // `srs`, о самой записи не говорит ничего.
    //
    // Одна дата на всех значит «правок мы не видели ни одной». Это правда о
    // наборе, приехавшем переносом, и с неё каскад начинает считать честно.
    tx.execute(
        "INSERT INTO entity_stamp (project_id, kind, id, text_hash, created_at, updated_at)
         SELECT e.project_id, e.kind, e.id, md5(e.body), нач.когда, нач.когда
           FROM entity_row e
           CROSS JOIN (SELECT coalesce(min(written_at), $2) AS когда
                         FROM project_document_revisions WHERE project_id = $1) нач
          WHERE e.project_id = $1
         ON CONFLICT (project_id, kind, id) DO UPDATE
            SET updated_at = CASE WHEN entity_stamp.text_hash <> EXCLUDED.text_hash
                                  THEN $2 ELSE entity_stamp.updated_at END,
                text_hash  = EXCLUDED.text_hash",
        &[&project, &crate::projector::now_ms()],
    )
    .await?;
    tx.commit().await?;
    Ok((requirements.len(), checks.len(), needs.len()))
}
