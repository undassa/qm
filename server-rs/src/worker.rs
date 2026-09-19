//! Воркер пульта: кто-то должен отвечать в беседах набора и вести прогоны
//! задач, иначе строка человека висит непрочитанной, а пульт показывает
//! разговор с пустотой.
//!
//! Прогон ведётся автоматом задачи (дверь `run-automaton`): план → ревью
//! плана → код → ревью → круги → закрытие. Состояние автомата — в Postgres,
//! поэтому переживает перезапуск воркера; этот файл — оркестрация: кому какой
//! промпт, какую сессию позвать, что сравнить после хода. Правила не здесь,
//! а в `projector::automaton_advance`.
//!
//! Останавливается прогон на четырёх вещах: вопросе владельцу, покрасневшем
//! гейте, правке прибора и остановке автомата. Закрытым он становится только
//! закрытой по конвейеру задаче. Молчание сессии — не итог: такой прогон
//! ждёт, а после третьего молчания подряд падает. Коммит и пуш кода он не
//! делает сам — просит подтверждение.

use deadpool_postgres::Pool;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

// Ключ --allowedTools забирает ВСЕ следующие слова, поэтому список идёт одним
// словом через запятую, а сама просьба — стандартным вводом.
const CHAT_TOOLS: &str = "Read,Grep,Glob,Bash(mh:*),Bash(git log:*),Bash(git diff:*),Bash(git status:*),Bash(ls:*),Bash(rg:*)";
const WORK_TOOLS: &str = "Read,Grep,Glob,Edit,Write,MultiEdit,TodoWrite,Bash";
const VETO: &str = "Bash(git commit:*),Bash(git push:*)";
// Прибор правится решением владельца, не прогоном — даже с одобренным коммитом:
// ослабленная проверка выглядит снаружи как зеленый гейт, и «чини причину, а не
// правило» превращается в «чини правило». Запрет инструментов — первый рубеж,
// сверка диффа после хода — второй: она ловит правку, сделанную обходным Bash.
const GUARD_TOOLS: &str = "Edit(instrument/**),Write(instrument/**),MultiEdit(instrument/**)";
const INSTRUMENT_PREFIXES: &[&str] = &["instrument/"];
pub const TICK_S: u64 = 5;
const LIMIT_S: u64 = 1800;
const SILENCE_LIMIT: i32 = 3;
const CIRCLES: i32 = 5; // потолок кругов починки — правило харнеса

static MARKER: Lazy<Regex> = Lazy::new(|| {
    // Маркеры контракта дословны (заглавными). Порядок — по тяжести: DRIFT
    // встаёт всегда, NEEDS FIX важнее случайного слова PLAN в прозе.
    Regex::new(r"\b(DRIFT|NEEDS FIX|RETHINK|PLAN|BLOCKED|NEEDS CONTEXT|CLOSED)\b").unwrap()
});

#[derive(Clone, Deserialize)]
pub struct Bundle {
    pub name: String,
    pub project: String,
    pub repo: String,
}

#[derive(Deserialize)]
pub struct Config {
    pub projects: Vec<Bundle>,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Work,  // исполнение: сессия исполнителя, правки разрешены
    Watch, // ревью: чистый лист, только читает
}

pub struct Worker {
    pool: Pool,
    kinds: Arc<crate::kinds::Kinds>,
    claude: String,
}

impl Worker {
    pub fn new(pool: Pool, kinds: Arc<crate::kinds::Kinds>, claude: String) -> Self {
        Worker { pool, kinds, claude }
    }

    /// Дверь в-процесса: та же логика, что у `mh call`, но без сети и без
    /// обёртки протокола. Отказ — словом, как везде в харнесе.
    async fn door(&self, project: &str, name: &str, args: Value) -> Result<Value, String> {
        let out = crate::mcp::Mcp {
            pool: self.pool.clone(),
            kinds: self.kinds.clone(),
            project: project.to_owned(),
            author: "worker".into(),
        }
        .call(name, &args)
        .await;
        if out.get("isError").and_then(Value::as_bool).unwrap_or(false) || out.get("error").is_some() {
            let why = out["content"][0]["text"].as_str().unwrap_or("дверь отказала без слов");
            return Err(format!("{name}: {}", &why[..why.len().min(400)]));
        }
        let text = out["content"][0]["text"].as_str().unwrap_or("");
        serde_json::from_str(text)
            .map_err(|e| format!("{name}: ответ не разбирается: {e}: {}", &text[..text.len().min(200)]))
    }

    /// Ответ сессии и флаг молчания: таймаут, отказ, неразобранный или пустой
    /// ответ — молчание. У прогона молчание не бывает итогом.
    async fn ask_session(
        &self, bundle: &Bundle, session: &str, prompt: &str, tools: &str, deny: &str,
    ) -> (String, String, bool) {
        let mut cmd = tokio::process::Command::new(&self.claude);
        cmd.args(["-p", "--output-format", "json", "--allowedTools", tools]);
        if !deny.is_empty() {
            cmd.arg("--disallowedTools").arg(deny);
        }
        if !session.is_empty() {
            cmd.arg("--resume").arg(session);
        }
        cmd.current_dir(&bundle.repo)
            .env("MH_PROJECT", &bundle.project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return (format!("сессия не завелась: {e}"), session.to_owned(), true),
        };
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(prompt.as_bytes()).await.is_err() {
                let _ = child.kill().await;
                return ("сессия не приняла ввод".into(), session.to_owned(), true);
            }
        }
        match tokio::time::timeout(std::time::Duration::from_secs(LIMIT_S), child.wait_with_output()).await {
            Err(_) => (format!("сессия не уложилась в {LIMIT_S} с"), session.to_owned(), true),
            Ok(Err(e)) => (format!("сессия не ответила: {e}"), session.to_owned(), true),
            Ok(Ok(out)) => {
                let text = String::from_utf8_lossy(&out.stdout);
                let err_tail = String::from_utf8_lossy(&out.stderr);
                if !out.status.success() {
                    let tail = format!("{text}{err_tail}");
                    return (format!("сессия не ответила: {}", &tail[..tail.len().min(600)]), session.to_owned(), true);
                }
                match serde_json::from_str::<Value>(&text) {
                    Err(_) => (format!("ответ не разобран: {}", &text[..text.len().min(600)]), session.to_owned(), true),
                    Ok(d) => {
                        let session = d["session_id"].as_str().unwrap_or(session).to_owned();
                        let result = d["result"].as_str().unwrap_or("").trim().to_owned();
                        if result.is_empty() {
                            ("сессия закончилась без последнего слова".into(), session, true)
                        } else {
                            (result, session, false)
                        }
                    }
                }
            }
        }
    }

    pub async fn chats(&self, bundle: &Bundle) {
        let project = &bundle.project;
        let threads = match self.door(project, "chat", json!({})).await {
            Ok(v) => v,
            Err(e) => {
                println!("{0}: {e}", bundle.name);
                return;
            }
        };
        for t in threads["threads"].as_array().cloned().unwrap_or_default() {
            if t["waiting"].as_i64().unwrap_or(0) == 0 {
                continue;
            }
            let thread = t["thread"].as_str().unwrap_or("");
            let inbox = match self.door(project, "chat-inbox", json!({ "thread": thread })).await {
                Ok(v) => v,
                Err(e) => {
                    println!("{0}: {e}", bundle.name);
                    continue;
                }
            };
            let lines: Vec<String> = inbox["said"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|s| s["text"].as_str().map(str::to_owned))
                .collect();
            if lines.is_empty() {
                continue;
            }
            println!("{} · беседа {thread}: {} строк", bundle.name, lines.len());
            let brief = format!(
                "Ты отвечаешь в беседе пульта харнеса про набор «{}». Отвечай по-русски, коротко и по делу. \
                 Двери набора: `mh call <дверь> k=v`. Беседа — это размышление: файлы не правь и не коммить, \
                 а скажи, что нужно сделать.\n\n{}",
                bundle.name,
                lines.join("\n\n")
            );
            let session = inbox["sessionId"].as_str().unwrap_or("");
            let (answer, session, _) = self.ask_session(bundle, session, &brief, CHAT_TOOLS, "").await;
            let _ = self.door(project, "chat-say", json!({ "thread": thread, "text": answer, "side": "agent" })).await;
            if !session.is_empty() {
                let _ = self.door(project, "chat-inbox", json!({ "thread": thread, "sessionId": session })).await;
            }
        }
    }

    fn git(repo: &str, args: &[&str]) -> String {
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// Голова дерева и приборные следы в нём — с чем сверять дерево после хода.
    /// След — путь с временем правки: правка файла, грязного ещё на снимке,
    /// иначе неотличима от тишины.
    fn tree_snapshot(repo: &str) -> (String, HashMap<String, u128>) {
        let mut marks = HashMap::new();
        for line in Self::git(repo, &["status", "--porcelain"]).lines() {
            let path = &line[3.min(line.len())..];
            if INSTRUMENT_PREFIXES.iter().any(|p| path.starts_with(p)) {
                let mtime = std::fs::metadata(std::path::Path::new(repo).join(path))
                    .and_then(|m| m.modified())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis())
                    .unwrap_or(0);
                marks.insert(path.to_owned(), mtime);
            }
        }
        (Self::git(repo, &["rev-parse", "HEAD"]).trim().to_owned(), marks)
    }

    /// Что ход сделал с прибором: и закоммиченное с головы снимка, и ещё не
    /// закоммиченное. Правка вне коммита тоже считается: до одобрения сессия
    /// работает в дереве, не коммитя, и по одним коммитам её правка не видна.
    fn instrument_touched(repo: &str, snapshot: &(String, HashMap<String, u128>)) -> Vec<String> {
        let (head, marks) = snapshot;
        let committed: Vec<String> = if head.is_empty() {
            Vec::new()
        } else {
            Self::git(repo, &["diff", "--name-only", &format!("{head}..HEAD")])
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        };
        let (_, marks_now) = Self::tree_snapshot(repo);
        let instrument = |p: &String| INSTRUMENT_PREFIXES.iter().any(|x| p.starts_with(x));
        let mut touched: Vec<String> = committed.iter().filter(|p| instrument(p)).cloned().collect();
        for (p, m) in &marks_now {
            if marks.get(p) != Some(m) && instrument(p) {
                touched.push(p.clone());
            }
        }
        for p in marks.keys() {
            if !marks_now.contains_key(p) && instrument(p) {
                touched.push(p.clone());
            }
        }
        touched.sort();
        touched.dedup();
        touched
    }

    async fn last_event(&self, project: &str, run_id: &str, kind: &str) -> String {
        if let Ok(v) = self.door(project, "runs", json!({ "limit": 10 })).await {
            for r in v["runs"].as_array().cloned().unwrap_or_default() {
                if r["runId"].as_str() == Some(run_id) {
                    for e in r["events"].as_array().cloned().unwrap_or_default() {
                        if e["kind"].as_str() == Some(kind) {
                            return e["text"].as_str().unwrap_or("").to_owned();
                        }
                    }
                }
            }
        }
        String::new()
    }

    /// Пункты гейта по id. Сверка идёт по пунктам, а не счётом: по счёту не
    /// видно, какое правило сломалось, и вопрос владельцу остаётся без довода.
    async fn gate_items(&self, project: &str) -> Result<HashMap<String, (Option<String>, String)>, String> {
        let gate = self.door(project, "gate", json!({})).await?;
        let mut out = HashMap::new();
        for g in gate["gates"].as_array().cloned().unwrap_or_default() {
            for i in g["items"].as_array().cloned().unwrap_or_default() {
                if let Some(id) = i["id"].as_str() {
                    out.insert(
                        id.to_owned(),
                        (i["computed"].as_str().map(str::to_owned), i["item"].as_str().unwrap_or("").to_owned()),
                    );
                }
            }
        }
        Ok(out)
    }

    /// Лестница статусов задачи и терминальные среди них. Имя терминального
    /// статуса нельзя зашивать в воркер: чужая лестница его бы молча сломала.
    async fn conveyor(&self, project: &str) -> Result<(Vec<String>, Vec<String>), String> {
        let st = self.door(project, "statuses", json!({ "kind": "task" })).await?;
        let ladder: Vec<String> = st["statuses"].as_array().cloned().unwrap_or_default()
            .iter().filter_map(|s| s["name"].as_str().map(str::to_owned)).collect();
        let terminal: Vec<String> = st["statuses"].as_array().cloned().unwrap_or_default()
            .iter().filter(|s| s["terminal"].as_bool().unwrap_or(false))
            .filter_map(|s| s["name"].as_str().map(str::to_owned)).collect();
        Ok((ladder, terminal))
    }

    /// Статус задачи по конвейеру и достигнутые ступени в порядке лестницы.
    async fn task_conveyor(&self, project: &str, task: &str, ladder: &[String]) -> Result<(Value, Vec<String>), String> {
        let st = self.door(project, "task-status", json!({ "id": task })).await?;
        let reached: Vec<String> = ladder
            .iter()
            .filter(|s| st["reached"].as_array().cloned().unwrap_or_default()
                .iter().any(|r| r.as_str() == Some(s.as_str())))
            .cloned()
            .collect();
        Ok((st, reached))
    }

    async fn wave_cards(&self, project: &str) -> Result<HashMap<String, Value>, String> {
        let waves = self.door(project, "waves", json!({})).await?;
        Ok(waves["cards"].as_array().cloned().unwrap_or_default()
            .iter().filter_map(|c| c["id"].as_str().map(|id| (id.to_owned(), c.clone())))
            .collect())
    }

    /// Первая фаза с неспрофилированным гейтом — открытая полоса набора.
    async fn current_phase(&self, project: &str) -> Result<Value, String> {
        let st = self.door(project, "phases", json!({})).await?;
        for p in st["phases"].as_array().cloned().unwrap_or_default() {
            if p["gateState"].as_str() != Some("passed") {
                return Ok(p);
            }
        }
        Ok(json!({}))
    }

    /// DRIFT где угодно — конвейер встал целиком: воркер не ставит ничего,
    /// пока владелец не снимет остановку. Прочие остановки — по одной задаче.
    async fn conveyor_halted(&self, project: &str) -> String {
        if let Ok(v) = self.door(project, "run-automaton", json!({})).await {
            for h in v["halts"].as_array().cloned().unwrap_or_default() {
                if h["drift"].as_bool().unwrap_or(false) {
                    return h["halt"].as_str().unwrap_or("DRIFT").to_owned();
                }
            }
        }
        String::new()
    }

    /// Промпт под шаг автомата, сессия и род работы. Ревью — чистый лист
    /// (читает, не правит), исполнение — прежняя сессия. Промпт None у
    /// закрытого автомата: судит конвейер, сессию звать не надо.
    async fn step_prompt(
        &self, project: &str, bundle: &Bundle, run: &Value, am: &Value, band_name: &str, decisions: &[Value],
    ) -> (Option<String>, String, Stage) {
        let task = run["task"].as_str().unwrap_or("");
        let rid = run["runId"].as_str().unwrap_or("");
        let session = run["sessionId"].as_str().unwrap_or("").to_owned();
        let step = am["step"].as_str().unwrap_or("");
        if !decisions.is_empty() {
            let said = String::from("Решение владельца:\n")
                + &decisions
                    .iter()
                    .map(|d| {
                        format!(
                            "- {} → {}: {}",
                            d["title"].as_str().unwrap_or(""),
                            d["state"].as_str().unwrap_or(""),
                            d["why"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            let tail = if step == "closing" {
                "\n\nОдобрено: коммить с трейлерами закрытия задачи и закончи ответ словом CLOSED."
            } else {
                "\n\nПродолжай прогон с учётом этого."
            };
            return (Some(said + tail), session, Stage::Work);
        }
        let fill = |template: &str, extra: &[(&str, &str)]| {
            let mut s = template
                .replace("{task}", task)
                .replace("{name}", &bundle.name)
                .replace("{run}", rid)
                .replace("{phase}", band_name);
            for (k, v) in extra {
                s = s.replace(k, v);
            }
            s
        };
        match step {
            "planning" => {
                let mut rethink = String::new();
                if am["rethinks"].as_i64().unwrap_or(0) > 0 {
                    let why = self.last_event(project, rid, "находки").await;
                    rethink = format!(
                        "\n\nРевью плана вернуло RETHINK, довод дословно:\n{why}\n\nПерепиши план и верни PLAN снова."
                    );
                }
                (Some(fill(PLANNING, &[("{rethink}", &rethink)])), session, Stage::Work)
            }
            "plan-review" => {
                let plan = self.last_event(project, rid, "план").await;
                (Some(fill(PLAN_REVIEW, &[("{plan}", &plan)])), String::new(), Stage::Watch)
            }
            "implementing" => (Some(fill(IMPLEMENTING, &[])), session, Stage::Work),
            "review" => (Some(fill(REVIEW, &[])), String::new(), Stage::Watch),
            "fixing" => {
                let findings = self.last_event(project, rid, "находки").await;
                (
                    Some(fill(FIXING, &[("{circle}", &am["circle"].as_i64().unwrap_or(0).to_string()), ("{findings}", &findings)])),
                    session,
                    Stage::Work,
                )
            }
            "closing" => (Some(fill(CLOSING, &[])), session, Stage::Work),
            _ => (None, session, Stage::Work),
        }
    }

    pub async fn drive_runs(&self, bundle: &Bundle) {
        let project = &bundle.project;
        let halted = self.conveyor_halted(project).await;
        if !halted.is_empty() {
            println!("{}: конвейер встал: {halted}", bundle.name);
            return;
        }
        let cards = self.wave_cards(project).await.unwrap_or_default();
        let band = self.current_phase(project).await.unwrap_or(json!({}));
        let band_name = match band["phase"].as_str() {
            Some(ph) => format!("{} — {}", ph, band["title"].as_str().unwrap_or("")),
            None => "все гейты пройдены".into(),
        };
        let runs = match self.door(project, "runs", json!({ "limit": 10 })).await {
            Ok(v) => v["runs"].as_array().cloned().unwrap_or_default(),
            Err(e) => {
                println!("{0}: {e}", bundle.name);
                return;
            }
        };
        for run in runs {
            if !matches!(run["state"].as_str(), Some("running") | Some("waiting")) {
                continue;
            }
            let rid = run["runId"].as_str().unwrap_or("");
            let task = run["task"].as_str().unwrap_or("");
            let decisions = self
                .door(project, "ask-inbox", json!({ "runId": rid }))
                .await
                .map(|v| v["decided"].as_array().cloned().unwrap_or_default())
                .unwrap_or_default();
            if run["state"].as_str() == Some("waiting") && decisions.is_empty() {
                continue;
            }
            if decisions.is_empty() {
                // Полоса: фаза задачи не открыта — прогон не ставится. Без этого
                // агент уходит в сторону: работает по будущей фазе, пока текущая
                // красная, и гейт её работы молча не судит. Барьер серверный
                // (`phase_open`): открыты все фазы с пройденными предыдущими
                // гейтами, поэтому строгое `is False` — задача без вида,
                // отображённого на фазу, null-ом не объявляет «можно всё», но и
                // не наказывается: её судит конвейер. Слово владельца решает.
                if let Some(card) = cards.get(task) {
                    if card["phaseOpen"].as_bool() == Some(false) {
                        let note = format!(
                            "полоса закрыта: задача фазы {}, открыта фаза «{band_name}»",
                            card["phase"].as_str().unwrap_or("")
                        );
                        if run["note"].as_str() != Some(&note) {
                            let _ = self
                                .door(project, "run-state",
                                    json!({ "runId": rid, "state": run["state"].as_str().unwrap_or("running"), "note": note, "session": run["sessionId"].as_str().unwrap_or("") }))
                                .await;
                        }
                        continue;
                    }
                }
            }
            let am = match self.door(project, "run-automaton", json!({ "runId": rid })).await {
                Ok(v) => v,
                Err(e) => {
                    println!("{0}: {e}", bundle.name);
                    continue;
                }
            };
            if let Some(halt) = am["halt"].as_str().filter(|h| !h.is_empty()) {
                // Вставший автомат ждёт слова владельца (resume=1), а не круга.
                let note = format!("автомат встал: {halt}");
                if run["note"].as_str() != Some(&note) {
                    let _ = self
                        .door(project, "run-state",
                            json!({ "runId": rid, "state": run["state"].as_str().unwrap_or("running"), "note": note, "session": run["sessionId"].as_str().unwrap_or("") }))
                        .await;
                }
                continue;
            }
            let approved = decisions
                .iter()
                .any(|d| d["kind"].as_str() == Some("approval") && d["state"].as_str() == Some("approved"));
            let (prompt, session, stage) =
                self.step_prompt(project, bundle, &run, &am, &band_name, &decisions).await;
            println!(
                "{} · прогон {task} ({} · {} {}/{CIRCLES})",
                bundle.name,
                run["state"].as_str().unwrap_or(""),
                am["step"].as_str().unwrap_or(""),
                am["circle"].as_i64().unwrap_or(0)
            );
            let was_gate = match self.gate_items(project).await {
                Ok(v) => v,
                Err(e) => {
                    println!("{0}: {e}", bundle.name);
                    continue;
                }
            };
            let (ladder, terminal) = self.conveyor(project).await.unwrap_or_default();
            let snapshot = Self::tree_snapshot(&bundle.repo);
            let deny = [if approved { "" } else { VETO }, GUARD_TOOLS]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(",");
            let tools = if stage == Stage::Work { WORK_TOOLS } else { CHAT_TOOLS };
            let (answer, session, silent) = match &prompt {
                None => (String::new(), session, false),
                Some(p) => self.ask_session(bundle, &session, p, tools, &deny).await,
            };
            // Ревьюерская сессия — чистый лист: в run.session_id её сохранять
            // нельзя, иначе исполнитель продолжится ревьюером.
            let kept_session = if stage == Stage::Work { session } else { run["sessionId"].as_str().unwrap_or("").to_owned() };
            let note_if = |state: &str, note: String| (state.to_owned(), note);
            if silent {
                // Молчание ждёт, а не закрывает: done по обрыву превращал
                // прогон, ничего не сделавший, в закрытый, и задача терялась
                // молча. Число молчаний — по событиям прогона в базе, поэтому
                // переживает перезапуск воркера; окно в последние события
                // занижает счёт — на исход уже не влияет.
                let n = 1 + run["events"].as_array().cloned().unwrap_or_default()
                    .iter().filter(|e| e["kind"].as_str() == Some("молчание")).count() as i32;
                let _ = self.door(project, "run-event",
                    json!({ "runId": rid, "kind": "молчание", "text": &answer[..answer.len().min(4000)] })).await;
                let (state, note) = if n >= SILENCE_LIMIT {
                    note_if("failed", format!("сессия молчит {n} раза подряд: {}", &answer[..answer.len().min(200)]))
                } else {
                    note_if("waiting", format!("сессия молчит, попытка {n}: {}", &answer[..answer.len().min(200)]))
                };
                let _ = self.door(project, "run-state",
                    json!({ "runId": rid, "state": state, "note": note, "session": kept_session })).await;
                return;
            }
            let answer_full = answer.clone();
            let answer = &answer_full[..answer_full.len().min(4000)];
            let _ = self.door(project, "run-event",
                json!({ "runId": rid, "kind": "итог", "text": answer })).await;
            let touched = Self::instrument_touched(&bundle.repo, &snapshot);
            if !touched.is_empty() {
                // Прибор правится решением владельца, не прогоном: ослабленная
                // проверка выглядит как зеленый гейт, и «чини причину, а не
                // правило» превращается в «чини правило».
                let list = touched.join(", ");
                let note = format!("правка прибора из прогона: {list} — нужно решение владельца");
                let _ = self.door(project, "run-event", json!({ "runId": rid, "kind": "отказ", "text": &note })).await;
                let _ = self.door(project, "question-ask", json!({
                    "runId": rid,
                    "title": format!("{task}: правка прибора"),
                    "body": format!("Прогон {rid} задачи {task} правил прибор: {list}.\n\nБез решения с записанным доводом прибор не меняется.") })).await;
                let _ = self.door(project, "run-state",
                    json!({ "runId": rid, "state": "waiting", "note": note, "session": kept_session })).await;
                return;
            }
            let step = am["step"].as_str().unwrap_or("");
            let adv;
            if step == "closed" {
                adv = am.clone(); // автомат закрыт: вердиктов не ждёт, судит конвейер
            } else {
                let verdict = verdict_of(&answer_full);
                if matches!(verdict.as_str(), "BLOCKED" | "NEEDS CONTEXT") {
                    let _ = self.door(project, "question-ask", json!({
                        "runId": rid, "title": format!("{task}: {verdict}"),
                        "body": &answer_full[..answer_full.len().min(2000)] })).await;
                    let _ = self.door(project, "run-state", json!({
                        "runId": rid, "state": "waiting",
                        "note": format!("ждёт владельца: {verdict} — {}", &answer_full[..answer_full.len().min(150)]),
                        "session": kept_session })).await;
                    return;
                }
                if matches!(step, "plan-review" | "review") {
                    // Довод ревью — в событиях: следующий шаг читает его оттуда
                    // и переживает перезапуск воркера.
                    let _ = self.door(project, "run-event",
                        json!({ "runId": rid, "kind": "находки", "text": answer })).await;
                }
                adv = match self.door(project, "run-automaton",
                    json!({ "runId": rid, "status": verdict, "note": &answer_full[..answer_full.len().min(300)] })).await
                {
                    Ok(v) => v,
                    Err(e) => {
                        println!("{0}: {e}", bundle.name);
                        return;
                    }
                };
            }
            if matches!(adv["status"].as_str(), Some("refused") | Some("halted")) {
                let why = adv["why"].as_str().unwrap_or("отказ автомата");
                let _ = self.door(project, "run-event",
                    json!({ "runId": rid, "kind": "отказ", "text": why })).await;
                // Владелец обязан услышать об отказе: иначе прогон ждёт вечно
                // — слово уже потреблено, а в очереди решений пусто.
                let _ = self.door(project, "question-ask", json!({
                    "runId": rid, "title": format!("{task}: автомат отказал"),
                    "body": format!("Прогон {rid} задачи {task}: {why}") })).await;
                let _ = self.door(project, "run-state",
                    json!({ "runId": rid, "state": "waiting", "note": why, "session": kept_session })).await;
                return;
            }
            if let Some(halt) = adv["halt"].as_str().filter(|h| !h.is_empty()) {
                // Круговой потолок и расхождение — остановка до слова владельца;
                // DRIFT — ещё и конвейер целиком: следующий круг встанет на всех.
                let note = format!("автомат встал: {halt}");
                let _ = self.door(project, "run-event",
                    json!({ "runId": rid, "kind": "отказ", "text": &note })).await;
                let _ = self.door(project, "question-ask", json!({
                    "runId": rid, "title": format!("{task}: автомат встал"),
                    "body": format!("Прогон {rid} задачи {task}: {halt}.\n\nСнять остановку может владелец: mh call run-automaton runId=… resume=1") })).await;
                let state = if halt.starts_with("DRIFT:") { "failed" } else { "waiting" };
                let _ = self.door(project, "run-state",
                    json!({ "runId": rid, "state": state, "note": note, "session": kept_session })).await;
                return;
            }
            let open_asks: Vec<Value> = self
                .door(project, "asks", json!({ "state": "open" })).await
                .map(|v| v["asks"].as_array().cloned().unwrap_or_default())
                .unwrap_or_default()
                .into_iter()
                .filter(|a| a["runId"].as_str() == Some(rid))
                .collect();
            let now_gate = self.gate_items(project).await.unwrap_or_default();
            // Покраснение — поимённо: пункт, который был зелёным и перестал,
            // плюс пункт, объявившийся красным посреди прогона.
            let regression = regression(&was_gate, &now_gate);
            let (status_after, now_reached) = self.task_conveyor(project, task, &ladder).await.unwrap_or((json!({}), Vec::new()));
            let terminal_hit = !terminal.is_empty() && terminal.iter().all(|t| now_reached.contains(t));
            let adv_step = adv["step"].as_str().unwrap_or("").to_owned();
            let adv_circle = adv["circle"].as_i64().unwrap_or(0);
            let current = status_after["current"].as_str().unwrap_or("");
            let (state, note) = if !open_asks.is_empty() {
                note_if("waiting", format!("ждёт решения владельца: {}", open_asks[0]["title"].as_str().unwrap_or("")))
            } else if !regression.is_empty() {
                let names: Vec<String> = regression.iter().take(4).map(|i| {
                    let title = now_gate.get(i).map(|(_, t)| t.as_str()).unwrap_or("");
                    format!("{} ({})", i, &title[..title.len().min(60)])
                }).collect();
                let names = names.join("; ");
                let _ = self.door(project, "question-ask", json!({
                    "runId": rid, "title": format!("{task}: гейт покраснел"),
                    "body": format!("Прогон {rid} задачи {task} покраснил пункты: {names}.") })).await;
                note_if("waiting", format!("гейт покраснел: {names}"))
            } else if terminal.is_empty() {
                // Лестницы нет — судить по задаче нечем: закрытие решает автомат.
                println!("{}: лестницы статусов задач нет — закрытие по конвейеру не проверить", bundle.name);
                if adv_step == "closed" {
                    note_if("done", "закрыта по автомату".to_string())
                } else {
                    note_if("running", adv_step.clone())
                }
            } else if status_after["known"].as_bool() == Some(false) {
                // Конвейер задачу не знает: закрытие судит автомат один.
                if adv_step == "closed" {
                    note_if("done", "закрыта по автомату: конвейер задачу не знает".to_string())
                } else {
                    note_if("running", format!("{adv_step} · круг {adv_circle}/{CIRCLES}"))
                }
            } else if terminal_hit && adv_step == "closing" {
                let _ = self.door(project, "run-automaton",
                    json!({ "runId": rid, "status": "CLOSED", "note": "закрыта трейлерами" })).await;
                note_if("done", format!("закрыта: конвейер «{current}», автомат дошёл"))
            } else if terminal_hit && adv_step == "closed" {
                note_if("done", format!("закрыта: конвейер «{current}», автомат закрыт"))
            } else if terminal_hit {
                note_if("running", format!("закрыта по конвейеру, автомат на «{adv_step}» — довожу шаг"))
            } else {
                note_if("running", format!("{adv_step} · круг {adv_circle}/{CIRCLES} · конвейер «{current}»"))
            };
            let _ = self.door(project, "run-state",
                json!({ "runId": rid, "state": state, "note": note, "session": kept_session })).await;
            return; // один прогон за круг: беседа не должна ждать конца задачи
        }
    }
}

/// Вердикт отчёта по маркеру контракта; пустая строка — чистый отчёт.
pub fn verdict_of(answer: &str) -> String {
    MARKER
        .captures(answer)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or_default()
}

/// Пункты, которые были зелёными и перестали, плюс объявившиеся красными.
/// Чистая функция: сверка гейта тестируется без базы.
pub fn regression(
    was: &HashMap<String, (Option<String>, String)>,
    now: &HashMap<String, (Option<String>, String)>,
) -> Vec<String> {
    let mut out: Vec<String> = was
        .iter()
        .filter(|(id, (computed, _))| {
            computed.as_deref() == Some("passed") && now.get(*id).and_then(|(c, _)| c.as_deref()) != Some("passed")
        })
        .map(|(id, _)| id.clone())
        .collect();
    out.extend(
        now.iter()
            .filter(|(id, (computed, _))| {
                !was.contains_key(*id) && !matches!(computed.as_deref(), Some("passed") | None)
            })
            .map(|(id, _)| id.clone()),
    );
    out.sort();
    out
}

const PLANNING: &str = r#"Веди задачу {task} набора «{name}». Прогон {run}.

Правила прогона:
- Работай в этом дереве. Набор правь дверями `mh call`, переменная MH_PROJECT задана.
- Коммит и пуш кода сам не делай. Когда правки готовы, позови
  `mh call approval-ask runId={run} title="что коммитим" body="дифф коротко"` и остановись.
- Нужен ответ владельца — позови `mh call question-ask runId={run} title="вопрос" body="что известно"` и остановись.
- Каждый заметный шаг отмечай `mh call run-event runId={run} text="что сделано"`.
- Ничего не обходи: если правило гейта краснеет, чини причину, а не правило.
- Прибор набора (`instrument/`, пробы и определения проверок) из прогона не правится:
  проверка непройдена по неправильной причине — вопрос владельцу, а не правка прибора.
- Фаза набора сейчас: {phase}. Работай по своей задаче и её требованиям; соседние фазы не трогай.

Начни с того, что прочитай задачу и её требования.

Задача больше одного шага — напиши план вызовом `mh call run-event runId={run} kind="план" text="шаги"` и закончи ответ словом PLAN. Не хватает ответа владельца — позови question-ask и закончи словами NEEDS CONTEXT либо BLOCKED. Задача разошлась с корпусом — слово DRIFT. Сделал за один шаг — скажи одним абзацем, что сделано, без маркера.{rethink}"#;

const PLAN_REVIEW: &str = r#"Ты ревьюер плана задачи {task} набора «{name}». Отвечай по-русски, коротко.

План из прогона {run}:
{plan}

Суди: отвечает ли план требованиям задачи, не лезет ли в соседние задачи, проверяемы ли шаги. Первой строкой вердикт одним словом: GO — план принят; RETHINK — план переписать. После RETHINK одна строка с доводом. Больше ничего не пиши."#;

const IMPLEMENTING: &str = r#"План по задаче {task} принят ревью вердиктом GO. Продолжай: сначала проверки и красное, затем минимальная реализация по плану. Отчёт — одним абзацем, что сделано."#;

const REVIEW: &str = r#"Ты ревьюер задачи {task} набора «{name}». Дерево набора — текущий каталог: дифф смотри через `git diff`, вопросы к корпусу — дверями `mh call` (MH_PROJECT задан). Смотри только эту задачу и её требования; ничего не правь — ты читаешь.

Первой строкой вердикт: CLEAN — принято; NEEDS FIX — править. После NEEDS FIX список находок, каждая одной строкой: где замечено, а не где искать. Больше ничего не пиши."#;

const FIXING: &str = r#"Круг починки {circle}/5 по задаче {task}. Находки ревью — дословно:
{findings}

Почини. Отчёт — одним абзацем."#;

const CLOSING: &str = r#"Ревью задачи {task} чистое. Осталось закрыть: позови `mh call approval-ask runId={run} title="что коммитим" body="дифф коротко"` и остановись. Коммит с трейлерами закрытия — после одобрения владельца."#;

#[cfg(test)]
mod tests {
    use super::{regression, verdict_of};
    use std::collections::HashMap;

    fn items(pairs: &[(&str, Option<&str>)]) -> HashMap<String, (Option<String>, String)> {
        pairs.iter().map(|(id, c)| (id.to_string(), (c.map(str::to_string), String::new()))).collect()
    }

    /// Маркеры дословны и идут по тяжести: DRIFT встаёт, даже когда в тексте
    /// есть более лёгкие слова; слово «план» по-русски не маркер.
    #[test]
    fn markers_are_literal_and_weighted() {
        assert_eq!(verdict_of("задача разошлась DRIFT и всё"), "DRIFT");
        assert_eq!(verdict_of("NEEDS FIX: файл x"), "NEEDS FIX");
        assert_eq!(verdict_of("RETHINK: слабовато"), "RETHINK");
        assert_eq!(verdict_of("PLAN"), "PLAN");
        assert_eq!(verdict_of("закоммичено CLOSED"), "CLOSED");
        assert_eq!(verdict_of("план простой, сделал без маркера"), "");
        assert_eq!(verdict_of("planner сказал"), "");
        assert_eq!(verdict_of(""), "");
    }

    /// Покраснение — поимённо: был зелёным и перестал, плюс объявившийся
    /// красным посреди прогона. Улучшение не считается регрессией.
    #[test]
    fn regression_names_items_and_ignores_improvements() {
        let was = items(&[("a", Some("passed")), ("b", Some("failed")), ("c", None)]);
        let now = items(&[("a", Some("failed")), ("b", Some("passed")), ("c", Some("failed")), ("d", Some("failed"))]);
        assert_eq!(regression(&was, &now), vec!["a".to_string(), "d".to_string()]);
        assert!(regression(&was, &was).is_empty(), "тишина — не регрессия");
    }
}
