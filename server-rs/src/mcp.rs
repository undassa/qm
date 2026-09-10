//! MCP по stdio — подкомандой того же бинаря.
//!
//! Не отдельная служба и не отдельная реализация: те же `db.rs`, тот же слой
//! сущностей, тот же писатель. Вторая реализация тех же ручек разошлась бы с
//! первой — и разошлась бы молча, потому что сверять их было бы нечем.
//!
//! Словарь — из раскладки видов: пара инструментов на вид (`decision` и
//! `decision-list`) плюс общие. Имена те же, что у `corpus <вид> <id>` в
//! харнесе, чтобы переключение источника было строкой в настройке, а не
//! переводом словаря.

use std::sync::Arc;

use deadpool_postgres::Pool;
use serde_json::{json, Value};

use crate::{corpus, documents, entities, entities::Miss, kinds::Kinds};

pub struct Mcp {
    pub pool: Pool,
    pub kinds: Arc<Kinds>,
    pub project: String,
    pub author: String,
}

/// Отказ говорит словом, что именно не так.
///
/// «Нет проекции» и «ничего не нашлось» — разные ответы, и агент обязан их
/// различать: на втором он построит задачу без контекста и не заметит.
fn refusal(m: Miss) -> Value {
    let text = match m {
        Miss::NoKind(k) => format!("нет такого вида: {k}"),
        Miss::NoEntity(k, id) if id.is_empty() => format!("виду {k} нужно имя"),
        Miss::NoEntity(k, id) => format!("нет такой сущности: {k} {id}"),
        Miss::Unprojected(k) => {
            format!("вид {k} в базу не спроецирован: ответ неизвестен, а не пуст — спрашивать нечего, а не ничего нет")
        }
        Miss::Db(e) => format!("база не ответила: {e:#}"),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

fn ok(value: Value) -> Value {
    let text = match &value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    json!({ "content": [{ "type": "text", "text": text }] })
}

impl Mcp {
    /// Перечень инструментов: пара на вид плюс общие.
    pub fn tools(&self) -> Vec<Value> {
        let mut tools = Vec::new();
        for (name, k) in &self.kinds.0 {
            let single = k.single;
            let about = if k.is_inner() {
                format!("сущность вида {name}, объявленная внутри документа «{}»", k.in_kind.clone().unwrap_or_default())
            } else if single {
                format!("{name} — одиночка: экземпляр один, имени у него нет")
            } else {
                format!("сущность вида {name} по имени")
            };
            let mut props = serde_json::Map::new();
            if !single {
                props.insert("id".into(), json!({ "type": "string", "description": "имя сущности" }));
            }
            tools.push(json!({
                "name": name,
                "description": about,
                "inputSchema": {
                    "type": "object",
                    "properties": props,
                    "required": if single { json!([]) } else { json!(["id"]) }
                }
            }));
            if !single {
                tools.push(json!({
                    "name": format!("{name}-list"),
                    "description": format!("имена всех сущностей вида {name}"),
                    "inputSchema": { "type": "object", "properties": {} }
                }));
            }
        }
        let s = |d: &str| json!({ "type": "string", "description": d });
        tools.push(json!({ "name": "kinds", "description": "какие виды есть и сколько каждого; у неспроецированных — не ноль, а пусто",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "sections", "description": "заголовки разделов сущности с якорями",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя, если вид не одиночка") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "section", "description": "один раздел сущности по якорю",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "anchor": s("якорь раздела") }, "required": ["kind", "anchor"] } }));
        tools.push(json!({ "name": "backlinks", "description": "кто ссылается на сущность — сущностями, а не файлами",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "search", "description": "где встречается строка",
            "inputSchema": { "type": "object", "properties": { "q": s("что ищем — короткое имя того же довода"), "query": s("что искать"), "limit": json!({"type":"integer"}) }, "required": ["query"] } }));
        tools.push(json!({ "name": "put", "description": "записать сущность целиком; expectedRevision бережёт от потери чужой правки",
            "inputSchema": { "type": "object", "properties": { "deferProjection": json!({"type":"boolean","description":"не пересобирать проекции сейчас; позвать `reproject` после серии правок"}), "kind": s("вид"), "id": s("имя"), "content": s("текст целиком"), "expectedRevision": json!({"type":"integer"}) }, "required": ["kind", "content"] } }));
        tools.push(json!({ "name": "put-section", "description": "заменить один раздел сущности; проекции пересобираются в этом же вызове",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "anchor": s("якорь"), "body": s("новое тело раздела"), "expectedRevision": json!({"type":"integer"}) }, "required": ["kind", "anchor", "body"] } }));
        tools.push(json!({ "name": "rm", "description": "удалить сущность",
            "inputSchema": { "type": "object", "properties": { "deferProjection": json!({"type":"boolean","description":"не пересобирать проекции сейчас; позвать `reproject` после серии правок"}), "kind": s("вид"), "id": s("имя") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "task-state-push", "description": "принять состояния задач, выведенные харнесом из закрывающих трейлеров; подача полная",
            "inputSchema": { "type": "object", "properties": {
                "states": { "type": "array", "description": "[{id, state, commit}]",
                            "items": { "type": "object", "properties": {
                              "id": s("имя задачи"), "state": s("not_started · claimed · closed"), "commit": s("закрывающий коммит") },
                              "required": ["id", "state"] } } }, "required": ["states"] } }));
        tools.push(json!({ "name": "state-disagreements", "description": "где документ и история расходятся о состоянии задачи",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "readiness", "description": "чек-лист сущности: объявленное рядом с вычисленным, честный unknown там, где способа нет",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид владельца: task · red-task · question · document"), "id": s("имя") }, "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "statuses", "description": "какие статусы бывают у вида и каким фактом достигается каждый",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид: task · decision") } } }));
        tools.push(json!({ "name": "task-status", "description": "статус задачи по конвейеру: достигнутые, текущий, неизвестные",
            "inputSchema": { "type": "object", "properties": { "id": s("имя задачи; без него — все") } } }));
        tools.push(json!({ "name": "status-anomaly", "description": "задачи, прошедшие мимо середины конвейера",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "blocks", "description": "блоки документа так, как их разобрал сервер: заголовки, проза, таблицы ячейками, код",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "anchor": s("якорь раздела; без него — весь документ"),
                "own": s("true — только собственные блоки раздела, без вложенных подразделов") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "history", "description": "правки сущности: кто, когда и сколько — из летописи, а не из памяти",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "limit": json!({"type":"integer"}) }, "required": ["kind"] } }));
        tools.push(json!({ "name": "at-revision", "description": "текст сущности, каким он был на названной правке",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "revision": json!({"type":"integer"}) }, "required": ["kind", "revision"] } }));
        tools.push(json!({ "name": "coverage", "description": "какие документы не достаются ни одним видом и какие достаются двумя",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "phases", "description": "цепочка фаз с полной картиной: документы, гейт и задачи каждой фазы порознь",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "waves", "description": "волны: что можно вести одновременно, с барьером красной фазы",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "pipeline", "description": "плитка конвейера задач: сколько на каждом статусе и где факт не пишется",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "next-step", "description": "первая невыполненная ступень процесса с владельцем; три списка: пройдено, пропущено с причиной, неотвечаемо",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса, по умолчанию godzy"), "record": json!({"type":"boolean"}) } } }));
        tools.push(json!({ "name": "process-state", "description": "все ступени процесса: вопрос, способ, владелец, сторона",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса") } } }));
        tools.push(json!({ "name": "process-history", "description": "проходы диспетчера: какая ступень скачет",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса") } } }));
        tools.push(json!({ "name": "progress", "description": "плитки прогресса тремя числами: сделано · открыто · не отвечается",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "method-set", "description": "объявить способ проверки пункта готовности: query считает сервер, command выполняет харнес",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}),
                "kind": s("вид владельца"), "id": s("имя владельца; у одиночки пусто"),
                "ord": json!({"type":"integer"}), "methodKind": s("query · command · signed · unknown"),
                "method": s("запрос либо команда") }, "required": ["kind", "ord", "methodKind"] } }));
        tools.push(json!({ "name": "links-of", "description": "связи сущности по видам: проверки, истории, задачи, решения — то, что показывает панель раздела",
            "inputSchema": { "type": "object", "properties": { "kind": s("requirement · story · decision"), "id": s("имя") }, "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "retired-terms", "description": "слова, снятые из словаря: встреченные в свежем тексте — находка",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "step-method-set", "description": "объявить способ проверки ступени лестницы; переживает пересборку и выкатку",
            "inputSchema": { "type": "object", "properties": { "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}), "methodKind": s("query · command · signed"), "method": s("запрос либо команда"),
                "drop": json!({"type":"boolean"}) },
                "required": ["ord", "methodKind"] } }));
        tools.push(json!({ "name": "document-add", "description": "завести новый документ объявленного вида; правит существующий — `put`, и заводить он отказывается",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид из раскладки"),
                "id": s("имя; у одиночки пусто"), "content": s("текст документа") },
                "required": ["kind", "content"] } }));
        tools.push(json!({ "name": "agents", "description": "субагенты набора: имя, описание, инструменты, модель; тело — по просьбе",
            "inputSchema": { "type": "object", "properties": { "set": s("набор, по умолчанию godzy"),
                "body": json!({"type":"boolean"}) } } }));
        tools.push(json!({ "name": "step-remove", "description": "снять ступень лестницы и сдвинуть номера следом идущих; способ и проба уходят вместе с ней",
            "inputSchema": { "type": "object", "properties": { "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}) }, "required": ["ord"] } }));
        tools.push(json!({ "name": "run-record-add", "description": "объявить запись прогона: задача, коммиты, даты, ревью, что появилось и что осталось открытым",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя прогона"), "task": s("задача"),
                "milestone": s("этап"), "title": s("заголовок"), "commits": s("диапазон коммитов"),
                "dates": s("даты"), "review": s("ревью"), "appeared": s("что появилось"),
                "leftOpen": s("что осталось открытым") }, "required": ["id"] } }));
        tools.push(json!({ "name": "decision-link-add", "description": "объявить связь решения: closes · supersedes · refines · touches · relates",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "decision": s("решение"),
                "kind": s("вид связи"), "target": s("цель") }, "required": ["decision", "kind", "target"] } }));
        tools.push(json!({ "name": "frame-rule-add", "description": "объявить гарантию продукта либо правило внешнего заявления",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "kind": s("guarantee либо claim"),
                "number": json!({"type":"integer"}), "title": s("формулировка"), "body": s("тело"),
                "heldBy": s("чем держится") }, "required": ["kind", "number", "title"] } }));
        tools.push(json!({ "name": "process-row-add", "description": "объявить стадию работы либо артефакт фазы из разбора процесса",
            "inputSchema": { "type": "object", "properties": { "kind": s("stage либо artifact"),
                "a": s("имя стадии либо фаза"), "b": s("что производит либо артефакт"),
                "c": s("чем закрывается либо что у нас"), "d": s("состояние"),
                "ord": json!({"type":"integer"}),
                "drop": json!({"type":"boolean","description":"снять строку, а не объявить"}) },
                "required": ["kind", "a"] } }));
        tools.push(json!({ "name": "milestone-detail-add", "description": "объявить подробности этапа: что делается, чем блокирован, какое требование закрывает, какой гейт включает",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "milestone": s("этап"), "what": s("что делается"),
                "blockedBy": s("чем блокирован"), "requirement": s("требование"), "gate": s("гейт"),
                "closed": s("дата закрытия вехи, как её называет сам документ") },
                "required": ["milestone"] } }));
        tools.push(json!({ "name": "screen-detail-add", "description": "объявить подробности экрана: зачем, когда открывается, что показывает пустым и сломанным, какое требование держит",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "screen": s("экран"), "purpose": s("зачем"),
                "opensWhen": s("когда открывается"), "emptyAndBroken": s("пусто и сломано"),
                "requirement": s("требование") }, "required": ["screen"] } }));
        tools.push(json!({ "name": "story-detail-add", "description": "объявить, кто действует в истории и каким экраном она закрывается",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "story": s("история"),
                "screen": s("экран"), "persona": s("кто действует") }, "required": ["story"] } }));
        tools.push(json!({ "name": "feature-link-add", "description": "объявить, что фича несёт требование либо опирается на статью конституции",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "feature": s("фича"),
                "requirement": s("требование"), "article": json!({"type":"integer"}) },
                "required": ["feature"] } }));
        tools.push(json!({ "name": "acceptance-add", "description": "объявить сценарий приёмки: история, предусловия, шаги, наблюдаемый результат, признак провала",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("ключ, например S1-AC-1"),
                "story": s("история"), "number": json!({"type":"integer"}), "title": s("название"),
                "preconditions": s("предусловия"), "steps": s("шаги"), "observed": s("наблюдаемый результат"),
                "failsWhen": s("провал") }, "required": ["id", "title"] } }));
        tools.push(json!({ "name": "goal-add", "description": "объявить цель проекта: уровень, формулировка, чем измеряется, когда проверяется, что считается провалом",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя, например Ц-01"),
                "number": json!({"type":"integer"}), "level": s("уровень"), "title": s("формулировка"),
                "measuredBy": s("чем измеряется"), "checkedWhen": s("когда проверяется"),
                "failsWhen": s("что считается провалом"), "stateNow": s("как сейчас") },
                "required": ["id", "title"] } }));
        tools.push(json!({ "name": "goals", "description": "цели проекта и их измерения",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "risk-add", "description": "объявить риск либо расхождение: чем оно, чем смягчается, по какому признаку видно, чей и до какого срока",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя, например R-01"),
                "number": json!({"type":"integer"}), "title": s("чем является"),
                "state": s("open · accepted · closed"), "mitigation": s("митигация"),
                "trigger": s("признак срабатывания"), "owner": s("владелец"), "source": s("где записан") },
                "required": ["id", "title"] } }));
        tools.push(json!({ "name": "question-add", "description": "объявить вопрос прямо: имя, номер, заголовок, состояние, чем закрыт",
            "inputSchema": { "type": "object", "properties": { "id": s("имя, например OQ-01"),
                "number": json!({"type":"integer"}), "title": s("о чём вопрос"),
                "state": s("open · decided · closed"), "answer": s("ответ, если записан"),
                "closedBy": s("решение, которым закрыт"),
                "drop": json!({"type":"boolean","description":"снять вопрос, а не объявить"}) },
                "required": ["id", "title"] } }));
        tools.push(json!({ "name": "task-requirement-add", "description": "объявить, что задача несёт требование",
            "inputSchema": { "type": "object", "properties": { "task": s("задача"), "drop": s("true — снять зависимость"), "requirement": s("требование") },
                "required": ["task", "requirement"] } }));
        tools.push(json!({ "name": "screen-reference-add", "description": "объявить, что документ ссылается на экран",
            "inputSchema": { "type": "object", "properties": { "source": s("имя источника"),
                "sourceKind": s("вид источника"), "screen": s("экран"), "drop": json!({"type":"boolean"}) },
                "required": ["source", "screen"] } }));
        tools.push(json!({ "name": "alternative-add", "description": "объявить отвергнутый вариант решения: чем он был и почему отвергнут",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "decision": s("имя решения"),
                "ord": json!({"type":"integer"}), "title": s("вариант"), "body": s("почему отвергнут") },
                "required": ["decision", "title"] } }));
        tools.push(json!({ "name": "version-add", "description": "объявить выпуск прямо",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя выпуска") }, "required": ["id"] } }));
        tools.push(json!({ "name": "milestone-add", "description": "объявить этап прямо: имя, выпуск, порядок, заголовок",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя"), "version": s("выпуск"),
                "ord": json!({"type":"integer"}), "title": s("заголовок") }, "required": ["id", "version"] } }));
        tools.push(json!({ "name": "task-add", "description": "объявить задачу прямо: имя, этап, порядок, заголовок, вид, состояние",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя"), "milestone": s("этап"),
                "ord": json!({"type":"integer"}), "title": s("заголовок"), "kind": s("вид: dev · red"),
                "state": s("not_started · claimed · closed"), "size": s("размер") },
                "required": ["id", "milestone"] } }));
        tools.push(json!({ "name": "decision-add", "description": "объявить решение прямо: имя, номер, заголовок, состояние, дата, решающие, контекст, решение, последствия",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя решения"),
                "number": json!({"type":"integer"}), "title": s("заголовок"),
                "status": s("accepted · superseded · proposed · rejected"), "statusText": s("как записано"),
                "date": s("дата"), "deciders": s("кто решал"), "context": s("контекст"),
                "decision": s("решение"), "consequences": s("последствия") }, "required": ["id", "title"] } }));
        tools.push(json!({ "name": "story-add", "description": "объявить историю прямо: имя, заголовок, область",
            "inputSchema": { "type": "object", "properties": { "id": s("имя"), "title": s("заголовок"),
                "area": s("область") }, "required": ["id", "title"] } }));
        tools.push(json!({ "name": "screen-add", "description": "объявить экран прямо: имя, заголовок, область",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя"), "title": s("заголовок"),
                "area": s("область") }, "required": ["id", "title"] } }));
        tools.push(json!({ "name": "article-add", "description": "объявить статью конституции прямо: номер, заголовок, тело; переживает пересборку",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "number": json!({"type":"integer"}),
                "title": s("заголовок"), "body": s("тело"), "anchor": s("якорь") },
                "required": ["number", "title"] } }));
        tools.push(json!({ "name": "requirement-add", "description": "объявить требование прямо: имя, вид FR/NFR, область, текст",
            "inputSchema": { "type": "object", "properties": { "id": s("имя, например FR-01"),
                "drop": s("true — снять объявленное требование"),
                "kind": s("FR · NFR · UI"), "area": s("область"), "title": s("заголовок — формулировка одной строкой"),
                "text": s("тело: чем требование обосновано"), "measuredBy": s("чем проверяется"),
                "priority": s("О · Ж · П") }, "required": ["id"] } }));
        tools.push(json!({ "name": "term-add", "description": "объявить термин словаря прямо: слово и что оно здесь значит",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "term": s("слово"), "meaning": s("значение"),
                "area": s("область") }, "required": ["term"] } }));
        tools.push(json!({ "name": "task-dep-add", "description": "объявить, что задача ждёт другую; обе обязаны существовать",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "task": s("задача"),
                "dependsOn": s("кого ждёт") }, "required": ["task", "dependsOn"] } }));
        tools.push(json!({ "name": "release-artifact-add", "description": "объявить артефакт поставки: что это и куда ставится",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя"), "what": s("что это"),
                "installedTo": s("куда ставится") }, "required": ["name"] } }));
        tools.push(json!({ "name": "freeze-row-add", "description": "внести строку объявленного слепка выпуска: документ и его отпечаток на заморозку",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "version": s("выпуск"), "kind": s("вид"),
                "name": s("имя"), "hash": s("отпечаток") }, "required": ["version", "kind"] } }));
        tools.push(json!({ "name": "postmortem-add", "description": "объявить разбор случившегося: сводка, хронология, корневая причина, урок",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя"), "title": s("заголовок"),
                "summary": s("сводка"), "timeline": s("хронология"), "rootCause": s("корневая причина"),
                "lesson": s("урок") }, "required": ["id"] } }));
        tools.push(json!({ "name": "token-add", "description": "объявить токен оформления: значение в тёмной и светлой теме и назначение",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя токена"),
                "dark": s("тёмная"), "light": s("светлая"), "purpose": s("назначение"),
                "section": s("раздел") }, "required": ["name"] } }));
        tools.push(json!({ "name": "reference-source-add", "description": "объявить происхождение справочного документа: откуда снят, когда, с каким отпечатком",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя документа"),
                "source": s("источник"), "note": s("исходная заметка"), "taken": s("когда снят"),
                "sha": s("отпечаток"), "refType": s("вид"), "fromProject": s("проект"),
                "repo": s("репозиторий"), "written": s("когда написан исходный документ"),
                "updated": s("когда исправлен"), "status": s("что о себе говорил исходный документ"),
                "tags": s("метки исходного документа"), "role": s("для кого написан") },
                "required": ["name"] } }));
        tools.push(json!({ "name": "donor-add", "description": "объявить донорское дерево: чужая реализация, замороженная на запись",
            "inputSchema": { "type": "object", "properties": { "path": s("путь дерева"),
                "what": s("что это за материал"), "frozenBy": s("чем заморожено: статья, решение"),
                "drop": json!({"type":"boolean"}) }, "required": ["path"] } }));
        tools.push(json!({ "name": "guard-add", "description": "объявить сторожа: чем правило принуждается ДО действия, а не меряется после",
            "inputSchema": { "type": "object", "properties": { "name": s("имя сторожа"),
                "enforces": s("что принуждает: статья, решение, требование"),
                "scope": s("где действует"), "refuses": s("что отвергает"),
                "actsOn": s("write либо command"), "pathRe": s("образец пути"),
                "contentRe": s("образец содержимого"), "commandRe": s("образец команды"),
                "drop": json!({"type":"boolean"}) }, "required": ["name"] } }));
        tools.push(json!({ "name": "agent-set", "description": "объявить субагента: тело, описание, инструменты, модель",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя субагента"),
                "body": s("тело целиком"), "description": s("описание"), "tools": s("инструменты"),
                "model": s("модель"), "set": s("набор умений") }, "required": ["name", "body"] } }));
        tools.push(json!({ "name": "sensor-spec-add", "description": "объявить датчик: где искать, чем вынимать, как назвать факт",
            "inputSchema": { "type": "object", "properties": { "skip": s("строка, которую датчик не считает находкой — образец"), "allow": s("места, где правило не действует — образцы пути через пробел"), "fact": s("род факта"),
                "reads": s("что читать: backend/**/*.rs"), "extract": s("образец: пустой — факт о самом файле"),
                "note": s("зачем"), "how": s("extract · files · secret-fields · declared-paths · lines · domain-vs-check · contract-vs-schema"), "drop": json!({"type":"boolean"}) }, "required": ["fact"] } }));
        tools.push(json!({ "name": "sensor-specs", "description": "чем снимать факты: объявленные датчики",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "donors", "description": "донорские деревья и сторожа: что здесь не наше и чем это держится",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "algorithm-add", "description": "объявить алгоритм истории: предусловия, поток, ветки отказа, чего не делает; либо одну его связь",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя, например A-S1"),
                "story": s("история"), "title": s("заголовок"), "preconditions": s("предусловия"),
                "flow": s("поток"), "failure": s("ветки отказа"), "notCovered": s("чего не делает"),
                "linkKind": s("вид связи: feature · screen · requirement · article"),
                "linkTarget": s("цель связи") }, "required": ["id"] } }));
        tools.push(json!({ "name": "stand-row-add", "description": "объявить строку стенда: раздел, имя, значение — то, чем выполнены правила эталонного размера",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "section": s("раздел"), "name": s("имя"),
                "value": s("значение") }, "required": ["name"] } }));
        tools.push(json!({ "name": "crate-add", "description": "объявить крейт: что делает и чего не делает",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя крейта"),
                "does": s("что делает"), "doesNot": s("чего не делает") }, "required": ["name"] } }));
        tools.push(json!({ "name": "protocol-op-add", "description": "объявить операцию протокола, её группу, парные события и требование группы",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "op": s("операция"), "group": s("группа"),
                "events": s("парные события"), "requirement": s("требование группы") } } }));
        tools.push(json!({ "name": "article-gate-add", "description": "объявить, каким гейтом исполняется статья конституции и работает ли он",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "article": json!({"type":"integer"}),
                "gate": s("имя гейта"), "state": s("enforced либо planned") },
                "required": ["article", "gate"] } }));
        tools.push(json!({ "name": "requirement-source-add", "description": "объявить, на что требование опирается: decision · article · screen · requirement",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("требование"),
                "kind": s("вид опоры"), "target": s("имя") }, "required": ["id", "kind", "target"] } }));
        tools.push(json!({ "name": "requirement-scope-set", "description": "объявить требование вне выпуска либо сквозным — с причиной; без причины не принимается",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("требование"),
                "outOfVersion": s("почему вне выпуска"), "crosscutting": s("почему не принадлежит фиче") },
                "required": ["id"] } }));
        tools.push(json!({ "name": "requirement-retire", "description": "объявить требование снятым: имя, причина, чем снято; живое снятым не объявляется",
            "inputSchema": { "type": "object", "properties": { "id": s("имя требования"), "why": s("почему снято"),
                "retiredBy": s("чем снято — решение либо документ"), "drop": json!({"type":"boolean"}) },
                "required": ["id"] } }));
        tools.push(json!({ "name": "sensor-declare", "description": "объявить датчик репозитория: имя факта, что меряет, через сколько молчание считается устареванием",
            "inputSchema": { "type": "object", "properties": { "fact": s("имя факта"), "about": s("что меряет"),
                "staleAfterMs": json!({"type":"integer"}), "drop": json!({"type":"boolean"}) },
                "required": ["fact"] } }));
        tools.push(json!({ "name": "sensors", "description": "объявленные датчики и когда каждый подавал; отдельно — подающие, которых никто не объявлял",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "step-add", "description": "завести ступень лестницы на указанное место, раздвинув номера; способ и пробу объявляют отдельно",
            "inputSchema": { "type": "object", "properties": { "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}), "question": s("условие словами"),
                "ownerKind": s("skill · agent · none"), "owner": s("имя скилла или субагента"),
                "touches": s("corpus · repository") }, "required": ["ord", "question", "touches"] } }));
        tools.push(json!({ "name": "version-close", "description": "объявить выпуск закрытым или снова открытым; от закрытого считают, что изменилось после него",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "version": s("имя выпуска"),
                "state": s("closed · open, по умолчанию closed") }, "required": ["version"] } }));
        tools.push(json!({ "name": "phase-gate-set", "description": "привязать гейт к фазе: чем фаза закрывается",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "phase": s("имя фазы"), "gate": s("имя гейта") },
                "required": ["phase", "gate"] } }));
        tools.push(json!({ "name": "step-when-set", "description": "объявить, когда ступень вообще в игре: запрос условия и причина пропуска",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}), "whenQuery": s("запрос условия; пусто — всегда в игре"),
                "whenWhy": s("причина пропуска словами") }, "required": ["ord"] } }));
        tools.push(json!({ "name": "step-probe-set", "description": "объявить пробу ступени: запрос, подсаживающий нарушение — им самотест её роняет",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}), "probe": s("запрос подсадки") },
                "required": ["ord", "probe"] } }));
        tools.push(json!({ "name": "step-selftest", "description": "самотест лестницы: каждая ступень роняется подсаженным нарушением в откатываемой транзакции; живой считается та, у которой число выросло",
            "inputSchema": { "type": "object", "properties": { "process": s("процесс, по умолчанию godzy") } } }));
        tools.push(json!({ "name": "step-question-set", "description": "переименовать ступень лестницы: условие, которое должно быть верно, чтобы она считалась пройденной",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "process": s("процесс, по умолчанию godzy"),
                "ord": json!({"type":"integer"}), "question": s("условие словами") },
                "required": ["ord", "question"] } }));
        tools.push(json!({ "name": "scheme-term-set", "description": "объявить слово схемы: роль, которую знает код, и как её зовёт набор",
            "inputSchema": { "type": "object", "properties": { "role": s("роль, латиницей"),
                "value": s("слово набора"), "ord": s("порядок, если слов несколько"),
                "why": s("зачем"), "drop": s("true — снять слово") },
                "required": ["role", "value"] } }));
        tools.push(json!({ "name": "scheme-terms", "description": "словарь схемы: какие роли объявлены и какими словами",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "orphans-purge", "description": "убрать строки наборов, которых нет среди проектов: остатки самопроверок",
            "inputSchema": { "type": "object", "properties": {
                "confirm": s("true — убрать; без него только перечень") } } }));
        tools.push(json!({ "name": "surface-source-add", "description": "объявить источник поверхности: вид документа и имя; пустое имя — весь вид",
            "inputSchema": { "type": "object", "properties": { "surface": s("имя поверхности"),
                "entityKind": s("вид документа"), "entityName": s("имя; пусто — весь вид"),
                "drop": s("true — снять источник") }, "required": ["surface", "entityKind"] } }));
        tools.push(json!({ "name": "frozen-tree-set", "description": "заморозить дерево донора: путь и хэш, на котором оно стоит",
            "inputSchema": { "type": "object", "properties": { "path": s("каталог"),
                "hash": s("хэш дерева git"), "why": s("зачем заморожено"),
                "drop": s("true — снять заморозку") }, "required": ["path"] } }));
        tools.push(json!({ "name": "frozen-trees", "description": "замороженные деревья доноров",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "addresses-declared", "description": "адреса «файл:строка», названные набором",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "tree-declared", "description": "объявленное дерево: путь и состояние из таблицы предмета file-tree",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "exception-set", "description": "объявить исключение из правила гейта: правило, сущность, причина, кто решил",
            "inputSchema": { "type": "object", "properties": { "rule": s("имя правила — оно же имя пункта гейта"),
                "entityKind": s("вид сущности"), "entityId": s("имя сущности"), "reason": s("почему это законно"),
                "closes": s("задача, которая побег отменит; пусто — побег бессрочный"),
                "drop": s("true — снять исключение") }, "required": ["rule", "entityId"] } }));
        tools.push(json!({ "name": "gate-sign", "description": "записать подпись гейта: кто, когда, под какой формулировкой и под какими документами",
            "inputSchema": { "type": "object", "properties": { "phase": s("гейт"), "signedAt": s("дата подписи"),
                "signedBy": s("имя человека"), "wording": s("формулировка, под которой стоит подпись"),
                "note": s("обстоятельства"), "under": { "type": "array", "items": { "type": "object" } } },
                "required": ["phase", "signedAt", "signedBy", "wording"] } }));
        tools.push(json!({ "name": "author-set", "description": "объявить автора документа: того, кто за него отвечает; пустое имя снимает объявление",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "kind": s("вид"), "id": s("имя"), "author": s("автор") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "authors", "description": "кто за какими документами стоит и у скольких автор не объявлен",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "principal-allow", "description": "объявить, что человеку можно войти; drop=true снимает объявление",
            "inputSchema": { "type": "object", "properties": { "principal": s("имя вошедшего"), "note": s("кто это"), "drop": s("true — снять") }, "required": ["principal"] } }));
        tools.push(json!({ "name": "principals", "description": "кто объявлен допущенным и кто на самом деле ходит",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "generated-check", "description": "сверить порождённый файл с тем, что считает сервер, и оставить след о самой сверке",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "version-freeze", "description": "заморозить набор на начало выпуска: хеш и правка каждого документа",
            "inputSchema": { "type": "object", "properties": { "version": s("имя выпуска") }, "required": ["version"] } }));
        tools.push(json!({ "name": "version-delta", "description": "что выпуск сделал с набором: изменил · удалил · добавил",
            "inputSchema": { "type": "object", "properties": { "version": s("имя выпуска") }, "required": ["version"] } }));
        tools.push(json!({ "name": "skill-set", "description": "записать один скилл харнеса: имя, описание, тело",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "name": s("имя скилла"), "body": s("тело целиком"), "description": s("описание"), "allowedTools": s("чем разрешено пользоваться"),
                "disableModelInvocation": json!({"type":"boolean"}),
                "set": s("набор, по умолчанию godzy") }, "required": ["name", "body"] } }));
        tools.push(json!({ "name": "skills-paths", "description": "где скиллы ещё водят агента по файлам набора",
            "inputSchema": { "type": "object", "properties": { "set": s("набор, по умолчанию godzy") } } }));
        tools.push(json!({ "name": "screen-area-set", "description": "объявить область экрана: раздел интерфейса; пустая область снимает объявление",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "screen": s("имя экрана"), "area": s("область") }, "required": ["screen"] } }));
        tools.push(json!({ "name": "links-retarget", "description": "переписать цель ссылок в `вид:имя` по уже разобранной связи; ярлык не трогается; сухой режим по умолчанию",
            "inputSchema": { "type": "object", "properties": { "apply": json!({"type":"boolean"}) } } }));
        tools.push(json!({ "name": "gate-item-waive", "description": "объявить пункт гейта неприменимым к этому проекту — с обязательной причиной",
            "inputSchema": { "type": "object", "properties": { "id": s("имя пункта, латиницей через дефис"), "phase": s("гейт"), "item": s("пункт"), "holdsWhileEmpty": s("род факта, который обязан оставаться пустым, пока отмена верна"),
                "why": s("почему неприменим"), "drop": json!({"type":"boolean"}) },
                "required": ["phase", "item"] } }));
        tools.push(json!({ "name": "gate-measure", "description": "перемерить пункты гейтов и сохранить измеренное; обычно не нужно — пересчёт идёт сам при изменении набора",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "gate-selftest", "description": "самотест гейтов: каждый запросный пункт роняется подсаженным нарушением в откатываемой транзакции",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "order", "description": "порядок выполнения задач: волны как топологические слои внутри этапа — вывод сервера, файл производен",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "summary", "description": "перечень сущностей вида с колонками-числами: сколько проверок у требования, вариантов у решения, требований у истории",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид: requirement · decision · story · screen · question · need") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "skills", "description": "скиллы харнеса, какими их держит база: имя, хеш, кто и когда подал",
            "inputSchema": { "type": "object", "properties": { "set": s("набор скиллов; по умолчанию godzy"),
                "body": s("true — отдать и тело скилла") } } }));
        tools.push(json!({ "name": "code-facts-push", "description": "принять наблюдение датчика о репозитории: таблицы миграций, операции контракта; подача полная в пределах вида",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид факта, например migration-table"),
                "facts": { "type": "array", "description": "[{name, detail}]", "items": { "type": "object" } } },
                "required": ["kind", "facts"] } }));
        tools.push(json!({ "name": "code-facts", "description": "что датчик подал о репозитории",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "skills-push", "description": "принять скиллы харнеса; подача полная",
            "inputSchema": { "type": "object", "properties": { "set": s("набор"),
                "skills": { "type": "array", "description": "[{name, description, body}]", "items": { "type": "object" } },
                "dry": s("true — только сверка: что разошлось, ничего не записывая") },
                "required": ["skills"] } }));
        tools.push(json!({ "name": "preflight-push", "description": "принять вердикты предполёта из истории репозитория; подача полная",
            "inputSchema": { "type": "object", "properties": {
                "verdicts": { "type": "array", "description": "[{task, at, taskRevision, verdict, findings, body}]",
                              "items": { "type": "object" } },
                "clear": json!({"type":"boolean","description":"пустая подача — ответ: снять все вердикты"}) },
                "required": ["verdicts"] } }));
        tools.push(json!({ "name": "worktree-push", "description": "принять открытые рабочие деревья — статус «в работе»; подача полная, пустая законна",
            "inputSchema": { "type": "object", "properties": {
                "open": { "type": "array", "description": "[{task, branch, since}]", "items": { "type": "object" } } } } }));
        tools.push(json!({ "name": "question-holders", "description": "вопрос и его задача-держатель: пора закрывать, закрыт рано, судить нечем",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "gate-item-set", "description": "объявить пункт гейта: запрос, команда или подпись",
            "inputSchema": { "type": "object", "properties": { "id": s("устойчивое имя пункта, латиницей через дефис — им пункт адресуется"), "phase": s("гейт, например G5"), "item": s("заголовок пункта для человека; переписывается свободно, ссылок не рвёт"),
                "itemKind": s("query · command · signed"), "query": s("запрос для вида query"),
                "owner": s("кто подписывает, для вида signed"),
                "probe": s("запрос, подсаживающий нарушение — им самотест роняет пункт"),
                "why": s("почему способа нет — для рода unknown"),
                "drop": json!({"type":"boolean","description":"снять пункт вместе с его замерами"}) }, "required": ["phase", "id", "itemKind"] } }));
        tools.push(json!({ "name": "ceiling-set", "description": "объявить потолок долга правила: сколько находок сегодня терпимо и почему; держит не долг, а его рост",
            "inputSchema": { "type": "object", "properties": {
                "rule": s("имя правила — оно же род факта"),
                "ceiling": json!({"type":"integer","description":"сколько находок терпимо"}),
                "why": s("что этот потолок держит; без причины дверь отказывает"),
                "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}) },
                "required": ["rule"] } }));
        tools.push(json!({ "name": "requirements-of", "description": "требования задачи; объявленное отсутствие доезжает фразой, а не пустотой",
            "inputSchema": { "type": "object", "properties": { "id": s("имя задачи") }, "required": ["id"] } }));
        tools.push(json!({ "name": "tasks-of", "description": "задачи истории через требования; исключение называется исключением",
            "inputSchema": { "type": "object", "properties": { "id": s("имя истории") }, "required": ["id"] } }));
        tools.push(json!({ "name": "preflight-queue", "description": "задачи, которым предполёт не делали либо делали до правки",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "claims", "description": "заявленные набором числа против факта, с оговоркой о том, что считается",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "exceptions", "description": "объявленные исключения из правил и те, что пора снять",
            "inputSchema": { "type": "object", "properties": { "rule": s("правило") } } }));
        tools.push(json!({ "name": "gate", "description": "состояние гейта, вычисленное сейчас: запрос выполняется, подпись сверяется хешем",
            "inputSchema": { "type": "object", "properties": { "id": s("имя гейта, например G2; без него — все") } } }));
        tools.push(json!({ "name": "next-task", "description": "следующая незакрытая задача с закрытыми зависимостями, со всем контекстом внутри",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "blockers", "description": "чего ждёт задача: задачи и этапы целиком",
            "inputSchema": { "type": "object", "properties": { "id": s("имя задачи") }, "required": ["id"] } }));
        tools.push(json!({ "name": "events", "description": "журнал сущности: что с ней происходило и кем",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя") }, "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "norm-versions", "description": "объявленные версии нормы и что каждая изменила",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид, по умолчанию constitution"), "version": s("номер версии") } } }));
        tools.push(json!({ "name": "measurements", "description": "датированные замеры: чем получено и сколько вышло",
            "inputSchema": { "type": "object", "properties": { "subject": s("предмет, поиск по вхождению") } } }));
        tools.push(json!({ "name": "plan", "description": "что план объявляет: есть · нет с причиной · не объявлено вовсе",
            "inputSchema": { "type": "object", "properties": { "name": s("имя позиции плана") } } }));
        tools.push(json!({ "name": "kinds-due", "description": "виды, которым положена таблица, но они ещё не разложены; и те, у кого это не объявлено",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "sweep", "description": "убрать разбор документов, которых больше нет: донорское удаление его не чистило",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "reproject", "description": "пересобрать проекции: обычно не нужно — запись делает это сама",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools
    }

    /// Инструменты, после которых набор стал другим.
    ///
    /// Перечень явный, а не «всё, что не чтение»: новая ручка попадёт сюда
    /// осознанно, а не молча получит право дёргать пересчёт. Забытая здесь
    /// пишущая ручка означает доску, отставшую до следующей правки, — заметно и
    /// поправимо; лишняя означала бы пересчёт на каждый чих.
    const WRITES: &[&str] = &[
        "put", "put-section", "rm", "document-add", "reparse", "reproject", "sweep",
        "task-state-push", "code-facts-push", "skills-push", "preflight-push", "worktree-push",
        "ceiling-set", "frozen-tree-set", "scheme-term-set", "orphans-purge", "surface-source-add", "sensor-spec-add", "agent-set", "donor-add", "guard-add", "gate-sign", "gate-item-set", "method-set", "step-method-set", "step-question-set", "step-probe-set", "step-when-set", "step-add", "step-remove", "sensor-declare", "sensors", "requirement-retire", "requirement-scope-set", "requirement-source-add", "article-gate-add", "protocol-op-add", "crate-add", "stand-row-add", "algorithm-add", "reference-source-add", "token-add", "postmortem-add", "freeze-row-add", "release-artifact-add", "task-dep-add", "article-add", "requirement-add", "term-add", "decision-add", "story-add", "screen-add", "version-add", "milestone-add", "task-add", "alternative-add", "task-requirement-add", "screen-reference-add", "question-add", "risk-add", "goal-add", "goals", "acceptance-add", "feature-link-add", "story-detail-add", "screen-detail-add", "milestone-detail-add", "process-row-add", "frame-rule-add", "decision-link-add", "run-record-add", "step-selftest", "version-close", "phase-gate-set", "exception-set",
        "author-set", "screen-area-set", "skill-set", "version-freeze",
        "links-rewrite", "links-retarget",
    ];

    /// Вызов инструмента — и отметка, если он писал.
    ///
    /// Ручки, правящие ОБЩЕЕ объявление: их итог виден каждому набору.
    const SHARED_WRITES: [&'static str; 8] = [
        "gate-item-set", "gate-item-waive", "phase-gate-set", "step-add", "step-remove",
        "step-method-set", "step-question-set", "step-probe-set",
    ];

    /// Отметка ставится ПОСЛЕ ответа и только на успешный: пересчитывать набор
    /// из-за отказа значит считать то же самое второй раз.
    pub async fn call(&self, name: &str, args: &Value) -> Value {
        let out = self.run(name, args).await;
        if Self::WRITES.contains(&name) && out.get("error").is_none() {
            // Правка ОБЩЕГО объявления метит все наборы: пункт гейта, фаза и
            // ступень лестницы одни на всех, и посчитать их надо всем.
            if Self::SHARED_WRITES.contains(&name) {
                crate::watch::touch_all(&self.pool, name).await;
            } else {
                crate::watch::touch(&self.pool, &self.project, name).await;
            }
        }
        out
    }

    async fn run(&self, name: &str, args: &Value) -> Value {
        // Имя сущности бывает числом — у статьи конституции оно и есть номер.
        // По HTTP оно приходит из адреса и разбирается в число; строкой его
        // здесь не увидели бы, и вид получил бы отказ «нужно имя» при поданном
        // имени. Одно место на оба входа.
        let numeric_id = args.get("id").and_then(|v| v.as_i64()).map(|n| n.to_string());
        let id = args.get("id").and_then(|v| v.as_str()).or(numeric_id.as_deref());
        let kind_arg = args.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
        let p = &self.project;

        // Общие инструменты старше видов. Имя `gate` носят оба: вид (строка
        // перечня гейтов) и вычисленное состояние. Спрашивают второе — первое
        // достаётся `gate-list`. Без этого старшинства вид молча перехватывал бы
        // вызов и отдавал строку таблицы вместо вычисления.
        const RESERVED: &[&str] = &[
            "method-set", "gate-item-set", "question-holders", "preflight-push", "worktree-push",
            "sensor-specs", "scheme-terms", "frozen-trees", "addresses-declared", "tree-declared", "donors", "skills-push", "skills", "agents", "code-facts-push", "code-facts", "summary", "links-of", "retired-terms", "order", "gate-measure", "gate-item-waive", "gate-selftest", "gate-sign", "links-rewrite", "links-retarget", "reparse", "screen-area-set", "skill-set", "skills-paths", "version-freeze", "version-delta", "generated-check", "principal-allow", "principals", "author-set", "authors", "step-method-set", "step-question-set", "step-probe-set", "step-when-set", "step-add", "step-remove", "sensor-declare", "sensors", "requirement-retire", "requirement-scope-set", "requirement-source-add", "article-gate-add", "protocol-op-add", "crate-add", "stand-row-add", "algorithm-add", "reference-source-add", "token-add", "postmortem-add", "freeze-row-add", "release-artifact-add", "task-dep-add", "article-add", "requirement-add", "term-add", "decision-add", "story-add", "screen-add", "version-add", "milestone-add", "task-add", "alternative-add", "task-requirement-add", "screen-reference-add", "question-add", "risk-add", "goal-add", "goals", "acceptance-add", "feature-link-add", "story-detail-add", "screen-detail-add", "milestone-detail-add", "process-row-add", "frame-rule-add", "decision-link-add", "run-record-add", "step-selftest", "version-close", "phase-gate-set", "exception-set", "next-step", "process-state", "statuses", "task-status", "status-anomaly", "pipeline", "waves", "phases", "coverage", "blocks", "history", "at-revision", "process-history", "progress",
            "kinds", "kinds-due", "sections", "section", "backlinks", "search", "put",
            "put-section", "rm", "document-add", "reproject", "sweep", "gate", "next-task", "blockers", "events",
            "norm-versions", "measurements", "plan", "readiness", "requirements-of",
            "tasks-of", "preflight-queue", "claims", "exceptions",
            "task-state-push", "state-disagreements",
        ];
        let reserved = RESERVED.contains(&name);

        // Инструмент вида: имя инструмента и есть вид.
        if let Some(kind) = name.strip_suffix("-list") {
            if self.kinds.get(kind).is_some() {
                return match entities::ids(&self.pool, &self.kinds, p, kind).await {
                    Ok(list) => ok(json!({ "kind": kind, "count": list.len(), "ids": list })),
                    Err(e) => refusal(e),
                };
            }
        }
        if !reserved && self.kinds.get(name).is_some() {
            return match entities::entity(&self.pool, &self.kinds, p, name, id).await {
                Ok(v) if args.get("brief").map(|b| b == "true" || b == true).unwrap_or(false) => {
                    ok(entities::without_body(v))
                }
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            };
        }

        match name {
            "kinds" => {
                let mut out = Vec::new();
                for (kind, k) in &self.kinds.0 {
                    let count = match entities::ids(&self.pool, &self.kinds, p, kind).await {
                        Ok(l) => json!(l.len()),
                        Err(Miss::Unprojected(_)) => Value::Null,
                        Err(e) => return refusal(e),
                    };
                    // Образец имени и назначение вида отдаются вместе со счётом:
                    // умения спрашивают у сервера «как зовётся сущность этого
                    // вида», и до сих пор не получали ответа — за ним ходили в
                    // карту проекта, файлом.
                    out.push(json!({
                        "kind": kind, "single": k.single,
                        "shape": if k.is_inner() { "inner" } else { "document" },
                        "count": count,
                        "idPattern": k.id.clone().unwrap_or_default(),
                        "nameIs": k.name_is.clone().unwrap_or_default(),
                        "projection": k.projection.clone().unwrap_or_default()
                    }));
                }
                ok(json!({ "kinds": out }))
            }
            "sections" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => match corpus::sections(&self.pool, p, &k, &n).await {
                    Ok(v) => ok(json!({ "sections": v })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                },
                Err(e) => refusal(e),
            },
            "section" => {
                let anchor = args.get("anchor").and_then(|v| v.as_str()).unwrap_or_default();
                match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                    Ok((k, n)) => match documents::read(&self.pool, p, &k, &n).await {
                        Ok(Some(doc)) => match corpus::section_body(&self.pool, p, &k, &n, anchor).await {
                            Ok(Some(body)) => ok(json!({ "anchor": anchor, "revision": doc.summary.revision, "body": body })),
                            Ok(None) => refusal(Miss::NoEntity(format!("{kind_arg}#раздел"), anchor.to_owned())),
                            Err(e) => refusal(Miss::Db(e.to_string())),
                        },
                        Ok(None) => refusal(Miss::NoEntity(kind_arg.to_owned(), id.unwrap_or("").to_owned())),
                        Err(e) => refusal(Miss::Db(e.to_string())),
                    },
                    Err(e) => refusal(e),
                }
            }
            "backlinks" => match entities::backlinks(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok(list) => ok(json!({ "count": list.len(), "backlinks": list })),
                Err(e) => refusal(e),
            },
            "search" => {
                // `query` и `q` — одна и та же просьба. Дверь принимала только
                // длинное имя, а короткое молча уходило пустотой: пустой запрос
                // совпадает со ВСЕМ, и ответ на «ADR-0138» был тот же, что на
                // «Q-372» — первые по алфавиту. Отказ был бы честнее молчания,
                // но принять оба имени честнее отказа.
                let q = args
                    .get("query")
                    .or_else(|| args.get("q"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if q.trim().is_empty() {
                    return refusal(Miss::Db(
                        "поиск без запроса вернул бы весь набор: назовите, что ищете (`query` или `q`)".into(),
                    ));
                }
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20).clamp(1, 200);
                match corpus::search(&self.pool, p, q, limit).await {
                    Ok(hits) => ok(json!({ "count": hits.len(), "hits": hits })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "task-state-push" => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let states: Vec<(String, String, String)> = args
                    .get("states")
                    .and_then(|v| v.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|it| {
                                Some((
                                    it.get("id")?.as_str()?.to_owned(),
                                    it.get("state")?.as_str()?.to_owned(),
                                    it.get("commit").and_then(|c| c.as_str()).unwrap_or("").to_owned(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if states.is_empty() {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача пуста: полная подача без задач стёрла бы все состояния, и это надо сказать явно" }],
                        "isError": true });
                }
                match crate::projector::push_task_state(&self.pool, p, &states, now).await {
                    Ok(mut v) => {
                        // Состояния только что изменились — план обязан их увидеть
                        // сейчас, а не после следующей записи в набор.
                        match self.projections().await {
                            Ok(own) => {
                                if let Some(m) = v.as_object_mut() {
                                    m.insert("own".into(), own);
                                }
                                ok(v)
                            }
                            Err(e) => refusal(e),
                        }
                    }
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "state-disagreements" => match crate::projector::state_disagreements(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "statuses" => {
                let k = if kind_arg.is_empty() { "task" } else { kind_arg };
                let client = self.pool.get().await.expect("пул отдал соединение");
                let all = client
                    .query("SELECT ord, name, title, fact, terminal, source FROM kind_status WHERE kind = $1 ORDER BY ord", &[&k])
                    .await;
                let mut recorded_now: Vec<String> = Vec::new();
                if let Ok(rows) = &all {
                    for r in rows {
                        let src: String = r.get(5);
                        if src.trim().is_empty()
                            || client.query(src.as_str(), &[&p]).await.map(|x| !x.is_empty()).unwrap_or(false)
                        {
                            recorded_now.push(r.get(1));
                        }
                    }
                }
                match all {
                    Ok(rows) => ok(json!({
                        "kind": k,
                        "statuses": rows.iter().map(|r| json!({
                            "ord": r.get::<_, i32>(0), "name": r.get::<_, String>(1),
                            "title": r.get::<_, String>(2),
                            "hasFact": !r.get::<_, String>(3).trim().is_empty(),
                            "sourceCheck": r.get::<_, String>(5),
                            "terminal": r.get::<_, bool>(4),
                        })).collect::<Vec<_>>(),
                        // Без ответа — и те, у кого запроса нет, и те, чей факт
                        // никто не записывает: спрашивать нечего и там и там.
                        "withoutFact": rows.iter().filter(|r| {
                            r.get::<_, String>(3).trim().is_empty() || {
                                let src: String = r.get(5);
                                !src.trim().is_empty() && !recorded_now.contains(&r.get::<_, String>(1))
                            }
                        }).count(),
                    })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "task-status" => match crate::projector::task_status(&self.pool, p, id).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "status-anomaly" => match crate::projector::status_anomaly(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "blocks" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let anchor = args.get("anchor").and_then(|v| v.as_str());
                    let own = args.get("own").map(|v| v == "true" || v == true).unwrap_or(false);
                    match crate::projector::blocks(&self.pool, p, &k, &n, anchor, own).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(Miss::Db(e.to_string())),
                    }
                }
                Err(e) => refusal(e),
            },
            "history" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20).clamp(1, 200);
                    match crate::projector::history(&self.pool, p, &k, &n, limit).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(Miss::Db(e.to_string())),
                    }
                }
                Err(e) => refusal(e),
            },
            "at-revision" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let rev = args.get("revision").and_then(|v| v.as_i64()).unwrap_or(0);
                    match crate::projector::at_revision(&self.pool, p, &k, &n, rev).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(Miss::Db(e.to_string())),
                    }
                }
                Err(e) => refusal(e),
            },
            "coverage" => match crate::projector::coverage(&self.pool, &self.kinds, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "phases" => match crate::projector::phases(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "waves" => match crate::projector::waves(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "pipeline" => match crate::projector::task_pipeline(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "next-step" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::next_step(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "process-state" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let client = self.pool.get().await.expect("пул отдал соединение");
                match client
                    .query(
                        "SELECT ord, question, method_kind, owner_kind, owner, touches, answerable
                           FROM process_state WHERE set_name = 'godzy' AND process = $1 ORDER BY ord",
                        &[&process],
                    )
                    .await
                {
                    Ok(rows) => ok(json!({ "steps": rows.iter().map(|r| json!({
                        "ord": r.get::<_, i32>(0), "question": r.get::<_, String>(1),
                        "method_kind": r.get::<_, String>(2), "owner_kind": r.get::<_, String>(3),
                        "owner": r.get::<_, String>(4), "touches": r.get::<_, String>(5),
                        "answerable": r.get::<_, String>(6),
                    })).collect::<Vec<_>>() })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "process-history" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::process_history(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "progress" => match crate::projector::progress(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "code-facts-push" => {
                let fact_kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let list: Vec<(String, String)> = args
                    .get("facts").and_then(|v| v.as_array())
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("name")?.as_str()?.to_owned(),
                        it.get("detail").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                    ))).collect())
                    .unwrap_or_default();
                // Пустая подача ЗАКОННА, если вид факта назван: датчик, ничего
                // не нашедший, говорит «чисто», а не «не смотрел». Это разные
                // ответы, и складывать их в один — та же ложь, что зелёный ноль.
                //
                // Молчание сломанного датчика видно иначе: `fact_push` держит
                // время последней подачи, и оно перестаёт двигаться. Отказ на
                // пустоте это не ловил — он ловил только исправного датчика на
                // чистом дереве.
                if fact_kind.is_empty() {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача без вида факта: неизвестно, что именно наблюдали" }],
                        "isError": true });
                }
                if args.get("facts").is_none() {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача без перечня: пустой перечень — это «ничего не нашёл», а отсутствие перечня — «не подали»" }],
                        "isError": true });
                }
                match crate::projector::push_code_facts(&self.pool, p, &fact_kind, &list, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "code-facts" => {
                let client = self.pool.get().await.expect("пул отдал соединение");
                match client
                    .query("SELECT kind, name, detail FROM code_fact WHERE project_id = $1 ORDER BY kind, name", &[&p])
                    .await
                {
                    Ok(rows) => ok(json!({ "count": rows.len(), "facts": rows.iter().map(|r| json!({
                        "kind": r.get::<_, String>(0), "name": r.get::<_, String>(1),
                        "detail": r.get::<_, String>(2) })).collect::<Vec<_>>() })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "skills-push" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let list: Vec<(String, String, String)> = args
                    .get("skills").and_then(|v| v.as_array())
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("name")?.as_str()?.to_owned(),
                        it.get("description").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                        it.get("body").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                    ))).collect())
                    .unwrap_or_default();
                if list.is_empty() {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача пуста: полная подача без скиллов сняла бы все, и это надо сказать явно" }],
                        "isError": true });
                }
                let dry = args.get("dry").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::push_skills(&self.pool, &set_name, &list, &self.author, dry).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "skills" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let client = self.pool.get().await.expect("пул отдал соединение");
                // Тело отдаётся по просьбе: перечень скиллов спрашивают часто,
                // и таскать в нём двести килобайт незачем.
                let with_body = args.get("body").map(|v| v == "true" || v == true).unwrap_or(false);
                let rows = match client
                    .query(
                        "SELECT name, description, content_hash, updated_at, updated_by, length(body),
                                CASE WHEN $2 THEN body ELSE '' END,
                                allowed_tools, disable_model_invocation
                           FROM harness_skill WHERE set_name = $1 ORDER BY name",
                        &[&set_name, &with_body],
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return refusal(Miss::Db(e.to_string())),
                };
                ok(json!({ "set": set_name, "count": rows.len(),
                    "skills": rows.iter().map(|r| json!({
                        "name": r.get::<_, String>(0), "description": r.get::<_, String>(1),
                        "hash": r.get::<_, String>(2), "updatedAt": r.get::<_, i64>(3),
                        "updatedBy": r.get::<_, String>(4), "bytes": r.get::<_, i32>(5),
                        "allowedTools": r.get::<_, String>(7),
                        "disableModelInvocation": r.get::<_, Option<bool>>(8),
                        "body": r.get::<_, String>(6) })).collect::<Vec<_>>() }))
            }
            "preflight-push" => {
                let list: Vec<(String, i64, i64, String, i32, String)> = args
                    .get("verdicts").and_then(|v| v.as_array())
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("task")?.as_str()?.to_owned(),
                        it.get("at").and_then(|v| v.as_i64()).unwrap_or(0),
                        it.get("taskRevision").and_then(|v| v.as_i64()).unwrap_or(0),
                        it.get("verdict")?.as_str()?.to_owned(),
                        it.get("findings").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        it.get("body").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                    ))).collect())
                    .unwrap_or_default();
                // «Сказать явно» должно быть ЧЕМ: без флага отказ был тупиком —
                // снять ошибочный вердикт можно было только запросом в базу мимо
                // сервера.
                let clear = args.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
                if list.is_empty() && !clear {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача пуста: полная подача без вердиктов стёрла бы все. Если это и есть \
                                 ответ — сказать явно: clear:=true" }],
                        "isError": true });
                }
                match crate::projector::push_preflight(&self.pool, p, &list, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "worktree-push" => {
                // Пустая подача здесь ЗАКОННА: ни одного открытого дерева — это
                // ответ, а не молчание. Поэтому отдельного отказа нет.
                let list: Vec<(String, String, i64)> = args
                    .get("open").and_then(|v| v.as_array())
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("task")?.as_str()?.to_owned(),
                        it.get("branch").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                        it.get("since").and_then(|v| v.as_i64()).unwrap_or(0),
                    ))).collect())
                    .unwrap_or_default();
                match crate::projector::push_worktrees(&self.pool, p, &list, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            // «Кто цитирует статью» приходит из запроса уже сущностями: вид и
            // имя стоят в самой строке ссылки. Прежде запрос отдавал путь, а
            // имя выводилось здесь на каждом вызове — второе место, где
            // составлялось имя, и оно расходилось с первым.
            "links-of" => match crate::projector::links_of(&self.pool, p, kind_arg, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            },
            "retired-terms" => {
                // Снятое слово в словаре не лежит — его оттуда убрали. Это
                // отдельный факт, и спрашивается он отдельно: встреченное в
                // свежем тексте снятое слово — находка, а не термин.
                let client = self.pool.get().await.expect("пул отдал соединение");
                match client
                    .query(
                        "SELECT term, retired_by, declared_in FROM term_retired
                          WHERE project_id = $1 ORDER BY term",
                        &[&p],
                    )
                    .await
                {
                    Ok(rows) => ok(json!({ "count": rows.len(),
                        "rows": rows.iter().map(|r| json!({
                            "term": r.get::<_, String>(0), "retiredBy": r.get::<_, String>(1),
                            "declaredIn": r.get::<_, String>(2) })).collect::<Vec<_>>() })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "step-method-set" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let mk = args.get("methodKind").and_then(|v| v.as_str()).unwrap_or("unknown");
                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::set_step_method(&self.pool, set_name, process, ord, mk, method,
                                                        &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "agents" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let with_body = args.get("body").map(|v| v == "true" || v == true).unwrap_or(false);
                let client = self.pool.get().await.expect("пул отдал соединение");
                // Тело — по просьбе, как у умений: перечень спрашивают часто, и
                // таскать в нём десятки килобайт незачем.
                let rows = match client
                    .query(
                        "SELECT name, description, tools, model, effort, content_hash, length(body),
                                CASE WHEN $2 THEN body ELSE '' END
                           FROM harness_agent WHERE set_name = $1 ORDER BY name",
                        &[&set_name, &with_body],
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return refusal(Miss::Db(e.to_string())),
                };
                ok(json!({ "set": set_name, "count": rows.len(),
                    "agents": rows.iter().map(|r| json!({
                        "name": r.get::<_, String>(0), "description": r.get::<_, String>(1),
                        "tools": r.get::<_, String>(2), "model": r.get::<_, String>(3),
                        "effort": r.get::<_, String>(4), "hash": r.get::<_, String>(5),
                        "bytes": r.get::<_, i32>(6), "body": r.get::<_, String>(7),
                    })).collect::<Vec<_>>() }))
            }
            "step-remove" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                match crate::projector::remove_step(&self.pool, p, set_name, process, ord).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "run-record-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_run_record(&self.pool, p, &g("id"), &g("task"),
                        &g("milestone"), &g("title"), &g("commits"), &g("dates"), &g("review"),
                        &g("appeared"), &g("leftOpen"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "decision-link-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_decision_link(&self.pool, p, &g("decision"),
                        &g("kind"), &g("target"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "frame-rule-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_frame_rule(&self.pool, p, &g("kind"), number,
                        &g("title"), &g("body"), &g("heldBy"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "process-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_process_row(&self.pool, p, &g("kind"), &g("a"),
                        &g("b"), &g("c"), &g("d"), ord, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "milestone-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_milestone_detail(&self.pool, p, &g("milestone"),
                        &g("what"), &g("blockedBy"), &g("requirement"), &g("gate"), &g("closed"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "screen-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_screen_detail(&self.pool, p, &g("screen"), &g("purpose"),
                        &g("opensWhen"), &g("emptyAndBroken"), &g("requirement"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "story-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_story_detail(&self.pool, p, &g("story"),
                        &g("screen"), &g("persona"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "feature-link-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let article = args.get("article").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_feature_link(&self.pool, p, &g("feature"),
                        &g("requirement"), article, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "acceptance-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_acceptance(&self.pool, p, &g("id"), &g("story"), number,
                        &g("title"), &g("preconditions"), &g("steps"), &g("observed"), &g("failsWhen"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "goal-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_goal(&self.pool, p, &g("id"), number, &g("level"),
                        &g("title"), &g("measuredBy"), &g("checkedWhen"), &g("failsWhen"), &g("stateNow"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "goals" => {
                let client = self.pool.get().await.expect("пул отдал соединение");
                match client.query("SELECT id, level, title, measured_by, checked_when, fails_when, state_now
                                      FROM project_goal WHERE project_id = $1 ORDER BY number, id", &[p]).await {
                    Ok(rows) => ok(json!({ "count": rows.len(), "goals": rows.iter().map(|r| json!({
                        "id": r.get::<_, String>(0), "level": r.get::<_, String>(1),
                        "title": r.get::<_, String>(2), "measuredBy": r.get::<_, String>(3),
                        "checkedWhen": r.get::<_, String>(4), "failsWhen": r.get::<_, String>(5),
                        "now": r.get::<_, String>(6),
                        // Цель без способа измерить — намерение, и ответ это говорит.
                        "isGoal": !r.get::<_, String>(3).trim().is_empty() })).collect::<Vec<_>>() })),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "risk-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_risk(&self.pool, p, &g("id"), number, &g("title"),
                        &g("state"), &g("mitigation"), &g("trigger"), &g("owner"), &g("source"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "question-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_question(&self.pool, p, &g("id"), number, &g("title"),
                        &g("state"), &g("answer"), &g("closedBy"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "task-requirement-add" | "screen-reference-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let done = if name == "task-requirement-add" {
                    crate::projector::declare_task_requirement(&self.pool, p, &g("task"), &g("requirement"),
                        args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await
                } else {
                    crate::projector::declare_screen_reference(&self.pool, p, &g("source"),
                        &g("sourceKind"), &g("screen"),
                        args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "alternative-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_alternative(&self.pool, p, &g("decision"), ord,
                                                             &g("title"), &g("body"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "version-add" | "milestone-add" | "task-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let done = match name {
                    "version-add" => crate::projector::declare_version(&self.pool, p, &g("id"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await,
                    "milestone-add" => crate::projector::declare_milestone(&self.pool, p, &g("id"),
                                          &g("version"), ord, &g("title"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await,
                    _ => crate::projector::declare_task(&self.pool, p, &g("id"), &g("milestone"), ord,
                             &g("title"), &g("kind"), &g("state"), &g("size"),
                             args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await,
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "decision-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_decision(&self.pool, p, &g("id"), number, &g("title"),
                        &g("status"), &g("statusText"), &g("date"), &g("deciders"),
                        &g("context"), &g("decision"), &g("consequences"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "story-add" | "screen-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let done = if name == "story-add" {
                    crate::projector::declare_story(&self.pool, p, &g("id"), &g("title"), &g("area"),
                        args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await
                } else {
                    crate::projector::declare_screen(&self.pool, p, &g("id"), &g("title"), &g("area"),
                        args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "article-add" => {
                let number = args.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let anchor = args.get("anchor").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::declare_article(&self.pool, p, number, title, body, anchor, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "requirement-add" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("FR");
                let area = args.get("area").and_then(|v| v.as_str()).unwrap_or("");
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let priority = args.get("priority").and_then(|v| v.as_str()).unwrap_or("");
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let measured = args.get("measuredBy").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::declare_requirement(&self.pool, p, id, kind, area, title,
                                                             text, measured, priority, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "term-add" => {
                let term = args.get("term").and_then(|v| v.as_str()).unwrap_or("");
                let meaning = args.get("meaning").and_then(|v| v.as_str()).unwrap_or("");
                let area = args.get("area").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::declare_term(&self.pool, p, term, meaning, area, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "task-dep-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_task_dep(&self.pool, p, &g("task"), &g("dependsOn"),
                                                         args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "release-artifact-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_release_artifact(&self.pool, p, &g("name"),
                        &g("what"), &g("installedTo"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "freeze-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_freeze_row(&self.pool, p, &g("version"), &g("kind"),
                        &g("name"), &g("hash"), &self.author, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "postmortem-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_postmortem(&self.pool, p, &g("id"), &g("title"),
                        &g("summary"), &g("timeline"), &g("rootCause"), &g("lesson"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "token-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_token(&self.pool, p, &g("name"), &g("dark"),
                        &g("light"), &g("purpose"), &g("section"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "reference-source-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_reference_source(&self.pool, p, &g("name"), &g("source"),
                        &g("note"), &g("taken"), &g("sha"), &g("refType"), &g("fromProject"),
                        &g("repo"), &g("written"), &g("updated"), &g("status"), &g("tags"),
                        &g("role"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "algorithm-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_algorithm(&self.pool, p, &g("id"), &g("story"),
                        &g("title"), &g("preconditions"), &g("flow"), &g("failure"),
                        &g("notCovered"), &g("linkKind"), &g("linkTarget"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "stand-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_stand_row(&self.pool, p, &g("section"), &g("name"),
                                                           &g("value"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "donor-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_donor(&self.pool, p, &g("path"), &g("what"),
                        &g("frozenBy"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "sensor-spec-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_sensor_spec(&self.pool, p, &g("fact"), &g("reads"),
                        &g("extract"), &g("note"), &g("how"), &g("skip"), &g("allow"),
                                                          args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "guard-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_guard(&self.pool, p, &g("name"), &g("enforces"),
                        &g("scope"), &g("refuses"), &g("actsOn"), &g("pathRe"), &g("contentRe"),
                        &g("commandRe"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "crate-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_crate(&self.pool, p, &g("name"), &g("does"),
                                                       &g("doesNot"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "protocol-op-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_protocol_op(&self.pool, p, &g("op"), &g("group"),
                        &g("events"), &g("requirement"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "article-gate-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let article = args.get("article").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match crate::projector::declare_article_gate(&self.pool, p, article, &g("gate"),
                                                              &g("state"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "requirement-source-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_requirement_source(&self.pool, p, &g("id"),
                        &g("kind"), &g("target"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "requirement-scope-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_requirement_scope(&self.pool, p, &g("id"),
                        &g("outOfVersion"), &g("crosscutting"), args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "requirement-retire" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
                let by = args.get("retiredBy").and_then(|v| v.as_str()).unwrap_or("");
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::retire_requirement(&self.pool, p, id, why, by, &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "sensor-declare" => {
                let fact = args.get("fact").and_then(|v| v.as_str()).unwrap_or("");
                let about = args.get("about").and_then(|v| v.as_str()).unwrap_or("");
                let stale = args.get("staleAfterMs").and_then(|v| v.as_i64());
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::declare_sensor(&self.pool, p, fact, about, stale, &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "sensor-specs" => match crate::projector::sensor_specs(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
            },
            "donors" => match crate::projector::donors_and_guards(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
            },
            "sensors" => match crate::projector::sensors(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
            },
            "step-add" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
                let owner_kind = args.get("ownerKind").and_then(|v| v.as_str()).unwrap_or("none");
                let owner = args.get("owner").and_then(|v| v.as_str()).unwrap_or("");
                let touches = args.get("touches").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::add_step(&self.pool, p, set_name, process, ord, question,
                                                  owner_kind, owner, touches).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "version-close" => {
                let version = args.get("version").and_then(|v| v.as_str()).unwrap_or("");
                let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("closed");
                match crate::projector::set_version_state(&self.pool, p, version, state, &self.author, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "phase-gate-set" => {
                let phase = args.get("phase").and_then(|v| v.as_str()).unwrap_or("");
                let gate = args.get("gate").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_phase_gate(&self.pool, p, phase, gate, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "step-when-set" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let when_query = args.get("whenQuery").and_then(|v| v.as_str()).unwrap_or("");
                let when_why = args.get("whenWhy").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_step_when(&self.pool, set_name, process, ord, when_query, when_why, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "step-probe-set" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let probe = args.get("probe").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_step_probe(&self.pool, set_name, process, ord, probe, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "step-selftest" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::step_selftest(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "step-question-set" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
                if question.trim().is_empty() {
                    return refusal(Miss::Db("ступень без условия не переименовывается".into()));
                }
                match crate::projector::set_step_question(&self.pool, set_name, process, ord, question, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "frozen-tree-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let path = g("path");
                if path.trim().is_empty() {
                    return refusal(Miss::Db("заморозка без каталога не объявляется".into()));
                }
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                // Заморозка без хэша — не заморозка. Дверь приняла пустой хэш
                // однажды (имя поля перепутали), и гейт позеленел: сверять было
                // не с чем, а «не с чем» прочиталось как «сошлось».
                if !drop && g("hash").trim().is_empty() {
                    return refusal(Miss::Db(
                        "заморозка без хэша ничего не держит: назовите хэш дерева либо снимите заморозку `drop=true`"
                            .into()));
                }
                let client = self.pool.get().await.expect("пул отдал соединение");
                let done = if drop {
                    client.execute("DELETE FROM project_frozen_tree WHERE project_id=$1 AND path=$2",
                                   &[&p, &path]).await
                } else {
                    client.execute(
                        "INSERT INTO project_frozen_tree (project_id, path, tree_hash, why)
                         VALUES ($1,$2,$3,$4)
                         ON CONFLICT (project_id, path) DO UPDATE SET tree_hash = EXCLUDED.tree_hash,
                           why = EXCLUDED.why",
                        &[&p, &path, &g("hash"), &g("why")]).await
                };
                match done {
                    Ok(n) => ok(json!({ "path": path, "written": n, "dropped": drop })),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "scheme-term-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let (role, value) = (g("role"), g("value"));
                if role.trim().is_empty() || value.trim().is_empty() {
                    return refusal(Miss::Db("слово без роли или роль без слова не объявляются".into()));
                }
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                let client = self.pool.get().await.expect("пул отдал соединение");
                let done = if drop {
                    client.execute("DELETE FROM scheme_term WHERE role=$1 AND value=$2",
                                   &[&role, &value]).await
                } else {
                    client.execute(
                        "INSERT INTO scheme_term (role, value, ord, why) VALUES ($1,$2,$3,$4)
                         ON CONFLICT (role, value) DO UPDATE SET ord = EXCLUDED.ord, why = EXCLUDED.why",
                        &[&role, &value, &ord, &g("why")]).await
                };
                match done {
                    Ok(n) => ok(json!({ "role": role, "value": value, "written": n, "dropped": drop })),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "scheme-terms" => {
                let client = self.pool.get().await.expect("пул отдал соединение");
                match client.query("SELECT role, value, ord, why FROM scheme_term
                                     ORDER BY role, ord, value", &[]).await {
                    Ok(rows) => ok(json!({ "count": rows.len(), "terms": rows.iter().map(|r| json!({
                        "role": r.get::<_, String>(0), "value": r.get::<_, String>(1),
                        "ord": r.get::<_, i32>(2), "why": r.get::<_, String>(3) })).collect::<Vec<_>>() })),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "orphans-purge" => {
                // Убирается ТОЛЬКО то, чей набор не значится проектом. Живой
                // проект этой ручкой не тронуть по устройству запроса, а не по
                // обещанию: условие сравнивает с перечнем проектов.
                let confirm = args.get("confirm").map(|v| v == "true" || v == true).unwrap_or(false);
                let client = self.pool.get().await.expect("пул отдал соединение");
                let tables = client
                    .query(
                        "SELECT table_name FROM information_schema.columns
                          WHERE column_name = 'project_id' AND table_schema = 'public'
                          GROUP BY table_name ORDER BY table_name",
                        &[],
                    )
                    .await;
                let Ok(tables) = tables else {
                    return refusal(Miss::Db("перечень таблиц не читается".into()));
                };
                let mut found: Vec<Value> = Vec::new();
                let mut gone = 0i64;
                for t in &tables {
                    let name: String = t.get(0);
                    if name == "projects" {
                        continue;
                    }
                    let count = client
                        .query_one(
                            &format!(
                                "SELECT count(*) FROM \"{name}\"
                                  WHERE project_id NOT IN (SELECT id FROM projects)"
                            ),
                            &[],
                        )
                        .await;
                    let Ok(row) = count else { continue };
                    let n: i64 = row.get(0);
                    if n == 0 {
                        continue;
                    }
                    found.push(json!({ "table": name, "rows": n }));
                    if confirm {
                        if let Ok(d) = client
                            .execute(
                                &format!(
                                    "DELETE FROM \"{name}\"
                                      WHERE project_id NOT IN (SELECT id FROM projects)"
                                ),
                                &[],
                            )
                            .await
                        {
                            gone += d as i64;
                        }
                    }
                }
                ok(json!({ "tables": found, "removed": gone, "confirmed": confirm }))
            }
            "surface-source-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let (surface, kind, name) = (g("surface"), g("entityKind"), g("entityName"));
                if surface.trim().is_empty() || kind.trim().is_empty() {
                    return refusal(Miss::Db("источник без поверхности или без вида не объявляется".into()));
                }
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                let client = self.pool.get().await.expect("пул отдал соединение");
                let done = if drop {
                    client.execute("DELETE FROM project_surface_source WHERE project_id=$1 AND surface=$2
                                      AND entity_kind=$3 AND entity_name=$4",
                                   &[&p, &surface, &kind, &name]).await
                } else {
                    client.execute(
                        "INSERT INTO project_surface_source (project_id, surface, entity_kind, entity_name)
                         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
                        &[&p, &surface, &kind, &name]).await
                };
                match done {
                    Ok(n) => ok(json!({ "surface": surface, "kind": kind, "name": name,
                                        "written": n, "dropped": drop })),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "exception-set" => {
                // Исключение объявляется для ЛЮБОГО правила, а не для одного
                // разобранного из таблицы в документе. Оно названо — правило,
                // сущность, причина, кто решил, — и видно в ответе гейта тремя
                // числами. Молча вычтенное исключение — дыра с разрешением.
                let rule = args.get("rule").and_then(|v| v.as_str()).unwrap_or("");
                let kind = args.get("entityKind").and_then(|v| v.as_str()).unwrap_or("");
                let id = args.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
                let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                // Задача, которая побег отменит. Пусто — побег бессрочный.
                let closes = args.get("closes").and_then(|v| v.as_str()).unwrap_or("");
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                if rule.is_empty() || id.is_empty() {
                    return refusal(Miss::Db("исключение без правила или без сущности не объявляется".into()));
                }
                if reason.trim().is_empty() && !drop {
                    return refusal(Miss::Db("исключение без причины — это дыра с разрешением, а не решение".into()));
                }
                let client = self.pool.get().await.expect("пул отдал соединение");
                let done = if drop {
                    client.execute("DELETE FROM rule_exception WHERE project_id=$1 AND rule=$2 AND entity_id=$3",
                                   &[&p, &rule, &id]).await
                } else {
                    client.execute(
                        "INSERT INTO rule_exception (project_id, rule, entity_kind, entity_id, reason, decided_by, closes)
                         VALUES ($1,$2,$3,$4,$5,$6,$7)
                         ON CONFLICT (project_id, rule, entity_kind, entity_id)
                           DO UPDATE SET reason = EXCLUDED.reason, decided_by = EXCLUDED.decided_by,
                                         closes = EXCLUDED.closes",
                        &[&p, &rule, &kind, &id, &reason, &self.author, &closes]).await
                };
                match done {
                    Ok(n) => ok(json!({ "rule": rule, "entity": id, "written": n, "closes": closes,
                                        "dropped": drop, "decidedBy": self.author })),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "gate-sign" => {
                let phase = args.get("phase").and_then(|v| v.as_str()).unwrap_or("");
                let at = args.get("signedAt").and_then(|v| v.as_str()).unwrap_or("");
                let by = args.get("signedBy").and_then(|v| v.as_str()).unwrap_or("");
                let wording = args.get("wording").and_then(|v| v.as_str()).unwrap_or("");
                let note = args.get("note").and_then(|v| v.as_str()).unwrap_or("");
                let docs: Vec<(String, String)> = args
                    .get("under").and_then(|v| v.as_array())
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("kind")?.as_str()?.to_owned(),
                        it.get("id").and_then(|v| v.as_str()).unwrap_or("").to_owned()))).collect())
                    .unwrap_or_default();
                match crate::projector::sign_gate(&self.pool, p, phase, at, by, wording, note, &docs).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e),
                }
            }
            "links-rewrite" => {
                let apply = args.get("apply").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::rewrite_links(&self.pool, &self.kinds, p, !apply).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "links-retarget" => {
                let apply = args.get("apply").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::retarget_links(&self.pool, p, !apply).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "author-set" => {
                let who = args.get("author").and_then(|v| v.as_str()).unwrap_or("");
                match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                    Ok((k, n)) => match crate::projector::set_author(&self.pool, p, &k, &n, who, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(Miss::Db(e.to_string())),
                    },
                    Err(e) => refusal(e),
                }
            }
            "authors" => match crate::projector::authors(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "principal-allow" => {
                let who = args.get("principal").and_then(|v| v.as_str()).unwrap_or("");
                let note = args.get("note").and_then(|v| v.as_str());
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                if who.is_empty() { return refusal(Miss::Db("человек не назван".into())); }
                match crate::projector::allow_principal(&self.pool, who, note, drop, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "principals" => match crate::projector::principals(&self.pool).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            // Сверка порождённого — отдельной ручкой, а не только внутри
            // тяжёлой пересборки: спросить «отстал ли файл» надо уметь, не
            // переписывая при этом все проекции.
            "generated-check" => match crate::projector::check_generated(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "version-freeze" => {
                let v = args.get("version").and_then(|x| x.as_str()).unwrap_or("");
                if v.is_empty() { return refusal(Miss::Db("выпуск не назван".into())); }
                match crate::projector::freeze_version(&self.pool, p, v, &self.author).await {
                    Ok(x) => ok(x),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "version-delta" => {
                let v = args.get("version").and_then(|x| x.as_str()).unwrap_or("");
                if v.is_empty() { return refusal(Miss::Db("выпуск не назван".into())); }
                match crate::projector::version_delta(&self.pool, p, v).await {
                    Ok(x) => ok(x),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "agent-set" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                if name.is_empty() || body.is_empty() {
                    return refusal(Miss::Db("субагент без имени или без тела не записывается".into()));
                }
                match crate::projector::set_agent(&self.pool, set, name,
                        args.get("description").and_then(|v| v.as_str()), body,
                        args.get("tools").and_then(|v| v.as_str()),
                        args.get("model").and_then(|v| v.as_str()), &self.author, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "skill-set" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let description = args.get("description").and_then(|v| v.as_str());
                if name.is_empty() || body.is_empty() {
                    return refusal(Miss::Db("скилл без имени или без тела не записывается".into()));
                }
                // Умение и субагент законно носят одно имя: `godzy-preflight` —
                // и процедура, которой следует сессия, и работник, которого
                // отправляют. Запрет на совпадение имён стоял здесь один раз и
                // ломал правку умения; настоящая защита — не запрет, а наличие
                // своей двери у субагента (`agent-set`), которой прежде не было.
                let allowed = args.get("allowedTools").and_then(|v| v.as_str());
                let flag = args.get("disableModelInvocation").and_then(|v| v.as_bool());
                match crate::projector::set_skill(&self.pool, set, name, description, body,
                                                   allowed, flag, &self.author, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "skills-paths" => {
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::skills_with_paths(&self.pool, set).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "screen-area-set" => {
                let screen = args.get("screen").and_then(|v| v.as_str()).unwrap_or("");
                let area = args.get("area").and_then(|v| v.as_str()).unwrap_or("");
                if screen.is_empty() {
                    return refusal(Miss::Db("область ставится экрану; экран не назван".into()));
                }
                match crate::projector::set_screen_area(&self.pool, p, screen, area, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "reparse" => match crate::store::reparse_all(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "gate-item-waive" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                // Отмена адресуется ИМЕНЕМ пункта. Прежде — заголовком, и стоило
                // переписать формулировку, как отмена оставалась висеть в пустоте.
                let who = if g("id").is_empty() { g("item") } else { g("id") };
                match crate::projector::waive_gate_item(&self.pool, p, &g("phase"), &who,
                                                         &g("why"), &self.author, drop,
                                                         &g("holdsWhileEmpty")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "gate-measure" => match crate::projector::measure_gates(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
            },
            "gate-selftest" => match crate::projector::gate_selftest(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "order" => match crate::projector::order(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "summary" => match crate::projector::summary(&self.pool, p, kind_arg).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            },
            "question-holders" => match crate::projector::question_holders(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "ceiling-set" => {
                let rule = args.get("rule").and_then(|v| v.as_str()).unwrap_or("");
                let ceiling = args.get("ceiling").and_then(|v| v.as_i64())
                    .or_else(|| args.get("ceiling").and_then(|v| v.as_str()).and_then(|t| t.parse().ok()))
                    .unwrap_or(0) as i32;
                let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_ceiling(&self.pool, p, rule, ceiling, why,
                        args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "gate-item-set" => {
                let phase = args.get("phase").and_then(|v| v.as_str()).unwrap_or("");
                // `id` адресует пункт, `item` — заголовок для человека. Прежде
                // ключом был заголовок, и всякая правка формулировки заводила
                // пункт заново, оставляя прежний сиротой вместе с его отметками.
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let item = args.get("item").and_then(|v| v.as_str()).unwrap_or("");
                let gk = args.get("itemKind").and_then(|v| v.as_str()).unwrap_or("query");
                let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
                let query = args.get("query").and_then(|v| v.as_str());
                let owner = args.get("owner").and_then(|v| v.as_str());
                let probe = args.get("probe").and_then(|v| v.as_str());
                if phase.is_empty() || id.is_empty() {
                    return refusal(Miss::Db(
                        "пункт гейта без фазы или без имени не заводится: `id` адресует пункт, `item` его объясняет".into()));
                }
                match crate::projector::set_gate_item(&self.pool, p, phase, id, item, gk, query, owner,
                                                      probe, why, args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false)).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(crate::projector::db_says(&e))),
                }
            }
            "method-set" => {
                let ord = args.get("ord").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                let mk = args.get("methodKind").and_then(|v| v.as_str()).unwrap_or("unknown");
                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let drop = args.get("drop").map(|v| v == "true" || v == true).unwrap_or(false);
                match crate::projector::set_method(&self.pool, p, kind_arg, id.unwrap_or(""), ord, mk,
                                                   method, &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "readiness" => match crate::projector::readiness_computed(&self.pool, p, kind_arg, id.unwrap_or("")).await {
                // Отказ остаётся отказом и в оболочке: успех с полем `unprojected`
                // агент прочтёт как ответ.
                Ok(v) if v.get("unprojected") == Some(&json!(true)) => json!({
                    "content": [{ "type": "text", "text": v["why"].as_str().unwrap_or("") }],
                    "isError": true
                }),
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "requirements-of" => match crate::projector::requirements_of(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "tasks-of" => match crate::projector::tasks_of_story(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "preflight-queue" => match crate::projector::preflight_queue(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "claims" => match crate::projector::claims(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "frozen-trees" | "addresses-declared" | "tree-declared" | "exceptions" => match self.ask(name, kind_arg, id, args).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            },
            "gate" => match crate::projector::gate(&self.pool, p, id).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "next-task" => match crate::projector::next_task(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "blockers" => match crate::projector::task_blockers(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "events" | "norm-versions" | "measurements" | "plan" | "kinds-due" => {
                match self.ask(name, kind_arg, id, args).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e),
                }
            }
            "document-add" => {
                // Заведение идёт СВОЕЙ дорогой, мимо `write`: тот сперва находит
                // сущность по виду и имени, а у заводимой находить нечего.
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                match crate::store::create(
                    &self.pool, &self.kinds, &self.project, kind_arg, id.unwrap_or(""),
                    args.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                    &self.author, now,
                )
                .await
                {
                    Ok(v) => ok(v),
                    Err(e) => refusal(Miss::Db(e.to_string())),
                }
            }
            "put" | "put-section" | "rm" => self.write(name, kind_arg, id, args).await,
            "sweep" => match crate::store::sweep_orphans(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(Miss::Db(e.to_string())),
            },
            "reproject" => match self.projections().await {
                Ok(v) => ok(json!({ "own": v })),
                Err(e) => refusal(e),
            },
            other => json!({
                "content": [{ "type": "text", "text": format!("нет такого инструмента: {other}") }],
                "isError": true
            }),
        }
    }

    /// Конвейер пересборки: до-проход → предметные проекции → после-проход.
    async fn projections(&self) -> Result<Value, Miss> {
        // `e.to_string()` у ошибки Postgres — слова «db error» и ничего больше:
        // причина лежит в источнике, и её показывает `db_says`. Пересборка,
        // упавшая молча, здесь три месяца отвечала «база не ответила».
        let say = |step: &str| {
            let step = step.to_owned();
            move |e: tokio_postgres::Error| Miss::Db(format!("{step}: {}", crate::projector::db_says(&e)))
        };
        // Время каждого шага — В ОТВЕТЕ. Правка документа шла сорок три секунды,
        // и без разметки виновным назначался тот шаг, на который думалось: те же
        // пересборки, позванные отдельно, укладываются в сто тридцать миллисекунд.
        let t = std::time::Instant::now();
        let before = crate::projector::rebuild_before(&self.pool, &self.project)
            .await
            .map_err(say("сверка до пересборки"))?;
        let ms_before = t.elapsed().as_millis() as u64;
        let t = std::time::Instant::now();
        let subject = crate::reproject::reproject(&self.pool, &self.project)
            .await
            .map_err(say("пересборка сущностей"))?;
        let ms_subject = t.elapsed().as_millis() as u64;
        let t = std::time::Instant::now();
        let after = crate::projector::rebuild(&self.pool, &self.project)
            .await
            .map_err(say("пересборка проекций"))?;
        let ms_after = t.elapsed().as_millis() as u64;
        let t = std::time::Instant::now();
        let generated = crate::projector::check_generated(&self.pool, &self.project)
            .await
            .map_err(say("сверка выведенного"))?;
        let ms_generated = t.elapsed().as_millis() as u64;
        Ok(json!({ "before": before, "subject": subject, "after": after, "generated": generated,
                   "мс": { "сверка до": ms_before, "сущности": ms_subject,
                           "проекции": ms_after, "выведенное": ms_generated } }))
    }

    /// Запросы к таблицам этого сервера.
    async fn ask(&self, name: &str, kind: &str, id: Option<&str>, args: &Value) -> Result<Value, Miss> {
        let client = self.pool.get().await.expect("пул отдал соединение");
        let p = &self.project;
        match name {
            "events" => {
                // Летопись сущности — из ДВУХ источников, и они разного рода.
                //
                // `declared` — то, что документ говорит о себе сам: журнал
                // «Дата · Событие · Кем» внутри вопроса. Он выводится из текста
                // и пересобирается вместе с ним.
                //
                // `edit` — то, что случилось на самом деле: правка документа,
                // её ревизия и имя правившего. Единственный писатель этого
                // факта — `project_document_revisions`, и второй копии он не
                // получает: две записи одного события расходятся молча.
                //
                // Поэтому здесь СЛИЯНИЕ НА ЧТЕНИИ, а не общая таблица.
                let kind = if kind.is_empty() { "question" } else { kind };
                let id = id.unwrap_or("");
                let rows = client
                    .query(
                        "SELECT ord, at::text, event, actor FROM entity_event
                          WHERE project_id = $1 AND entity_kind = $2 AND entity_id = $3 ORDER BY ord",
                        &[p, &kind, &id],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                let mut events: Vec<Value> = rows
                    .iter()
                    .map(|r| json!({
                        "source": "declared",
                        "ord": r.get::<_, i32>(0), "at": r.get::<_, Option<String>>(1),
                        "event": r.get::<_, String>(2), "actor": r.get::<_, String>(3) }))
                    .collect();
                let declared = events.len();

                let mut edits = 0usize;
                if let Ok((k, n)) = entities::locate(&self.pool, &self.kinds, p, kind,
                                                     if id.is_empty() { None } else { Some(id) }).await {
                    let rows = client
                        .query(
                            "SELECT revision, to_timestamp(written_at/1000)::date::text, written_by, bytes
                               FROM project_document_revisions
                              WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3
                              ORDER BY revision",
                            &[p, &k, &n],
                        )
                        .await
                        .map_err(|e| Miss::Db(e.to_string()))?;
                    edits = rows.len();
                    for r in &rows {
                        let rev: i64 = r.get(0);
                        events.push(json!({
                            "source": "edit",
                            "revision": rev,
                            "at": r.get::<_, Option<String>>(1),
                            "event": format!("правка {rev}"),
                            "actor": r.get::<_, String>(2),
                            "bytes": r.get::<_, i32>(3),
                        }));
                    }
                }
                Ok(json!({ "kind": kind, "id": id, "count": events.len(),
                           "declared": declared, "edits": edits, "events": events }))
            }
            "norm-versions" => {
                let kind = if kind.is_empty() { "constitution" } else { kind };
                let version = args.get("version").and_then(|v| v.as_str()).unwrap_or("");
                let rows = client
                    .query(
                        "SELECT version, at::text, changed, by_decision FROM norm_version
                          WHERE project_id = $1 AND entity_kind = $2 AND ($3 = '' OR version = $3)
                          ORDER BY at DESC, version DESC",
                        &[p, &kind, &version],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                Ok(json!({ "kind": kind, "count": rows.len(),
                    "versions": rows.iter().map(|r| json!({
                        "version": r.get::<_, String>(0), "at": r.get::<_, Option<String>>(1),
                        "changed": r.get::<_, String>(2), "byDecision": r.get::<_, Option<String>>(3) })).collect::<Vec<_>>() }))
            }
            "measurements" => {
                let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let like = format!("%{subject}%");
                let rows = client
                    .query(
                        "SELECT subject, at::text, value, stated_in_id FROM measurement
                          WHERE project_id = $1 AND ($2 = '' OR subject ILIKE $3 OR value ILIKE $3)
                          ORDER BY at, subject",
                        &[p, &subject, &like],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                Ok(json!({ "count": rows.len(),
                    "measurements": rows.iter().map(|r| json!({
                        "subject": r.get::<_, String>(0), "at": r.get::<_, Option<String>>(1),
                        "value": r.get::<_, String>(2), "statedIn": r.get::<_, Option<String>>(3) })).collect::<Vec<_>>() }))
            }
            "frozen-trees" => {
                let rows = client
                    .query("SELECT path, tree_hash FROM project_frozen_tree WHERE project_id = $1
                             ORDER BY path", &[p])
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                Ok(json!({ "count": rows.len(), "trees": rows.iter().map(|r| json!({
                    "path": r.get::<_, String>(0), "hash": r.get::<_, String>(1) })).collect::<Vec<_>>() }))
            }
            "addresses-declared" => {
                // Адреса «файл:строка», названные набором. Датчик спрашивает их
                // здесь, а не индексирует всё дерево: шестьдесят восемь тысяч
                // строк ради семнадцати адресов — не измерение, а склад.
                let rows = client
                    .query(
                        "SELECT DISTINCT x[1] AS path, x[3] AS line
                           FROM project_documents d
                           CROSS JOIN LATERAL regexp_matches(d.content,
                             '`([A-Za-z0-9_./-]+[.](rs|sql|ts|tsx|yaml|toml)):([0-9]+)`', 'g') AS x
                          WHERE d.project_id = $1
                          ORDER BY 1, 2",
                        &[p],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                let out: Vec<Value> = rows
                    .iter()
                    .map(|r| json!({ "path": r.get::<_, String>(0), "line": r.get::<_, String>(1) }))
                    .collect();
                Ok(json!({ "count": out.len(), "addresses": out }))
            }
            "tree-declared" => {
                // Объявленное дерево — таблица «Путь | Состояние» в предмете
                // `file-tree`. Блок ищется по заголовку, а не по номеру: номер
                // блока меняется от любой правки выше по документу.
                let rows = client
                    .query(
                        "WITH head AS (
                           SELECT block_ord FROM project_document_cells
                            WHERE project_id = $1 AND entity_name = 'file-tree'
                              AND row_ord = 0 AND col = 0 AND lower(value) = 'путь')
                         SELECT c.row_ord,
                                max(CASE WHEN c.col = 0 THEN c.value END),
                                max(CASE WHEN c.col = 1 THEN c.value END)
                           FROM project_document_cells c JOIN head h ON h.block_ord = c.block_ord
                          WHERE c.project_id = $1 AND c.entity_name = 'file-tree'
                            AND c.row_ord > 0
                          GROUP BY c.row_ord ORDER BY c.row_ord",
                        &[p],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                let paths: Vec<Value> = rows
                    .iter()
                    .filter_map(|r| {
                        let path = r.get::<_, Option<String>>(1)?;
                        Some(json!({ "path": path.trim().trim_matches('`').to_owned(),
                                     "state": r.get::<_, Option<String>>(2).unwrap_or_default() }))
                    })
                    .collect();
                Ok(json!({ "count": paths.len(), "paths": paths }))
            }
            "exceptions" => {
                let rule = args.get("rule").and_then(|v| v.as_str()).unwrap_or("");
                let rows = client
                    .query(
                        "SELECT e.rule, e.entity_kind, e.entity_id, e.reason, e.decided_by,
                                EXISTS (SELECT 1 FROM project_story_requirements sr
                                         WHERE sr.project_id = e.project_id AND sr.story_id = e.entity_id)
                           FROM rule_exception e
                          WHERE e.project_id = $1 AND ($2 = '' OR e.rule = $2)
                          ORDER BY e.rule, e.entity_id",
                        &[p, &rule],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                let items: Vec<Value> = rows
                    .iter()
                    .map(|r| json!({
                        "rule": r.get::<_, String>(0), "kind": r.get::<_, String>(1),
                        "id": r.get::<_, String>(2), "reason": r.get::<_, String>(3),
                        "declaredIn": r.get::<_, String>(4),
                        // Дыры уже нет, а исключение стоит: его пора снять.
                        "holeGone": r.get::<_, bool>(5),
                    }))
                    .collect();
                let stale = items.iter().filter(|i| i["holeGone"] == json!(true)).count();
                Ok(json!({ "exceptions": items, "count": items.len(), "toRetire": stale }))
            }
            "plan" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let like = format!("%{name}%");
                let rows = client
                    .query(
                        "SELECT name, claim, state_text FROM project_document_plan
                          WHERE project_id = $1 AND ($2 = '' OR name ILIKE $3) ORDER BY name",
                        &[p, &name, &like],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                Ok(json!({ "count": rows.len(),
                    "plan": rows.iter().map(|r| json!({
                        "name": r.get::<_, String>(0), "claim": r.get::<_, String>(1),
                        "says": r.get::<_, Option<String>>(2) })).collect::<Vec<_>>() }))
            }
            // Три состояния различаются и не смешиваются: положено и разложено;
            // положено и не разложено; не объявлено вовсе — дефект плана.
            _ => {
                let mut due = Vec::new();
                let mut undeclared = Vec::new();
                for (kind, k) in &self.kinds.0 {
                    match k.projection.as_deref() {
                        Some("due") => due.push(json!({ "kind": kind, "documents": k.at.is_none() })),
                        None => undeclared.push(kind.clone()),
                        _ => {}
                    }
                }
                Ok(json!({
                    "due": due,
                    "undeclared": undeclared,
                    "note": "«не объявлено» — не «не надо»: этого не сказал никто, и спросить придётся человека"
                }))
            }
        }
    }

    async fn write(&self, name: &str, kind: &str, id: Option<&str>, args: &Value) -> Value {
        let (owner_kind, owner_name) = match entities::locate(&self.pool, &self.kinds, &self.project, kind, id).await {
            Ok(pair) => pair,
            Err(e) => return refusal(e),
        };
        // Запись документа идёт на Rust; у донора остаётся только пересборка
        // предметных проекций, и она вызывается следом.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let expected = args.get("expectedRevision").and_then(|v| v.as_i64());
        let done = match name {
            "put" => crate::store::put(
                &self.pool, &self.project, &owner_kind, &owner_name,
                args.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                &self.author, expected, now,
            ).await,
            "put-section" => crate::store::put_section(
                &self.pool, &self.project, &owner_kind, &owner_name,
                args.get("anchor").and_then(|v| v.as_str()).unwrap_or(""),
                args.get("body").and_then(|v| v.as_str()).unwrap_or(""),
                &self.author, expected, now,
            ).await,
            _ => crate::store::remove(&self.pool, &self.project, &owner_kind, &owner_name).await,
        };
        let mut v = match done {
            Ok(v) => v,
            Err(e) => return refusal(Miss::Db(crate::projector::db_says(&e))),
        };
        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("").to_owned();
        if matches!(status.as_str(), "written" | "deleted") {
            // След правки — ДО пересборки: пересборка стирает только выведенное
            // из текста, но порядок здесь важен и по смыслу — сначала записано,
            // что случилось, потом пересчитано, что из этого следует.
            let revision = v.get("revision").and_then(|r| r.as_i64()).unwrap_or(0);
            let bytes = v.get("bytes").and_then(|b| b.as_i64()).unwrap_or(0);
            let event = if status == "deleted" {
                "снято".to_owned()
            } else {
                format!("{name}: правка {revision}, {bytes} байт")
            };
            if let Err(e) = crate::projector::record_edit(
                &self.pool, &self.project, kind, id.unwrap_or(kind), revision, &self.author, &event,
            ).await {
                // Летопись, промолчавшая об ошибке, — та же потеря следа.
                return refusal(Miss::Db(format!("правка принята, но след не записан: {e}")));
            }
            // Отложенная пересборка — для правки МНОГИХ документов подряд: она идёт
            // по всему набору, и делать её после каждой из восьмидесяти записей
            // значит платить восемьдесят раз за одно и то же. Дверь остаётся дверью:
            // запись прошла сервером, отложено только следствие. Ответ говорит об
            // этом ВСЛУХ — молча устаревшая проекция читалась бы как свежая.
            if args.get("deferProjection").map(|v| v == "true" || v == true).unwrap_or(false) {
                if let Some(m) = v.as_object_mut() {
                    m.insert("projection".into(),
                             json!("отложена: проекции устарели, пока не позван `reproject`"));
                }
                return ok(v);
            }
            match self.projections().await {
                Ok(own) => {
                    if let Some(m) = v.as_object_mut() {
                        m.insert("own".into(), own);
                    }
                }
                Err(e) => return refusal(e),
            }
        }
        if matches!(status.as_str(), "conflict" | "not_found" | "no_such_section" | "invalid_path") {
            return json!({ "content": [{ "type": "text", "text": v.to_string() }], "isError": true });
        }
        ok(v)

    }

}
