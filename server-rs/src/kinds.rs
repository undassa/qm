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
    #[serde(default, rename = "in")]
    pub in_kind: Option<String>,
    /// Положена ли виду предметная таблица: `done` · `due` · `prose` ·
    /// `render` · `provenance`. Пусто — не объявлено, и это третье состояние.
    #[serde(default)]
    pub projection: Option<String>,
    /// Область проекта: чем вид по сути. Объявляется дверью `kind-domain`.
    #[serde(default)]
    pub domain: Option<String>,
    /// Таблицы, в которые ложатся сущности этого вида. Проверяются дверью по
    /// связи `entity_kind`: объявление, которого не подтверждают строки, — та
    /// же пустота, только подписанная.
    #[serde(default)]
    pub holds: Option<Vec<String>>,
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
    pub(crate) fn is_inner(&self) -> bool {
        self.shape == "inner"
    }
}

/// Куда смотреть за сущностью вида: таблица, колонка имени, колонка подписи.
///
/// Те же имена, что у потребителя в харнесе: словарь один на обе стороны, иначе
/// переключение источника станет переводом.
pub(crate) const TABLES: &[(&str, &str, &str, &str)] = &[
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
    ("gate", "project_gates", "phase", "id"),
    // Виды, заведённые дверью `kind-add` в эту сессию. Перечень ЗАШИТ, а вид
    // уже объявил свою таблицу сам — `kind-projection … holds`, и дверь это
    // объявление проверила по живым строкам. Пока `table_of` читает список, а
    // не объявление, всякий новый вид будет невидим при живых записях:
    // `rationale` пропал из корпуса при 381 записи ровно так.
    ("rationale", "project_rationale", "id", "title"),
    ("lint-rule", "project_lint_rules", "id", "area"),
    ("assertion", "project_assertions", "id", "area"),
];

pub(crate) fn table_of(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    TABLES.iter().find(|t| t.0 == kind).map(|t| (t.1, t.2, t.3))
}

#[derive(Debug, Clone)]
pub struct Kinds(pub BTreeMap<String, Kind>);

impl Kinds {
    /// Раскладка из базы, куда её кладёт объявление репозитория.
    pub async fn from_db(pool: &deadpool_postgres::Pool) -> Result<Self, crate::entities::Miss> {
        let client = crate::db::conn(pool).await?;
        let rows = client.query("SELECT name, spec FROM kind_layout", &[]).await?;
        let mut kinds = BTreeMap::new();
        for r in &rows {
            let name: String = r.get(0);
            let spec: serde_json::Value = r.get(1);
            let kind: Kind = serde_json::from_value(spec)
                .map_err(|e| crate::entities::Miss::Refused(format!("вид {name} не разбирается: {e}")))?;
            kinds.insert(name, kind);
        }
        Ok(Kinds(kinds))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Kind> {
        self.0.get(name)
    }

}
