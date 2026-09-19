//! Пересборка предметных проекций — на Rust, вместо донорского прохода.
//!
//! Донор пересобирает тридцать таблиц Node-процессом, который сервер запускает
//! на каждую правку. Пока он там, «старого кода не работает» — неправда, и
//! четыре секунды на правку тоже его. Перенос идёт по одной проекции: каждая
//! сверяется с тем, что оставил донор, — по числу строк и по хешу
//! упорядоченного дампа, — и только сошедшаяся заменяет донорскую.
//!
//! Порядок сохраняется донорский: он не произволен, каждая следующая опирается
//! на предыдущую.

use deadpool_postgres::Pool;
use serde_json::{json, Value};

mod articles;
mod data_model;
mod decisions;
mod document_plan;
mod glossary;
pub mod ids;
pub mod tree_leaf;
pub mod relations;
mod surface_landing;
mod milestone_count;
mod named_ids;
mod traceability_said;
mod needs;
pub(crate) mod plan;
mod plan_status;
mod proof;
mod questions;
mod risks;
mod rows;
mod traceability;
mod runs;
mod surface;

/// Что уже перенесено. Остальное по-прежнему делает донор.
/// Заголовок без имени сущности: `ADR-0046: Донорский код…` → `Донорский код…`.
///
/// Имя уже стоит КОЛОНКОЙ. Повторённое в заголовке, оно даёт «ADR-0046 —
/// ADR-0046: …» всюду, где имя и заголовок показывают рядом: в списках, на
/// карточках, в деталях гейта. У одного набора так было у 786 сущностей из 890,
/// у другого — ни у одной, и разница была не в наборах, а в том, что второй
/// чистили руками.
///
/// Снимается только ОДНА форма: имя в начале и сразу за ним разделитель. Имя
/// внутри фразы — часть фразы, и его никто не трогает.
pub(crate) fn title_without_name(title: &str, name: &str) -> String {
    let t = title.trim();
    if name.is_empty() || !t.starts_with(name) {
        return t.to_owned();
    }
    let rest = t[name.len()..].trim_start();
    match rest.chars().next() {
        Some(c) if matches!(c, ':' | '·' | '—' | '–' | '-') => {
            let cut = rest[c.len_utf8()..].trim_start();
            if cut.is_empty() { t.to_owned() } else { cut.to_owned() }
        }
        _ => t.to_owned(),
    }
}

pub async fn reproject(pool: &Pool, project: &str) -> Result<Value, crate::db::Fail> {
    let mut done = serde_json::Map::new();
    let (a, r) = articles::project(pool, project).await?;
    done.insert("project_articles".into(), json!(a));
    done.insert("project_article_references".into(), json!(r));
    let (f, l, runs) = runs::project(pool, project).await?;
    done.insert("project_features".into(), json!(f));
    done.insert("project_feature_stories".into(), json!(l));
    done.insert("project_runs_log".into(), json!(runs));
    done.insert("project_terms".into(), json!(glossary::project(pool, project).await?));
    done.insert("project_risks".into(), json!(risks::project(pool, project).await?));
    done.insert("project_traceability_claims".into(), json!(traceability::project(pool, project).await?));
    let (v, m, t, d) = plan::project(pool, project).await?;
    done.insert("project_plan_versions".into(), json!(v));
    done.insert("project_plan_milestones".into(), json!(m));
    done.insert("project_plan_tasks".into(), json!(t));
    done.insert("project_plan_task_deps".into(), json!(d));
    // Волна кладётся сразу за связями: она из них и считается, и считать её
    // второй раз где-либо ещё — это тот самый «второй порядок на один план».
    done.insert("project_plan_status".into(), json!(plan_status::project(pool, project).await?));
    let (n, ns) = needs::project(pool, project).await?;
    done.insert("project_needs".into(), json!(n));
    done.insert("project_need_stories".into(), json!(ns));
    let (req, chk, rn, home) = proof::project(pool, project).await?;
    done.insert("project_requirements".into(), json!(req));
    done.insert("project_checks".into(), json!(chk));
    done.insert("project_requirement_needs".into(), json!(rn));
    if home > 0 {
        done.insert("проверок переписан источник".into(), json!(home));
    }
    done.insert("project_questions".into(), json!(questions::project(pool, project).await?));
    let (d, dl, da) = decisions::project(pool, project).await?;
    done.insert("project_decisions".into(), json!(d));
    done.insert("project_decision_links".into(), json!(dl));
    done.insert("project_decision_alternatives".into(), json!(da));
    let (t, mg) = data_model::project(pool, project).await?;
    done.insert("project_db_tables".into(), json!(t));
    done.insert("project_db_migrations".into(), json!(mg));
    let (st, sc, sr, sq) = surface::project(pool, project).await?;
    done.insert("project_stories".into(), json!(st));
    done.insert("project_screens".into(), json!(sc));
    done.insert("project_screen_references".into(), json!(sr));
    done.insert("project_story_requirements".into(), json!(sq));
    done.insert("project_milestone_count".into(), json!(milestone_count::project(pool, project).await?));
    done.insert("project_named_id".into(), json!(named_ids::project(pool, project).await?));
    done.insert("project_traceability_said".into(), json!(traceability_said::project(pool, project).await?));
    done.insert("project_requirement_surface".into(), json!(surface_landing::project(pool, project).await?));
    let (dp, dc) = document_plan::project(pool, project).await?;
    done.insert("project_document_plan".into(), json!(dp));
    done.insert("project_document_plan_counts".into(), json!(dc));
    done.insert("entity_stamp_changed".into(), json!(proof::stamp(pool, project).await?));
    done.insert("owner_questions_asked".into(), json!(crate::projector::sync_owner_questions(pool, project).await?));
    Ok(Value::Object(done))
}
