//! Личность приходит заголовком `x-portal-identity` и выглядит как
//! `base64url(claims).HMAC-SHA256(payload)`.
//!
//! Это **не** JWT, хотя похоже: у JWT три части и заголовок с алгоритмом, здесь
//! две и алгоритм один. Библиотека JWT такой токен отвергнет по форме, а
//! попытка подогнать его под JWT завела бы второй формат рядом с существующим.
//!
//! Срок `exp` — в **миллисекундах**. Секунды здесь дали бы токен, протухший
//! пятьдесят лет назад, и отказ выглядел бы как поломка подписи.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

#[derive(Debug, Deserialize)]
pub struct Claims {
    /// Кто: идентификатор участника.
    pub p: String,
    /// Докуда действителен, в миллисекундах.
    pub exp: i64,
}

#[derive(Debug, PartialEq)]
pub enum Refusal {
    Malformed,
    BadSignature,
    Expired,
}

pub fn verify(token: &str, secret: &[u8], now_ms: i64) -> Result<Claims, Refusal> {
    let dot = token.rfind('.').ok_or(Refusal::Malformed)?;
    let (payload, signature) = token.split_at(dot);
    let signature = &signature[1..];

    let raw = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| Refusal::BadSignature)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Refusal::Malformed)?;
    mac.update(payload.as_bytes());
    // `verify_slice` сравнивает за постоянное время. Своё сравнение — хоть `==`,
    // хоть побайтовый цикл с ранним выходом — рассказывает подбирающему, сколько
    // знаков он уже угадал, и подпись перестаёт что-либо значить.
    mac.verify_slice(&raw).map_err(|_| Refusal::BadSignature)?;

    let json = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| Refusal::Malformed)?;
    let claims: Claims = serde_json::from_slice(&json).map_err(|_| Refusal::Malformed)?;
    if claims.p.is_empty() {
        return Err(Refusal::Malformed);
    }
    if now_ms > claims.exp {
        return Err(Refusal::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Строковый литерал, а не байтовый: `b"..."` в Rust принимает только ASCII.
    const SECRET: &[u8] = "секрет".as_bytes();

    fn mint(payload: &str, secret: &[u8]) -> String {
        let body = URL_SAFE_NO_PAD.encode(payload);
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body.as_bytes());
        format!("{body}.{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn живая_личность_принимается() {
        let token = mint(r#"{"p":"кто-то","exp":2000}"#, SECRET);
        assert_eq!(verify(&token, SECRET, 1000).unwrap().p, "кто-то");
    }

    #[test]
    fn срок_читается_миллисекундами() {
        // 2000 мс — это будущее для 1000 мс и прошлое для 3000 мс. Прочитанный
        // секундами, тот же токен протух бы в обоих случаях.
        let token = mint(r#"{"p":"кто-то","exp":2000}"#, SECRET);
        assert_eq!(verify(&token, SECRET, 3000).unwrap_err(), Refusal::Expired);
    }

    #[test]
    fn чужая_подпись_не_проходит() {
        let token = mint(r#"{"p":"кто-то","exp":2000}"#, b"another");
        assert_eq!(verify(&token, SECRET, 1000).unwrap_err(), Refusal::BadSignature);
    }

    #[test]
    fn подмена_полезной_части_ломает_подпись() {
        let token = mint(r#"{"p":"кто-то","exp":2000}"#, SECRET);
        let forged = format!("{}.{}", URL_SAFE_NO_PAD.encode(r#"{"p":"чужой","exp":2000}"#), token.split('.').nth(1).unwrap());
        assert_eq!(verify(&forged, SECRET, 1000).unwrap_err(), Refusal::BadSignature);
    }

    #[test]
    fn токен_без_точки_не_личность() {
        assert_eq!(verify("простострока", SECRET, 1000).unwrap_err(), Refusal::Malformed);
    }
}
