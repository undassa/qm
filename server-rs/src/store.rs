//! Запись документа: **тот же порядок, что у донора, и на Rust.**
//!
//! Порядок существен, и он перенесён целиком: взять строку под замок, сверить
//! ожидаемую правку, узнать неизменность по хешу, поднять номер правки, положить
//! документ и его копию в летопись, разобрать заново структуру. Разойдись здесь
//! хоть шагом — и набор получит правку без летописи либо структуру от прошлого
//! текста.

use deadpool_postgres::Pool;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::parse::{parse_document, Block};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) fn hash_document(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}


/// Завести НОВЫЙ документ.
///
/// Отдельно от `put`, и это не удобство. `put` правит существующее: он берёт
/// строку под замок, сверит ожидаемую правку и откажет `not_found`, если строки
/// нет. Разрешить ему заводить значило бы превратить опечатку в имени в молча
/// созданный второй документ — и оба выглядели бы одинаково настоящими.
///
/// Вид обязан быть объявлен раскладкой и быть документом: сущность, объявленная
/// ВНУТРИ другого документа (требование, статья, термин), своего документа не
/// имеет, и заводить его для неё — заводить двойника.
/// Пустой текст — не документ, а след сломанной подачи: при заполненном
/// `/tmp` `content=@файл` читал пустой файл, и дверь заводила пустые карточки
/// задач с ответом «записано» (tot-space, 2026-10-07, три штуки).
fn empty_text(content: &str) -> Option<Value> {
    content.trim().is_empty().then(|| json!({ "status": "empty",
        "why": "текст документа пуст: пустой документ не заводится и не пишется поверх. Если текст шёл файлом (`content=@файл`), проверьте файл и место на диске" }))
}

pub(crate) async fn create(pool: &Pool, kinds: &crate::kinds::Kinds, project: &str, fields: Document<'_>, author: &str, now_ms: i64) -> Result<Value, crate::entities::Miss> {
    let Document { kind, name, content } = fields;
    let Some(declared) = kinds.get(kind) else {
        return Ok(json!({ "status": "unknown_kind",
                          "why": format!("вид «{kind}» раскладкой не объявлен") }));
    };
    if declared.is_inner() {
        return Ok(json!({ "status": "not_a_document",
                          "why": format!("вид «{kind}» объявляется внутри другого документа: своего у него нет") }));
    }
    // У одиночки имя заменяет сам вид; у остальных имя обязано быть и обязано
    // подходить под объявленный образец. Безымянный документ вида, где имена
    // есть, потом не найти ничем.
    if declared.single && !name.is_empty() {
        return Ok(json!({ "status": "named_singleton",
                          "why": format!("вид «{kind}» одиночка: имени у него нет") }));
    }
    if !declared.single {
        if name.is_empty() {
            return Ok(json!({ "status": "nameless", "why": "документ без имени не заводится" }));
        }
        if !crate::entities::matches_id_of(pool, project, kind, declared, name).await? {
            return Ok(json!({ "status": "bad_name",
                              "why": format!("имя «{name}» не подходит под образец вида «{kind}»") }));
        }
    }
    if let Some(refused) = empty_text(content) {
        return Ok(refused);
    }
    let bytes = content.len();
    if bytes > MAX_DOCUMENT_BYTES {
        return Ok(json!({ "status": "too_big", "bytes": bytes }));
    }

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    let content_hash = hash_document(content);
    let bytes32 = bytes as i32;
    // ЛЕТОПИСЬ ПРОДОЛЖАЕТСЯ, А НЕ НАЧИНАЕТСЯ ЗАНОВО.
    //
    // Снятие документа летопись не трогает — и не должно: `history` и
    // `at-revision` читают её, а снятая запись, у которой стёрли прошлое,
    // перестаёт быть историей. Но заведение писало ревизию 1 всегда, а у
    // летописи есть ключ `(проект, вид, имя, ревизия)`. Значит имя, однажды
    // снятое, завести обратно было НЕЛЬЗЯ НИКОГДА: вставка падала «база не
    // ответила», и содержимое, если его не сохранили на диск заранее, пропадало.
    // Так в tot-ade сгорело 295 имён — весь след переименований, которые вели
    // обходом «завести новое → снять старое».
    //
    // Номер берётся следующим за прожитым этим именем. Единица у нового имени,
    // шестая — у имени, прожившего пять правок до снятия, и это правда о нём.
    // Номер считается ВНУТРИ той же команды, что занимает имя, а не до неё.
    // Отдельный `SELECT` замка не берёт — строки документа в этот миг ещё нет,
    // брать нечего, — и между ним и вставкой успевала пройти чужая жизнь имени:
    // завести, править, снять. Тогда номер оказывался уже занятым, вставка в
    // летопись падала на уникальности, и содержимое пропадало ровно так, как
    // эта правка и взялась чинить.
    //
    // `ON CONFLICT DO NOTHING` вместо проверки-и-вставки: двое заводящих
    // одновременно прочли бы «нет такого» оба, и второй затёр бы первого.
    let busy = tx
        .query_opt(
            "INSERT INTO project_documents
                (project_id, entity_kind, entity_name, content, content_hash, bytes,
                 revision, updated_at, updated_by, author)
             VALUES ($1,$2,$3,$4,$5,$6,
                     (SELECT coalesce(max(revision), 0) + 1 FROM project_document_revisions
                       WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3),
                     $7,$8,$8)
             ON CONFLICT (project_id, entity_kind, entity_name) DO NOTHING
             RETURNING revision",
            &[&project, &kind, &name, &content, &content_hash, &bytes32, &now_ms, &author],
        )
        .await?;
    let Some(row) = busy else {
        return Ok(json!({ "status": "exists",
                          "why": "документ с таким видом и именем уже есть: заводить нечего" }));
    };
    let revision: i64 = row.get(0);
    tx.execute(
        "INSERT INTO project_document_revisions
            (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, written_at, written_by)
         VALUES ($1,$2,$3,$4,$5,$6,$9,$7,$8)",
        &[&project, &kind, &name, &content, &content_hash, &bytes32, &now_ms, &author, &revision],
    )
    .await?;
    let torn = write_structure(&tx, project, kind, name, content).await?;
    crate::watch::mark(&tx, project, "заведён документ").await?;
    tx.commit().await?;
    let mut answer = json!({ "status": "created", "kind": kind, "name": name,
                            "revision": revision, "bytes": bytes });
    // Слово говорится только когда есть что сказать: всегда пустое `why` веб
    // читает как отказ без довода.
    if revision > 1 {
        answer["why"] = json!("имя уже жило: номер продолжает его летопись, а не начинает заново");
    }
    Ok(with_torn(answer, torn))
}

/// Записать сущность целиком.
///
/// Адресуется ВИДОМ И ИМЕНЕМ. Путь не приходит снаружи и ключом не служит: он
/// читается у найденной строки и пишется как поле происхождения — то, откуда
/// документ когда-то взялся. Пока он ещё первичный ключ, писать по нему
/// приходится; но выбирать, КУДА писать, он уже не может.
pub(crate) async fn put(pool: &Pool, project: &str, fields: Document<'_>, author: &str, expected_revision: Option<i64>, now_ms: i64) -> Result<Value, crate::db::Fail> {
    let Document { kind, name, content } = fields;
    if kind.is_empty() {
        return Ok(json!({ "status": "not_found",
                          "why": "вид документа не назван, а без вида адреса нет: `kind=` обязателен" }));
    }
    if let Some(refused) = empty_text(content) {
        return Ok(refused);
    }
    let bytes = content.len();
    if bytes > MAX_DOCUMENT_BYTES {
        return Ok(json!({ "status": "invalid_path" }));
    }
    let content_hash = hash_document(content);

    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    // Строка берётся под замок до сверки правки: без него двое пишущих прочли бы
    // одну и ту же правку и второй затёр бы первого, не увидев конфликта.
    let current = tx
        .query(
            "SELECT revision, content_hash FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 FOR UPDATE",
            &[&project, &kind, &name],
        )
        .await?;
    // Сущности нет — писать некуда. Завести новую нечем: путь снаружи не
    // приходит, а первичным ключом он ещё остаётся. Отказ называется словом, а
    // не молчаливым созданием документа под выдуманным адресом.
    let Some(row) = current.first() else {
        // Отказ НАЗЫВАЕТ ДВЕРЬ, в которую идти. Голое `not_found` верно и
        // бесполезно: `put` правит написанное, а заводит `document-add`, и
        // догадаться об этом из одного слова нельзя.
        return Ok(json!({
            "status": "not_found",
            "why": format!(
                "документа «{kind} {name}» нет, а `put` только ПРАВИТ написанное. \
                 Завести его — `document-add kind={kind} id={name} content=…`; \
                 после этого `put` будет писать в него."),
        }));
    };
    let seen: i64 = row.get(0);
    if let Some(expected) = expected_revision {
        if expected != seen {
            return Ok(json!({ "status": "conflict", "revision": seen }));
        }
    }
    if row.get::<_, String>(1) == content_hash {
        return Ok(json!({ "status": "unchanged", "revision": seen, "bytes": bytes }));
    }

    let revision = seen + 1;
    let bytes32 = bytes as i32;
    // Правка, а не вставка-с-разрешением-конфликта. Строка уже взята под замок
    // выше, и её отсутствие отвечено `not_found`; `ON CONFLICT (project_id, path)`
    // здесь требовал уникального указателя по пути — того самого, который
    // уходит вместе с путём.
    tx.execute(
        "UPDATE project_documents SET
           content = $4, content_hash = $5, bytes = $6, revision = $7, updated_at = $8, updated_by = $9
          WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
        &[&project, &kind, &name, &content, &content_hash, &bytes32, &revision, &now_ms, &author],
    )
    .await?;
    tx.execute(
        "INSERT INTO project_document_revisions
            (project_id, entity_kind, entity_name, content, content_hash, bytes, revision, written_at, written_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        &[&project, &kind, &name, &content, &content_hash, &bytes32, &revision, &now_ms, &author],
    )
    .await?;
    let torn = write_structure(&tx, project, kind, name, content).await?;

    // ЛЕТОПИСЬ ЗДЕСЬ НЕ ПИШЕТСЯ, и это решение, а не пропуск.
    //
    // Она уже пишется строкой выше — в `project_document_revisions`, вместе с
    // содержимым, хешем, временем и именем писавшего. Я было дописал событие и
    // в `entity_event` — и оно исчезло на следующей же пересборке: та таблица
    // не летопись, а ПРОЕКЦИЯ того, что документы говорят о себе, и пересборка
    // её удаляет и пишет заново. Дописанное в проекцию теряется молча, а лог,
    // теряющий записи, хуже отсутствующего.
    // Документ изменился — гейты пора мерить заново. Отметка ставится в той же
    // транзакции: она фиксируется вместе с правкой или не фиксируется вовсе, и
    // второго соединения из пула не просит.
    crate::watch::mark(&tx, project, "правка документа").await?;
    tx.commit().await?;
    Ok(with_torn(json!({ "status": "written", "revision": revision, "bytes": bytes }), torn))
}

/// Дописать к ответу то, о чём предупреждает разбор.
///
/// ПУСТОГО ПОЛЯ НЕ БЫВАЕТ: всегда присутствующее «предупреждений нет» читается
/// как украшение и перестаёт замечаться на второй день. Поле появляется ровно
/// тогда, когда есть что сказать.
fn with_torn(mut answer: Value, torn: Vec<String>) -> Value {
    if !torn.is_empty() {
        answer["таблицы"] = json!(torn);
    }
    answer
}

/// Что за документ и что в нём: одни и те же три поля у заведения и у правки.
pub(crate) struct Document<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub content: &'a str,
}

/// Поля правки одного раздела документа.
pub(crate) struct SectionEdit<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub anchor: &'a str,
    pub body: &'a str,
}

/// Заменить один раздел документа вместе с его подразделами.
pub(crate) async fn put_section(pool: &Pool, project: &str, fields: SectionEdit<'_>, author: &str, expected_revision: Option<i64>, now_ms: i64) -> Result<Value, crate::db::Fail> {
    let SectionEdit { kind, name, anchor, body } = fields;
    let client = crate::db::conn(pool).await?;
    let rows = client
        .query(
            "SELECT content FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?;
    let Some(row) = rows.first() else {
        return Ok(json!({
            "status": "not_found",
            "why": format!(
                "документа «{kind} {name}» нет, а `put-section` правит раздел написанного. \
                 Завести — `document-add kind={kind} id={name} content=…`."),
        }));
    };
    let content: String = row.get(0);
    drop(client);
    match splice_section(&content, anchor, body) {
        Splice::NoSection => Ok(json!({ "status": "no_such_section" })),
        Splice::Drops(dropped) => Ok(json!({
            "status": "drops_sections",
            "dropped": dropped,
            "why": format!(
                "раздел «{anchor}» заменяется вместе с подразделами, а в новом теле нет {} из них: {}. \
                 Возьмите тело дверью `section` и правьте его целиком, правьте подразделы порознь \
                 или, чтобы убрать подраздел, запишите документ целиком через `put`.",
                dropped.len(), dropped.join(", ")),
        })),
        Splice::Done(next) => put(pool, project, Document { kind, name, content: &next }, author, expected_revision, now_ms).await,
    }
}

/// Чем кончилась подстановка раздела в документ.
enum Splice {
    Done(String),
    NoSection,
    /// Якоря подразделов, которых в новом теле нет.
    Drops(Vec<String>),
}

/// Тело раздела заменяет и его подразделы: границы те же, какими раздел читает
/// дверь `section`, и прочитанное ею тело ложится обратно без потерь.
///
/// Тело, в котором подраздела нет, отказано. `put-section` по шапке
/// `# Указатель вопросов` с телом до первого `## ` снёс три подраздела и таблицу
/// на 171 вопрос, а ответ был тем же «записано» (undassa/mh#123). Убрать
/// подраздел можно записью документа целиком — там потерю видит сам пишущий.
fn splice_section(content: &str, anchor: &str, body: &str) -> Splice {
    let structure = parse_document(content);
    let Some(section) = structure.sections.iter().find(|s| s.anchor == anchor) else {
        return Splice::NoSection;
    };
    let normalized = if body.is_empty() || body.ends_with('\n') { body.to_owned() } else { format!("{body}\n") };
    // Счёт, а не множество: два подраздела с одним заголовком («### Решение» под
    // двумя вопросами) — тело с одним из них теряет второй.
    let mut kept: Vec<String> = parse_document(&normalized).sections.into_iter().map(|s| s.anchor).collect();
    let dropped: Vec<String> = structure.sections.iter()
        .filter(|s| s.ord > section.first_block && s.ord <= section.last_block)
        .filter(|s| match kept.iter().position(|k| *k == s.anchor) {
            Some(i) => { kept.swap_remove(i); false }
            None => true,
        })
        .map(|s| s.anchor.clone())
        .collect();
    if !dropped.is_empty() {
        return Splice::Drops(dropped);
    }
    let head: Vec<Block> = structure.blocks.iter().filter(|b| b.ord <= section.first_block).cloned().collect();
    let tail: Vec<Block> = structure.blocks.iter().filter(|b| b.ord > section.last_block).cloned().collect();
    Splice::Done(format!(
        "{}{normalized}{}",
        crate::parse::render_document(&head),
        crate::parse::render_document(&tail)
    ))
}

/// Удалить документ вместе с его разбором.
///
/// Донор чистил только документ: в базе от двух удалённых остались 132 блока,
/// 739 ячеек и 242 ссылки. Ничего живого они не задевали лишь по случайности —
/// осиротевшая ссылка на живой документ попала бы в обратные ссылки.
pub(crate) async fn remove(pool: &Pool, project: &str, kind: &str, name: &str) -> Result<Value, crate::db::Fail> {
    // Безымянный документ не удаляется. Пустые вид и имя — не адрес одного
    // документа, а условие, под которое в песочных проектах подходят все сразу:
    // `p6` держит четыре таких, и одно удаление снесло бы четыре.
    if kind.is_empty() {
        return Ok(json!({ "status": "not_found",
                          "why": "вид документа не назван, а без вида адреса нет: `kind=` обязателен" }));
    }
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    let gone = tx
        .execute(
            "DELETE FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &kind, &name],
        )
        .await?;
    for table in DERIVED {
        tx.execute(
            &format!("DELETE FROM {table} WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3"),
            &[&project, &kind, &name],
        )
        .await?;
    }
    if gone > 0 {
        crate::watch::mark(&tx, project, "документ снят").await?;
    }
    tx.commit().await?;
    Ok(json!({ "status": if gone > 0 { "deleted" } else { "not_found" } }))
}

/// Производное ОДНОГО текста документа: разбор, который переписывается вместе с
/// ним и снимается вместе с ним. Список стоял тремя копиями — при снятии
/// документа, при подметании сирот и при перезаписи, — и шестая таблица попала
/// бы в одну-две из них, а строки оставшейся протекли бы молча.
const DERIVED: [&str; 5] = [
    "project_document_blocks",
    "project_document_sections",
    "project_document_cells",
    "project_document_links",
    "project_document_fields",
];

/// Убрать разбор документов, которых больше нет.
pub(crate) async fn sweep_orphans(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    let mut swept = serde_json::Map::new();
    for table in DERIVED {
        let n = tx
            .execute(
                &format!(
                    "DELETE FROM {table} t WHERE t.project_id = $1 AND NOT EXISTS
                       (SELECT 1 FROM project_documents d WHERE d.project_id = t.project_id AND d.entity_kind = t.entity_kind AND d.entity_name = t.entity_name)"
                ),
                &[&project],
            )
            .await?;
        swept.insert(table.to_owned(), json!(n));
    }
    tx.commit().await?;
    Ok(Value::Object(swept))
}

/// Перечитать разбор всего набора, не трогая содержания.
///
/// Разбор перечитывается записью, а запись с тем же текстом отвечает
/// `unchanged` и правильно делает. Но правило разбора меняется отдельно от
/// текста — как сменилось, когда ссылки стали адресовать сущность, — и тогда
/// набору нужен проход, который перечитает старый текст новым правилом.
pub(crate) async fn reparse_all(pool: &Pool, project: &str, _: &crate::watch::Lease) -> Result<Value, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let docs = client
        .query(
            "SELECT entity_kind, entity_name FROM project_documents
              WHERE project_id = $1 ORDER BY entity_kind, entity_name",
            &[&project],
        )
        .await?;
    drop(client);
    let mut done = 0usize;
    let mut uneven: Vec<String> = Vec::new();
    for d in &docs {
        let (kind, name): (String, String) = (d.get(0), d.get(1));
        let mut c = crate::db::conn(pool).await?;
        let tx = c.transaction().await?;
        // Текст читается здесь, под замком строки, как у `put`, а не одним
        // запросом заранее: правка, прошедшая посреди прохода, иначе получала
        // бы разбор прежнего текста, а две записи разбора одного документа
        // разом сносили и вставляли бы его строки наперегонки.
        let Some(row) = tx
            .query_opt(
                "SELECT content FROM project_documents
                  WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3 FOR UPDATE",
                &[&project, &kind, &name],
            )
            .await?
        else {
            continue;
        };
        let content: String = row.get(0);
        let torn = write_structure(&tx, project, &kind, &name, &content).await?;
        tx.commit().await?;
        done += 1;
        for note in torn {
            uneven.push(format!("{kind} {name}: {note}"));
        }
    }
    Ok(with_torn(json!({ "reparsed": done }), uneven))
}

/// Куда ведёт ссылка: вид и имя цели.
///
/// Цель формы `вид:имя` спрашивается у документов напрямую. Вид, названный не
/// тем, каким документ записан (`task:R-M1-T1` о красной задаче), разрешается
/// предметной таблицей — и найденный документ отдаёт СВОЁ имя, то, под которым
/// он записан, а не то, каким его позвали.
///
/// Цель формы «путь к файлу» не разрешается больше НИКАК, и это следствие, а не
/// упущение: относительный путь разрешался относительно пути документа, а пути
/// у документа нет. Такие цели записываются как есть, с пустым именем — ссылка,
/// ведущая в файловую эпоху, названа, а не выдана за разрешённую. Набор от них
/// уже очищен проходом `links-retarget`: 1999 целей переписаны в `вид:имя`,
/// 66 остались путями потому, что не вели ни к одному документу и раньше.
async fn link_target(
    tx: &deadpool_postgres::Transaction<'_>,
    project: &str,
    target: &str,
) -> Result<(String, String), crate::db::Fail> {
    // Цель формы «путь к файлу» не разрешается больше НИКАК, и это следствие,
    // а не упущение: относительный путь разрешался относительно пути документа,
    // а пути у документа нет. Набор от таких целей очищен проходом
    // `links-retarget`: 2000 переписаны в `вид:имя`, 66 остались путями потому,
    // что не вели ни к одному документу и раньше.
    let Some((k, id)) = target.split_once(':') else {
        return Ok(Default::default());
    };
    if k.is_empty() || k.contains('/') || k.contains('.') {
        return Ok(Default::default());
    }
    let direct = tx
        .query(
            "SELECT entity_kind, entity_name FROM project_documents
              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
            &[&project, &k, &id],
        )
        .await?;
    if let Some(r) = direct.first() {
        return Ok((r.get(0), r.get(1)));
    }
    let rows = tx
        .query(
            "SELECT d.entity_kind, d.entity_name FROM project_documents d
              WHERE d.project_id = $1 AND (d.entity_kind, d.entity_name) = (
                SELECT entity_kind, entity_name FROM (
                   SELECT id, entity_kind, entity_name FROM project_decisions WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_questions WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_plan_tasks WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_stories WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_screens WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_requirements WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_checks WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_needs WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_features WHERE project_id = $1
                   -- Внутренние виды тоже адресуют: `risk:R-01` ведёт к `risks`,
                   -- `term:...` — к словарю. Их не было в перечне, и семнадцать
                   -- ссылок на риск разрешались лишь потому, что документ был
                   -- ошибочно назван `risk R-01`. Имя исправили — и промах стал
                   -- виден. Он был всегда.
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_risks WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_terms WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_plan_milestones WHERE project_id = $1
                   UNION ALL SELECT id, entity_kind, entity_name FROM project_runs_log WHERE project_id = $1
                 ) e WHERE e.id = $2 AND e.entity_kind <> '' LIMIT 1)",
            &[&project, &id],
        )
        .await?;
    Ok(rows.first().map(|r| (r.get(0), r.get(1))).unwrap_or_default())
}

/// Разложить документ по таблицам — и вернуть то, о чём стоит предупредить
/// пишущего. Предупреждение, а не отказ: таблица с лишней клеткой бывает и
/// намеренной, а запись, отбитая догадкой, стоит дороже разъехавшейся колонки.
async fn write_structure(
    tx: &deadpool_postgres::Transaction<'_>,
    project: &str,
    kind: &str,
    name: &str,
    content: &str,
) -> Result<Vec<String>, crate::db::Fail> {
    for table in DERIVED {
        tx.execute(
            &format!("DELETE FROM {table} WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3"),
            &[&project, &kind, &name],
        )
        .await?;
    }
    let s = parse_document(content);
    for b in &s.blocks {
        tx.execute(
            "INSERT INTO project_document_blocks(project_id, entity_kind, entity_name, ord, kind, level, raw)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[&project, &kind, &name, &b.ord, &b.kind, &b.level, &b.raw],
        )
        .await?;
    }
    for x in &s.sections {
        tx.execute(
            "INSERT INTO project_document_sections
                (project_id, entity_kind, entity_name, ord, level, title, anchor, parent_ord, first_block, last_block)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[&project, &kind, &name, &x.ord, &x.level, &x.title, &x.anchor, &x.parent_ord, &x.first_block, &x.last_block],
        )
        .await?;
    }
    for c in &s.cells {
        tx.execute(
            "INSERT INTO project_document_cells(project_id, entity_kind, entity_name, block_ord, row_ord, col, raw, value)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            &[&project, &kind, &name, &c.block_ord, &c.row, &c.col, &c.raw, &c.value],
        )
        .await?;
    }
    for l in &s.links {
        let (target_kind, target_name) = link_target(tx, project, &l.target_path).await?;
        tx.execute(
            "INSERT INTO project_document_links
                (project_id, entity_kind, entity_name, block_ord, ord, label,
                 target_path, target_anchor, target_kind, target_name)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[&project, &kind, &name, &l.block_ord, &l.ord, &l.text,
              &l.target_path, &l.target_anchor, &target_kind, &target_name],
        )
        .await?;
    }
    for f in &s.fields {
        tx.execute(
            "INSERT INTO project_document_fields
                (project_id, entity_kind, entity_name, section_ord, ord, name, shape, value_raw, value)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[&project, &kind, &name, &f.section_ord, &f.ord, &f.name, &f.shape, &f.value_raw, &f.value],
        )
        .await?;
    }
    Ok(crate::parse::ragged_rows(&s.cells))
}

#[cfg(test)]
mod splice_tests {
    use super::{splice_section, Splice};

    const DOC: &str = "# Указатель\n\nОткрытых: 38\n\n## Открытые\n\nq1\n\n## Закрытые\n\nq2\n";

    /// Случай undassa/mh#123: тело шапки без подразделов снесло бы их молча.
    #[test]
    fn a_body_without_the_subsections_is_refused() {
        match splice_section(DOC, "указатель", "\nОткрытых: 39\n\n") {
            Splice::Drops(d) => assert_eq!(d, ["открытые", "закрытые"]),
            Splice::Done(next) => panic!("тело без подразделов принято: {next:?}"),
            Splice::NoSection => panic!("раздел не найден"),
        }
    }

    /// Тело, прочитанное дверью `section` и поправленное, ложится на место.
    #[test]
    fn a_body_with_the_subsections_replaces_the_section() {
        match splice_section(DOC, "указатель", "\nОткрытых: 39\n\n## Открытые\n\nq1\n\n## Закрытые\n\nq2\n") {
            Splice::Done(next) => assert_eq!(next, DOC.replace("38", "39")),
            _ => panic!("тело с подразделами отказано"),
        }
    }

    #[test]
    fn a_leaf_section_is_replaced_alone() {
        match splice_section(DOC, "открытые", "\nq1, q3\n\n") {
            Splice::Done(next) => assert_eq!(next, DOC.replace("q1\n", "q1, q3\n")),
            _ => panic!("лист отказан"),
        }
    }

    /// Два подраздела с одним заголовком: тело, сохранившее один, теряет второй.
    #[test]
    fn a_repeated_subsection_is_counted() {
        let doc = "# Вопросы\n\n## Q-1\n\n### Решение\n\nа\n\n## Q-2\n\n### Решение\n\nб\n";
        match splice_section(doc, "вопросы", "\n## Q-1\n\n### Решение\n\nа\n\n## Q-2\n\nб\n") {
            Splice::Drops(d) => assert_eq!(d, ["решение"]),
            _ => panic!("потеря второго «Решения» принята"),
        }
    }
}

/// Пустой текст отказан до базы: адрес базы здесь нерабочий, и дошедшая до
/// неё запись упала бы ошибкой соединения, а не ответом «пусто».
#[cfg(test)]
mod empty_text {
    #[tokio::test]
    async fn an_empty_text_is_refused_not_written() {
        let pool = crate::db::pool("postgres://nobody@127.0.0.1:1/none", 1).expect("пул без соединения");
        for content in ["", "  \n"] {
            let doc = super::Document { kind: "task", name: "M9-T1", content };
            let v = super::put(&pool, "p", doc, "t", None, 0).await.expect("отказ без базы");
            assert_eq!(v["status"], "empty", "пустой текст «{content:?}» не пишется поверх");
        }
    }
}
