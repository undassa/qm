//! Связи, вынутые из фактов датчиков и полей документов.
//!
//! Каждая живёт своей таблицей и соединяется РАВЕНСТВОМ. Иначе связь пришлось
//! бы искать подстрокой в чужом тексте — а такая связь рвётся от правки прозы
//! и чинится молча: правило перестаёт находить и выглядит зелёным.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;

static CHECK_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"TC-[A-Z]+-[0-9]+[a-z]?").expect("образец проверки"));

pub async fn project(pool: &Pool, project: &str) -> Result<usize, tokio_postgres::Error> {
    let client = pool.get().await.expect("пул отдал соединение");
    // Слова схемы — из словаря, не из кода. Роль без слова НЕ подставляет
    // пустое: пустое совпало бы со всем подряд. Такая связь просто не
    // считается, и её отсутствие видно перечнем ниже.
    let terms = crate::scheme::Terms::load(pool).await?;
    let missing = terms.missing(&[
        "field.task-contract-ops", "field.red-parent", "field.red-checks",
        "section.proof", "word.not-a-subject", "path.crate-home",
    ]);

    // Требование ↔ операция контракта: датчик подаёт пару одним именем, потому
    // что у факта одно имя. Здесь она разбирается ОДИН раз и дальше живёт
    // двумя колонками.
    let ops = client
        .query(
            "SELECT name, detail FROM code_fact
              WHERE project_id = $1 AND kind = 'requirement-op'",
            &[&project],
        )
        .await?;
    let mut req_op = Vec::new();
    for r in &ops {
        let name: String = r.get(0);
        let operation: String = r.get(1);
        let Some((requirement, _)) = name.split_once(" · ") else { continue };
        req_op.push((requirement.to_owned(), operation));
    }

    // Написанная проверка: имя `TC` из строки комментария кода.
    let written = client
        .query(
            "SELECT detail FROM code_fact WHERE project_id = $1 AND kind = 'written-tc'",
            &[&project],
        )
        .await?;
    let mut checks: Vec<String> = Vec::new();
    for r in &written {
        let line: String = r.get(0);
        for m in CHECK_ID.find_iter(&line) {
            checks.push(m.as_str().to_owned());
        }
    }
    checks.sort();
    checks.dedup();

    // Каталоги дерева кода: из фактов о файлах. Каталог — то, в чём лежит хотя
    // бы один файл; пустого каталога дерево не знает.
    let files = client
        .query(
            "SELECT name FROM code_fact WHERE project_id = $1 AND kind = 'code-file'",
            &[&project],
        )
        .await?;
    let more = client
        .query(
            "SELECT name FROM code_fact WHERE project_id = $1 AND kind = 'repo-file'",
            &[&project],
        )
        .await?;
    let mut dirs: Vec<String> = files
        .iter()
        .chain(more.iter())
        .filter_map(|r| {
            let name: String = r.get(0);
            name.rfind('/').map(|i| name[..i].to_owned())
        })
        .collect();
    dirs.sort();
    dirs.dedup();

    // Разбор имени файла: каталог, имя, приставка. Приставка — часть до первого
    // подчёркивания, если она не короче четырёх знаков и не предлог: `to_task`
    // и `from_monitor` общи направлением, а не предметом.
    let not_a_subject = terms.all("word.not-a-subject");
    let mut code_files: Vec<(String, String, String, String)> = Vec::new();
    for r in &files {
        let path: String = r.get(0);
        let Some(i) = path.rfind('/') else { continue };
        let (dir, base) = (path[..i].to_owned(), path[i + 1..].to_owned());
        let prefix = match base.find('_') {
            Some(j) if j >= 4 && !not_a_subject.iter().any(|w| w == &base[..j]) => base[..j].to_owned(),
            _ => String::new(),
        };
        code_files.push((path, dir, base, prefix));
    }

    // Задача ↔ операция: поле «Операции контракта».
    let said = if let Some(term) = terms.one("field.task-contract-ops") { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = $2
              WHERE t.project_id = $1",
            &[&project, &term],
        )
        .await? } else { Vec::new() };
    let op_re = Lazy::new(|| {
        Regex::new(r"(?i)\b(GET|POST|PUT|PATCH|DELETE)\s+(/\S+)").expect("образец операции")
    });
    let mut task_op = Vec::new();
    for r in &said {
        let task: String = r.get(0);
        let value: String = r.get(1);
        for c in op_re.captures_iter(&value) {
            task_op.push((task.clone(), format!("{} {}", c[1].to_uppercase(), &c[2])));
        }
    }

    // Задача ↔ проверка: доказательство обычной задачи и перечень красной.
    let proof = if let Some(term) = terms.one("section.proof") { client
        .query(
            "SELECT t.id, c.value FROM project_plan_tasks t
               JOIN project_document_sections s
                 ON s.project_id = t.project_id AND s.entity_kind = t.entity_kind
                    AND s.entity_name = t.entity_name AND s.title = $2
               JOIN project_document_cells c
                 ON c.project_id = t.project_id AND c.entity_kind = t.entity_kind
                    AND c.entity_name = t.entity_name AND c.col = 0
                    AND c.block_ord BETWEEN s.first_block AND s.last_block
              WHERE t.project_id = $1 AND t.kind = 'dev'",
            &[&project, &term],
        )
        .await? } else { Vec::new() };
    let mut task_check: Vec<(String, String, &str)> = Vec::new();
    for r in &proof {
        let task: String = r.get(0);
        let value: String = r.get(1);
        for m in CHECK_ID.find_iter(&value) {
            task_check.push((task.clone(), m.as_str().to_owned(), "доказательство"));
        }
    }
    let red = if let Some(term) = terms.one("field.red-checks") { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = $2
              WHERE t.project_id = $1 AND t.kind = 'red'",
            &[&project, &term],
        )
        .await? } else { Vec::new() };
    for r in &red {
        let task: String = r.get(0);
        let value: String = r.get(1);
        for m in CHECK_ID.find_iter(&value) {
            task_check.push((task.clone(), m.as_str().to_owned(), "красная фаза"));
        }
    }

    // Родитель красной задачи: имя из поля «Родительская задача», сведённое к
    // существующей задаче. Не первые два знака имени — их разбирают глазами.
    let parents = if let Some(term) = terms.one("field.red-parent") { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = $2
              WHERE t.project_id = $1 AND t.kind = 'red'",
            &[&project, &term],
        )
        .await? } else { Vec::new() };
    let task_id = Lazy::new(|| Regex::new(r"M[0-9]+-T[0-9]+[a-z]?").expect("образец задачи"));
    let mut parent_of: Vec<(String, String)> = Vec::new();
    for r in &parents {
        let task: String = r.get(0);
        let value: String = r.get(1);
        if let Some(m) = task_id.find(&value) {
            parent_of.push((task, m.as_str().to_owned()));
        }
    }

    // Номер задачи внутри этапа: разбирается ЗДЕСЬ, один раз.
    let numbered = client
        .query("SELECT id FROM project_plan_tasks WHERE project_id = $1", &[&project])
        .await?;
    let num_re = Lazy::new(|| Regex::new(r"T([0-9]+)").expect("образец номера"));
    let mut numbers: Vec<(String, i32)> = Vec::new();
    for r in &numbered {
        let id: String = r.get(0);
        if let Some(c) = num_re.captures(&id) {
            if let Ok(n) = c[1].parse::<i32>() {
                numbers.push((id, n));
            }
        }
    }

    // Описанное вперёд: путь листа или любой его предок объявлен документом
    // дерева как отсутствующий. Сравнение предков — работа проекции: запрос
    // соединяет колонку равенством и о путях ничего не знает.
    let ahead: std::collections::HashSet<String> = client
        .query(
            "SELECT name FROM code_fact
              WHERE project_id = $1 AND kind = 'tree-file' AND detail LIKE 'объявлено нет%'",
            &[&project],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    let leaves = client
        .query(
            "SELECT task_id, ord, target_dir FROM project_task_tree_leaf WHERE project_id = $1",
            &[&project],
        )
        .await?;
    let mut forward: Vec<(String, i32)> = Vec::new();
    for r in &leaves {
        let dir: String = r.get(2);
        if dir.is_empty() {
            continue;
        }
        // Вверх — по предкам: объявлено `mobile`, значит и `mobile/src` описан
        // вперёд. Вниз — по потомкам: объявлено `mobile/src/app`, значит и сам
        // `mobile` ещё не заведён, иначе документ не называл бы вложенное
        // отсутствующим.
        let под = ahead.iter().any(|a| a.starts_with(&format!("{dir}/")));
        let mut at = dir.as_str();
        let mut hit = под;
        while !hit {
            if ahead.contains(at) {
                hit = true;
                break;
            }
            match at.rfind('/') {
                Some(i) => at = &at[..i],
                None => break,
            }
        }
        if hit {
            forward.push((r.get(0), r.get(1)));
        }
    }

    // Крейты репозитория и тестовые функции в них. Имя крейта — каталог, где
    // лежит его манифест; разбор пути делается здесь, один раз.
    let manifests = client
        .query(
            "SELECT name FROM code_fact WHERE project_id = $1 AND kind = 'crate-manifest'",
            &[&project],
        )
        .await?;
    let mut crates: std::collections::BTreeMap<String, i32> = Default::default();
    for r in &manifests {
        let path: String = r.get(0);
        if let Some((dir, _)) = path.rsplit_once('/') {
            if let Some(name) = dir.rsplit('/').next() {
                crates.entry(name.to_owned()).or_insert(0);
            }
        }
    }
    let tests = client
        .query(
            "SELECT name FROM code_fact WHERE project_id = $1 AND kind = 'test-fn'",
            &[&project],
        )
        .await?;
    for r in &tests {
        let at: String = r.get(0);
        // Имя факта — путь и номер строки; крейт стоит третьим отрезком пути.
        let path = at.rsplit_once(':').map(|(p, _)| p).unwrap_or(&at);
        let mut parts = path.split('/');
        let (_, home, name) = (parts.next(), parts.next(), parts.next());
        if let (Some(home), Some(name)) = (home, name) {
            if terms.all("path.crate-home").iter().any(|h| h == home) {
                *crates.entry(name.to_owned()).or_insert(0) += 1;
            }
        }
    }


    let mut client = pool.get().await.expect("пул отдал соединение");
    let tx = client.transaction().await?;
    for (table, _) in [
        ("project_requirement_op", ()),
        ("project_task_operation", ()),
        ("project_written_check", ()),
        ("project_task_check", ()),
        ("project_code_dir", ()),
        ("project_code_file", ()),
    ] {
        tx.execute(&format!("DELETE FROM {table} WHERE project_id = $1"), &[&project]).await?;
    }
    for (r, o) in &req_op {
        tx.execute("INSERT INTO project_requirement_op(project_id, requirement_id, operation)
                    VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, r, o]).await?;
    }
    for (t, o) in &task_op {
        tx.execute("INSERT INTO project_task_operation(project_id, task_id, operation)
                    VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[&project, t, o]).await?;
    }
    for c in &checks {
        tx.execute("INSERT INTO project_written_check(project_id, check_id) VALUES ($1,$2)
                    ON CONFLICT DO NOTHING", &[&project, c]).await?;
    }
    for (t, c, how) in &task_check {
        tx.execute("INSERT INTO project_task_check(project_id, task_id, check_id, said_as)
                    VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
                   &[&project, t, c, &how.to_string()]).await?;
    }
    for (path, dir, base, prefix) in &code_files {
        tx.execute("INSERT INTO project_code_file(project_id, path, dir, base, prefix)
                    VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
                   &[&project, path, dir, base, prefix]).await?;
    }
    for d in &dirs {
        tx.execute("INSERT INTO project_code_dir(project_id, dir) VALUES ($1,$2)
                    ON CONFLICT DO NOTHING", &[&project, d]).await?;
    }
    // Ключ фазы: номер из подписи строки процесса, сведённый к самой фазе.
    // Дальше правила соединяют фазу равенством и о подписи ничего не знают.
    tx.execute("UPDATE project_crate SET in_repo = false, test_functions = 0 WHERE project_id = $1",
               &[&project]).await?;
    for (name, n) in &crates {
        tx.execute(
            "INSERT INTO project_crate (project_id, name, does, does_not, in_repo, test_functions)
             VALUES ($1,$2,'','',true,$3)
             ON CONFLICT (project_id, name) DO UPDATE SET in_repo = true,
               test_functions = EXCLUDED.test_functions",
            &[&project, name, n],
        )
        .await?;
    }
    tx.execute(
        "UPDATE project_phase_artifact a SET phase_id = p.id
           FROM phase p
          WHERE a.project_id = $1
            AND a.phase ~ ('(^|[^0-9])' || p.ord::text || '([^0-9]|$)')
            AND a.phase ILIKE 'Фаза%'",
        &[&project],
    )
    .await?;
    tx.execute("UPDATE project_task_tree_leaf SET forward_declared = false WHERE project_id = $1",
               &[&project]).await?;
    for (task, ord) in &forward {
        tx.execute("UPDATE project_task_tree_leaf SET forward_declared = true
                     WHERE project_id = $1 AND task_id = $2 AND ord = $3",
                   &[&project, task, ord]).await?;
    }
    for (id, n) in &numbers {
        tx.execute("UPDATE project_plan_tasks SET number = $3 WHERE project_id = $1 AND id = $2",
                   &[&project, id, n]).await?;
    }
    for (task, parent) in &parent_of {
        tx.execute("UPDATE project_plan_tasks SET parent_task_id = $3
                     WHERE project_id = $1 AND id = $2", &[&project, task, parent]).await?;
    }
    tx.commit().await?;
    if !missing.is_empty() {
        // Слово, которого нет, называется вслух. Связь, для которой его нет, не
        // считается вовсе — и это видно числом, а не догадкой по пустой таблице.
        eprintln!("связи: словарь схемы неполон, не объявлено: {}", missing.join(" · "));
    }
    Ok(req_op.len() + task_op.len() + checks.len() + task_check.len() + dirs.len())
}
