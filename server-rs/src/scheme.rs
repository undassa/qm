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
    /// Словарь ЭТОГО набора: своё, если роль объявлена, иначе общее.
    pub async fn load(pool: &Pool, project: &str) -> Result<Self, tokio_postgres::Error> {
        let client = pool.get().await.expect("пул отдал соединение");
        let rows = client
            .query("SELECT role, value FROM scheme($1) ORDER BY role, ord, value", &[&project])
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
    ///
    /// `None` и при ДВУСМЫСЛЕННОСТИ: у роли два слова, а спрашивают одно.
    /// Прежде брался первый по порядку и о втором не говорилось. На `myack` у
    /// роли `field.red-parent` стояли «Пара» и «Родительская задача», обе с
    /// нулевым порядком; взялось не то, и восемьдесят две красные задачи разом
    /// потеряли родителя — два пункта покраснели на сто шестьдесят четыре
    /// нарушения, а выглядело это порчей набора.
    ///
    /// Молча выбирать нельзя. Правило без слова не считается и говорит об этом;
    /// правило с двумя словами не считается тем более — оно не знает, каким.
    pub fn one(&self, role: &str) -> Option<&str> {
        match self.by_role.get(role) {
            Some(v) if v.len() == 1 => v.first().map(String::as_str),
            _ => None,
        }
    }

    /// Роли, у которых слов больше одного, а спрашивают их одним.
    ///
    /// Отдельным перечнем, потому что `one()` про такую роль отвечает `None` —
    /// тем же словом, что и про необъявленную. Для человека это разные беды:
    /// одну чинят объявлением, другую — снятием лишнего.
    pub fn ambiguous(&self, roles: &[&str]) -> Vec<String> {
        roles
            .iter()
            .filter(|r| self.by_role.get(**r).map(|v| v.len() > 1).unwrap_or(false))
            .map(|r| format!("{}: {}", r, self.by_role[*r].join(" · ")))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(pairs: &[(&str, &str)]) -> Terms {
        let mut by_role: HashMap<String, Vec<String>> = HashMap::new();
        for (r, v) in pairs {
            by_role.entry((*r).to_owned()).or_default().push((*v).to_owned());
        }
        Terms { by_role }
    }

    #[test]
    fn одно_слово_роли_отдаётся() {
        assert_eq!(terms(&[("field.parent", "Родитель")]).one("field.parent"), Some("Родитель"));
    }

    /// Ровно тот слом, что стоил восьмидесяти двух красных задач: у роли два
    /// слова, бралось первое по порядку, и о втором никто не узнавал.
    #[test]
    fn два_слова_роли_не_выбираются_молча() {
        let t = terms(&[("field.red-parent", "Пара"), ("field.red-parent", "Родительская задача")]);
        assert_eq!(t.one("field.red-parent"), None);
        assert_eq!(t.ambiguous(&["field.red-parent"]).len(), 1);
    }

    #[test]
    fn необъявленная_роль_и_двусмысленная_различаются() {
        let t = terms(&[("a", "x"), ("a", "y")]);
        assert_eq!(t.missing(&["b"]), vec!["b".to_owned()]);
        assert!(t.missing(&["a"]).is_empty());
        assert_eq!(t.ambiguous(&["b"]).len(), 0);
    }
}
