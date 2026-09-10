//! Виды сущностей: чем проект населён и как это спросить.
//!
//! Раскладка приходит из карты проекта (`corpus-layout.json`) — она принадлежит
//! проекту, а не серверу: у другого проекта виды свои. Карта таблиц живёт
//! здесь, потому что это знание сервера о собственной схеме.
//!
//! **Путь в раскладке есть, и это последнее место, где он есть.** Пока часть
//! документов опознаётся только адресом, вид «одиночка» иначе не найти. Наружу
//! путь не выходит ни одним маршрутом и ни одним инструментом.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Kind {
    #[serde(default)]
    pub shape: String,
    #[serde(default)]
    pub single: bool,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub under: Option<String>,
    #[serde(default, rename = "not-under")]
    pub not_under: Option<String>,
    /// Образец имени файла: `{id}.md`. Пусто — имя и есть путь под корнем.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, rename = "in")]
    pub in_kind: Option<String>,
    /// Положена ли виду предметная таблица: `done` · `due` · `prose` ·
    /// `render` · `provenance`. Пусто — не объявлено, и это третье состояние.
    #[serde(default)]
    pub projection: Option<String>,
    /// Образец имени: `^Q-\d+$`. Применяется и на чтении, и на перечислении.
    #[serde(default)]
    pub id: Option<String>,
    /// Чем имя является: «имя задачи, как её зовёт сам набор».
    #[serde(default, rename = "name-is")]
    pub name_is: Option<String>,
    /// Обязателен ли вид: без этого документа проект не заводится.
    ///
    /// Минимальный набор объявлен ОДИН раз и общий, как гейты и лестница: мы
    /// разрабатываем по единой схеме, значит и мерка одна. Прежде «что нужно
    /// проекту» знала только проза плана документов — и молчала о семи
    /// заведённых документах из двадцати двух, потому что её никто не сверял.
    #[serde(default)]
    pub required: bool,
    /// Почему вид обязателен.
    #[serde(default, rename = "required-why")]
    pub required_why: Option<String>,
}

impl Kind {
    pub fn is_inner(&self) -> bool {
        self.shape == "inner"
    }
}

#[derive(Debug, Deserialize)]
struct Layout {
    kinds: BTreeMap<String, Kind>,
}

/// Куда смотреть за сущностью вида: таблица, колонка имени, колонка подписи.
///
/// Те же имена, что у потребителя в харнесе: словарь один на обе стороны, иначе
/// переключение источника станет переводом.
pub const TABLES: &[(&str, &str, &str, &str)] = &[
    ("question", "project_questions", "id", "title"),
    ("decision", "project_decisions", "id", "title"),
    ("requirement", "project_requirements", "id", "text"),
    ("check", "project_checks", "id", "spec"),
    ("need", "project_needs", "id", "text"),
    ("story", "project_stories", "id", "title"),
    ("screen", "project_screens", "id", "title"),
    ("feature", "project_features", "id", "title"),
    ("task", "project_plan_tasks", "id", "title"),
    ("milestone", "project_plan_milestones", "id", "title"),
    ("version", "project_plan_versions", "id", "id"),
    ("risk", "project_risks", "id", "title"),
    ("term", "project_terms", "id", "meaning"),
    ("article", "project_articles", "number::text", "title"),
    ("run", "project_runs_log", "id", "title"),
    ("gate", "project_gates", "phase", "item"),
];

pub fn table_of(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    TABLES.iter().find(|t| t.0 == kind).map(|t| (t.1, t.2, t.3))
}

#[derive(Debug, Clone)]
pub struct Kinds(pub BTreeMap<String, Kind>);

impl Kinds {
    pub fn load(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("раскладка видов не читается ({path}): {e}"))?;
        let layout: Layout = serde_json::from_str(&raw).map_err(|e| format!("раскладка видов не разбирается: {e}"))?;
        Ok(Kinds(layout.kinds))
    }

    /// Раскладка из базы — обычный способ; файл остаётся способом её завести.
    pub async fn from_db(pool: &deadpool_postgres::Pool) -> Result<Self, String> {
        let client = pool.get().await.map_err(|e| format!("пул не отдал соединение: {e}"))?;
        let rows = client
            .query("SELECT name, spec FROM kind_layout", &[])
            .await
            .map_err(|e| format!("раскладка видов не читается из базы: {e}"))?;
        let mut kinds = BTreeMap::new();
        for r in &rows {
            let name: String = r.get(0);
            let spec: serde_json::Value = r.get(1);
            let kind: Kind = serde_json::from_value(spec)
                .map_err(|e| format!("вид {name} не разбирается: {e}"))?;
            kinds.insert(name, kind);
        }
        Ok(Kinds(kinds))
    }

    /// Записать раскладку в базу: файл → таблица, один раз при переносе.
    pub async fn into_db(&self, pool: &deadpool_postgres::Pool, by: &str) -> Result<usize, String> {
        let client = pool.get().await.map_err(|e| format!("пул не отдал соединение: {e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let mut n = 0;
        for (name, kind) in &self.0 {
            let spec = serde_json::to_value(kind).map_err(|e| format!("вид {name} не сериализуется: {e}"))?;
            client
                .execute(
                    "INSERT INTO kind_layout (name, spec, declared_at, declared_by) VALUES ($1,$2,$3,$4)
                     ON CONFLICT (name) DO UPDATE SET spec = EXCLUDED.spec,
                       declared_at = EXCLUDED.declared_at, declared_by = EXCLUDED.declared_by",
                    &[name, &spec, &now, &by],
                )
                .await
                .map_err(|e| format!("вид {name} не записан: {e}"))?;
            n += 1;
        }
        Ok(n)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Kind> {
        self.0.get(name)
    }

}
