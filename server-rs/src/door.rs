//! Что дверь говорит о себе в ответе — и где это лежит.
//!
//! Признак занятости ставит сервер, а читают его четверо: сеть, примерка,
//! клиент и сам протокол. Пока читатели знали место каждый по себе, перенос
//! поля из `content`-уровня в `_meta` оставил ветку примерки мёртвой — молча,
//! и перегрузка снова выдавалась за приговор набору. Место названо здесь один
//! раз, и обе сборки — сервер и клиент — берут его отсюда.

use serde_json::{json, Value};

/// Пометить ответ двери занятостью. `_meta` — объявленное протоколом место для
/// своего поля; поле рядом с `content` строгий клиент вправе выбросить.
pub fn mark_busy(busy: bool) -> Value {
    json!({ "busy": busy })
}

/// Сказала ли дверь «занято».
pub fn busy_said(ответ: &Value) -> bool {
    ответ["_meta"]["busy"].as_bool().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{busy_said, mark_busy};
    use serde_json::json;

    #[test]
    fn what_is_written_is_what_is_read() {
        let занято = json!({ "content": [], "isError": true, "_meta": mark_busy(true) });
        assert!(busy_said(&занято));
        let по_существу = json!({ "content": [], "isError": true, "_meta": mark_busy(false) });
        assert!(!busy_said(&по_существу));
        assert!(!busy_said(&json!({ "content": [], "isError": false })), "ответ без пометки — не занятость");
        assert!(!busy_said(&json!("строка")), "не предмет — не занятость");
    }
}
