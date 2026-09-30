//! Связи, вынутые из фактов датчиков и полей документов.
//!
//! Каждая живёт своей таблицей и соединяется РАВЕНСТВОМ. Иначе связь пришлось
//! бы искать подстрокой в чужом тексте — а такая связь рвётся от правки прозы
//! и чинится молча: правило перестаёт находить и выглядит зелёным.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;

/// Образец имени проверки ПО УМОЛЧАНИЮ — когда набор своего не объявил.
///
/// Зашитый один на всех: `TC-[A-Z]+-[0-9]+[a-z]?`. У `tot-ade` проверок с таким
/// именем НОЛЬ из 191 — там их зовут именем тестовой функции Rust, ключом
/// сценария приёмки (`S1-AC-4`) и именем гейта (`criterion:agent-write-denied`).
/// Поэтому `project_task_check` пуст, и оба пункта, которые его читают, зелены
/// на пустоте либо красны на молчании.
///
/// Роль `id.check` в словаре набора; объявленных образцов может быть несколько.
static CHECK_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"TC-[A-Z]+-[0-9]+[a-z]?").expect("образец проверки"));

/// Образцы имени проверки этого набора: объявленные либо один зашитый.
pub(crate) fn check_ids(terms: &crate::scheme::Terms) -> Vec<Regex> {
    let own: Vec<Regex> = terms
        .all("id.check")
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();
    if own.is_empty() { vec![CHECK_ID.clone()] } else { own }
}

/// Имена проверок в строке — по всем объявленным образцам — и слово, которое
/// их сняло.
///
/// Снятое имя не считается: зачёркнутое `~~`имя`~~` помянуто как убранное, и
/// связь по нему поехала бы. Словарь уже знает `word.caveat`, и разбор проверок
/// обязан его спрашивать.
///
/// НО СЛОВО ОГОВОРКИ ВОЗВРАЩАЕТСЯ ВМЕСТЕ С ИМЕНАМИ, а не глотает их молча. Слова
/// — обычные основы («отвергнут», «прежн», «было»), и ими же описывают
/// поведение продукта. На `tot-ade` 30.09 ячейка доказательства M5-T146
/// «…запись через неё в закрытое отвергнута» потеряла имя проверки, и
/// `red-checks-match-parent` четыре круга правок называл его «лишним» у
/// красной пары: пункт видел следствие, причины не видел никто. Снятое имя
/// обязано дойти до автора вместе со словом, которое его сняло.
fn checks_in<'a>(line: &str, res: &[Regex], caveats: &'a [String]) -> (Vec<String>, Option<&'a str>) {
    if line.contains("~~") {
        return (Vec::new(), None);
    }
    let names = res.iter().flat_map(|re| re.find_iter(line).map(|m| m.as_str().to_owned())).collect();
    let low = line.to_lowercase();
    (names, caveats.iter().find(|c| low.contains(&c.to_lowercase())).map(String::as_str))
}

pub(crate) const FACT_KINDS: [&str; 7] =
    ["code-file", "crate-manifest", "repo-file", "requirement-op", "test-fn", "tree-file", "written-tc"];

pub(crate) async fn project(pool: &Pool, project: &str) -> Result<usize, crate::db::Fail> {
    let mut connection = crate::db::conn(pool).await?;
    let tx = connection.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('relations:' || $1))", &[&project]).await?;
    let client = &tx;
    // Слова схемы — из словаря, не из кода. Роль без слова НЕ подставляет
    // пустое: пустое совпало бы со всем подряд. Такая связь просто не
    // считается, и её отсутствие видно перечнем ниже.
    let terms = crate::scheme::Terms::load_at(client, project).await?;
    let check_res = check_ids(&terms);
    // СЛОВО ОТКАЗА СНИМАЕТ ИМЯ ТАК ЖЕ, КАК СЛОВО ОГОВОРКИ.
    //
    // `word.caveat` снимает снятое — «удалён», «отвергнут». `word.elsewhere`
    // снимает переадресованное — «здесь не закрывается: закрывает `Y`». Для
    // связи это одно и то же: строка называет имя, ничего им не обещая, и
    // считать её объявлением значит объявить сделанным чужое.
    //
    // Датчик `written-tc` берёт ЛЮБУЮ строку комментария с ключом `TC-…`.
    // Значит строка «сценарий `TC-X` здесь не закрывается» объявляла бы
    // сценарий написанным — и гасила бы находку о нём. Замер 21.09: таких
    // строк в наборе ноль, то есть сторож ничего не двигает сегодня и
    // закрывает класс наперёд. Поставлен он потому, что правка, которая такую
    // строку заводит, уже написана и лежит в очереди на слияние.
    let mut caveats: Vec<String> = terms.all("word.caveat").to_vec();
    caveats.extend(terms.all("word.elsewhere").iter().cloned());
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
    // Снятые словом: задача, имя, откуда названо, слово.
    let mut dropped: Vec<(String, String, &str, String)> = Vec::new();
    for r in &written {
        let line: String = r.get(0);
        match checks_in(&line, &check_res, &caveats) {
            (names, None) => checks.extend(names),
            (names, Some(w)) => dropped.extend(names.into_iter().map(|n| (String::new(), n, "написана", w.to_owned()))),
        }
    }
    checks.sort();
    checks.dedup();

    // Каталоги дерева кода: из фактов о файлах. Каталог — то, в чём лежит хотя
    // бы один файл, прямо или глубже; пустого каталога дерево не знает. Без
    // «глубже» `crates` не был каталогом — в нём только крейты, — и лист
    // `! crates/tot-mcp/`, лежащий в `crates`, читался бы лежащим нигде.
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
        .flat_map(|r| ancestors(&r.get::<_, String>(0)))
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
    let said = if !terms.all("field.task-contract-ops").is_empty() { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = ANY($2)
              WHERE t.project_id = $1",
            &[&project, &terms.all("field.task-contract-ops")],
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
    // СЛОВ У РОЛИ БЫВАЕТ НЕСКОЛЬКО, и здесь их спрашивают все.
    //
    // `terms.one` отдаёт слово только когда оно единственное: набор, где обычная
    // и красная задача зовут раздел доказательства по-разному, объявить оба не
    // мог — второе объявление делало роль двусмысленной, и связь «задача ↔
    // проверка» переставала выводиться вовсе. Соседний `id.check` читается через
    // `all` ровно поэтому, и это верная форма для обоих.
    let proof = if !terms.all("section.proof").is_empty() { client
        .query(
            "SELECT t.id, c.value FROM project_plan_tasks t
               JOIN project_document_sections s
                 ON s.project_id = t.project_id AND s.entity_kind = t.entity_kind
                    AND s.entity_name = t.entity_name AND s.title = ANY($2)
               JOIN project_document_cells c
                 ON c.project_id = t.project_id AND c.entity_kind = t.entity_kind
                    AND c.entity_name = t.entity_name AND c.col = 0
                    AND c.block_ord BETWEEN s.first_block AND s.last_block
              WHERE t.project_id = $1 AND t.kind = 'dev'",
            &[&project, &terms.all("section.proof")],
        )
        .await? } else { Vec::new() };
    let mut task_check: Vec<(String, String, &str)> = Vec::new();
    for r in &proof {
        let task: String = r.get(0);
        let value: String = r.get(1);
        match checks_in(&value, &check_res, &caveats) {
            (names, None) => task_check.extend(names.into_iter().map(|n| (task.clone(), n, "доказательство"))),
            (names, Some(w)) => dropped.extend(names.into_iter().map(|n| (task.clone(), n, "доказательство", w.to_owned()))),
        }
    }
    let red = if !terms.all("field.red-checks").is_empty() { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = ANY($2)
              WHERE t.project_id = $1 AND t.kind = 'red'",
            &[&project, &terms.all("field.red-checks")],
        )
        .await? } else { Vec::new() };
    for r in &red {
        let task: String = r.get(0);
        let value: String = r.get(1);
        match checks_in(&value, &check_res, &caveats) {
            (names, None) => task_check.extend(names.into_iter().map(|n| (task.clone(), n, "красная фаза"))),
            (names, Some(w)) => dropped.extend(names.into_iter().map(|n| (task.clone(), n, "красная фаза", w.to_owned()))),
        }
    }
    // Снятым считается только имя, которого не засчитала НИ ОДНА строка того же
    // места. Замер 30.09 на `tot-ade`: тринадцать строк комментариев сняты
    // словом, и у всех тринадцати имя засчитано соседней строкой — назови их
    // все, и автор получил бы тринадцать ложных тревог.
    dropped.retain(|(task, check, how, _)| {
        if task.is_empty() {
            !checks.contains(check)
        } else {
            !task_check.iter().any(|(t, c, h)| t == task && c == check && h == how)
        }
    });

    // Родитель красной задачи: имя из поля «Родительская задача», сведённое к
    // существующей задаче. Не первые два знака имени — их разбирают глазами.
    // РОЛЬ С ДВУМЯ СЛОВАМИ — ЗДЕСЬ ПРАВИЛО, А НЕ ИСКЛЮЧЕНИЕ. У `field.red-parent`
    // на `myack` объявлены и «Пара», и «Родительская задача»; `one` про такую
    // роль честно отвечает `None`, и весь этот путь молчал. Колонка
    // `parent_task_id` при этом выглядела заполненной — значения остались от
    // времени, когда слово было одно, и ни одна правка их больше не трогала.
    // Первая же новая красная задача получила бы пустого родителя, а `red_task`
    // (`WHERE parent_task_id <> ''`) выронил бы её молча.
    let parents = if !terms.all("field.red-parent").is_empty() { client
        .query(
            "SELECT t.id, f.value FROM project_plan_tasks t
               JOIN project_document_fields f
                 ON f.project_id = t.project_id AND f.entity_kind = t.entity_kind
                    AND f.entity_name = t.entity_name AND f.name = ANY($2)
              WHERE t.project_id = $1 AND t.kind = 'red'",
            &[&project, &terms.all("field.red-parent")],
        )
        .await? } else { Vec::new() };
    // ОБРАЗЕЦ — ИЗ РАСКЛАДКИ, а не зашитый. Зашитый знал только `M`, и у
    // `tot-ade` восемьдесят две задачи из ста шестидесяти четырёх, названные на
    // `V`, для этого обхода не существовали: у красной задачи не находился
    // родитель, и `red_task` роняла её молча.
    //
    // Образец вида — якорный (`^…$`), он для сверки имени целиком; здесь имя
    // ищут внутри строки, поэтому якоря снимаются, а границы слова ставятся.
    let patterns_tasks = crate::scheme::id_pattern(client, project, "task").await?;
    let task_id: Vec<Regex> = patterns_tasks
        .iter()
        .filter_map(|p| Regex::new(&format!(r"\b(?:{})\b", p.trim_start_matches('^').trim_end_matches('$'))).ok())
        .collect();
    let mut parent_of: Vec<(String, String)> = Vec::new();
    for r in &parents {
        let task: String = r.get(0);
        let value: String = r.get(1);
        if let Some(m) = task_id.iter().find_map(|r| r.find(&value)) {
            parent_of.push((task, m.as_str().to_owned()));
        }
    }

    // Описанное вперёд: путь листа или любой его предок объявлен документом
    // дерева как отсутствующий. Сравнение предков — работа проекции: запрос
    // соединяет колонку равенством и о путях ничего не знает.
    //
    // ЧИТАЕТСЯ ОБЪЯВЛЕНИЕ, А НЕ ЕГО СНИМОК. Прежде здесь стоял `code_fact` рода
    // `tree-file` — пересказ этой же таблицы, снятый клиентом в момент прогона
    // датчика. Набор заводил строку, документ её отдавал, а правило
    // `task-path-exists` держало красноту, пока кто-нибудь не позовёт датчик
    // заново: снимок отставал на правку и держал строку, которую уже заменили.
    // Набору при этом говорилось, что такого пути среди прочитанного нет, —
    // правда про замер и ложь про причину. Снимок остаётся: им
    // `tree-matches-disk` сверяет объявление с диском, и это его дело, а не
    // это.
    //
    // Косая черта на конце снимается здесь: в дереве каталог пишут с нею, а
    // лист несёт путь без неё. Прежде совпадение вытягивал шаг «вниз по
    // потомкам» — путь со слешем выглядел потомком самого себя, — и работало
    // это по случайности.
    let ahead: std::collections::HashSet<String> = crate::projector::declared_tree(client, project)
        .await?
        .into_iter()
        .filter(|(_, state)| state.to_lowercase().starts_with("нет"))
        .map(|(path, _)| path.trim_end_matches('/').to_owned())
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
        let under = ahead.iter().any(|a| a.starts_with(&format!("{dir}/")));
        let mut at = dir.as_str();
        let mut hit = under;
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


    for (table, _) in [
        ("project_requirement_op", ()),
        ("project_task_operation", ()),
        ("project_written_check", ()),
        ("project_task_check", ()),
        ("project_check_dropped", ()),
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
    for (t, c, how, word) in &dropped {
        tx.execute("INSERT INTO project_check_dropped(project_id, task_id, check_id, said_as, word)
                    VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
                   &[&project, t, c, &how.to_string(), word]).await?;
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

/// Каталоги, в которых лежит файл: `a/b/c.rs` лежит в `a/b` и в `a`.
fn ancestors(file: &str) -> Vec<String> {
    file.match_indices('/').map(|(i, _)| file[..i].to_owned()).filter(|d| !d.is_empty()).collect()
}

#[cfg(test)]
mod checks_in {
    use regex::Regex;

    /// Строка ячейки M5-T146 с `tot-ade`: имя снято словом из описания
    /// поведения. Имя обязано вернуться вместе со словом, а не пропасть.
    #[test]
    fn a_caveat_word_returns_the_name_with_the_word() {
        let res = [Regex::new(r"\b[mv][0-9]+_t[0-9]+[a-z]?_[a-z0-9_]+\b").expect("образец")];
        let caveats = ["прежн".to_owned(), "отвергнут".to_owned()];
        let line = "m5_t146_a_symlink_gets_no_rule_of_its_own — запись через неё в закрытое отвергнута";
        assert_eq!(super::checks_in(line, &res, &caveats),
                   (vec!["m5_t146_a_symlink_gets_no_rule_of_its_own".to_owned()], Some("отвергнут")));
        assert_eq!(super::checks_in("m5_t146_b_plain — ссылка правила не получает", &res, &caveats),
                   (vec!["m5_t146_b_plain".to_owned()], None));
        assert_eq!(super::checks_in("~~m5_t146_c_gone~~", &res, &caveats), (Vec::<String>::new(), None),
                   "зачёркнутое имя снято самим автором и не называется");
    }
}

/// Имя, снятое словом оговорки, видно пункту `red-checks-match-parent` вместе
/// со словом. Прежде оно просто отсутствовало, и пункт называл его «лишним» у
/// красной пары, не говоря почему.
#[cfg(test)]
mod dropped_check {
    #[tokio::test]
    #[ignore = "нужна пустая база Postgres: MH_TEST_DB_URL"]
    async fn a_caveat_dropped_name_surfaces_with_its_word() {
        let url = std::env::var("MH_TEST_DB_URL").expect("MH_TEST_DB_URL: адрес пустой базы");
        let apart = format!("{}{}", if url.contains('?') { '&' } else { '?' },
                            "options=-c%20search_path%3Ddropped_check");
        let pool = crate::db::pool(&format!("{url}{apart}"), 2).expect("пул тестовой базы");
        pool.get().await.expect("соединение")
            .batch_execute("DROP SCHEMA IF EXISTS dropped_check CASCADE; CREATE SCHEMA dropped_check;")
            .await.expect("своя схема заводится");
        crate::projector::ensure(&pool).await.expect("схема встаёт на пустой базе");
        let client = pool.get().await.expect("соединение");
        // Родитель T-1 называет две проверки в доказательстве, одну — со словом
        // «отвергнута». Красная R-1 перечисляет обе. Вторая строка T-1 называет
        // `m1_t1_b_kept` снова со словом «было»: имя засчитано соседней
        // строкой, и называть его снятым было бы ложью.
        client
            .batch_execute(
                "INSERT INTO scheme_term (project_id, role, value) VALUES
                   ('p', 'id.check', '\\b[mv][0-9]+_t[0-9]+[a-z]?_[a-z0-9_]+\\b'),
                   ('p', 'word.caveat', 'отвергнут'), ('p', 'word.caveat', 'было'),
                   ('p', 'section.proof', 'Чем доказывается'), ('p', 'field.red-checks', 'Какие');
                 INSERT INTO project_plan_versions (project_id, id) VALUES ('p', 'v1');
                 INSERT INTO project_plan_milestones (project_id, id, version_id, ord, title)
                 VALUES ('p', 'M1', 'v1', 1, 'веха');
                 INSERT INTO project_plan_tasks (project_id, id, milestone_id, ord, title, size, state, kind,
                                                 entity_kind, entity_name, parent_task_id)
                 VALUES ('p', 'T-1', 'M1', 1, 'задача', '', 'not_started', 'dev', 'task', 'T-1', ''),
                        ('p', 'R-1', 'M1', 2, 'зеркало', '', 'not_started', 'red', 'task', 'R-1', 'T-1');
                 INSERT INTO project_documents (project_id, entity_kind, entity_name, content, content_hash,
                                                bytes, revision, updated_at, updated_by)
                 VALUES ('p', 'task', 'T-1', '', '', 0, 1, 0, 't'), ('p', 'task', 'R-1', '', '', 0, 1, 0, 't');
                 INSERT INTO project_document_sections (project_id, ord, level, title, anchor,
                                                        first_block, last_block, entity_kind, entity_name)
                 VALUES ('p', 1, 2, 'Чем доказывается', 'a', 1, 3, 'task', 'T-1');
                 INSERT INTO project_document_cells (project_id, block_ord, row_ord, col, raw, value,
                                                     entity_kind, entity_name)
                 VALUES ('p', 2, 1, 0, '', 'm1_t1_a_denied — запись в закрытое отвергнута', 'task', 'T-1'),
                        ('p', 2, 2, 0, '', 'm1_t1_b_kept — ответ тот же', 'task', 'T-1'),
                        ('p', 2, 3, 0, '', 'm1_t1_b_kept — как было', 'task', 'T-1');
                 INSERT INTO project_document_fields (project_id, section_ord, ord, name, shape,
                                                      value_raw, value, entity_kind, entity_name)
                 VALUES ('p', 1, 1, 'Какие', 'row', '', 'm1_t1_a_denied, m1_t1_b_kept', 'task', 'R-1');",
            )
            .await
            .expect("набор подсаживается");
        super::project(&pool, "p").await.expect("связи пересобираются");

        let gate = include_str!("../../../instrument/gate/G3/red-checks-match-parent.sql");
        let v = crate::projector::execute_method_upto(&client, "p", "query", gate, 200, 0).await;
        assert_eq!(v.detail, vec!["R-1 — разошёлся с родителем T-1: не хватает —, лишние m1_t1_a_denied; \
                                   T-1: `m1_t1_a_denied` не засчитана: в строке слово «отвергнут»".to_owned()],
                   "пункт называет снятое имя и слово, которое его сняло ({})", v.why);
        let seen = crate::projector::links_of(&pool, "p", "task", "T-1").await.expect("связи задачи");
        let checks = seen["sets"]["checks"].to_string();
        assert!(checks.contains("не засчитана: в строке слово «отвергнут»") && !checks.contains("«было»"),
                "в проверках задачи снятое имя стоит со словом, засчитанное соседней строкой — нет: {checks}");
    }
}

#[cfg(test)]
mod code_dirs {
    #[test]
    fn a_file_lies_in_every_directory_above_it() {
        assert_eq!(super::ancestors("crates/tot-mcp/src/lib.rs"), vec!["crates", "crates/tot-mcp", "crates/tot-mcp/src"]);
        assert!(super::ancestors("Cargo.toml").is_empty());
    }
}
