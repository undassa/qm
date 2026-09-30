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

use crate::db::Says;
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

/// Ключ счёта вызовов: набор, дверь, автор, исход.
pub(crate) type DoorKey = (String, String, String, bool);
/// Что посчитано: вызовов, суммарно миллисекунд, самый долгий.
pub(crate) type DoorTally = (i64, i64, i32);

/// Счёт вызовов дверей, копящийся в памяти до круга сборщика.
///
/// Образец взят у `db::BUSY`: счёт живёт в памяти, а строкой ложится раз в
/// круг. Потерять при выкатке можно неполный круг счёта — это цена, названная
/// вслух, и она меньше цены записи на каждый вызов, посчитанной ревью.
static CALLS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<DoorKey, DoorTally>>> =
    std::sync::LazyLock::new(Default::default);

/// Снять накопленное и обнулить. Зовёт сборщик.
pub(crate) fn take_calls() -> std::collections::HashMap<DoorKey, DoorTally> {
    match CALLS.lock() {
        Ok(mut calls) => std::mem::take(&mut *calls),
        Err(_) => Default::default(),
    }
}

/// Вернуть снятое обратно в память: неудавшаяся запись не должна стирать счёт.
///
/// Тот же довод, что у `watch::remember_strain`: счёт, потерянный вместе с
/// отказом записи, делает вызовы невидимыми ровно в тот круг, когда база и была
/// занята, — то есть врёт именно там, где интересно.
pub(crate) fn return_calls(counted: std::collections::HashMap<DoorKey, DoorTally>) {
    let Ok(mut calls) = CALLS.lock() else { return };
    for (key, (n, sum, max)) in counted {
        let seen = calls.entry(key).or_insert((0, 0, 0));
        seen.0 += n;
        seen.1 += sum;
        seen.2 = seen.2.max(max);
    }
}

/// Отказ говорит словом, что именно не так.
///
/// «Нет проекции» и «ничего не нашлось» — разные ответы, и агент обязан их
/// различать: на втором он построит задачу без контекста и не заметит.
fn refusal(m: Miss) -> Value {
    let m_busy = matches!(m, Miss::Busy(_));
    let text = match m {
        Miss::NoKind(k) => format!("нет такого вида: {k}"),
        // «Нужно имя» отправляло искать имя, которого не существует: у
        // одиночного вида имени и не бывает, а документа не было вовсе.
        // Верная причина сразу говорит, что делать.
        Miss::NoEntity(k, id) if id.is_empty() => format!(
            "документа вида {k} в наборе нет. Если вид одиночный, имени у него и не бывает — \
             заводить надо документ; если нет, назовите имя"),
        Miss::NoEntity(k, id) => format!(
            "нет такой сущности: {k} {id}. Если она объявлена дверью, а документа нет, — \
             `declared-unwritten` покажет такие имена, а завести документ — \
             `document-add kind={k} id={id} content=…`"),
        Miss::Unprojected(k) => {
            format!("вид {k} в базу не спроецирован: ответ неизвестен, а не пуст — спрашивать нечего, а не ничего нет")
        }
        Miss::Busy(why) => format!("{why}. Работа не сделана: повторите этот же вызов"),
        Miss::Refused(why) => why,
        Miss::Db(e) => format!("база не ответила: {e:#}"),
    };
    // Занятость помечается ПОЛЕМ, а не словами: по словам её не отличить от
    // отказа по существу, и цикл повторов у агента не знает, повторять ли.
    // Признак занятости — в `_meta`: это объявленное место для своего поля в
    // ответе инструмента. Поле рядом с `content` протокол не обещает, и строгий
    // клиент вправе его выбросить — вместе с единственным сигналом «повторите».
    json!({ "content": [{ "type": "text", "text": text }], "isError": true,
            "_meta": crate::door::mark_busy(m_busy) })
}

/// Запись прошла, а пересборка за ней упала.
///
/// Ответ «база не ответила» читался как «ничего не записано», хотя ревизии уже
/// сдвинулись: вызвавший повторял запись или искал соединение, а не причину.
fn written_unprojected(done: &str, m: Miss) -> Value {
    // Занятость остаётся занятостью и здесь: правка записана, а пересборка
    // упёрлась в перегрузку — повторять стоит её, и вызвавший должен это знать.
    let busy = matches!(m, Miss::Busy(_));
    let why = match m {
        Miss::Db(e) | Miss::Refused(e) | Miss::Busy(e) => e,
        other => return refusal(other),
    };
    let said = format!(
        "{done}, но проекции не собраны: {why}. Пока `reproject` не пройдёт, гейт и план судят по прежним"
    );
    refusal(if busy { Miss::Busy(said) } else { Miss::Refused(said) })
}

/// Число, пришедшее СТРОКОЙ, — то же число.
///
/// Двери читали `as_i64()`, а клиент передаёт доводы как `ключ=значение`, то
/// есть строками: `ord=4` приходил как `"4"`, `as_i64()` отдавал `None`, и
/// умолчание превращало четвёртую ступень в `-1`. Дверь отвечала «ступени -1 в
/// процессе нет» — про ступень, которую не спрашивали.
///
/// Тот же разряд, что был у `drop`: там строка `"true"` не совпадала с булевым
/// `true`, и шесть дверей молча писали вместо того, чтобы удалять.
/// Список, приехавший ЛИБО массивом (MCP), ЛИБО строкой с массивом внутри (CLI).
///
/// `mh call holds='["a","b"]'` кладёт в аргумент строку: `as_array()` отдаёт
/// None, список молча становится пустым, и дверь проверяет пустоту вместо
/// названного. На пробе это выглядело отказом «ничего не держит» — правильным
/// словом о неверном предмете. Тот же род, что чинил `num()`.
/// То же для массива ОБЪЕКТОВ: `facts`, `verdicts`, `skills`, `open`, `under`,
/// `states`. Из CLI они приезжают строкой и молча становились пустыми — а
/// пустая подача читается как «сказано, что ничего нет».
fn rows(args: &Value, key: &str) -> Vec<Value> {
    match args.get(key) {
        Some(Value::String(t)) => serde_json::from_str::<Value>(t).ok()
            .and_then(|v| v.as_array().cloned()).unwrap_or_default(),
        Some(v) => v.as_array().cloned().unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Булев довод двери, в любом написании, каким его подают.
///
/// Командная строка отдаёт ВСЕ доводы строками, а сравнение шло с одним
/// написанием — `"true"`. Замер 21.09: сессия сняла объявленное ребро вызовом
/// `task-requirement-add … drop=1`, дверь ответила успехом и не сняла ничего;
/// «нет» получилось молча, и две находки гейта стояли до тех пор, пока не
/// угадали `drop:=true`. Дверь, принимающая довод и не понимающая его, хуже
/// двери, которая отказывает.
pub(crate) fn flag(args: &Value, key: &str) -> bool {
    args.get(key).is_some_and(truthy)
}

/// Поданное значение как «да».
fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().is_some_and(|n| n != 0),
        Value::String(s) => matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "y" | "on" | "да"),
        _ => false,
    }
}

fn num(args: &Value, key: &str) -> Option<i64> {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn ok(value: Value) -> Value {
    let text = match &value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    json!({ "content": [{ "type": "text", "text": text }] })
}

#[cfg(test)]
mod near {
    use super::one_letter_apart;

    /// Подсказка обязана быть подсказкой, а не вторым списком дверей: похожим
    /// считается слово, отличающееся ОДНОЙ буквой. Проба держит эту границу —
    /// без неё «похожие» разрастаются, и читать их будет тот, кто уже ошибся.
    #[test]
    fn a_typo_is_near_and_another_word_is_not() {
        assert!(one_letter_apart("gate", "gat"), "пропущенная буква");
        assert!(one_letter_apart("gate", "gates"), "лишняя буква");
        assert!(one_letter_apart("task", "tesk"), "другая буква");
        assert!(!one_letter_apart("gate", "gate"), "то же слово не похоже, а равно");
        assert!(!one_letter_apart("release", "search"), "разные слова");
        assert!(!one_letter_apart("task", "tasks-of"), "разница больше буквы");
    }
}

#[cfg(test)]
mod taken_away {
    /// Снятая дверь не может быть одновременно живой.
    ///
    /// Перечень снятых отвечает «дверь снята, прибор объявлен репозиторием», и
    /// это обещание. Вернут дверь, забыв убрать её отсюда, — обещание станет
    /// ложью, и звавший пойдёт искать по ложному следу. Пишущей дверь обязана
    /// числиться в `WRITES` (все четырнадцать снятых писали), так что пересечение
    /// двух перечней и есть возврат.
    #[test]
    fn a_door_said_to_be_gone_is_not_alive() {
        let gone = super::Mcp::TAKEN_AWAY_NAMES;
        assert!(!gone.is_empty(), "перечень снятых пуст: ответ про прибор некому дать");
        for name in gone {
            assert!(!super::Mcp::writes().contains(name), "дверь «{name}» объявлена снятой и живой разом");
        }
        let mut seen = gone.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "имя в перечне снятых дважды");
    }
}

/// Отличаются ли слова одной буквой: опечатка, а не другое слово.
fn one_letter_apart(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let (long, short) = if a.chars().count() >= b.chars().count() { (a, b) } else { (b, a) };
    let (l, s): (Vec<char>, Vec<char>) = (long.chars().collect(), short.chars().collect());
    if l.len() - s.len() > 1 {
        return false;
    }
    let (mut i, mut j, mut slips) = (0usize, 0usize, 0usize);
    while i < l.len() && j < s.len() {
        if l[i] == s[j] {
            i += 1;
            j += 1;
            continue;
        }
        slips += 1;
        if slips > 1 {
            return false;
        }
        if l.len() == s.len() {
            i += 1;
            j += 1;
        } else {
            i += 1;
        }
    }
    slips + (l.len() - i) + (s.len() - j) <= 1
}

/// Доводы двери: обязательные и остальные, которые она тоже принимает.
fn door_arguments(schema: Option<&Value>) -> (Value, Vec<String>) {
    let needs = schema.and_then(|s| s.get("required")).cloned().unwrap_or(json!([]));
    let required: Vec<&str> =
        needs.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|v| v.as_str()).collect());
    let takes = schema
        .and_then(|s| s.get("properties"))
        .and_then(|p| p.as_object())
        .map_or(Vec::new(), |p| p.keys().filter(|k| !required.contains(&k.as_str())).cloned().collect());
    (needs, takes)
}

impl Mcp {
    /// Перечень инструментов: пара на вид плюс общие.
    /// Двери, похожие на названную: общая приставка, вхождение, либо разница в
    /// одну букву. Не «умный подбор», а три дешёвых правила: длинный список
    /// похожих хуже короткого — читать его будет тот, кто уже ошибся.
    fn near(&self, asked: &str) -> Vec<String> {
        let asked = asked.trim().to_lowercase();
        if asked.len() < 2 {
            return Vec::new();
        }
        let mut near: Vec<String> = self
            .tools()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .filter(|name| {
                let n = name.to_lowercase();
                n.contains(&asked) || asked.contains(&n) || one_letter_apart(&n, &asked)
            })
            .collect();
        near.sort();
        // ПЯТЬ ИМЁН ВИДА НОСЯТ ДВЕ ДВЕРИ: `gate`, `board`, `claims`, `coverage`,
        // `goals` — и вид, и разборная ручка; какая из двух отвечает, решается
        // доводом `kind` либо приставкой `-list`. В перечне они стоят дважды
        // законно, а в подсказке выходило «Похожие: gate · gate» — ответ,
        // который читает тот, кто уже ошибся, и второе имя не говорит ему
        // ничего.
        near.dedup();
        near.truncate(5);
        near
    }

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
        tools.push(json!({ "name": "strain", "description": "натуга сервера: отказы «занято» и взятия второго соединения при живом первом",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "sections", "description": "заголовки разделов сущности с якорями",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя, если вид не одиночка") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "section", "description": "один раздел сущности по якорю",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя"), "anchor": s("якорь раздела") }, "required": ["kind", "anchor"] } }));
        tools.push(json!({ "name": "backlinks", "description": "кто ссылается на сущность — сущностями, а не файлами",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "documents", "description": "ПЕРЕЧЕНЬ ДОКУМЕНТОВ НАБОРА: вид, имя и ЧЕМ ДОСТАЁТСЯ — готовая строка вызова. Спрашивайте, когда не знаете, какой дверью открыть документ",
            "inputSchema": { "type": "object", "properties": {
                "kind": s("только этот вид"),
                "q": s("только те, в чьём виде или имени встречается это слово"),
                "limit": json!({"type":"integer","description":"сколько строк отдать, по умолчанию 200"}) } } }));
        tools.push(json!({ "name": "search", "description": "ПОИСК ПО НАБОРУ: найти, где в документах встречается слово или строка — вид, имя и строки вокруг совпадения. Спрашивайте прежде, чем тянуть документы по одному: ответ на вопрос чаще уже записан",
            "inputSchema": { "type": "object", "properties": { "q": s("что ищем — короткое имя того же довода"), "query": s("что искать"), "kinds": s("виды через запятую: искать только в них, например decision,srs,feature; пусто — искать везде"), "limit": json!({"type":"integer"}) }, "required": ["query"] } }));
        tools.push(json!({ "name": "put", "description": "записать сущность целиком; expectedRevision бережёт от потери чужой правки",
            "inputSchema": { "type": "object", "properties": { "deferProjection": json!({"type":"boolean","description":"не пересобирать проекции сейчас; позвать `reproject` после серии правок"}), "kind": s("вид"), "id": s("имя"), "content": s("текст целиком"), "expectedRevision": json!({"type":"integer"}),
                "create": json!({"type":"boolean","description":"завести, если сущности ещё нет; без этого `put` только правит"}) }, "required": ["kind", "content"] } }));
        tools.push(json!({ "name": "put-section", "description": "заменить один раздел сущности вместе с подразделами (тело — как отдаёт `section`); тело без какого-то подраздела отказано; проекции пересобираются в этом же вызове",
            "inputSchema": { "type": "object", "properties": { "deferProjection": json!({"type":"boolean","description":"не пересобирать проекции сейчас; позвать `reproject` после серии правок"}), "kind": s("вид"), "id": s("имя"), "anchor": s("якорь"), "body": s("новое тело раздела"), "expectedRevision": json!({"type":"integer"}) }, "required": ["kind", "anchor", "body"] } }));
        tools.push(json!({ "name": "rm", "description": "удалить сущность",
            "inputSchema": { "type": "object", "properties": { "deferProjection": json!({"type":"boolean","description":"не пересобирать проекции сейчас; позвать `reproject` после серии правок"}), "kind": s("вид"), "id": s("имя") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "task-state-push", "description": "принять состояния задач, выведенные харнесом из закрывающих трейлеров; подача полная. `at` — время коммита в мс: без него харнес знает лишь «когда увидел», а этим порядок не судится",
            "inputSchema": { "type": "object", "properties": {
                "commit": s("коммит дерева, с которого сняты состояния: без него подача читается как «неизвестно»"),
                "dirty": json!({"type":"boolean","description":"дерево было грязным: состояния выведены не из ствола"}),
                "states": { "type": "array", "description": "[{id, state, commit, at}]",
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
        tools.push(json!({ "name": "readiness-gaps", "description": "пункты готовности без способа проверки — разбивкой по виду владельца и чем это закрывается",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "coverage", "description": "что не покрыто: какие документы не достаются ни одним видом и какие достаются двумя",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "phases", "description": "цепочка фаз с полной картиной: документы, гейт и задачи каждой фазы порознь",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "waves", "description": "волны: что можно вести одновременно, с барьером красной фазы",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "pipeline", "description": "плитка конвейера задач: сколько на каждом статусе и где факт не пишется",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "board", "description": "доска задач разработки: каждая стоит в самой дальней достигнутой ступени, с пропущенными ступенями и зеркалом",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "next-step", "description": "что сейчас держит проект: первая невыполненная ступень процесса с владельцем; три списка: пройдено, пропущено с причиной, неотвечаемо",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса, по умолчанию godzy"), "record": json!({"type":"boolean"}) } } }));
        tools.push(json!({ "name": "process-state", "description": "все ступени процесса целиком: вопрос, запрос, проба, вычисленное состояние, нарушения",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса") } } }));
        tools.push(json!({ "name": "process-history", "description": "проходы диспетчера: какая ступень скачет",
            "inputSchema": { "type": "object", "properties": { "process": s("имя процесса") } } }));
        tools.push(json!({ "name": "progress", "description": "плитки прогресса тремя числами: сделано · открыто · не отвечается",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "method-set", "description": "объявить, чем судится пункт готовности. `checks-green`: в `method` — имена проверок через пробел, и пункт закрывается, когда все они зелены на последнем чистом прогоне ствола. Имя, которого в прогоне нет, даёт красное наравне с упавшим; измерившего прогона (где что-то кроме упавшей сборки) не было вовсе — `unknown`, а не красное",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}),
                "kind": s("вид владельца"), "id": s("имя владельца; у одиночки пусто"),
                "ord": json!({"type":"integer"}), "methodKind": s("checks-green — список имён проверок, зелёных на последнем чистом прогоне ствола; command — выполняет харнес; query — только пунктам гейта, пунктам приёмки отозван"),
                "method": s("запрос либо команда") }, "required": ["kind", "ord", "methodKind"] } }));
        tools.push(json!({ "name": "links-of", "description": "чем доказано и с чем связано: связи сущности по видам: проверки, истории, задачи, решения — то, что показывает панель раздела",
            "inputSchema": { "type": "object", "properties": { "kind": s("requirement · story · decision"), "id": s("имя") }, "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "entity-confirm", "description": "перечитал переоткрытую запись — закрытие в силе: с доводом и автором; правка опоры снова её переоткроет",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид записи: task, milestone, question, requirement"),
                "id": s("имя записи"), "why": s("что перечитано и почему закрытие в силе — обязательно") },
                "required": ["kind", "id", "why"] } }));
        tools.push(json!({ "name": "term-retire", "description": "объявить снятый термин набора: слово, которым больше не называют, и чем оно снято",
            "inputSchema": { "type": "object", "properties": { "term": s("снятое слово"),
                "retiredBy": s("чем снято: решение или статья — обязательно"), "declaredIn": s("где снятие записано"),
                "drop": json!({"type":"boolean","description":"вернуть слово: снять объявление"}) },
                "required": ["term"] } }));
        tools.push(json!({ "name": "retired-terms", "description": "слова, снятые из словаря: встреченные в свежем тексте — находка",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "document-add", "description": "завести новый документ объявленного вида; правит существующий — `put`, и заводить он отказывается",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид из раскладки"),
                "id": s("имя; у одиночки пусто"), "content": s("текст документа") },
                "required": ["kind", "content"] } }));
        tools.push(json!({ "name": "agents", "description": "субагенты набора: имя, описание, инструменты, модель; тело — по просьбе",
            "inputSchema": { "type": "object", "properties": { "set": s("набор, по умолчанию godzy"),
                "body": json!({"type":"boolean"}) } } }));
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
        tools.push(json!({ "name": "story-requirement-add", "description": "объявить, что история несёт требование",
            "inputSchema": { "type": "object", "properties": { "story": s("имя истории"),
                "requirement": s("имя требования"),
                "drop": json!({"type":"boolean","description":"снять связь"}) },
                "required": ["story", "requirement"] } }));
        tools.push(json!({ "name": "feature-story-add", "description": "объявить, что фича несёт историю",
            "inputSchema": { "type": "object", "properties": { "feature": s("имя фичи"),
                "story": s("имя истории"),
                "drop": json!({"type":"boolean","description":"снять связь"}) },
                "required": ["feature", "story"] } }));
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
        tools.push(json!({ "name": "question-add", "description": "объявить вопрос прямо: имя, номер, заголовок, состояние, чем закрыт; занятое имя отказано — правка объявленного идёт с replace:=true; owner:=true — решает владелец: вопрос не держит ступень 5 и встаёт в очередь пульта, ответ владельца — `asks state=done`",
            "inputSchema": { "type": "object", "properties": { "id": s("имя, например OQ-01"),
                "number": json!({"type":"integer"}), "title": s("о чём вопрос"),
                "state": s("open · decided · closed"), "answer": s("ответ, если записан"),
                "closedBy": s("решение, которым закрыт"),
                "owner": json!({"type":"boolean","description":"решает владелец; объявление без него снимает пометку"}),
                "replace": json!({"type":"boolean","description":"заменить объявленный вопрос с этим именем целиком; без него занятое имя отказано"}),
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
                "state": s("not_started; взятие и закрытие приходят трейлером и подачей task-state-push"), "size": s("размер") },
                "required": ["id", "milestone"] } }));
        tools.push(json!({ "name": "decision-add", "description": "объявить решение прямо: имя, номер, заголовок, состояние, дата, решающие, контекст, решение, последствия",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя решения"),
                "number": json!({"type":"integer"}), "title": s("заголовок"),
                "status": s("accepted · superseded · proposed · rejected"), "statusText": s("как записано"),
                "date": s("дата"), "deciders": s("кто решал"), "context": s("контекст"),
                "decision": s("решение"), "consequences": s("последствия") }, "required": ["id", "title"] } }));
        tools.push(json!({ "name": "story-add", "description": "объявить историю прямо: имя, заголовок, область",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "id": s("имя"), "title": s("заголовок"),
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
                "note": s("зачем"), "how": s("extract · files · secret-fields · declared-paths · lines · domain-vs-check · contract-vs-schema · task-trailers"), "drop": json!({"type":"boolean"}) }, "required": ["fact"] } }));
        tools.push(json!({ "name": "ci-job-add", "description": "объявить задание CI, чей журнал несёт прогон проверок другой платформы: харнес сам читает журналы его прогонов на стволе и пишет упавшие как наблюдение красной фазы. Вердиктов дверь не принимает",
            "inputSchema": { "type": "object", "properties": { "repo": s("репозиторий GitHub: владелец/имя"),
                "workflow": s("файл работы: probe-windows.yml"), "job": s("имя задания в работе"),
                "platform": s("платформа раннера: windows"), "note": s("зачем"),
                "drop": json!({"type":"boolean"}) }, "required": ["workflow", "job"] } }));
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
        tools.push(json!({ "name": "version-close", "description": "объявить выпуск закрытым или снова открытым; от закрытого считают, что изменилось после него",
            "inputSchema": { "type": "object", "properties": { "drop": json!({"type":"boolean","description":"снять объявленное этой же дверью"}), "version": s("имя выпуска"),
                "state": s("closed · open, по умолчанию closed") }, "required": ["version"] } }));
        tools.push(json!({ "name": "step-selftest", "description": "самотест лестницы: каждая ступень роняется подсаженным нарушением в откатываемой транзакции; живой считается та, у которой число выросло — и выросло при всех зелёных гейтах и при всех красных, а не только в сегодняшнем состоянии",
            "inputSchema": { "type": "object", "properties": { "set": s("набор, по умолчанию godzy"),
                "process": s("процесс, по умолчанию godzy"),
                "under": s("green · red — прогнать при всех зелёных либо всех красных гейтах; пусто — как есть") } } }));
        tools.push(json!({ "name": "scheme-roles", "description": "роли словаря: что спрашивают проекции, что объявлено набором либо общим слоем, что молчит — и какое правило молчанием выключено",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "scheme-term-set", "description": "объявить слово схемы этого набора; роль, объявленная набором, замещает общий список целиком",
            "inputSchema": { "type": "object", "properties": { "role": s("роль, латиницей"),
                "value": s("слово набора"), "ord": s("порядок, если слов несколько"),
                "why": s("зачем"), "drop": s("true — снять слово"),
                "shared": json!({"type":"boolean","description":"объявить для ВСЕХ наборов; по умолчанию слово принадлежит этому"}) },
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
        tools.push(json!({ "name": "entity-rename", "description": "переименовать сущность во всём наборе: имя сверяется с общим образцом вида; сухой ход по умолчанию",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид сущности"),
                "from": s("старое имя"), "to": s("новое имя"),
                "apply": json!({"type":"boolean","description":"записать; без него только показ"}),
                "merge": json!({"type":"boolean","description":"новое имя уже занято ТОЙ ЖЕ сущностью: слить, сняв старую строку"}) },
                "required": ["kind", "from", "to"] } }));
        tools.push(json!({ "name": "links-retarget", "description": "переписать цель ссылок в `вид:имя` по уже разобранной связи; ярлык не трогается; сухой режим по умолчанию",
            "inputSchema": { "type": "object", "properties": { "apply": json!({"type":"boolean"}) } } }));
        tools.push(json!({ "name": "gate-measure", "description": "перемерить все пункты, лестницу и фазы СЕЙЧАС и дождаться итога; зовите, когда нужно измеренное сейчас — сам пересчёт идёт фоном, и узнать, что он кончился, вызывающему нечем",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "gate-selftest", "description": "самотест гейтов: каждый запросный пункт роняется подсаженным нарушением в откатываемой транзакции; `under` прогоняет его при всех зелёных либо всех красных гейтах — проба, живая лишь в одном из двух, зависит от состояния, которого не форсирует",
            "inputSchema": { "type": "object", "properties": {
                "under": s("green · red — прогнать при всех зелёных либо всех красных гейтах; пусто — как есть") } } }));
        tools.push(json!({ "name": "what-if", "description": "примерка: правка кладётся в копию набора, гейт меряется по ней, копия снимается. Отвечает, ЧТО покраснеет и ЧТО погаснет — до того, как править по-настоящему. Идёт десятки секунд",
            "inputSchema": { "type": "object", "properties": {
                "tool": s("имя двери, которую примеряем"),
                "args": json!({"type":"object","description":"доводы этой двери, как если бы звали её саму"}) },
                "required": ["tool"] } }));
        tools.push(json!({ "name": "order", "description": "порядок выполнения задач: волны как топологические слои внутри этапа — вывод сервера, файл производен",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "summary", "description": "перечень сущностей вида с колонками-числами: сколько проверок у требования, вариантов у решения, требований у истории",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид: requirement · decision · story · screen · question · need") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "skills", "description": "скиллы харнеса, какими их держит база: имя, хеш, кто и когда подал",
            "inputSchema": { "type": "object", "properties": { "set": s("набор скиллов; по умолчанию godzy"),
                "body": s("true — отдать и тело скилла") } } }));
        tools.push(json!({ "name": "code-facts-push", "description": "принять наблюдение датчика о репозитории: таблицы миграций, операции контракта; подача полная в пределах вида",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид факта, например migration-table"),
                "facts": { "type": "array", "description": "[{name, detail, place?}]: place — файл, где имя названо; одно имя в разных местах — разные факты", "items": { "type": "object" } },
                "commit": s("коммит рабочего каталога, которым снят факт: без него подача читается как «неизвестно»"),
                "dirty": json!({"type":"boolean","description":"дерево было грязным: факт рассказывает не про ствол, гейт отвечает «неизвестно»"}),
                "read": json!({"type":"integer","description":"сколько файлов датчик прочёл: пустая подача без read>0 отказ"}) },
                "required": ["kind", "facts"] } }));
        tools.push(json!({ "name": "code-facts", "description": "что датчик подал о репозитории",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "skills-push", "description": "принять скиллы харнеса; подача полная",
            "inputSchema": { "type": "object", "properties": { "set": s("набор"),
                "skills": { "type": "array", "description": "[{name, description, body}]", "items": { "type": "object" } },
                "dry": s("true — только сверка: что разошлось, ничего не записывая") },
                "required": ["skills"] } }));
        tools.push(json!({ "name": "preflight-push", "description": "принять вердикты предполёта: подача СЛИВАЕТСЯ — добавляет и заменяет по имени задачи, чужое остаётся; полное стирание — `clear`",
            "inputSchema": { "type": "object", "properties": {
                "verdicts": { "type": "array", "description": "вердикты предполёта",
                              "items": { "type": "object",
                                "properties": { "task": s("имя задачи"), "verdict": s("вердикт: ready · blocked"),
                                  "findings": json!({"type":"integer","description":"ЧИСЛО находок, не перечень"}),
                                  "at": json!({"type":"integer","description":"время разбора в мс"}),
                                  "taskRevision": json!({"type":"integer","description":"ревизия задачи, на которой разбирали"}),
                                  "body": s("разбор словами") },
                                "required": ["task", "verdict"] } },
                "clear": json!({"type":"boolean","description":"снять ВСЕ прежние вердикты, а не только поданные заново"}) },
                "required": ["verdicts"] } }));
        tools.push(json!({ "name": "task-plan-push", "description": "записать план задачи — как исполнитель собирается её делать; время ставит сервер, и им доказывается, что план был раньше правки",
            "inputSchema": { "type": "object", "properties": { "task": s("имя задачи"),
                "body": s("план: что меняется, чем доказывается, что остаётся нетронутым") },
                "required": ["task", "body"] } }));
        tools.push(json!({ "name": "worktree-push", "description": "принять открытые рабочие деревья — статус «в работе»; подача полная, пустая законна",
            "inputSchema": { "type": "object", "properties": {
                "open": { "type": "array", "description": "[{task, branch, since}]", "items": { "type": "object" } } } } }));
        tools.push(json!({ "name": "question-holders", "description": "вопрос и его задача-держатель: пора закрывать, закрыт рано, судить нечем",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "requirements-of", "description": "требования задачи; объявленное отсутствие доезжает фразой, а не пустотой",
            "inputSchema": { "type": "object", "properties": { "id": s("имя задачи") }, "required": ["id"] } }));
        tools.push(json!({ "name": "tasks-of", "description": "задачи истории через требования",
            "inputSchema": { "type": "object", "properties": { "id": s("имя истории") }, "required": ["id"] } }));
        tools.push(json!({ "name": "preflight-queue", "description": "задачи, которым предполёт не делали либо делали до правки",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "claims", "description": "где расходятся числа: заявленные набором числа против факта, с оговоркой о том, что считается",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "gate", "description": "состояние гейта, вычисленное сейчас: запрос выполняется, подпись сверяется хешем. Тексты запроса, подсадки и предмета — по `sql=true`: без него ответ вчетверо короче",
            "inputSchema": { "type": "object", "properties": { "id": s("имя гейта, например G2; без него — все"),
                "sql": json!({"type":"boolean","description":"отдать тексты запроса, подсадки и предмета у каждого пункта; они три четверти ответа, и нужны тому, кто чинит сам пункт"}) } } }));
        tools.push(json!({ "name": "next-task", "description": "следующая незакрытая задача с закрытыми зависимостями, со всем контекстом внутри",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "blockers", "description": "что держит задачу: чего ждёт задача: задачи и этапы целиком",
            "inputSchema": { "type": "object", "properties": { "id": s("имя задачи") }, "required": ["id"] } }));
        tools.push(json!({ "name": "events", "description": "журнал сущности: что с ней происходило и кем",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид"), "id": s("имя") }, "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "norm-versions", "description": "объявленные версии нормы и что каждая изменила",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид, по умолчанию constitution"), "version": s("номер версии") } } }));
        tools.push(json!({ "name": "measurements", "description": "датированные замеры: чем получено и сколько вышло",
            "inputSchema": { "type": "object", "properties": { "subject": s("предмет, поиск по вхождению") } } }));
        tools.push(json!({ "name": "plan", "description": "что план объявляет: есть · нет с причиной · не объявлено вовсе",
            "inputSchema": { "type": "object", "properties": { "name": s("имя позиции плана") } } }));
        tools.push(json!({ "name": "blame-set", "description": "чей предмет спора у находки: не читает сервер или работа набора; находка остаётся красной",
            "inputSchema": { "type": "object", "properties": { "rule": s("имя пункта гейта"),
                "entityId": s("ключ находки"),
                "blame": s("harness · corpus"), "fixedBy": s("чем это чинится — обязательно"),
                "why": s("довод"), "drop": json!({"type":"boolean"}) },
                "required": ["rule", "entityId", "blame", "fixedBy"] } }));
        tools.push(json!({ "name": "counts-sync", "description": "объявленные числа против измеренных: без ключа показывает разницу, с `apply` записывает посчитанное",
            "inputSchema": { "type": "object", "properties": {
                "apply": json!({"type":"boolean","description":"записать измеренное; без него только показ"}),
                "kind": s("вид документа — сузить область"), "id": s("имя документа — сузить область") } } }));
        tools.push(json!({ "name": "request-add", "description": "заявка на изменение харнеса: что не так, доказательство, что держит; ждёт решения на пульте",
            "inputSchema": { "type": "object", "properties": { "title": s("одна строка: что не так"),
                "body": s("доказательство, что уже сделано в наборе, чего ждём от харнеса, что держит"),
                "runId": s("прогон, из которого пришла заявка") }, "required": ["title"] } }));
        tools.push(json!({ "name": "approval-ask", "description": "попросить подтверждение владельца на коммит и пуш кода проекта",
            "inputSchema": { "type": "object", "properties": { "title": s("одна строка: что коммитим"),
                "body": s("дифф или его выжимка и почему так"), "runId": s("прогон, который ждёт ответа") },
                "required": ["title", "runId"] } }));
        tools.push(json!({ "name": "question-ask", "description": "вопрос владельцу из прогона: без ответа работа не идёт дальше",
            "inputSchema": { "type": "object", "properties": { "title": s("вопрос одной строкой"),
                "body": s("что уже известно и какие есть варианты"), "runId": s("прогон, который ждёт ответа") },
                "required": ["title", "runId"] } }));
        tools.push(json!({ "name": "asks", "description": "очередь решений владельца: заявки на харнес и просьбы подтвердить коммит",
            "inputSchema": { "type": "object", "properties": {
                "state": s("open · taken · owner · declined · done · approved · rejected; пусто — все"),
                "limit": json!({"type":"integer","description":"сколько отдать, по умолчанию 40"}) } } }));
        tools.push(json!({ "name": "ask-decide", "description": "решение по заявке из очереди: с доводом, он остаётся в ней",
            "inputSchema": { "type": "object", "properties": { "ask": json!({"type":"integer","description":"номер заявки"}),
                "state": s("taken · owner · declined · done · approved · rejected"),
                "why": s("довод — обязательно") }, "required": ["ask", "state", "why"] } }));
        tools.push(json!({ "name": "run-start", "description": "завести прогон задачи: с него начинается всё, что о нём расскажут",
            "inputSchema": { "type": "object", "properties": { "task": s("имя задачи"), "agent": s("кто ведёт"),
                "note": s("с чего начали") }, "required": ["task"] } }));
        tools.push(json!({ "name": "run-state", "description": "состояние прогона словом: running · waiting · done · failed · cancelled; кроме running нужна причина",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон"), "state": s("running · waiting · done · failed · cancelled"),
                "note": s("на чём встали либо чем кончилось"), "session": s("сессия, которой прогон продолжается") },
                "required": ["runId", "state"] } }));
        tools.push(json!({ "name": "ask-inbox", "description": "решения владельца, которых прогон ещё не видел; прочитанное помечается",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон") }, "required": ["runId"] } }));
        tools.push(json!({ "name": "run-event", "description": "шаг прогона: что сделано или на чём остановились",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон"), "kind": s("род шага: шаг · вопрос · отказ · итог"),
                "text": s("что случилось") }, "required": ["runId", "text"] } }));
        tools.push(json!({ "name": "run-automaton", "description": "автомат задачи: шаг · круг · RETHINK'и · остановки; без runId — что встало по конвейеру",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон; пусто — остановки конвейера"),
                "status": s("вердикт отчёта сессии: PLAN · GO · RETHINK · NEEDS FIX · DRIFT · BLOCKED · NEEDS CONTEXT · CLOSED"),
                "note": s("довод: почему такой вердикт"), "resume": s("1 — конвейер пущен владельцем, остановка снята") },
                "required": [] } }));
        tools.push(json!({ "name": "run-say", "description": "сказать прогону строку с пульта либо ответить от его имени",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон"), "text": s("что сказано"),
                "side": s("owner — с пульта, agent — от прогона; по умолчанию owner") }, "required": ["runId", "text"] } }));
        tools.push(json!({ "name": "run-inbox", "description": "что человек сказал прогону и он ещё не прочёл; прочитанное помечается",
            "inputSchema": { "type": "object", "properties": { "runId": s("прогон") }, "required": ["runId"] } }));
        tools.push(json!({ "name": "runs", "description": "прогоны набора с последними шагами и сказанным",
            "inputSchema": { "type": "object", "properties": { "limit": json!({"type":"integer","description":"сколько прогонов, по умолчанию 10"}) } } }));
        tools.push(json!({ "name": "ready", "description": "пункты приёмки задачи из «Признак готовности»: что открыто, что проверено; без задачи — по всему набору",
            "inputSchema": { "type": "object", "properties": { "task": s("задача; пусто — весь набор") } } }));
        tools.push(json!({ "name": "test-run", "description": "прогоны тестов, снятые харнесом: когда последний раз, каким коммитом и чем гоняли, что упало на чистом дереве. `names` — имена через пробел: о каждом отвечает поимённо, и «в прогоне нет» отличается от «упала»",
            "inputSchema": { "type": "object", "properties": { "names": s("имена проверок через пробел; пусто — общий ответ") } } }));
        tools.push(json!({ "name": "chat-start", "description": "завести беседу над набором: место, где думают вслух и спрашивают по ходу",
            "inputSchema": { "type": "object", "properties": { "title": s("о чём беседа") } } }));
        tools.push(json!({ "name": "chat-say", "description": "сказать в беседе: side owner — человек с пульта, agent — ответ",
            "inputSchema": { "type": "object", "properties": { "thread": s("беседа"), "text": s("что сказано"),
                "side": s("owner · agent; по умолчанию owner") }, "required": ["thread", "text"] } }));
        tools.push(json!({ "name": "chat-inbox", "description": "непрочитанное беседой; тем же вызовом она называет свою сессию",
            "inputSchema": { "type": "object", "properties": { "thread": s("беседа"), "sessionId": s("сессия, которой отвечают") },
                "required": ["thread"] } }));
        tools.push(json!({ "name": "chat", "description": "беседы набора, а с именем беседы — её строки",
            "inputSchema": { "type": "object", "properties": { "thread": s("беседа; пусто — перечень бесед"),
                "limit": json!({"type":"integer","description":"сколько строк, по умолчанию 50"}) } } }));
        tools.push(json!({ "name": "console", "description": "пульт: что делают агенты сейчас, что ждёт ответа человека, что поехало от правок",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "links-graph", "description": "скелет зависимостей: какие роды на каких стоят, сколькими связями, и где сейчас переоткрыто",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "impact", "description": "что переоткроется, если эту запись изменить — обход вперёд по тем же связям, что и каскад",
            "inputSchema": { "type": "object", "properties": { "kind": s("род записи"),
                "id": s("имя записи"), "depth": json!({"type":"integer","description":"глубина 1..6, по умолчанию 3"}) },
                "required": ["kind", "id"] } }));
        tools.push(json!({ "name": "tree", "description": "дерево связанного: от одного документа всё, с чем он связан, по записям связей",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид документа, например srs"),
                "name": s("имя документа; у одиночного вида пусто"),
                "depth": json!({"type":"integer","description":"глубина обхода, 1..4; по умолчанию 2"}) },
                "required": ["kind"] } }));
        tools.push(json!({ "name": "document-coverage", "description": "что документа уже живёт в таблицах, а что держит только текст — по разделам",
            "inputSchema": { "type": "object", "properties": { "kind": s("вид документа"),
                "name": s("имя документа; у одиночного вида пусто") }, "required": ["kind"] } }));
        tools.push(json!({ "name": "holders", "description": "объявленные держатели инварианта: требование и путь",
            "inputSchema": { "type": "object", "properties": {} } }));
        tools.push(json!({ "name": "derived-copy-set", "description": "вот источник, вот копия, вот чем сверять: одна дверь для чисел и для тел",
            "inputSchema": { "type": "object", "properties": { "name": s("величина или предмет копии"),
                "source": s("источник — `вид:имя`"), "copy": s("копия — `вид:имя`"),
                "compare": s("count — число по образцу · body — тело названного куска"),
                "pattern": s("образец у КОПИИ: чем взять число либо кусок"),
                "sourcePattern": s("образец у ИСТОЧНИКА, если он говорит другими словами; пусто — тот же"),
                "why": s("почему это копия — обязательно"), "drop": json!({"type":"boolean"}) },
                "required": ["name", "source", "copy", "compare", "pattern", "why"] } }));
        tools.push(json!({ "name": "doors", "description": "какой вопрос какой дверью закрывается: поиск по дверям словами вопроса",
            "inputSchema": { "type": "object", "properties": { "q": s("вопрос словами: «кто ссылается на требование», «что сейчас держит»"),
                "limit": json!({"type":"integer","description":"сколько дверей назвать, по умолчанию 8"}) } } }));
        tools.push(json!({ "name": "declared-unwritten", "description": "что объявлено и не написано: объявлено дверью и не написано документом: имена, по которым `get` откажет",
            "inputSchema": { "type": "object", "properties": {} } }));
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
    /// Двери, которые ПИШУТ. Только они заказывают пересчёт.
    ///
    /// Читающая дверь в этом списке — заказ пересчёта за то, что кто-то
    /// посмотрел: так здесь стояли `sensors` и `goals`, обе — чистые перечни.
    /// Обратная ошибка того же рода — `step-selftest`: он работает в
    /// транзакции, которую всегда откатывает, и писал ноль строк, заказывая
    /// полный круг каждым прогоном.
    /// Пишущие двери — перечнем, и он же отдаётся наружу: проба на пустом
    /// наборе спрашивает только читающие, а знать их порознь значило бы завести
    /// второй перечень.
    pub fn writes() -> &'static [&'static str] {
        Self::WRITES
    }

    pub(crate) const TAKEN_AWAY_NAMES: &'static [&'static str] = &[
        "gate-item-set", "phase-set",
        "step-add", "step-remove", "step-method-set", "step-probe-set", "step-question-set", "step-when-set",
        "kind-add", "kind-domain", "kind-id-set", "kind-projection", "kind-proves", "kind-reopens", "kind-required",
    ];

    const WRITES: &[&str] = &[
        "put", "put-section", "rm", "document-add", "reparse", "reproject", "sweep",
        "task-state-push", "code-facts-push", "skills-push", "preflight-push", "worktree-push",
        "task-plan-push",
        "blame-set", "derived-copy-set", "counts-sync", "frozen-tree-set", "scheme-term-set", "orphans-purge", "surface-source-add", "sensor-spec-add", "ci-job-add", "agent-set", "donor-add", "guard-add", "method-set", "sensor-declare", "requirement-retire", "requirement-scope-set", "requirement-source-add", "article-gate-add", "protocol-op-add", "crate-add", "stand-row-add", "algorithm-add", "reference-source-add", "token-add", "postmortem-add", "freeze-row-add", "release-artifact-add", "task-dep-add", "article-add", "requirement-add", "term-add", "decision-add", "story-add", "screen-add", "version-add", "milestone-add", "task-add", "alternative-add", "task-requirement-add", "screen-reference-add", "question-add", "risk-add", "goal-add", "acceptance-add", "feature-link-add", "story-requirement-add", "feature-story-add", "story-detail-add", "screen-detail-add", "milestone-detail-add", "process-row-add", "frame-rule-add", "decision-link-add", "run-record-add", "version-close", "gate-selftest",
        "author-set", "screen-area-set", "skill-set", "version-freeze",
        "links-rewrite", "links-retarget", "entity-rename", "entity-confirm", "term-retire",
    ];

    /// Вызов инструмента — и отметка, если он писал.
    ///
    /// Ручки, правящие ОБЩЕЕ объявление: их итог виден каждому набору.
    // `gate-selftest` здесь потому, что приговор пробе живёт в `gate_item` —
    // объявлении, общем на все наборы. `step-selftest` не здесь и не в `WRITES`
    // вовсе: он работает в транзакции, которую всегда откатывает, и не пишет
    // ни строки. Пока он числился пишущим, каждый его прогон — а их в проверке
    // шесть — заказывал полный пересчёт всем проектам ни за чем.
    const SHARED_WRITES: [&'static str; 1] = [
        "gate-selftest",
    ];

    const TRY_ON: &[&str] = &[
        "put", "put-section", "rm", "document-add",
        "task-state-push", "task-plan-push", "preflight-push", "code-facts-push", "worktree-push",
    ];

    /// Прибавить вызов к счёту. В памяти и без единого ожидания.
    ///
    /// **Строкой на вызов это было написано сначала, и было неверно.** Запись
    /// брала соединение из пула прямо на пути ответа, и ревью намерило цену:
    /// ~35 тысяч строк в сутки от ОДНОЙ открытой вкладки готовности; накрутку
    /// счётчика «занято», который оператор читает как перегрузку; и до трёх
    /// секунд ожидания пула ПЕРЕД отметкой «пересчитать» — то есть расширение
    /// ровно того окна потери, о котором сказано десятью строками ниже. Счёт
    /// отвечает на тот же вопрос и не трогает ни пул, ни ответ.
    ///
    /// Замок занят или отравлен — счёт молча пропускается. Он ничего не решает:
    /// ни ответа, ни отметки «пересчитать», ни вердикта, и обменивать на него
    /// работу набора нельзя.
    fn tally(&self, name: &str, failed: bool, took: std::time::Duration) {
        let millis = i32::try_from(took.as_millis()).unwrap_or(i32::MAX);
        let Ok(mut calls) = CALLS.lock() else { return };
        let seen = calls
            .entry((self.project.clone(), name.to_owned(), self.author.clone(), failed))
            .or_insert((0, 0, 0));
        seen.0 += 1;
        seen.1 += i64::from(millis);
        seen.2 = seen.2.max(millis);
    }

    /// Отметка ставится ПОСЛЕ ответа и только на успешный: пересчитывать набор
    /// из-за отказа значит считать то же самое второй раз.
    pub async fn call(&self, name: &str, args: &Value) -> Value {
        let started = std::time::Instant::now();
        let out = self.run(name, args).await;
        // ОТКАЗ УЗНАЁТСЯ ПО `isError`, а не по ключу `error`, которого отказ не
        // несёт вовсе: `refusal` отдаёт `{content, isError}`. Условие было верно
        // ВСЕГДА, и отметку ставил всякий отказ — а для общей двери это
        // `touch_all`: кривая общая дверь, отвергнутая базой, запускала полный
        // пересчёт всем проектам разом. Ровно то, чего доводом выше сказано не
        // делать.
        let failed = out.get("isError").and_then(|v| v.as_bool()).unwrap_or(false)
            || out.get("error").is_some();
        // СЧЁТ СТОИТ ДО ОТМЕТКИ, И ЭТО БЕЗОПАСНО РОВНО ПОТОМУ, ЧТО ОН НИЧЕГО НЕ
        // ЖДЁТ. Первая редакция писала строку в базу и могла простоять здесь до
        // трёх секунд на исчерпанном пуле — то есть расширяла окно «правка
        // легла, отметка не поставлена», о котором сказано абзацем ниже. Счёт в
        // памяти этого окна не трогает, а место перед отметкой застаёт и путь с
        // ранним возвратом внутри блока.
        self.tally(name, failed, started.elapsed());
        if Self::WRITES.contains(&name) && !failed {
            // Правка ОБЩЕГО объявления метит все наборы: пункт гейта, фаза и
            // ступень лестницы одни на всех, и посчитать их надо всем.
            // ПОТЕРЯННАЯ ОТМЕТКА НАЗЫВАЕТСЯ. Правка записана, а пересчёт без
            // отметки не случится: гейт продолжит отдавать прежний замер, и
            // никто об этом не узнает. Ответ об этом говорит прямо — правку
            // повторять не надо, надо позвать пересчёт.
            let marked = if Self::SHARED_WRITES.contains(&name) {
                crate::watch::touch_all(&self.pool, name).await
            } else {
                crate::watch::touch(&self.pool, &self.project, name).await
            };
            if let Err(e) = marked {
                // Предупреждение кладётся ВНУТРЬ ответа двери, а не рядом с ним:
                // оболочку снимают все — и `mh call`, и веб, и сам протокол, —
                // и поле снаружи не доходило ни до кого.
                let mut out = out;
                let said = format!(
                    "записано, но отметка «пересчитать» не поставлена: {}. \
                     Правку повторять не надо — позовите `reproject` и `gate-measure`",
                    e.says()
                );
                let text = out["content"][0]["text"].as_str().unwrap_or("").to_owned();
                let inner = match serde_json::from_str::<Value>(&text) {
                    Ok(Value::Object(mut map)) => {
                        map.insert("измерение".to_owned(), json!(said));
                        Value::Object(map)
                    }
                    _ => json!({ "ответ": text, "измерение": said }),
                };
                out["content"][0]["text"] = json!(serde_json::to_string_pretty(&inner).unwrap_or(text));
                return out;
            }
        }
        out
    }

    fn args_refusal(&self, name: &str, args: &Value) -> Option<Value> {
        // ДОВОДЫ СОБИРАЮТСЯ СО ВСЕХ ЗАПИСЕЙ ЭТОГО ИМЕНИ, А НЕ С ПЕРВОЙ.
        //
        // Имя инструмента не единственно: виды сущностей кладутся в перечень
        // первыми и дают запись с именем вида, а дверь с тем же именем — позже.
        // У набора есть вид `gate`, и оттого дверь `gate` для этой сверки
        // читалась записью вида: схема вида знает один `id`, и всякий другой
        // довод двери отвергался как неизвестный. Довод `sql`, заведённый
        // 2026-09-25, так и не сработал ни разу — отказ приходил до разбора.
        //
        // Объединение никогда не отвергает лишнего: довод, известный хоть одной
        // записи имени, известен. Исполняет вызов не эта функция, а разбор
        // имени ниже, и он про столкновение имён знает.
        let tools = self.tools();
        let mut known = serde_json::Map::new();
        for tool in tools.iter().filter(|t| t["name"] == name) {
            if let Some(props) = tool["inputSchema"]["properties"].as_object() {
                known.extend(props.clone());
            }
        }
        if known.is_empty() && !tools.iter().any(|t| t["name"] == name) {
            return None;
        }
        let (given, known) = (args.as_object()?, &known);
        let unknown: Vec<String> = given
            .keys()
            .filter(|k| !known.contains_key(*k) && !matches!(k.as_str(), "id" | "kind" | "brief"))
            .map(|k| format!("«{k}»"))
            .collect();
        if unknown.is_empty() {
            return None;
        }
        let mut names: Vec<&str> = known.keys().map(String::as_str).collect();
        names.sort_unstable();
        Some(refusal(Miss::Refused(format!(
            "дверь {name} не знает {}: неизвестный довод пропал бы молча. Знает: {}",
            unknown.join(", "),
            names.join(", ")
        ))))
    }

    async fn run(&self, name: &str, args: &Value) -> Value {
        if let Some(refused) = self.args_refusal(name, args) {
            return refused;
        }
        // Имя сущности бывает числом — у статьи конституции оно и есть номер.
        // По HTTP оно приходит из адреса и разбирается в число; строкой его
        // здесь не увидели бы, и вид получил бы отказ «нужно имя» при поданном
        // имени. Одно место на оба входа.
        let numeric_id = num(args, "id").map(|n| n.to_string());
        let id = args.get("id").and_then(|v| v.as_str()).or(numeric_id.as_deref());
        let kind_arg = args.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
        let p = &self.project;

        // Общие инструменты старше видов. Имя `gate` носят оба: вид (строка
        // перечня гейтов) и вычисленное состояние. Спрашивают второе — первое
        // достаётся `gate-list`. Без этого старшинства вид молча перехватывал бы
        // вызов и отдавал строку таблицы вместо вычисления.
        // ДВЕРИ, СНЯТЫЕ ВМЕСТЕ С ПРАВКОЙ ПРИБОРА ВО ВРЕМЯ РАБОТЫ. Перечень
        // берётся не из головы: это ровно те имена, что исчезли из `tools()`
        // между `b755b3e^` и сегодняшним стволом, когда прибор переехал в
        // `instrument/` по решению владельца (#19). Они остаются здесь, чтобы
        // звавший вчера узнал, куда они делись, — иначе снятая дверь
        // неотличима от опечатки.
        const TAKEN_AWAY: &[&str] = Mcp::TAKEN_AWAY_NAMES;

        const RESERVED: &[&str] = &[
            "documents",
            "method-set", "question-holders", "preflight-push", "worktree-push",
            "sensor-specs", "scheme-terms", "scheme-roles", "frozen-trees", "addresses-declared", "tree-declared", "donors", "skills-push", "skills", "agents", "code-facts-push", "code-facts", "summary", "links-of", "retired-terms", "term-retire", "entity-confirm", "request-add", "approval-ask", "question-ask", "asks", "ask-decide", "ask-inbox", "chat-start", "chat-say", "chat-inbox", "chat", "run-start", "run-state", "run-event", "run-automaton", "run-say", "run-inbox", "runs", "ready", "test-run", "order", "gate-measure", "gate-selftest", "links-rewrite", "links-retarget", "reparse", "screen-area-set", "skill-set", "skills-paths", "version-freeze", "version-delta", "generated-check", "principal-allow", "principals", "author-set", "authors", "sensor-declare", "sensors", "requirement-retire", "requirement-scope-set", "requirement-source-add", "article-gate-add", "protocol-op-add", "crate-add", "stand-row-add", "algorithm-add", "reference-source-add", "token-add", "postmortem-add", "freeze-row-add", "release-artifact-add", "task-dep-add", "article-add", "requirement-add", "term-add", "decision-add", "story-add", "screen-add", "version-add", "milestone-add", "task-add", "alternative-add", "task-requirement-add", "screen-reference-add", "question-add", "risk-add", "goal-add", "goals", "acceptance-add", "feature-link-add", "story-requirement-add", "feature-story-add", "story-detail-add", "screen-detail-add", "milestone-detail-add", "process-row-add", "frame-rule-add", "decision-link-add", "run-record-add", "version-close", "gate-selftest", "next-step", "process-state", "statuses", "task-status", "status-anomaly", "pipeline", "board", "waves", "phases", "coverage", "blocks", "history", "at-revision", "process-history", "progress",
            "kinds", "kinds-due", "readiness-gaps", "declared-unwritten", "blame-set", "doors", "derived-copy-set", "holders", "counts-sync", "tree", "document-coverage", "sections", "section", "backlinks", "search", "put",
            "put-section", "rm", "document-add", "reproject", "sweep", "gate", "next-task", "what-if", "blockers", "events", "task-plan-push",
            "norm-versions", "measurements", "plan", "readiness", "requirements-of",
            "tasks-of", "preflight-queue", "claims",
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
        // ИМЯ РУЧКИ, СОВПАВШЕЕ С ИМЕНЕМ ВИДА, разрешается доводом `kind`.
        //
        // Три вида зовутся так же, как разборные ручки: `coverage`, `claims`,
        // `goals`. `mh call coverage` отдавала разбор, а документ того же имени
        // достать было нечем: какой из двух ответов придёт, решал список в коде,
        // и спросить об этом было негде. Теперь `kind=coverage` значит «мне
        // документ», и это работает у любого имени, а не только у столкнувшихся.
        if args.get("kind").and_then(|v| v.as_str()) == Some(name) && self.kinds.get(name).is_some() {
            return match entities::entity(&self.pool, &self.kinds, p, name, id).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            };
        }
        if !reserved && self.kinds.get(name).is_some() {
            return match entities::entity(&self.pool, &self.kinds, p, name, id).await {
                Ok(v) if flag(args, "brief") => {
                    ok(entities::without_body(v))
                }
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            };
        }

        match name {
            "kinds" => {
                let registry = match self.live_registry().await { Ok(k) => k, Err(e) => return refusal(e) };
                // ОБЪЯВЛЕНИЕ БЕРЁТСЯ ИЗ БАЗЫ СЫРЫМ, а не собирается обратно из
                // разобранного. Разбор знает не все ключи: `proves` и `reopens`
                // кладут прямо в `jsonb`, и `Kind` их молча теряет — собранное
                // им объявление стёрло бы оба на первой же записи. Виды читают
                // эти ключи представлениями, и потеря была бы тихой.
                let raw: std::collections::HashMap<String, Value> = match crate::db::conn(&self.pool).await {
                    Ok(client) => match client.query("SELECT name, spec FROM kind_layout", &[]).await {
                        Ok(rows) => rows.iter().map(|r| (r.get(0), r.get(1))).collect(),
                        Err(e) => return refusal(crate::db::Fail::from(e).into()),
                    },
                    Err(e) => return refusal(e.into()),
                };
                let mut out = Vec::new();
                for (kind, k) in &registry.0 {
                    let count = match entities::ids(&self.pool, &registry, p, kind).await {
                        Ok(l) => json!(l.len()),
                        Err(Miss::Unprojected(_)) => Value::Null,
                        Err(e) => return refusal(e),
                    };
                    // Образец имени и назначение вида отдаются вместе со счётом:
                    // умения спрашивают у сервера «как зовётся сущность этого
                    // вида», и до сих пор не получали ответа — за ним ходили в
                    // карту проекта, файлом.
                    // ФАЙЛОВОГО АДРЕСА ЗДЕСЬ НЕТ НАМЕРЕННО. Сущность живёт в
                    // базе, а не в файле; `at`/`under`/`file` — след переезда,
                    // и отдавать их наружу значит звать обратно.
                    out.push(json!({
                        "kind": kind, "single": k.single,
                        "shape": if k.is_inner() { "inner" } else { "document" },
                        "count": count,
                        "in": k.in_kind.clone().unwrap_or_default(),
                        "required": k.required,
                        "requiredWhy": k.required_why.clone().unwrap_or_default(),
                        "idPattern": k.id.clone().unwrap_or_default(),
                        "nameIs": k.name_is.clone().unwrap_or_default(),
                        "projection": k.projection.clone().unwrap_or_default(),
                        "domain": k.domain.clone().unwrap_or_default(),
                        "holds": k.holds.clone().unwrap_or_default(),
                        // ОБЪЯВЛЕНИЕ ЦЕЛИКОМ, как оно лежит. Пересказ выше удобен
                        // человеку, но пустая строка в нём и «не объявлено»
                        // читаются одинаково: вернуть по нему объявление нельзя,
                        // а объявление вида — часть прибора, и оно переезжает.
                        "spec": raw.get(kind).cloned().unwrap_or(Value::Null)
                    }));
                }
                ok(json!({ "kinds": out }))
            }
            "sections" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => match corpus::sections(&self.pool, p, &k, &n).await {
                    Ok(v) => ok(json!({ "sections": v })),
                    Err(e) => refusal(e.into()),
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
                            Err(e) => refusal(e.into()),
                        },
                        Ok(None) => refusal(Miss::NoEntity(kind_arg.to_owned(), id.unwrap_or("").to_owned())),
                        Err(e) => refusal(e.into()),
                    },
                    Err(e) => refusal(e),
                }
            }
            "backlinks" => match entities::backlinks(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok(list) => ok(json!({ "count": list.len(), "backlinks": list })),
                Err(e) => refusal(e),
            },
            // ЧЕМ ДОСТАЁТСЯ ДОКУМЕНТ — вопрос, на который набор отвечал перебором.
            // Имена дверей у документов неоднородны по устройству: дверь названа
            // ВИДОМ, а вид не всегда зовётся так же, как документ. Здесь на
            // каждый документ стоит готовая строка вызова, и гадать больше не о
            // чем. Просьба сессии `tot-ade`, пункт второй.
            "documents" => {
                let only = if kind_arg.is_empty() {
                    args.get("kind").and_then(|v| v.as_str()).unwrap_or("")
                } else {
                    kind_arg
                };
                let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
                let limit = num(args, "limit").unwrap_or(200).clamp(1, 2000);
                let client = match crate::db::conn(&self.pool).await {
                    Ok(c) => c,
                    Err(e) => return refusal(e.into()),
                };
                let rows = client
                    .query(
                        "SELECT entity_kind, entity_name, bytes FROM project_documents
                          WHERE project_id = $1
                            AND ($2 = '' OR entity_kind = $2)
                            AND ($3 = '' OR entity_kind ILIKE '%' || $3 || '%'
                                         OR entity_name ILIKE '%' || $3 || '%')
                          ORDER BY entity_kind, entity_name LIMIT $4",
                        &[&p, &only, &q, &limit],
                    )
                    .await;
                match rows {
                    Ok(rows) => ok(json!({
                        "count": rows.len(),
                        "documents": rows.iter().map(|r| {
                            let (kind, name): (String, String) = (r.get(0), r.get(1));
                            let call = if name.is_empty() {
                                format!("mh call {kind}")
                            } else {
                                format!("mh call {kind} id={name}")
                            };
                            json!({ "kind": kind, "name": name, "bytes": r.get::<_, i32>(2), "достаётся": call })
                        }).collect::<Vec<_>>(),
                    })),
                    Err(e) => refusal(crate::db::Fail::Db(e).into()),
                }
            }
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
                    return refusal(Miss::Refused(
                        "поиск без запроса вернул бы весь набор: назовите, что ищете (`query` или `q`)".into(),
                    ));
                }
                let limit = num(args, "limit").unwrap_or(20).clamp(1, 200);
                // Виды — через запятую: «ищи в решениях и требованиях» вместо
                // «ищи везде и разбирайся сам». Просьба набора: ответ находился,
                // но тонул в реестре вопросов и указателе.
                let kinds: Vec<String> = args
                    .get("kinds")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .split(',')
                    .map(|k| k.trim().to_owned())
                    .filter(|k| !k.is_empty())
                    .collect();
                match corpus::search(&self.pool, p, q, &kinds, limit).await {
                    Ok(hits) => ok(json!({ "count": hits.len(), "hits": hits })),
                    Err(e) => refusal(e.into()),
                }
            }
            "task-state-push" => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let states: Vec<(String, String, String, i64)> = Some(rows(args, "states"))
                    .map(|list| {
                        list.iter()
                            .filter_map(|it| {
                                Some((
                                    it.get("id")?.as_str()?.to_owned(),
                                    it.get("state")?.as_str()?.to_owned(),
                                    it.get("commit").and_then(|c| c.as_str()).unwrap_or("").to_owned(),
                                    // Время коммита. Нет его — ноль: подающий
                                    // старой сборки не ломается, а правило
                                    // порядка падает обратно на «когда увидели».
                                    it.get("at").and_then(|a| a.as_i64()).unwrap_or(0),
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
                let commit = args.get("commit").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::push_task_state(
                    &self.pool, p, &states, now, &commit, flag(args, "dirty"),
                )
                .await
                {
                    Ok(v) if v.get("status").is_some() => json!({ "content": [{ "type": "text",
                        "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "isError": true }),
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
                            Err(e) => written_unprojected("состояния записаны", e),
                        }
                    }
                    Err(e) => refusal(e.into()),
                }
            }
            "state-disagreements" => match crate::projector::state_disagreements(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "statuses" => {
                let k = if kind_arg.is_empty() { "task" } else { kind_arg };
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
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
                    Err(e) => refusal(e.into()),
                }
            }
            "task-status" => match crate::projector::task_status(&self.pool, p, id).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "status-anomaly" => match crate::projector::status_anomaly(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "blocks" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let anchor = args.get("anchor").and_then(|v| v.as_str());
                    let own = flag(args, "own");
                    match crate::projector::blocks(&self.pool, p, &k, &n, anchor, own).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(e.into()),
                    }
                }
                Err(e) => refusal(e),
            },
            "history" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let limit = num(args, "limit").unwrap_or(20).clamp(1, 200);
                    match crate::projector::history(&self.pool, p, &k, &n, limit).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(e.into()),
                    }
                }
                Err(e) => refusal(e),
            },
            "at-revision" => match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                Ok((k, n)) => {
                    let rev = num(args, "revision").unwrap_or(0);
                    match crate::projector::at_revision(&self.pool, p, &k, &n, rev).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(e.into()),
                    }
                }
                Err(e) => refusal(e),
            },
            "blame-set" => {
                let rule = args.get("rule").and_then(|v| v.as_str()).unwrap_or("");
                let entity = args.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
                let blame = args.get("blame").and_then(|v| v.as_str()).unwrap_or("");
                let fixed = args.get("fixedBy").and_then(|v| v.as_str()).unwrap_or("");
                let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_blame(&self.pool, p, crate::projector::Blame { rule, entity_id: entity, blame, fixed_by: fixed, why, decided_by: &self.author }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            // УКАЗАТЕЛЬ ДВЕРЕЙ. Их двести двадцать четыре, плоским списком, и
            // агент, которому нужен ответ на «кто ссылается на это требование»,
            // должен угадать слово `backlinks` среди них — либо обойти набор
            // руками. Обход при этом НЕ ОТКАЗЫВАЕТ: даёт число, даже верное, и в
            // отчёт уходит работа, которой не требовалось. Дыра, которой нет,
            // стоит дороже настоящей: настоящую видно по отказу.
            //
            // Три жалобы «такой ручки нет» были написаны за одну ночь, и все три
            // оказались о существующих дверях.
            "counts-sync" => {
                let apply = flag(args, "apply");
                match crate::projector::counts_sync(&self.pool, p, kind_arg,
                        id.unwrap_or(""), apply, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "chat-start" => {
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::start_chat(&self.pool, p, &title).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "chat-say" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let side = if g("side").trim().is_empty() { "owner".to_owned() } else { g("side") };
                match crate::projector::say_in_chat(&self.pool, p, &g("thread"), &side, &g("text")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "chat-inbox" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::chat_inbox(&self.pool, p, &g("thread"), &g("sessionId")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "chat" => {
                let thread = args.get("thread").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let limit = num(args, "limit").unwrap_or(50).clamp(1, 400);
                match crate::projector::read_chat(&self.pool, p, &thread, limit).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "request-add" | "approval-ask" | "question-ask" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let kind = match name {
                    "request-add" => "request",
                    "approval-ask" => "approval",
                    _ => "question",
                };
                match crate::projector::add_ask(&self.pool, p, kind, &g("title"), &g("body"), &g("runId"), &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "asks" => {
                let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let limit = num(args, "limit").unwrap_or(40).clamp(1, 200);
                match crate::projector::list_asks(&self.pool, p, &state, limit).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "ask-decide" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::decide_ask(&self.pool, num(args, "ask").unwrap_or(0), &g("state"), &g("why"), &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-start" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::start_run(&self.pool, p, &g("task"), &g("agent"), &g("note")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-state" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::set_run_state(&self.pool, p, &g("runId"), &g("state"), &g("note"), &g("session"), &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "ask-inbox" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::ask_inbox(&self.pool, p, &g("runId")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-event" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let kind = if g("kind").trim().is_empty() { "шаг".to_owned() } else { g("kind") };
                match crate::projector::add_run_event(&self.pool, p, &g("runId"), &kind, &g("text")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-automaton" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                // Статус, переданный даже пустым, — вердикт; отсутствующий — чтение.
                let status = args.get("status").and_then(|v| v.as_str());
                match crate::projector::automaton(&self.pool, p, &g("runId"), status, &g("note"),
                                                  flag(args, "resume")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-say" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let side = if g("side").trim().is_empty() { "owner".to_owned() } else { g("side") };
                match crate::projector::say_to_run(&self.pool, p, &g("runId"), &side, &g("text")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "run-inbox" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::run_inbox(&self.pool, p, &g("runId")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "runs" => {
                let limit = num(args, "limit").unwrap_or(10).clamp(1, 60);
                match crate::projector::list_runs(&self.pool, p, limit).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "ready" => {
                let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::ready_items(&self.pool, p, task).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "test-run" => match crate::projector::test_status(
                &self.pool, p, args.get("names").and_then(|v| v.as_str()).unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "console" => match crate::projector::console(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "links-graph" => match crate::projector::links_graph(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "impact" => {
                match crate::projector::impact(&self.pool, p, kind_arg,
                        args.get("id").and_then(|v| v.as_str()).or(id).unwrap_or(""),
                        num(args, "depth").unwrap_or(3) as i32).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "tree" => {
                let name = args.get("name").and_then(|v| v.as_str())
                    .or(id).unwrap_or("");
                match crate::projector::tree(&self.pool, p, kind_arg, name,
                        num(args, "depth").unwrap_or(2) as i32).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "document-coverage" => {
                let name = args.get("name").and_then(|v| v.as_str()).or(id).unwrap_or("");
                match crate::projector::document_coverage(&self.pool, p, kind_arg, name).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "holders" => match crate::projector::holders(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "derived-copy-set" => {
                let g = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::set_derived_copy(&self.pool, p, crate::projector::DerivedCopy { name: g("name"), source: g("source"), copy: g("copy"), compare: g("compare"), pattern: g("pattern"), source_pattern: g("sourcePattern"), why: g("why"), decided_by: &self.author }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "doors" => {
                let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
                let limit = num(args, "limit").unwrap_or(8).clamp(1, 60) as usize;
                if q.is_empty() {
                    return ok(json!({ "count": self.tools().len(),
                        "why": "спросите вопросом: `doors q=\"кто ссылается на требование\"`. \
                                Без вопроса это тот же плоский список из двух с лишним сотен." }));
                }
                // Слова короче трёх букв не различают ничего и вытягивают всё
                // подряд: «на», «что», «где».
                // Слово сводится к ОСНОВЕ: «держит», «держат», «держащий» —
                // одно и то же для того, кто ищет. Пять знаков хватает русскому
                // корню и не склеивает разные слова.
                let stem = |w: &str| -> String { w.chars().take(5).collect() };
                let words: Vec<String> = q
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| w.chars().count() >= 3)
                    .map(stem)
                    .collect();
                let mut hits: Vec<(i32, Value)> = Vec::new();
                for t in self.tools() {
                    let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                    let about = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                    let mut score = 0i32;
                    if name == q {
                        score += 100;
                    }
                    for w in &words {
                        // Имя весит больше описания: дверь, названная словом
                        // вопроса, — почти наверняка та самая.
                        //
                        // Короткое слово весит вдвое меньше: «что», «где», «кто»,
                        // «чем» стоят в половине описаний и вытягивают всё
                        // подряд, ничего не различая.
                        let ves = if w.chars().count() <= 3 { 1 } else { 2 };
                        if name.contains(w.as_str()) { score += 5 * ves }
                        if about.contains(w.as_str()) { score += 2 * ves }
                    }
                    if score > 0 {
                        // НЕОБЯЗАТЕЛЬНЫЕ ДОВОДЫ — ТОЖЕ ОТВЕТ.
                        //
                        // Перечень показывал только обязательные, и дверь,
                        // умеющая обратное действие, выглядела не умеющей его.
                        // Замер 21.09: сессия объявила задаче требование
                        // `task-requirement-add`, поняла, что оно чужое, и не
                        // нашла, чем снять, — довод `drop` у двери есть и в
                        // схеме подписан, а в перечне его не было. Две находки
                        // `G2` стояли, пока об этом не спросили голосом; дверей
                        // с `drop` в приборе больше сотни.
                        let (needs, takes) = door_arguments(t.get("inputSchema"));
                        hits.push((score, json!({
                            "door": t.get("name").cloned().unwrap_or(Value::Null),
                            "about": t.get("description").cloned().unwrap_or(Value::Null),
                            "needs": needs,
                            "takes": takes,
                            "score": score,
                        })));
                    }
                }
                hits.sort_by_key(|h| std::cmp::Reverse(h.0));
                let total = hits.len();
                ok(json!({
                    "asked": q,
                    "found": total,
                    "doors": hits.into_iter().take(limit).map(|(_, v)| v).collect::<Vec<_>>(),
                    // Ноль — это ОТВЕТ, а не пустой список: двери с такими словами
                    // нет, и обходить набор руками, не сказав об этом, нельзя.
                    "why": if total == 0 {
                        "ни одна дверь не названа этими словами. Это не значит, что ответа нет: \
                         попробуйте другими словами предмета — «связи», «покрытие», «замер», «план»."
                    } else { "" },
                }))
            }
            "declared-unwritten" => match crate::projector::declared_unwritten(&self.pool, &self.kinds, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "readiness-gaps" => match crate::projector::readiness_gaps(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            // ИМЯ ЗАНЯТО ДВЕРЬЮ, И ДОКУМЕНТ ИМ ПЕРЕКРЫТ. Вид `coverage` — тоже
            // `coverage`, и `mh call coverage` отдавал разбор покрытия вместо
            // документа, а причина отказа называла ключ, из которого не
            // следовало, что делать.
            //
            // Разбор остаётся ответом по умолчанию — его зовут чаще, — но
            // поданное имя означает документ, как у всякой другой двери.
            "coverage" if id.is_some() => {
                match entities::entity(&self.pool, &self.kinds, p, "coverage", id).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e),
                }
            }
            "coverage" => match crate::projector::coverage(&self.pool, &self.kinds, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "phases" => match crate::projector::phases(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "waves" => match crate::projector::waves(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "pipeline" => match crate::projector::task_pipeline(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "board" => match crate::projector::task_board(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "task-plan-push" => {
                let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                if task.is_empty() {
                    return refusal(Miss::Refused("план без задачи не записывается".into()));
                }
                match crate::projector::push_task_plan(&self.pool, p, task, body, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "scheme-roles" => match crate::projector::scheme_roles(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "next-step" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::next_step(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "process-state" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::process_state(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "process-history" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::process_history(&self.pool, p, process).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "progress" => match crate::projector::progress(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "code-facts-push" => {
                let fact_kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let list: Vec<(String, String, String)> = Some(rows(args, "facts"))
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("name")?.as_str()?.to_owned(),
                        it.get("detail").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                        it.get("place").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                    ))).collect())
                    .unwrap_or_default();
                // Пустая подача ЗАКОННА только с названным объёмом прочтения.
                // Датчик, прочитавший двадцать файлов и ничего не нашедший,
                // говорит «чисто», а не «не смотрел» — это разные ответы, и
                // складывать их в один — та же ложь, что зелёный ноль. Но
                // подача, не назвавшая, сколько прочитано, неотличима от
                // пустой подачи рукой: она гасила 46 находок G3 одним вызовом
                // (заявка 18). «Прочитано 0» — отказ, а не ноль; молчание
                // сломанного датчика видно по остановившемуся времени подачи
                // и по объявленному сроку свежести.
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
                // Отказ по пустой подаче решается ТАМ, ГДЕ ВИДНО ПРЕЖНЕЕ ЧИСЛО:
                // «было 437, стало 0, и прочитано ноль файлов» — подозрение, а
                // «было 0, стало 0» — честный ответ проекта, у которого такого
                // предмета нет. Здесь прежнего числа не видно, и запрет без
                // него стоил тринадцати датчиков: у набора без HTTP-контракта
                // глоб не находил файлов, отказ не записывал подачу, отметка не
                // двигалась — и двадцать пунктов краснели вечно.
                let read = num(args, "read").unwrap_or(0);
                // Чем снят факт: коммит рабочего каталога и его чистота. Датчик
                // читает каталог, а не `HEAD`, и молчащий об этом след делал
                // факт с незакоммиченной правки неотличимым от фактa со ствола.
                let commit = args.get("commit").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let dirty = flag(args, "dirty");
                let snapshot = crate::projector::Snapshot { commit: &commit, dirty, reads: read };
                match crate::projector::push_code_facts(
                    &self.pool, p, &fact_kind, &list, &self.author, snapshot).await {
                    // Отказ подачи — `isError`, а не ответ с полем `status`: по
                    // признаку отказа судят `mh sense` и обходчик, и без него
                    // отказанный вид считался снятым (undassa/mh#129).
                    Ok(v) if v.get("status").is_some() => refusal(Miss::Refused(v.to_string())),
                    Ok(mut v) => {
                        if crate::reproject::relations::FACT_KINDS.contains(&fact_kind.as_str()) {
                            match crate::reproject::relations::project(&self.pool, p).await {
                                Ok(n) => v["relations"] = json!(n),
                                Err(e) => v["relationsError"] = json!(e.says()),
                            }
                        }
                        ok(v)
                    }
                    Err(e) => refusal(e.into()),
                }
            }
            "code-facts" => {
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                match client
                    .query("SELECT kind, name, detail, place FROM code_fact WHERE project_id = $1 ORDER BY kind, name, place", &[&p])
                    .await
                {
                    Ok(rows) => ok(json!({ "count": rows.len(), "facts": rows.iter().map(|r| json!({
                        "kind": r.get::<_, String>(0), "name": r.get::<_, String>(1),
                        "detail": r.get::<_, String>(2), "place": r.get::<_, String>(3) })).collect::<Vec<_>>() })),
                    Err(e) => refusal(e.into()),
                }
            }
            "skills-push" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let list: Vec<(String, String, String)> = Some(rows(args, "skills"))
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
                let dry = flag(args, "dry");
                match crate::projector::push_skills(&self.pool, &set_name, &list, &self.author, dry).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "skills" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                // Тело отдаётся по просьбе: перечень скиллов спрашивают часто,
                // и таскать в нём двести килобайт незачем.
                let with_body = flag(args, "body");
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
                    Err(e) => return refusal(e.into()),
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
                // СТРОКА С НЕВЕРНЫМ ПОЛЕМ ОТКАЗЫВАЕТСЯ, А НЕ ВЫБРАСЫВАЕТСЯ МОЛЧА.
                //
                // Прежде разбор шёл `filter_map` с `?`: строка без `task` или без
                // `verdict` исчезала, а дверь отвечала «подача пуста». Набор,
                // искавший форму, получал один и тот же ответ на `[]`, на верную
                // строку с опечаткой в ключе и на файл — и пошёл подбирать форму
                // БОЕВЫМИ ВЫЗОВАМИ. Один подбор прошёл, и в набор на минуту лёг
                // вердикт «готово», которого никто не делал.
                //
                // Вынудил его к этому отказ без адреса. Теперь отказ называет
                // строку и поле, а `findings` не вида «число» не превращается в
                // ноль: перечень находок, поданный прозой, уходил в никуда, и
                // пункт гейта говорил «находок ноль».
                let raw = rows(args, "verdicts");
                let mut list: Vec<(String, i64, i64, String, i32, String)> = Vec::new();
                let mut bad: Vec<String> = Vec::new();
                for (n, it) in raw.iter().enumerate() {
                    let task = it.get("task").and_then(|v| v.as_str());
                    let verdict = it.get("verdict").and_then(|v| v.as_str());
                    let findings = it.get("findings");
                    if task.is_none() {
                        bad.push(format!("строка {n}: нет поля `task` со строкой"));
                        continue;
                    }
                    if verdict.is_none() {
                        bad.push(format!("строка {n}: нет поля `verdict` со строкой"));
                        continue;
                    }
                    if findings.is_some_and(|v| !v.is_number()) {
                        bad.push(format!(
                            "строка {n}: `findings` — ЧИСЛО находок, а подано {}. \
                             Перечень уйдёт в никуда, и пункт скажет «находок ноль»",
                            if findings.is_some_and(Value::is_array) { "перечнем" } else { "не числом" }
                        ));
                        continue;
                    }
                    list.push((
                        task.unwrap_or_default().to_owned(),
                        num(it, "at").unwrap_or(0),
                        num(it, "taskRevision").unwrap_or(0),
                        verdict.unwrap_or_default().to_owned(),
                        num(it, "findings").unwrap_or(0) as i32,
                        it.get("body").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                    ));
                }
                if !bad.is_empty() {
                    return json!({ "content": [{ "type": "text", "text": format!(
                        "подача не принята, и вот чем: {}. Форма строки: \
                         {{task, verdict, findings (число), at, taskRevision, body}}",
                        bad.join("; ")) }], "isError": true });
                }
                // «Сказать явно» должно быть ЧЕМ: без флага отказ был тупиком —
                // снять ошибочный вердикт можно было только запросом в базу мимо
                // сервера.
                let clear = flag(args, "clear");
                if list.is_empty() && !clear {
                    return json!({ "content": [{ "type": "text",
                        "text": "подача пуста: полная подача без вердиктов стёрла бы все. Если это и есть \
                                 ответ — сказать явно: clear:=true" }],
                        "isError": true });
                }
                match crate::projector::push_preflight(&self.pool, p, &list,
                        flag(args, "clear"),
                        &self.author).await {
                    Ok(v) => ok(v),
                    // `e.to_string()` у клиентской ошибки — «db error», и причина
                    // теряется. Соседние двери зовут `says`, эта звала иначе.
                    Err(e) => refusal(e.into()),
                }
            }
            "worktree-push" => {
                // Пустая подача здесь ЗАКОННА: ни одного открытого дерева — это
                // ответ, а не молчание. Поэтому отдельного отказа нет.
                let list: Vec<(String, String, i64)> = Some(rows(args, "open"))
                    .map(|l| l.iter().filter_map(|it| Some((
                        it.get("task")?.as_str()?.to_owned(),
                        it.get("branch").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                        num(it, "since").unwrap_or(0),
                    ))).collect())
                    .unwrap_or_default();
                match crate::projector::push_worktrees(&self.pool, p, &list, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
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
            "entity-confirm" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::confirm_entity(&self.pool, p, &g("kind"), &g("id"), &g("why"), &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "term-retire" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_retired_term(&self.pool, p, &g("term"), &g("retiredBy"), &g("declaredIn"),
                        flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "retired-terms" => {
                // Снятое слово в словаре не лежит — его оттуда убрали. Это
                // отдельный факт, и спрашивается он отдельно: встреченное в
                // свежем тексте снятое слово — находка, а не термин.
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
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
                    Err(e) => refusal(e.into()),
                }
            }
            "agents" => {
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy").to_owned();
                let with_body = flag(args, "body");
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
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
                    Err(e) => return refusal(e.into()),
                };
                ok(json!({ "set": set_name, "count": rows.len(),
                    "agents": rows.iter().map(|r| json!({
                        "name": r.get::<_, String>(0), "description": r.get::<_, String>(1),
                        "tools": r.get::<_, String>(2), "model": r.get::<_, String>(3),
                        "effort": r.get::<_, String>(4), "hash": r.get::<_, String>(5),
                        "bytes": r.get::<_, i32>(6), "body": r.get::<_, String>(7),
                    })).collect::<Vec<_>>() }))
            }
            "run-record-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_run_record(&self.pool, p, crate::projector::RunRecord { id: &g("id"), task: &g("task"), milestone: &g("milestone"), title: &g("title"), commits: &g("commits"), dates: &g("dates"), review: &g("review"), appeared: &g("appeared"), left_open: &g("leftOpen") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "decision-link-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_decision_link(&self.pool, p, &g("decision"),
                        &g("kind"), &g("target"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "frame-rule-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_frame_rule(&self.pool, p, crate::projector::FrameRule { kind: &g("kind"), number, title: &g("title"), body: &g("body"), held_by: &g("heldBy") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "process-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = num(args, "ord").unwrap_or(0) as i32;
                match crate::projector::declare_process_row(&self.pool, p, crate::projector::ProcessRow { kind: &g("kind"), a: &g("a"), b: &g("b"), c: &g("c"), d: &g("d"), ord }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "milestone-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_milestone_detail(&self.pool, p, crate::projector::MilestoneDetail { milestone: &g("milestone"), what: &g("what"), blocked_by: &g("blockedBy"), requirement: &g("requirement"), gate: &g("gate"), closed: &g("closed") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "screen-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_screen_detail(&self.pool, p, crate::projector::ScreenDetail { screen: &g("screen"), purpose: &g("purpose"), opens_when: &g("opensWhen"), empty_and_broken: &g("emptyAndBroken"), requirement: &g("requirement") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "story-detail-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_story_detail(&self.pool, p, &g("story"),
                        &g("screen"), &g("persona"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "story-requirement-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_story_requirement(&self.pool, p, &g("story"),
                        &g("requirement"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "feature-story-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_feature_story(&self.pool, p, &g("feature"),
                        &g("story"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "feature-link-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let article = num(args, "article").unwrap_or(0) as i32;
                match crate::projector::declare_feature_link(&self.pool, p, &g("feature"),
                        &g("requirement"), article, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "acceptance-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_acceptance(&self.pool, p, crate::projector::Acceptance { id: &g("id"), story: &g("story"), number, title: &g("title"), preconditions: &g("preconditions"), steps: &g("steps"), observed: &g("observed"), fails_when: &g("failsWhen") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "goal-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_goal(&self.pool, p, crate::projector::Goal { id: &g("id"), number, level: &g("level"), title: &g("title"), measured_by: &g("measuredBy"), checked_when: &g("checkedWhen"), fails_when: &g("failsWhen"), state_now: &g("stateNow") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "goals" => {
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                match client.query("SELECT id, level, title, measured_by, checked_when, fails_when, state_now
                                      FROM project_goal WHERE project_id = $1 ORDER BY number, id", &[p]).await {
                    Ok(rows) => ok(json!({ "count": rows.len(), "goals": rows.iter().map(|r| json!({
                        "id": r.get::<_, String>(0), "level": r.get::<_, String>(1),
                        "title": r.get::<_, String>(2), "measuredBy": r.get::<_, String>(3),
                        "checkedWhen": r.get::<_, String>(4), "failsWhen": r.get::<_, String>(5),
                        "now": r.get::<_, String>(6),
                        // Цель без способа измерить — намерение, и ответ это говорит.
                        "isGoal": !r.get::<_, String>(3).trim().is_empty() })).collect::<Vec<_>>() })),
                    Err(e) => refusal(e.into()),
                }
            }
            "risk-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_risk(&self.pool, p, crate::projector::Risk { id: &g("id"), number, title: &g("title"), state: &g("state"), mitigation: &g("mitigation"), trigger: &g("trigger"), owner: &g("owner"), source: &g("source") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "question-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_question(&self.pool, p, crate::projector::Question { id: &g("id"), number, title: &g("title"), state: &g("state"), answer: &g("answer"), closed_by: &g("closedBy"), owner: flag(args, "owner"), replace: flag(args, "replace") }, flag(args, "drop")).await {
                    // Отказ — отказом, а не ответом с полем: «ничего не записано»
                    // под `isError: false` читается записанным.
                    Ok(v) if matches!(v["status"].as_str(), Some("declared" | "dropped")) => ok(v),
                    Ok(v) => refusal(Miss::Refused(v.to_string())),
                    Err(e) => refusal(e.into()),
                }
            }
            "task-requirement-add" | "screen-reference-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let done = if name == "task-requirement-add" {
                    crate::projector::declare_task_requirement(&self.pool, p, &g("task"), &g("requirement"),
                        flag(args, "drop")).await
                } else {
                    crate::projector::declare_screen_reference(&self.pool, p, &g("source"),
                        &g("sourceKind"), &g("screen"),
                        flag(args, "drop")).await
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "alternative-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = num(args, "ord").unwrap_or(0) as i32;
                match crate::projector::declare_alternative(&self.pool, p, &g("decision"), ord,
                                                             &g("title"), &g("body"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "version-add" | "milestone-add" | "task-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let ord = num(args, "ord").unwrap_or(0) as i32;
                let done = match name {
                    "version-add" => crate::projector::declare_version(&self.pool, p, &g("id"), flag(args, "drop")).await,
                    "milestone-add" => crate::projector::declare_milestone(&self.pool, p, &g("id"),
                                          &g("version"), ord, &g("title"), flag(args, "drop")).await,
                    _ => crate::projector::declare_task(&self.pool, p, crate::projector::Task { id: &g("id"), milestone: &g("milestone"), ord, title: &g("title"), kind: &g("kind"), state: &g("state"), size: &g("size") }, flag(args, "drop")).await,
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "decision-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let number = num(args, "number").unwrap_or(0) as i32;
                match crate::projector::declare_decision(&self.pool, p, crate::projector::Decision { id: &g("id"), number, title: &g("title"), status: &g("status"), status_text: &g("statusText"), date: &g("date"), deciders: &g("deciders"), context: &g("context"), decision: &g("decision"), consequences: &g("consequences") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "story-add" | "screen-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let done = if name == "story-add" {
                    crate::projector::declare_story(&self.pool, p, &g("id"), &g("title"), &g("area"),
                        flag(args, "drop")).await
                } else {
                    crate::projector::declare_screen(&self.pool, p, &g("id"), &g("title"), &g("area"),
                        flag(args, "drop")).await
                };
                match done {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "article-add" => {
                let number = num(args, "number").unwrap_or(0) as i32;
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let anchor = args.get("anchor").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::declare_article(&self.pool, p, number, title, body, anchor, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "requirement-add" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let got = |n: &str| args.get(n).and_then(|v| v.as_str());
                let (kind, area, text, priority, title, measured) =
                    (got("kind"), got("area"), got("text"), got("priority"), got("title"), got("measuredBy"));
                match crate::projector::declare_requirement(&self.pool, p, crate::projector::Requirement { id, kind, area, title, text, measured_by: measured, priority }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "term-add" => {
                let term = args.get("term").and_then(|v| v.as_str()).unwrap_or("");
                let meaning = args.get("meaning").and_then(|v| v.as_str()).unwrap_or("");
                let area = args.get("area").and_then(|v| v.as_str()).unwrap_or("");
                match crate::projector::declare_term(&self.pool, p, term, meaning, area, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "task-dep-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_task_dep(&self.pool, p, &g("task"), &g("dependsOn"),
                                                         flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "release-artifact-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_release_artifact(&self.pool, p, &g("name"),
                        &g("what"), &g("installedTo"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "freeze-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_freeze_row(&self.pool, p, crate::projector::FreezeRow { version: &g("version"), kind: &g("kind"), name: &g("name"), hash: &g("hash") }, &self.author, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "postmortem-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_postmortem(&self.pool, p, crate::projector::Postmortem { id: &g("id"), title: &g("title"), summary: &g("summary"), timeline: &g("timeline"), root_cause: &g("rootCause"), lesson: &g("lesson") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "token-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_token(&self.pool, p, crate::projector::Token { name: &g("name"), dark: &g("dark"), light: &g("light"), purpose: &g("purpose"), section: &g("section") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "reference-source-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_reference_source(&self.pool, p, crate::projector::ReferenceSource { name: &g("name"), source: &g("source"), note: &g("note"), taken: &g("taken"), sha: &g("sha"), ref_type: &g("refType"), from_project: &g("fromProject"), repo: &g("repo"), written: &g("written"), updated: &g("updated"), status: &g("status"), tags: &g("tags"), role: &g("role") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "algorithm-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_algorithm(&self.pool, p, crate::projector::Algorithm { id: &g("id"), story: &g("story"), title: &g("title"), preconditions: &g("preconditions"), flow: &g("flow"), failure: &g("failure"), not_covered: &g("notCovered"), link_kind: &g("linkKind"), link_target: &g("linkTarget") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "stand-row-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_stand_row(&self.pool, p, &g("section"), &g("name"),
                                                           &g("value"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "donor-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_donor(&self.pool, p, &g("path"), &g("what"),
                        &g("frozenBy"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "sensor-spec-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_sensor_spec(&self.pool, p, crate::projector::SensorSpec { fact: &g("fact"), reads: &g("reads"), extract: &g("extract"), note: &g("note"), how: &g("how"), skip: &g("skip"), allow: &g("allow") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "ci-job-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_ci_job(&self.pool, p, crate::projector::CiJob { repo: &g("repo"), workflow: &g("workflow"), job: &g("job"), platform: &g("platform"), note: &g("note") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "guard-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_guard(&self.pool, p, crate::projector::Guard { name: &g("name"), enforces: &g("enforces"), scope: &g("scope"), refuses: &g("refuses"), acts_on: &g("actsOn"), path_re: &g("pathRe"), content_re: &g("contentRe"), command_re: &g("commandRe") }, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "crate-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_crate(&self.pool, p, &g("name"), &g("does"),
                                                       &g("doesNot"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "protocol-op-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_protocol_op(&self.pool, p, &g("op"), &g("group"),
                        &g("events"), &g("requirement"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "article-gate-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let article = num(args, "article").unwrap_or(0) as i32;
                match crate::projector::declare_article_gate(&self.pool, p, article, &g("gate"),
                                                              &g("state"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "requirement-source-add" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_requirement_source(&self.pool, p, &g("id"),
                        &g("kind"), &g("target"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "requirement-scope-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::declare_requirement_scope(&self.pool, p, &g("id"),
                        &g("outOfVersion"), &g("crosscutting"), flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "requirement-retire" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
                let by = args.get("retiredBy").and_then(|v| v.as_str()).unwrap_or("");
                let drop = flag(args, "drop");
                match crate::projector::retire_requirement(&self.pool, p, id, why, by, &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "sensor-declare" => {
                let fact = args.get("fact").and_then(|v| v.as_str()).unwrap_or("");
                let about = args.get("about").and_then(|v| v.as_str()).unwrap_or("");
                let stale = num(args, "staleAfterMs");
                let drop = flag(args, "drop");
                match crate::projector::declare_sensor(&self.pool, p, fact, about, stale, &self.author, drop).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "sensor-specs" => match crate::projector::sensor_specs(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "donors" => match crate::projector::donors_and_guards(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "sensors" => match crate::projector::sensors(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "strain" => match crate::projector::strain(&self.pool).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "version-close" => {
                let version = args.get("version").and_then(|v| v.as_str()).unwrap_or("");
                let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("closed");
                match crate::projector::set_version_state(&self.pool, p, version, state, &self.author, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "step-selftest" => {
                let process = args.get("process").and_then(|v| v.as_str()).unwrap_or("godzy");
                let set_name = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::step_selftest(&self.pool, p, set_name, process,
                        args.get("under").and_then(|v| v.as_str()).unwrap_or("")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "frozen-tree-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let path = g("path");
                if path.trim().is_empty() {
                    return refusal(Miss::Refused("заморозка без каталога не объявляется".into()));
                }
                let drop = flag(args, "drop");
                // Заморозка без хэша — не заморозка. Дверь приняла пустой хэш
                // однажды (имя поля перепутали), и гейт позеленел: сверять было
                // не с чем, а «не с чем» прочиталось как «сошлось».
                if !drop && g("hash").trim().is_empty() {
                    return refusal(Miss::Refused(
                        "заморозка без хэша ничего не держит: назовите хэш дерева либо снимите заморозку `drop=true`"
                            .into()));
                }
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
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
                    Err(e) => refusal(e.into()),
                }
            }
            "scheme-term-set" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let (role, value) = (g("role"), g("value"));
                if role.trim().is_empty() || value.trim().is_empty() {
                    return refusal(Miss::Refused("слово без роли или роль без слова не объявляются".into()));
                }
                let ord = num(args, "ord").unwrap_or(0) as i32;
                let drop = flag(args, "drop");
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                // ОБРАЗЕЦ ИМЕНИ ЧИТАЮТ ДВА РАЗНЫХ ДВИЖКА, И ОБА ДОЛЖНЫ ЕГО ПОНЯТЬ.
                //
                // Роли `id.*` — это выражения. По ним `scheme-roles` сверяет
                // живые имена запросом `id ~ ANY($2)`, а проекции на Rust строят
                // `Regex`. Дверь не проверяла ничего: незакрытая скобка в слове
                // роняла `scheme-roles` отказом базы — ту самую дверь, которой
                // ищут сломанные слова, — а выражение, понятное лишь одному из
                // движков, молча выключало половину читателей.
                if !drop && role.starts_with("id.") {
                    if client.query_one("SELECT 'проба' ~ $1", &[&value]).await.is_err() {
                        return refusal(Miss::Refused(format!(
                            "образец «{value}» не разбирается Postgres: по нему сверяются живые имена, и роль молча перестала бы считать")));
                    }
                    if let Err(e) = regex::Regex::new(&value) {
                        return refusal(Miss::Refused(format!(
                            "образец «{value}» не разбирается Rust: по нему строят проекции. {e}. Помните про расхождение движков: `\\b` в Postgres — не граница слова, а `\\y`")));
                    }
                }
                // СЛОЙ НАЗЫВАЕТСЯ. По умолчанию слово принадлежит ЭТОМУ набору:
                // дверь зовётся из репозитория и отвечает про проект, и писать
                // из неё в общее значило бы менять чужой разбор молча — так и
                // было, пока слоя не было.
                //
                // `shared=true` — объявление для всех, и его надо сказать вслух.
                let shared = flag(args, "shared");
                let owner = if shared { "" } else { p };
                let done = if drop {
                    client.execute("DELETE FROM scheme_term WHERE project_id=$3 AND role=$1 AND value=$2",
                                   &[&role, &value, &owner]).await
                } else {
                    client.execute(
                        "INSERT INTO scheme_term (project_id, role, value, ord, why) VALUES ($5,$1,$2,$3,$4)
                         ON CONFLICT (project_id, role, value) DO UPDATE SET ord = EXCLUDED.ord,
                           why = EXCLUDED.why",
                        &[&role, &value, &ord, &g("why"), &owner]).await
                };
                match done {
                    Ok(n) => ok(json!({ "role": role, "value": value, "written": n, "dropped": drop,
                        "layer": if shared { "общий — действует на все наборы" } else { "этого набора" },
                        "means": if shared { "" } else {
                            "роль, объявленная набором, ЗАМЕЩАЕТ общий список целиком, а не дополняет его" } })),
                    Err(e) => refusal(e.into()),
                }
            }
            "scheme-terms" => {
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                // Отдаётся РАЗРЕШЁННЫЙ словарь — тот, которым говорит этот набор.
                // Показывать оба слоя вперемешку значило бы показывать список,
                // которым не пользуется никто.
                match client.query("SELECT s.role, s.value, s.ord, s.why,
                                           EXISTS (SELECT 1 FROM scheme_term o
                                                    WHERE o.project_id = $1 AND o.role = s.role) AS своё
                                      FROM scheme($1) s ORDER BY s.role, s.ord, s.value", &[p]).await {
                    Ok(rows) => ok(json!({ "count": rows.len(), "terms": rows.iter().map(|r| json!({
                        "role": r.get::<_, String>(0), "value": r.get::<_, String>(1),
                        "ord": r.get::<_, i32>(2), "why": r.get::<_, String>(3),
                        "layer": if r.get::<_, bool>(4) { "набора" } else { "общий" } })).collect::<Vec<_>>() })),
                    Err(e) => refusal(e.into()),
                }
            }
            "orphans-purge" => {
                // Убирается ТОЛЬКО то, чей набор не значится проектом. Живой
                // проект этой ручкой не тронуть по устройству запроса, а не по
                // обещанию: условие сравнивает с перечнем проектов.
                let confirm = flag(args, "confirm");
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
                let tables = client
                    .query(
                        "SELECT table_name FROM information_schema.columns
                          WHERE column_name = 'project_id' AND table_schema = current_schema()
                          GROUP BY table_name ORDER BY table_name",
                        &[],
                    )
                    .await;
                let Ok(tables) = tables else {
                    return refusal(Miss::Refused("перечень таблиц не читается".into()));
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
                                // ПУСТОЙ `project_id` — НЕ СИРОТА, А ОБЩИЙ СЛОЙ.
                                //
                                // Словарь схемы устроен в два слоя: своё у набора
                                // и общее на всех, и общее хранится с пустым
                                // именем проекта. Пустое имя среди проектов не
                                // значится — и чистка, взяв «всех, кого нет
                                // среди проектов», сносила ОБЩИЙ СЛОВАРЬ ЦЕЛИКОМ.
                                // Проверено ценой: один вызов с подтверждением
                                // снёс его, и у соседнего набора разом потухли
                                // правила, которым не стало на чём считать —
                                // выглядело это находками в наборе.
                                //
                                // Восстановить нечем: летописи вызовов дверей
                                // база не ведёт.
                                "SELECT count(*) FROM \"{name}\"
                                  WHERE project_id <> '' AND project_id NOT IN (SELECT id FROM projects)"
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
                                      WHERE project_id <> '' AND project_id NOT IN (SELECT id FROM projects)"
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
                    return refusal(Miss::Refused("источник без поверхности или без вида не объявляется".into()));
                }
                let drop = flag(args, "drop");
                let client = match crate::db::conn(&self.pool).await { Ok(c) => c, Err(e) => return refusal(e.into()) };
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
                    Err(e) => refusal(e.into()),
                }
            }
            "links-rewrite" => {
                let apply = flag(args, "apply");
                match crate::projector::rewrite_links(&self.pool, p, !apply).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
                }
            }
            "entity-rename" => {
                let g = |n: &str| args.get(n).and_then(|v| v.as_str()).unwrap_or("").to_owned();
                match crate::projector::rename_entity(&self.pool, p, kind_arg, &g("from"), &g("to"),
                        flag(args, "apply"),
                        flag(args, "merge")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "links-retarget" => {
                let apply = flag(args, "apply");
                match crate::projector::retarget_links(&self.pool, p, !apply).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "author-set" => {
                let who = args.get("author").and_then(|v| v.as_str()).unwrap_or("");
                match entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                    Ok((k, n)) => match crate::projector::set_author(&self.pool, p, &k, &n, who, flag(args, "drop")).await {
                        Ok(v) => ok(v),
                        Err(e) => refusal(e.into()),
                    },
                    Err(e) => refusal(e),
                }
            }
            "authors" => match crate::projector::authors(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "principal-allow" => {
                let who = args.get("principal").and_then(|v| v.as_str()).unwrap_or("");
                let note = args.get("note").and_then(|v| v.as_str());
                let drop = flag(args, "drop");
                if who.is_empty() { return refusal(Miss::Refused("человек не назван".into())); }
                match crate::projector::allow_principal(&self.pool, who, note, drop, &self.author).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "principals" => match crate::projector::principals(&self.pool).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            // Сверка порождённого — отдельной ручкой, а не только внутри
            // тяжёлой пересборки: спросить «отстал ли файл» надо уметь, не
            // переписывая при этом все проекции.
            "generated-check" => match crate::projector::check_generated(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "version-freeze" => {
                let v = args.get("version").and_then(|x| x.as_str()).unwrap_or("");
                if v.is_empty() { return refusal(Miss::Refused("выпуск не назван".into())); }
                match crate::projector::freeze_version(&self.pool, p, v, &self.author).await {
                    Ok(x) => ok(x),
                    Err(e) => refusal(e.into()),
                }
            }
            "version-delta" => {
                let v = args.get("version").and_then(|x| x.as_str()).unwrap_or("");
                if v.is_empty() { return refusal(Miss::Refused("выпуск не назван".into())); }
                match crate::projector::version_delta(&self.pool, p, v).await {
                    Ok(x) => ok(x),
                    Err(e) => refusal(e.into()),
                }
            }
            "agent-set" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                if name.is_empty() || body.is_empty() {
                    return refusal(Miss::Refused("субагент без имени или без тела не записывается".into()));
                }
                match crate::projector::set_agent(&self.pool, crate::projector::Agent { set_name: set, name, description: args.get("description").and_then(|v| v.as_str()), body, tools: args.get("tools").and_then(|v| v.as_str()), model: args.get("model").and_then(|v| v.as_str()) }, &self.author, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "skill-set" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                let description = args.get("description").and_then(|v| v.as_str());
                if name.is_empty() || body.is_empty() {
                    return refusal(Miss::Refused("скилл без имени или без тела не записывается".into()));
                }
                // Умение и субагент законно носят одно имя: `godzy-preflight` —
                // и процедура, которой следует сессия, и работник, которого
                // отправляют. Запрет на совпадение имён стоял здесь один раз и
                // ломал правку умения; настоящая защита — не запрет, а наличие
                // своей двери у субагента (`agent-set`), которой прежде не было.
                let allowed = args.get("allowedTools").and_then(|v| v.as_str());
                // Умолчание — НЕ «нет», а «не трогать»: довод трёхзначен, и
                // подать его строкой можно так же, как всякий другой.
                let invocation = match args.get("disableModelInvocation") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(truthy(v)),
                };
                match crate::projector::set_skill(&self.pool, crate::projector::Skill { set_name: set, name, description, body, allowed_tools: allowed, disable_model_invocation: invocation }, &self.author, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "skills-paths" => {
                let set = args.get("set").and_then(|v| v.as_str()).unwrap_or("godzy");
                match crate::projector::skills_with_paths(&self.pool, set).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "screen-area-set" => {
                let screen = args.get("screen").and_then(|v| v.as_str()).unwrap_or("");
                let area = args.get("area").and_then(|v| v.as_str()).unwrap_or("");
                if screen.is_empty() {
                    return refusal(Miss::Refused("область ставится экрану; экран не назван".into()));
                }
                match crate::projector::set_screen_area(&self.pool, p, screen, area, flag(args, "drop")).await {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "reparse" => match crate::watch::rebuilding(&self.pool, p, async |lease| crate::store::reparse_all(&self.pool, p, lease).await).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "gate-measure" => match crate::watch::measure(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "gate-selftest" => match crate::projector::gate_selftest(&self.pool, p,
                    args.get("under").and_then(|v| v.as_str()).unwrap_or("")).await {
                Ok(v) => ok(v),
                // `e.to_string()` у ошибки Postgres — это слово «db error» и
                // ничего больше: отказ, по которому не видно, что случилось.
                Err(e) => refusal(e.into()),
            },
            "order" => match crate::projector::order(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "summary" => match crate::projector::summary(&self.pool, p, kind_arg).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            },
            "question-holders" => match crate::projector::question_holders(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "method-set" => {
                let ord = num(args, "ord").unwrap_or(-1) as i32;
                let mk = args.get("methodKind").and_then(|v| v.as_str()).unwrap_or("unknown");
                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let drop = flag(args, "drop");
                match crate::projector::set_method(&self.pool, p, crate::projector::Method { kind: kind_arg, id: id.unwrap_or(""), ord, method_kind: mk, method, declared_by: &self.author, drop }).await {
                    // ОТКАЗ ОСТАЁТСЯ ОТКАЗОМ И В ОБОЛОЧКЕ. Успех с полем
                    // `status` уходит с кодом 0, и цикл на `set -e`,
                    // объявляющий полтораста способов несуществующим номерам,
                    // отрапортовал бы полтораста успехов. Так же поступает
                    // соседняя дверь `readiness` с полем `unprojected`.
                    Ok(v) if v.get("status") == Some(&json!("no_item")) => json!({
                        "content": [{ "type": "text", "text": v["why"].as_str().unwrap_or("") }],
                        "isError": true
                    }),
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
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
                Err(e) => refusal(e.into()),
            },
            "requirements-of" => match crate::projector::requirements_of(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "tasks-of" => match crate::projector::tasks_of_story(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "preflight-queue" => match crate::projector::preflight_queue(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "claims" => match crate::projector::claims(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "frozen-trees" | "addresses-declared" | "tree-declared" => match self.ask(name, kind_arg, id, args).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e),
            },
            "gate" => match crate::projector::gate(
                &self.pool,
                p,
                id,
                flag(args, "sql"),
            )
            .await
            {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "what-if" => {
                let tool = args.get("tool").and_then(|v| v.as_str()).unwrap_or("").trim().to_owned();
                if tool.is_empty() {
                    return refusal(Miss::Refused("примерять нечего: не названа дверь".into()));
                }
                // Примерка внутри примерки — копия копии: десятки секунд на
                // каждом уровне и ни одного нового ответа.
                if tool == "what-if" {
                    return refusal(Miss::Refused(
                        "примерка примерки — это копия копии: ответ тот же, цена вдвое".into()));
                }
                // ПРИМЕРЯЕТСЯ НАБОР, А НЕ ПРИБОР.
                //
                // Копия копирует то, у чего есть имя набора. Пункт гейта, фаза,
                // ступень лестницы имени набора не имеют — они общие, и правка
                // их на копии ушла бы в ЖИВОЕ, а снятие копии её не отменило бы:
                // снимать нечего, строка лежит в общей таблице. Тихая правка
                // прибора под видом примерки хуже, чем её отсутствие.
                if !Self::TRY_ON.contains(&tool.as_str()) {
                    return refusal(Miss::Refused(format!(
                        "дверь «{tool}» не примеряется: примерять можно только двери, про которые проверено, \
                         что они пишут лишь в таблицы набора — правка общего ушла бы в живое, и снятие \
                         копии её не вернуло бы. Примеряются: {}", Self::TRY_ON.join(" · "))));
                }
                let inner = args.get("args").cloned().unwrap_or_else(|| json!({}));
                if let Some(refused) = self.args_refusal(&tool, &inner) {
                    return refused;
                }
                match crate::projector::what_if(
                    &self.pool, &self.kinds, p, &self.author, &tool, &inner).await
                {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e.into()),
                }
            }
            "next-task" => match crate::projector::next_task(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "blockers" => match crate::projector::task_blockers(&self.pool, p, id.unwrap_or("")).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
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
                match crate::store::create(&self.pool, &self.kinds, &self.project, crate::store::Document { kind: kind_arg, name: id.unwrap_or(""), content: args.get("content").and_then(|v| v.as_str()).unwrap_or("") }, &self.author, now)
                .await
                {
                    Ok(v) => ok(v),
                    Err(e) => refusal(e),
                }
            }
            // ОДНА ДВЕРЬ ПОД ОДНУ РАБОТУ. `put` правил написанное, заводил
            // `document-add`, и какая из двух нужна, выяснялось отказом.
            //
            // Молчаливого заведения при этом не будет: `create` надо сказать
            // вслух. Отказ без него остаётся — он и называет, куда идти, — но
            // сказавшему «заводи» больше не приходится звать вторую дверь.
            "put" if flag(args, "create") => {
                // «Не нашлось» и «спросить не вышло» — разные ответы. Пока
                // они были одним, занятый пул читался как «документа нет»:
                // заведение упиралось в `ON CONFLICT DO NOTHING`, и дверь
                // отвечала «уже есть», выбросив поданный текст без ошибки.
                let exists = match crate::entities::locate(&self.pool, &self.kinds, p, kind_arg, id).await {
                    Ok(_) => true,
                    Err(Miss::NoEntity(..)) | Err(Miss::NoKind(_)) | Err(Miss::Unprojected(_)) => false,
                    Err(e) => return refusal(e),
                };
                if exists {
                    self.write(name, kind_arg, id, args).await
                } else {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    match crate::store::create(&self.pool, &self.kinds, &self.project, crate::store::Document { kind: kind_arg, name: id.unwrap_or(""), content: args.get("content").and_then(|v| v.as_str()).unwrap_or("") }, &self.author, now)
                    .await
                    {
                        Ok(v) => ok(v),
                        Err(e) => refusal(e),
                    }
                }
            }
            "put" | "put-section" | "rm" => self.write(name, kind_arg, id, args).await,
            "sweep" => match crate::store::sweep_orphans(&self.pool, p).await {
                Ok(v) => ok(v),
                Err(e) => refusal(e.into()),
            },
            "reproject" => match self.projections().await {
                Ok(v) => ok(json!({ "own": v })),
                Err(e) => refusal(e),
            },
            // НЕИЗВЕСТНОЕ ИМЯ НАЗЫВАЕТ БЛИЖАЙШИЕ. «Нет такого инструмента:
            // release» — правда, от которой нечего делать: имена дверей у
            // документов неоднородны, и спрашивающий не знает, чем этот документ
            // достаётся. Ответ теперь называет похожие двери и два пути дальше:
            // `kinds` — чем набор вправе быть, `search` — где это слово вообще
            // встречается. Просьба сессии `tot-ade`, пункт второй.
            // СНЯТАЯ ДВЕРЬ ОТВЕЧАЕТ, КУДА ОНА ДЕЛАСЬ. «Нет такого инструмента»
            // — правда, но для звавшего её вчера она неотличима от опечатки, и
            // он идёт искать имя. Четырнадцать дверей правки прибора сняты
            // решением владельца по #19: прибор объявлен репозиторием. Набор,
            // у которого в задаче стоит такой вызов, должен узнать это от
            // харнеса, а не вывести из молчания: три задачи `tot-ade` звали
            // `gate-item-set` и упёрлись в «похожие: gate».
            other if TAKEN_AWAY.contains(&other) => {
                json!({ "content": [{ "type": "text", "text": format!(
                    "дверь `{other}` снята: прибор объявлен репозиторием, и правок во время работы у него больше нет. \
                     Пункт гейта, фаза, ступень лестницы и раскладка вида живут файлами в `instrument/` репозитория \
                     `undassa/mh` и меняются коммитом с ревью, а не вызовом. Что объявлено сейчас — `gate` и `phases`. \
                     Нужна правка прибора — заявка сессии харнеса либо issue с меткой `harness-request`") }],
                    "isError": true })
            }
            other => {
                let near = self.near(other);
                let mut text = format!("нет такого инструмента: {other}");
                if !near.is_empty() {
                    text.push_str(&format!(". Похожие: {}", near.join(" · ")));
                }
                text.push_str(". Что вообще бывает — `kinds` (виды набора) и `doors q=…` (двери по вопросу); \
                               где встречается слово — `search q=…`");
                json!({ "content": [{ "type": "text", "text": text }], "isError": true })
            }
        }
    }

    /// Конвейер пересборки: до-проход → предметные проекции → после-проход.
    /// Реестр видов ИЗ БАЗЫ, а не слепок на момент старта.
    ///
    /// `kind-projection` и `kind-required` пишут в `kind_layout`, а ручки
    /// отвечали из памяти: дверь говорила «объявлено», `kinds-due` продолжала
    /// звать вид необъявленным. Двадцать два объявления не были видны ни одной
    /// ручкой — запись прошла, ответ остался прежним, и отличить это от отказа
    /// было нечем. Разбор документов по-прежнему идёт по слепку: он меняет
    /// поведение проекции, и его смена — перезапуск.
    async fn live_registry(&self) -> Result<crate::kinds::Kinds, Miss> {
        crate::kinds::Kinds::from_db(&self.pool).await
    }

    async fn projections(&self) -> Result<Value, Miss> {
        // `e.to_string()` у ошибки Postgres — слова «db error» и ничего больше:
        // причина лежит в источнике, и её показывает `says`. Пересборка,
        // упавшая молча, здесь три месяца отвечала «база не ответила».
        let say = |step: &str| {
            let step = step.to_owned();
            move |e: crate::db::Fail| Miss::from(e).step(&step)
        };
        // Время каждого шага — В ОТВЕТЕ. Правка документа шла сорок три секунды,
        // и без разметки виновным назначался тот шаг, на который думалось: те же
        // пересборки, позванные отдельно, укладываются в сто тридцать миллисекунд.
        let (before, subject, after, ms_before, ms_subject, ms_after) =
            crate::watch::rebuilding(&self.pool, &self.project, async |lease| {
                let t = std::time::Instant::now();
                let before = crate::projector::rebuild_before(&self.pool, &self.project, lease)
                    .await
                    .map_err(say("сверка до пересборки"))?;
                let ms_before = t.elapsed().as_millis() as u64;
                let t = std::time::Instant::now();
                // Исход пересборки ЗАПИСЫВАЕТСЯ. Упавшая на полпути оставляет
                // проекции недособранными, а гейт продолжает отдавать прежние
                // числа — уверенно и неверно; так был потерян час на тридцать
                // одну ложную находку.
                let subject = match crate::reproject::reproject(&self.pool, &self.project, lease).await {
                    Ok(v) => {
                        crate::projector::note_reproject(&self.pool, &self.project, true, "").await;
                        v
                    }
                    Err(e) => {
                        let said = e.says();
                        crate::projector::note_reproject(&self.pool, &self.project, false, &said).await;
                        return Err(say("пересборка сущностей")(e));
                    }
                };
                let ms_subject = t.elapsed().as_millis() as u64;
                let t = std::time::Instant::now();
                let after = crate::projector::rebuild(&self.pool, &self.project, lease)
                    .await
                    .map_err(say("пересборка проекций"))?;
                let ms_after = t.elapsed().as_millis() as u64;
                Ok((before, subject, after, ms_before, ms_subject, ms_after))
            })
            .await?;
        let t = std::time::Instant::now();
        let generated = crate::projector::check_generated(&self.pool, &self.project)
            .await
            .map_err(say("сверка выведенного"))?;
        let ms_generated = t.elapsed().as_millis() as u64;
        // ЗАМЕР ГЕЙТОВ — ЧАСТЬЮ ПЕРЕСБОРКИ, а не отдельной командой. Дверь
        // гейта отдаёт СОХРАНЁННОЕ, и это правильно: считать 124 пункта на
        // каждое чтение дорого. Но сохранённое должно быть свежим.
        //
        // Пока замера здесь не было, каскад работал вхолостую: отметки
        // пересчитывались, переоткрытия появлялись, а гейт отвечал вчерашним.
        // Целую сессию я списывал это на «замер до синхронизации» и объяснял
        // неверно — кэш был не отставанием, а тишиной.
        let t = std::time::Instant::now();
        let gates = crate::watch::measure(&self.pool, &self.project)
            .await
            .map_err(say("замер гейтов"))?;
        let ms_gates = t.elapsed().as_millis() as u64;
        Ok(json!({ "before": before, "subject": subject, "after": after, "generated": generated,
                   "gates": gates,
                   "мс": { "сверка до": ms_before, "сущности": ms_subject,
                           "проекции": ms_after, "выведенное": ms_generated,
                           "гейты": ms_gates } }))
    }

    /// Запросы к таблицам этого сервера.
    async fn ask(&self, name: &str, kind: &str, id: Option<&str>, args: &Value) -> Result<Value, Miss> {
        let client = crate::db::conn(&self.pool).await?;
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
                // Место сущности спрашивается ТЕМ ЖЕ соединением: второе при
                // живом первом запирает пул на себе же.
                let where_ = entities::locate_at(&*client, &self.kinds, p, kind,
                                                 if id.is_empty() { None } else { Some(id) }).await;
                if let Err(e @ (Miss::Busy(_) | Miss::Db(_))) = where_ {
                    return Err(e);
                }
                if let Ok((k, n)) = where_ {
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
                        // ЧИТАЕТСЯ ПРОЕКЦИЯ, А НЕ ДОКУМЕНТЫ ЗАНОВО. Дверь держала
                        // свой образец адреса рядом с образцом проекции, и они
                        // отличались: образец проекции знает ЯКОРЬ — цитату
                        // строки после адреса, — а образец двери нет. Оттого
                        // якорь не показывался вовсе, и увидеть, что у всех
                        // пятидесяти девяти адресов `tot-ade` он пуст, было
                        // нечем: пункт `code-address-resolves` при пустом якоре
                        // сверяет лишь существование строки, то есть зелен там,
                        // где сверять нечего.
                        //
                        // КТО НАЗВАЛ АДРЕС — в ответе. Без этого свой адрес не
                        // отличить от чужого: у `tot-ade` 40 адресов из 112
                        // пришли из справки о соседнем продукте, гейт это
                        // видел, а дверь не отвечала.
                        // ИМЯ ПРЕДМЕТА ИЗ ФРАЗЫ — тоже в ответе, и оно сильнее
                        // якоря: якорь сверяет одну строку, а фраза почти
                        // всегда говорит о предмете целиком. Датчику оно нужно
                        // затем, чтобы ответить, лежит ли названное там, куда
                        // ведёт адрес: файл у него в руках, а у правила нет.
                        "SELECT path, line::text, entity_kind, entity_name, anchor, named
                           FROM project_code_address
                          WHERE project_id = $1
                          ORDER BY path, line",
                        &[p],
                    )
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?;
                let out: Vec<Value> = rows
                    .iter()
                    .map(|r| json!({ "path": r.get::<_, String>(0), "line": r.get::<_, String>(1),
                                     "kind": r.get::<_, String>(2), "name": r.get::<_, String>(3),
                                     "anchor": r.get::<_, String>(4),
                                     "named": r.get::<_, String>(5) }))
                    .collect();
                let bare = out.iter().filter(|a| a["anchor"].as_str().unwrap_or("").is_empty()).count();
                Ok(json!({ "count": out.len(), "bare": bare, "addresses": out,
                           "means": "`kind`/`name` — кто адрес назвал: адрес внутри справки \
                                     о соседнем продукте указывает в ЕГО дерево, а не в наше. \
                                     `anchor` — цитата строки, которую адрес называет; пустой \
                                     якорь значит, что сверяется только существование строки, \
                                     а не то, что в ней написано. Якорь объявляется в тексте \
                                     так: `путь.rs:123` — `цитата строки`" }))
            }
            "tree-declared" => {
                let paths: Vec<Value> = crate::projector::declared_tree(&*client, p)
                    .await
                    .map_err(|e| Miss::Db(e.to_string()))?
                    .into_iter()
                    .map(|(path, state)| json!({ "path": path, "state": state }))
                    .collect();
                Ok(json!({ "count": paths.len(), "paths": paths }))
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
                let registry = match self.live_registry().await { Ok(k) => k, Err(e) => return Err(e) };
                let mut due = Vec::new();
                let mut undeclared = Vec::new();
                for (kind, k) in &registry.0 {
                    match k.projection.as_deref() {
                        // «Много ли документов» спрашивалось через `at.is_none()` — «нет
                        // одного адреса». У семи одиночных видов адреса и так не
                        // объявлено, и они отвечали «много». Форма сущности это
                        // знает без файла.
                        Some("due") => due.push(json!({ "kind": kind, "documents": !k.single })),
                        None => undeclared.push(kind.clone()),
                        _ => {}
                    }
                }
                // «НЕ ОБЪЯВЛЕНО» И «НЕ РАЗБИРАЕТСЯ» — РАЗНОЕ. Ручка звала
                // необъявленными шестнадцать видов, и среди них `srs`,
                // `constitution`, `glossary`, `test-cases` — те, из которых гейты
                // берут требования, статьи, термины и проверки КАЖДЫМ ЗАМЕРОМ.
                // Вести по плану, который не знает своих же видов, нельзя.
                //
                // Разбирается вид или нет — не мнение, а наблюдение: проекция
                // называет источник колонкой `entity_kind`. Таблицы берутся из
                // схемы, а не перечислены здесь: перечень разошёлся бы с ней при
                // первой новой проекции.
                let read: std::collections::HashSet<String> = {
                    // Таблицы берутся из РЕЕСТРА ВИДОВ, а не из схемы по наличию
                    // колонки: `entity_kind` есть и у `project_documents`, и у
                    // разделов, и у ячеек — туда попадает КАЖДЫЙ вид, у которого
                    // есть хоть один документ, и разбор перестал бы значить
                    // что-либо. Предметная таблица — та, что объявлена виду.
                    let tables: Vec<String> = self
                        .kinds
                        .0
                        .keys()
                        .filter_map(|k| crate::kinds::table_of(k).map(|(t, _, _)| t.to_owned()))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let mut seen = std::collections::HashSet::new();
                    for t in &tables {
                        if let Ok(rows) = client
                            .query(
                                &format!("SELECT DISTINCT entity_kind FROM {t} WHERE project_id = $1 AND entity_kind <> ''"),
                                &[p],
                            )
                            .await
                        {
                            for r in &rows {
                                seen.insert(r.get::<_, String>(0));
                            }
                        }
                    }
                    seen
                };
                let (parsed, silent): (Vec<String>, Vec<String>) =
                    undeclared.iter().cloned().partition(|k| read.contains(k));
                Ok(json!({
                    "due": due,
                    "undeclared": undeclared,
                    "undeclaredButParsed": parsed,
                    "undeclaredAndSilent": silent,
                    "note": "«не объявлено» — не «не надо»: этого не сказал никто, и спросить придётся человека. \
                             Но «не объявлено» и «не разбирается» — разное: вид из `undeclaredButParsed` набор \
                             читает каждым замером, и необъявленной у него осталась только раскладка."
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
        let expected = num(args, "expectedRevision");
        let done = match name {
            "put" => crate::store::put(&self.pool, &self.project, crate::store::Document { kind: &owner_kind, name: &owner_name, content: args.get("content").and_then(|v| v.as_str()).unwrap_or("") }, &self.author, expected, now).await,
            "put-section" => crate::store::put_section(&self.pool, &self.project, crate::store::SectionEdit { kind: &owner_kind, name: &owner_name, anchor: args.get("anchor").and_then(|v| v.as_str()).unwrap_or(""), body: args.get("body").and_then(|v| v.as_str()).unwrap_or("") }, &self.author, expected, now).await,
            _ => crate::store::remove(&self.pool, &self.project, &owner_kind, &owner_name).await,
        };
        let mut v = match done {
            Ok(v) => v,
            Err(e) => return refusal(e.into()),
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
                return refusal(Miss::Refused(format!("правка принята, но след не записан: {e}")));
            }
            // Отложенная пересборка — для правки МНОГИХ документов подряд: она идёт
            // по всему набору, и делать её после каждой из восьмидесяти записей
            // значит платить восемьдесят раз за одно и то же. Дверь остаётся дверью:
            // запись прошла сервером, отложено только следствие. Ответ говорит об
            // этом ВСЛУХ — молча устаревшая проекция читалась бы как свежая.
            if flag(args, "deferProjection") {
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
                Err(e) => {
                    let done = if status == "deleted" { "документ снят".to_owned() } else { format!("правка записана: ревизия {revision}") };
                    return written_unprojected(&done, e);
                }
            }
        }
        if matches!(status.as_str(), "conflict" | "not_found" | "no_such_section" | "drops_sections" | "invalid_path") {
            return json!({ "content": [{ "type": "text", "text": v.to_string() }], "isError": true });
        }
        ok(v)

    }

}

/// Отказанная подача фактов — `isError` двери (undassa/mh#129): без признака
/// `mh sense` и обходчик считали отказанный вид снятым.
#[cfg(test)]
mod refused_facts {
    use serde_json::json;

    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn an_empty_push_over_rows_is_an_error_of_the_door() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Drefused_facts");
        let pool = crate::db::pool(&format!("{url}{apart}"), 2).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS refused_facts CASCADE; CREATE SCHEMA refused_facts;")
            .await.expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        let door = super::Mcp {
            pool: pool.clone(),
            kinds: std::sync::Arc::new(crate::kinds::Kinds::from_db(&pool).await.expect("виды")),
            project: "p".to_owned(),
            author: "проба".to_owned(),
        };
        let first = door.call("code-facts-push", &json!({ "kind": "probe", "read": 1,
            "facts": [{ "name": "a", "detail": "есть" }] })).await;
        assert_ne!(first["isError"], json!(true), "подача принята не была: {first}");

        let empty = door.call("code-facts-push", &json!({ "kind": "probe", "read": 0, "facts": [] })).await;
        assert_eq!(empty["isError"], json!(true), "отказанная подача ответила успехом: {empty}");
        assert!(empty["content"][0]["text"].as_str().unwrap_or("").contains("empty_push_over_rows"),
                "отказ не называет причину: {empty}");

        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS refused_facts CASCADE").await.expect("схема снимается");
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::refusal;
    use crate::entities::Miss;

    /// Занятость доезжает до вызывающего ПРИЗНАКОМ. Пока двери складывали её
    /// в `Miss::Db` словами, агент получал «база не ответила» и код отказа по
    /// существу — и повторял вызов как ошибку, а не как перегрузку.
    /// Признак занятости читается ОДНИМ местом, и это место — рядом с тем, где
    /// он ставится: разъехавшись, они уже однажды оставили ветку примерки
    /// мёртвой, и перегрузка на копии снова читалась приговором набору.
    #[test]
    fn busyness_is_read_where_it_is_written() {
        let busy: Miss = crate::db::Fail::Busy("занято".to_owned()).into();
        assert!(crate::door::busy_said(&refusal(busy)), "дверь сказала «занято», а читатель не увидел");
        assert!(!crate::door::busy_said(&refusal(Miss::Refused("довода нет".to_owned()))));
        assert!(!crate::door::busy_said(&serde_json::json!({ "content": [], "isError": false })));
    }

    /// Шаг называется, а род отказа остаётся: `reproject` на перегрузке
    /// отвечал «база не ответила» и кодом отказа по существу — и это ровно та
    /// дверь, которую советуют позвать, когда что-то не досчиталось.
    #[test]
    fn a_named_step_keeps_the_kind_of_refusal() {
        let busy: Miss = crate::db::Fail::Busy("все соединения заняты".to_owned()).into();
        let with_step = busy.step("сверка до пересборки");
        assert!(matches!(with_step, Miss::Busy(_)), "шаг не превращает занятость в отказ базы");
        let out = refusal(with_step);
        assert_eq!(out["_meta"]["busy"], serde_json::json!(true));
        assert!(out["content"][0]["text"].as_str().unwrap_or("").contains("сверка до пересборки"));
    }

    #[test]
    fn busyness_keeps_its_flag_through_a_door() {
        let busy: Miss = crate::db::Fail::Busy("все соединения заняты".to_owned()).into();
        let out = refusal(busy);
        assert_eq!(out["_meta"]["busy"], serde_json::json!(true));
        assert_eq!(out["isError"], serde_json::json!(true));
        let by_merits = refusal(Miss::Refused("довода нет".to_owned()));
        assert_eq!(by_merits["_meta"]["busy"], serde_json::json!(false));
        let database: Miss = crate::db::Fail::Down("база не принимает соединение".to_owned()).into();
        assert_eq!(refusal(database)["_meta"]["busy"], serde_json::json!(false), "недоступная база — не занятость");
    }

    /// Булев довод понимается в том написании, в каком его подают. Командная
    /// строка отдаёт всё строками, и `drop=1` молча значил «нет»: сравнение
    /// шло с единственным написанием `"true"`. Дверь принимала довод, отвечала
    /// успехом и не делала ничего.
    #[test]
    fn a_yes_is_a_yes_in_every_spelling() {
        use super::flag;
        for yes in [serde_json::json!(true), serde_json::json!(1), serde_json::json!("1"),
                    serde_json::json!("true"), serde_json::json!("TRUE"), serde_json::json!(" да "),
                    serde_json::json!("yes"), serde_json::json!("on")] {
            let args = serde_json::json!({ "drop": yes });
            assert!(flag(&args, "drop"), "{yes} — это «да»");
        }
        for no in [serde_json::json!(false), serde_json::json!(0), serde_json::json!("0"),
                   serde_json::json!("false"), serde_json::json!(""), serde_json::json!(null),
                   serde_json::json!("нет"), serde_json::json!("моё имя")] {
            let args = serde_json::json!({ "drop": no });
            assert!(!flag(&args, "drop"), "{no} — не «да»");
        }
        assert!(!flag(&serde_json::json!({}), "drop"), "довода нет — значит «нет»");
    }

    /// Перечень дверей называет и необязательные доводы. Пока он называл одни
    /// обязательные, дверь `task-requirement-add` выглядела не умеющей снять
    /// объявленное ребро — а `drop` у неё есть, и подписан. Двух находок `G2`
    /// это стоило до того, как спросили голосом.
    #[test]
    fn a_door_listing_names_what_else_it_takes() {
        use super::door_arguments;
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "task": {}, "requirement": {}, "drop": {} },
            "required": ["task", "requirement"],
        });
        let (needs, takes) = door_arguments(Some(&schema));
        assert_eq!(needs, serde_json::json!(["task", "requirement"]));
        assert_eq!(takes, vec!["drop".to_string()], "необязательный довод обязан быть назван");

        // Двери без схемы и без необязательных доводов отвечают пустотой, а не падают.
        assert_eq!(door_arguments(None), (serde_json::json!([]), Vec::new()));
        let bare = serde_json::json!({ "type": "object", "properties": { "id": {} }, "required": ["id"] });
        assert!(door_arguments(Some(&bare)).1.is_empty(), "лишнего не выдумывается");
    }
}

#[cfg(test)]
mod tests {
    use super::{Mcp, Value};
    use serde_json::json;
    use std::sync::Arc;

    /// Имя инструмента не единственно, и сверка доводов обязана это знать.
    ///
    /// Виды сущностей кладутся в перечень первыми, дверь с тем же именем —
    /// позже, а сверка искала первую запись по имени. У набора есть вид `gate`,
    /// и оттого дверь `gate` сверялась схемой вида: та знает один `id`, и
    /// заведённый 2026-09-25 довод `sql` отвергался как неизвестный — до
    /// разбора, то есть названный в описании двери способ не работал ни разу.
    #[test]
    fn a_door_is_not_shadowed_by_a_kind_of_the_same_name() {
        let kind: crate::kinds::Kind = serde_json::from_value(json!({})).expect("вид из умолчаний");
        let kinds = crate::kinds::Kinds([("gate".to_owned(), kind)].into_iter().collect());
        let mcp = Mcp {
            pool: crate::db::pool("postgres://нет/нет", 1).expect("пул без соединения"),
            kinds: Arc::new(kinds),
            project: "набор".to_owned(),
            author: "проверка".to_owned(),
        };

        let same_name = mcp.tools().iter().filter(|t: &&Value| t["name"] == "gate").count();
        assert_eq!(same_name, 2, "предмет проверки — два инструмента с одним именем");

        assert!(
            mcp.args_refusal("gate", &json!({ "sql": true })).is_none(),
            "довод двери отвергнут схемой вида с тем же именем"
        );
        assert!(
            mcp.args_refusal("gate", &json!({ "такого-довода-нет": 1 })).is_some(),
            "неизвестный довод обязан отвергаться, иначе объединение сняло бы сверку целиком"
        );
    }
}
