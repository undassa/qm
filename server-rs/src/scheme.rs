//! Словарь схемы: роль, которую знает код, и слово, которым её зовёт набор.
//!
//! Бинарник обязан работать с любым проектом. Слово, зашитое в него, делает
//! его знающим один: набор, назвавший раздел иначе, получил бы не отказ, а
//! молчаливый ноль — правило нашло бы пусто и позеленело.
//!
//! Отсюда правило: **умолчаний нет**. Роль, для которой значение не объявлено,
//! называется вслух, и правило, на неё опирающееся, отвечает «не знаю», а не
//! «сошлось».

use deadpool_postgres::Pool;
use std::collections::HashMap;

pub struct Terms {
    by_role: HashMap<String, Vec<String>>,
}

impl Terms {
    pub async fn load(pool: &Pool) -> Result<Self, tokio_postgres::Error> {
        let client = pool.get().await.expect("пул отдал соединение");
        let rows = client
            .query("SELECT role, value FROM scheme_term ORDER BY role, ord, value", &[])
            .await?;
        let mut by_role: HashMap<String, Vec<String>> = HashMap::new();
        for r in &rows {
            by_role.entry(r.get(0)).or_default().push(r.get(1));
        }
        Ok(Terms { by_role })
    }

    /// Единственное слово роли. `None` — роль не объявлена, и это НЕ пустая
    /// строка: пустая строка совпала бы с пустым значением в базе и правило
    /// нашло бы «всё», а не «ничего».
    pub fn one(&self, role: &str) -> Option<&str> {
        self.by_role.get(role).and_then(|v| v.first()).map(String::as_str)
    }

    /// Все слова роли: список стоп-слов, синонимы заголовка.
    pub fn all(&self, role: &str) -> &[String] {
        static EMPTY: Vec<String> = Vec::new();
        self.by_role.get(role).unwrap_or(&EMPTY)
    }

    /// Роли, которых нет. Их называют вслух: правило без слова не считается.
    pub fn missing(&self, roles: &[&str]) -> Vec<String> {
        roles
            .iter()
            .filter(|r| !self.by_role.contains_key(**r))
            .map(|r| (*r).to_owned())
            .collect()
    }
}
