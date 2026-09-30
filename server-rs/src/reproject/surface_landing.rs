//! Поверхности, на которые приземляются требования.
//!
//! Множество поверхностей ЗАКРЫТО решением: требование либо видно на одной из
//! них, либо названо держателем инварианта. Третьего нет. Правило, читающее
//! поверхности «как получится», молча теряет требование: оно есть в перечне,
//! у него есть проверка, а показать его негде — и никто этого не видит.
//!
//! Читаются поверхности РАСКРЫВАТЕЛЕМ, а не голым образцом: документ экрана
//! называет требования перечнем `FR-SIT-04, 07, 09`, и голое чтение
//! недосчитывается сорока шести из двухсот шести.

use deadpool_postgres::Pool;

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let client = crate::db::conn(pool).await?;
    let terms = crate::scheme::Terms::load_at(&*client, project).await?;
    let surface_marker = terms.one("marker.surface-list").unwrap_or("").to_owned();
    // Живые требования: имя, которого нет в перечне, поверхностью не считается.
    // Документ вправе поминать удалённое — считать это приземлением значит
    // показывать покрытие шире настоящего.
    let live: std::collections::HashSet<String> = client
        .query(
            "SELECT id FROM project_requirements WHERE project_id = $1 AND kind = 'FR'",
            &[&project],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();

    // Из чего состоит поверхность — ОБЪЯВЛЕНО, а не угадано по имени.
    let sources = client
        .query(
            "SELECT surface, entity_kind, entity_name FROM project_surface_source
              WHERE project_id = $1 ORDER BY surface, entity_kind, entity_name",
            &[&project],
        )
        .await?;
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for src in &sources {
        let surface: String = src.get(0);
        let kind: String = src.get(1);
        let name: String = src.get(2);
        // Источник бывает не только документом: поверхность «контракт HTTP»
        // живёт в репозитории, и подаёт её датчик. Читается она отсюда же,
        // чтобы перечень поверхностей был один.
        if kind == "code-fact" {
            let facts = client
                .query(
                    "SELECT name FROM code_fact WHERE project_id = $1 AND kind = $2",
                    &[&project, &name],
                )
                .await?;
            for f in &facts {
                let id: String = f.get(0);
                if !live.contains(&id) {
                    continue;
                }
                if seen.insert(format!("{id} {surface}")) {
                    rows.push((id, surface.clone(), name.clone()));
                }
            }
            continue;
        }
        let docs = if name.is_empty() {
            client
                .query(
                    "SELECT entity_name, content FROM project_documents
                      WHERE project_id = $1 AND entity_kind = $2",
                    &[&project, &kind],
                )
                .await?
        } else {
            client
                .query(
                    "SELECT entity_name, content FROM project_documents
                      WHERE project_id = $1 AND entity_kind = $2 AND entity_name = $3",
                    &[&project, &kind, &name],
                )
                .await?
        };
        for d in &docs {
            let said_in: String = d.get(0);
            let content: String = d.get(1);
            for line in content.lines() {
                for id in super::ids::expand(line) {
                    if !id.starts_with("FR-") || !live.contains(&id) {
                        continue;
                    }
                    if seen.insert(format!("{id} {surface}")) {
                        rows.push((id, surface.clone(), said_in.clone()));
                    }
                }
            }
        }
    }

    // Держатели инварианта: строка перечня требований вида
    // `- ` + имя + ` — ... ` + путь в бэктиках.
    // ОБРАЗЕЦ ИМЕНИ ДАЁТ НАБОР. Зашитый требовал буквенного участка —
    // `FR-SIT-04`, — а требования бывают `FR-01…FR-125`, без него. Ни одна
    // строка не совпадала никогда: держателей объявлено ноль, и объявить их
    // нечем.
    //
    // Здесь это дороже, чем в других образцах имён: правило формулирует
    // альтернативу — требование либо видно на поверхности, ЛИБО названо
    // держателем инварианта, — и зашитый образец ОТНИМАЛ у правила его
    // собственную вторую половину. «Либо-либо» превращалось в «только».
    //
    // Роль `id.requirement`; не объявлена — образец прежний.
    let id_re = terms
        .all("id.requirement")
        .first()
        .cloned()
        .unwrap_or_else(|| "FR-[A-Z]+-[0-9]+".to_owned());
    let holder_row = regex::Regex::new(&holder_pattern(&id_re)).expect("образец строки держателя");
    let holder_path = regex::Regex::new(r"`([^`\s]+\.rs)`").expect("образец пути держателя");
    let srs = client
        .query(
            "SELECT content FROM project_documents WHERE project_id = $1 AND entity_kind = 'srs'",
            &[&project],
        )
        .await?;
    let mut holders: Vec<(String, String)> = Vec::new();
    for d in &srs {
        let content: String = d.get(0);
        for line in content.lines() {
            let Some(c) = holder_row.captures(line) else { continue };
            let id = c[1].to_owned();
            if !live.contains(&id) {
                continue;
            }
            for p in holder_path.captures_iter(&c[2]) {
                holders.push((id.clone(), p[1].to_owned()));
            }
        }
    }

    // Перечень, названный решением: строка «**Поверхности … :** имя · имя».
    // Разбор строки — здесь, один раз; дальше два перечня сверяются равенством.
    let said = if surface_marker.is_empty() { Vec::new() } else { client
        .query(
            "SELECT entity_name, content FROM project_documents
              WHERE project_id = $1 AND entity_kind = 'decision' AND position($2 in content) > 0",
            &[&project, &surface_marker],
        )
        .await? };
    let mut declared: Vec<(String, String)> = Vec::new();
    for d in &said {
        let name: String = d.get(0);
        let content: String = d.get(1);
        for line in content.lines() {
            let Some(rest) = line.strip_prefix(surface_marker.as_str()) else { continue };
            let Some((_, tail)) = rest.split_once(":**") else { continue };
            for part in tail.split('·') {
                let clean = part
                    .trim()
                    .trim_matches(|c: char| c == '`' || c == '.' || c == ',' || c == ';')
                    .trim()
                    .to_owned();
                if !clean.is_empty() {
                    declared.push((clean, name.clone()));
                }
            }
        }
    }

    // Читающее соединение отпущено: писать и читать одновременно здесь нечем,
    // а два соединения на одну проекцию запирают пул на себе же.
    drop(client);
    let mut client = crate::db::conn(pool).await?;
    let tx = client.transaction().await?;
    crate::db::hold(&tx, "projection", project).await?;
    // Поверхность контракта HTTP приходит датчиком: она в репозитории, а не в
    // наборе, и удалять её здесь нечем.
    tx.execute(
        "DELETE FROM project_requirement_surface WHERE project_id = $1 AND surface <> 'контракт HTTP'",
        &[&project],
    )
    .await?;
    tx.execute("DELETE FROM project_requirement_holder WHERE project_id = $1", &[&project]).await?;
    for (id, path) in &holders {
        tx.execute("INSERT INTO project_requirement_holder(project_id, requirement_id, path)
                    VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                   &[&project, id, path]).await?;
    }
    tx.execute("DELETE FROM project_surface_declared WHERE project_id = $1", &[&project]).await?;
    for (surface, said_in) in &declared {
        tx.execute("INSERT INTO project_surface_declared(project_id, surface, said_in)
                    VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                   &[&project, surface, said_in]).await?;
    }
    for (id, surface, said_in) in &rows {
        tx.execute(
            "INSERT INTO project_requirement_surface(project_id, requirement_id, surface, said_in)
             VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&project, id, surface, said_in],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

/// Образец строки держателя из объявленного образца имени требования.
///
/// Якоря снимаются: имя стоит ВНУТРИ строки, а `^…$` из объявления имени
/// относятся к имени целиком, и оставленные они не совпадут никогда.
pub(crate) fn holder_pattern(id_re: &str) -> String {
    format!(
        r"^\s*[-*]\s+.?({}).?\s*[—–-]\s*(.+)$",
        id_re.trim_start_matches('^').trim_end_matches('$')
    )
}

#[cfg(test)]
mod holder {
    use super::holder_pattern;

    fn catches(id_re: &str, line: &str) -> bool {
        regex::Regex::new(&holder_pattern(id_re))
            .expect("образец")
            .is_match(line)
    }

    const WITHOUT_LETTERS: &str = "- `FR-01` — инвариант держится `crates/core/src/rule.rs`";
    const WITH_WITH_LETTERS: &str = "- `FR-SIT-04` — инвариант держится `crates/core/src/rule.rs`";

    #[test]
    fn hardcoded_pattern_not_saw_names_without_letter_part() {
        assert!(!catches("FR-[A-Z]+-[0-9]+", WITHOUT_LETTERS), "прежний образец совпал, а не должен был");
        assert!(catches("FR-[A-Z]+-[0-9]+", WITH_WITH_LETTERS));
    }

    #[test]
    fn declared_pattern_sees_own_name() {
        assert!(catches("FR-[0-9]+", WITHOUT_LETTERS), "объявленный образец не поймал строку набора");
    }

    #[test]
    fn anchor_declaration_cleared() {
        // `id.requirement` объявляется как образец ИМЕНИ — `^FR-[0-9]+$`.
        // Оставленные якоря не совпали бы ни с одной строкой перечня.
        assert!(catches("^FR-[0-9]+$", WITHOUT_LETTERS));
    }
}
