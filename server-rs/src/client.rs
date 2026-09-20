//! Клиент: та же дверь, но по сети.
//!
//! Ходит в сервер по HTTP с краевым секретом и НЕ ЗНАЕТ О БАЗЕ НИЧЕГО. В этом
//! вся разница с прежним путём: `mcp.sh` грузил `.env` с паролем к Postgres, и
//! пароль лежал на каждой машине, где запускался скилл. Клиенту нужны адрес,
//! секрет и имя — доступа к набору у него нет.
//!
//! Своего разбора здесь нет и быть не должно. Клиент переносит и оформляет;
//! судит сервер. Как только тут появится «если нарушений больше трёх — плохо»,
//! у нас снова два ответа на один вопрос, только ближе к серверу и оттого
//! незаметнее.

use serde_json::{json, Value};

/// Куда и от чьего имени ходить.
pub struct Door {
    pub url: String,
    pub project: String,
    pub secret: String,
    pub principal: String,
}

impl Door {
    /// Собрать из окружения. Отсутствие любого — отказ словом, а не пустой вызов.
    pub fn from_env() -> Result<Self, String> {
        let near = Self::from_mcp_json();
        let need = |name: &str, fallback: Option<&str>| -> Result<String, String> {
            match std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
                Some(v) => Ok(v),
                None => match near.get(name) {
                    Some(v) => Ok(v.clone()),
                    None => match fallback {
                        Some(f) => Ok(f.to_owned()),
                        None => Err(format!(
                            "не задано {name}: ни переменной окружения, ни в `.mcp.json` рядом"
                        )),
                    },
                },
            }
        };
        // Секрет края — из окружения, а если его там нет, из личного файла
        // пользователя. Без этой второй двери каждая сессия начиналась с правки
        // `~/.zshrc`, а `.mcp.json`, подставляющий `${MH_EDGE_SECRET}`, молча
        // отказывал: «не задано» видел клиент, а человек видел неработающий
        // инструмент. Файл лежит ВНЕ репозитория и потому не уезжает с ним.
        let secret = match need("MH_EDGE_SECRET", None) {
            Ok(v) => v,
            Err(e) => match std::fs::read_to_string(Self::secret_file()) {
                Ok(v) if !v.trim().is_empty() => v.trim().to_owned(),
                _ => return Err(format!("{e}, и в {} его тоже нет", Self::secret_file().display())),
            },
        };
        let url = need("MH_URL", Some("http://127.0.0.1:8096"))?;
        let principal = need("MH_PRINCIPAL", Some(&Self::whoami()))?;
        // Проект спрашивается У СЕРВЕРА по дереву, в котором стоим, и только
        // потом читается из файла. Знание «чей это репозиторий» принадлежит
        // серверу: держать ответ файлом внутри самого дерева значит завести
        // вторую запись о том, что сервер и так знает, — и однажды разойтись.
        let project = match need("MH_PROJECT", None) {
            Ok(v) => v,
            Err(e) => Self::ask_whose(&url, &secret, &principal).ok_or(e)?,
        };
        Ok(Door { url, project, secret, principal })
    }

    /// Спросить сервер, какому проекту принадлежит текущее дерево.
    fn ask_whose(url: &str, secret: &str, principal: &str) -> Option<String> {
        let here = std::env::current_dir().ok()?;
        Self::ask_whose_at(url, secret, principal, &here)
    }

    /// То же — для названного дерева.
    fn ask_whose_at(url: &str, secret: &str, principal: &str, here: &std::path::Path) -> Option<String> {
        let out = std::process::Command::new("curl")
            .arg("-sS")
            .arg("-H").arg(format!("X-Mh-Edge: {secret}"))
            .arg("-H").arg(format!("X-Mh-Principal: {principal}"))
            .arg(format!("{url}/api/whose?path={}", here.display()))
            .output()
            .ok()?;
        let v: Value = serde_json::from_slice(&out.stdout).ok()?;
        v["project"].as_str().map(str::to_owned)
    }

    /// Кто ходит, если не сказано: имя пользователя системы.
    fn whoami() -> String {
        std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default()
    }

    /// Настройка из файла рядом — когда переменных в окружении нет.
    ///
    /// Проект, его край и адрес сервера уже записаны в `.mcp.json` каждого
    /// репозитория: установщик их туда и кладёт. Требовать те же значения ещё
    /// и переменными значит просить человека повторить то, что уже сказано —
    /// и `mh install .`, набранный в корне проекта, отвечал «не задано
    /// MH_PROJECT», стоя на файле, где всё написано.
    ///
    /// Ищется вверх от текущего каталога: команду набирают и из подкаталога.
    fn from_mcp_json() -> std::collections::HashMap<String, String> {
        let mut out = std::collections::HashMap::new();
        // Своя настройка — `.harness/mh.json`; `.mcp.json` читается следом и
        // только ради тех репозиториев, где он уже лежит. Держать настройку
        // клиента в файле, который называется «MCP-сервер», значит объявлять
        // механизм, которым не пользуется ни одно умение: их двадцать пять, и
        // MCP не зовёт ни одно.
        let mut dir = std::env::current_dir().unwrap_or_default();
        for _ in 0..6 {
            for (file, path) in [(dir.join(".harness/mh.json"), &["env"][..]),
                                 (dir.join(".mcp.json"), &["mcpServers", "harness", "env"][..])] {
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                let mut node = &v;
                for step in path {
                    node = &node[*step];
                }
                for key in ["MH_URL", "MH_PROJECT", "MH_PRINCIPAL", "MH_EDGE_SECRET"] {
                    if let Some(x) = node[key].as_str() {
                        // `${MH_EDGE_SECRET}` — не значение, а имя переменной.
                        if !x.starts_with("${") && !x.trim().is_empty() {
                            out.entry(key.to_owned()).or_insert_with(|| x.to_owned());
                        }
                    }
                }
            }
            if !out.is_empty() || !dir.pop() {
                break;
            }
        }
        out
    }

    /// Где лежит секрет края, когда его нет в окружении.
    pub(crate) fn secret_file() -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join(".config/mh/edge-secret")
    }

    fn body(&self, path: &str, args: Option<&Value>) -> Result<Value, String> {
        // Разговор ведёт `curl`, а не своя реализация HTTP: одна зависимость на
        // клиента, которого и так носят по чужим машинам, дороже той пользы,
        // что она даёт. Тело подаётся через stdin — доводы бывают длиннее, чем
        // выдерживает список аргументов, и подача фактов на этом уже спотыкалась.
        let mut cmd = std::process::Command::new("curl");
        // Код ответа — ПОСЛЕДНЕЙ строкой: без него «сервер занят» (503) и «дверь
        // отказала» (502) приходили одинаковыми, и цикл повторов у агента не
        // знал, ждать ему или чинить.
        cmd.arg("-sS")
            .arg("-w").arg("\n%{http_code}")
            .arg("-H").arg(format!("X-Mh-Edge: {}", self.secret))
            .arg("-H").arg(format!("X-Mh-Principal: {}", self.principal));
        if let Some(a) = args {
            cmd.arg("-X").arg("POST")
                .arg("-H").arg("Content-Type: application/json")
                .arg("--data-binary").arg("@-")
                .stdin(std::process::Stdio::piped());
            cmd.arg(format!("{}{}", self.url, path));
            let mut child = cmd.stdout(std::process::Stdio::piped()).spawn()
                .map_err(|e| format!("curl не запустился: {e}"))?;
            {
                use std::io::Write;
                let stdin = child.stdin.as_mut().ok_or("curl не принял тело")?;
                stdin.write_all(a.to_string().as_bytes())
                    .map_err(|e| format!("тело не ушло: {e}"))?;
            }
            let out = child.wait_with_output().map_err(|e| format!("curl не ответил: {e}"))?;
            return parse(&out.stdout);
        }
        cmd.arg(format!("{}{}", self.url, path));
        let out = cmd.output().map_err(|e| format!("curl не запустился: {e}"))?;
        parse(&out.stdout)
    }

    /// Спросить ручку. Доводы — объектом; пусто значит «без доводов».
    ///
    /// Возвращается ОТВЕТ РУЧКИ, а не конверт протокола: тому, кто зовёт из
    /// оболочки, `content[0].text` не нужен ни на что. Вторым значением идёт
    /// признак отказа — он в конверте и есть, и потерять его нельзя: «нет такой
    /// ручки» выглядит ответом ровно до тех пор, пока флаг не прочитан.
    pub fn call(&self, name: &str, args: &Value) -> Result<(Value, bool), String> {
        let path = format!("/api/projects/{}/tool/{}", self.project, name);
        let envelope = self.body(&path, Some(args))?;
        let refused = envelope.get("isError").and_then(|e| e.as_bool()).unwrap_or(false)
            || envelope.get("error").is_some();
        // Занятость доезжает до вызвавшего: она помечена и в конверте двери, и
        // кодом ответа, но внутрь ответа не попадала — а смотрят именно внутрь.
        let busy = crate::door::busy_said(&envelope)
            || envelope.get("error").and_then(|e| e.as_str()) == Some("busy");
        let inner = envelope
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .and_then(|t| serde_json::from_str::<Value>(t).ok()
                .or_else(|| Some(json!({ "why": t }))));
        let mut inner = inner.unwrap_or(envelope);
        if busy {
            match inner.as_object_mut() {
                Some(map) => {
                    map.insert("_meta".to_owned(), crate::door::mark_busy(true));
                }
                None => inner = json!({ "ответ": inner, "_meta": crate::door::mark_busy(true) }),
            }
        }
        Ok((inner, refused))
    }

    /// Перечень ручек — у сервера, а не свой.
    pub fn tools(&self) -> Result<Value, String> {
        let path = format!("/api/projects/{}/tools", self.project);
        self.body(&path, None)
    }
}

fn parse(bytes: &[u8]) -> Result<Value, String> {
    let whole = String::from_utf8_lossy(bytes);
    let (text, code) = match whole.rsplit_once('\n') {
        Some((body, code)) if code.trim().len() == 3 && code.trim().chars().all(|c| c.is_ascii_digit()) => {
            (body.to_owned(), code.trim().parse::<u16>().unwrap_or(0))
        }
        _ => (whole.to_string(), 0),
    };
    let text = std::borrow::Cow::from(text);
    if text.trim().is_empty() {
        // Пустой ответ — не пустой набор. Сервер мог не подняться, край мог не
        // пустить; назвать это «ничего не нашлось» значит соврать в ту сторону,
        // в которую врать нельзя.
        return Err("сервер ответил пустотой: он поднят и край пускает?".into());
    }
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("ответ не разбирается: {e}; было: {}",
                                                    text.chars().take(200).collect::<String>()))?;
    // Занятость помечается в самом ответе: её читает и `mh call` кодом выхода,
    // и всякий, кто зовёт дверь из скрипта.
    // Тело у 503 бывает и не предметом: край отвечает своей строкой, а
    // `v["busy"] = …` по строке роняет клиента ровно в час перегрузки.
    if code == 503 {
        match v.as_object_mut() {
            Some(map) => {
                map.insert("_meta".to_owned(), crate::door::mark_busy(true));
            }
            None => v = json!({ "error": "busy", "message": v, "_meta": crate::door::mark_busy(true) }),
        }
    }
    Ok(v)
}

/// Разбор доводов из командной строки: `k=v`, `k=@файл`, `k:=<json>`, `k:=@файл`.
///
/// Четыре записи, и каждая нужна. `k=v` — обычное; `k=@файл` — длинное тело
/// строкой; `k:=<json>` — там, где довод не строка: число, да/нет, список;
/// `k:=@файл` — то же, но из файла. Последняя не роскошь: датчик подаёт сотни
/// фактов разом, и список аргументов на этом ломается — «Argument list too long»
/// я сегодня получил ровно на такой подаче.
pub fn args_of(pairs: &[String]) -> Result<Value, String> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let Some((k, v)) = pair.split_once('=') else {
            return Err(format!("довод «{pair}» не в виде имя=значение"));
        };
        if let Some(k) = k.strip_suffix(':') {
            let text = match v.strip_prefix('@') {
                Some(file) => std::fs::read_to_string(file)
                    .map_err(|e| format!("файл довода «{file}» не читается: {e}"))?,
                None => v.to_owned(),
            };
            let parsed: Value = serde_json::from_str(&text)
                .map_err(|e| format!("довод «{k}» объявлен json, но не разбирается: {e}"))?;
            map.insert(k.to_owned(), parsed);
        } else if let Some(file) = v.strip_prefix('@') {
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("файл довода «{file}» не читается: {e}"))?;
            map.insert(k.to_owned(), json!(text));
        } else {
            map.insert(k.to_owned(), json!(v));
        }
    }
    Ok(Value::Object(map))
}

/// MCP по stdio — но за спиной у него сеть, а не база.
///
/// Агент говорит тем же протоколом, что и раньше; меняется только то, откуда
/// берётся ответ. Ради этого и затевалось: `.mcp.json` больше не носит пароль от
/// Postgres, а носит адрес, секрет края и имя — то, что край и так проверяет.
///
/// Перечень ручек тоже спрашивается у сервера. Свой, собранный на месте,
/// однажды разошёлся бы с настоящим — и разошёлся бы молча.
pub fn serve_mcp(door: Door) {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "initialize" => Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mh-corpus", "version": env!("CARGO_PKG_VERSION") }
            })),
            "tools/list" => Some(match door.tools() {
                Ok(v) => json!({ "tools": v.get("tools").cloned().unwrap_or(json!([])) }),
                // Недоступный сервер называется словом. Пустой перечень агент
                // прочтёт как «инструментов нет» и пойдёт работать руками.
                Err(why) => json!({ "tools": [], "why": why }),
            }),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Some(match door.call(name, &args) {
                    Ok((v, refused)) => json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }],
                        "isError": refused
                    }),
                    Err(why) => json!({
                        "content": [{ "type": "text", "text": why }], "isError": true
                    }),
                })
            }
            "ping" => Some(json!({})),
            _ => None,
        };
        let Some(id) = id else { continue };
        let body = match result {
            Some(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            None => json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("нет такого метода: {method}") } }),
        };
        let _ = writeln!(out, "{body}");
        let _ = out.flush();
    }
}

/// Установка харнеса в репозиторий: умения, субагенты и связка с сервером.
///
/// Всё берётся У СЕРВЕРА и ничего не лежит в репозитории заранее. Прежде умения
/// возили копиями: копия отстаёт молча, и разошедшийся скилл выглядит рабочим
/// ровно до того дня, когда сделает не то. Здесь источник один, и он же тот, у
/// кого спрашивают состояние.
///
/// Пишется рядом, а не поверх: файл, совпавший до знака, не трогается вовсе —
/// иначе установка каждый раз выглядит правкой и тонет в истории.
/// Сторож: перехват вызова инструмента ДО его исполнения.
///
/// Читает наряд хука со стандартного входа, спрашивает у сервера объявленных
/// сторожей и отвечает отказом, если правило поймало. Правило живёт в базе, а
/// не в файле рядом: прежде оно лежало в `.harness/probity.ts`, который сервер
/// не читает, и «что запрещено» знали двое по-разному.
///
/// Выход 0 — путь свободен. Выход 2 со словом в stderr — отказ: так его читает
/// Claude Code.
pub fn guard(door: &Door) -> i32 {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let call: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let tool = call["tool_name"].as_str().unwrap_or("");
    let arg = &call["tool_input"];
    // Что именно делают: пишут файл или запускают команду. Третьего сторожа не
    // знают, и молчать о неизвестном честнее, чем угадывать.
    let (acts_on, path, text) = match tool {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => (
            "write",
            arg["file_path"].as_str().unwrap_or("").to_owned(),
            [arg["content"].as_str().unwrap_or(""), arg["new_string"].as_str().unwrap_or("")].concat(),
        ),
        "Bash" => ("command", String::new(), arg["command"].as_str().unwrap_or("").to_owned()),
        _ => return 0,
    };
    let Ok((v, _)) = door.call("donors", &json!({})) else { return 0 };
    let empty = vec![];
    for g in v["guards"].as_array().unwrap_or(&empty) {
        if g["actsOn"].as_str().unwrap_or("write") != acts_on {
            continue;
        }
        let hit = |re: &str, hay: &str| -> bool {
            if re.trim().is_empty() {
                return true;
            }
            regex::Regex::new(re).map(|r| r.is_match(hay)).unwrap_or(false)
        };
        // Пустой образец значит «любой», и потому путь с содержимым должны
        // совпасть ОБА: иначе сторож пути ловил бы всякую запись.
        let caught = match acts_on {
            "command" => !g["commandRe"].as_str().unwrap_or("").trim().is_empty()
                && hit(g["commandRe"].as_str().unwrap_or(""), &text),
            _ => {
                let p = g["pathRe"].as_str().unwrap_or("");
                let c = g["contentRe"].as_str().unwrap_or("");
                !(p.trim().is_empty() && c.trim().is_empty()) && hit(p, &path) && hit(c, &text)
            }
        };
        if caught {
            eprintln!(
                "\n✗ {}\n\n    {}\n\n    Держит: {}\n",
                g["guard"].as_str().unwrap_or(""),
                g["refuses"].as_str().unwrap_or(""),
                g["enforces"].as_str().unwrap_or("")
            );
            return 2;
        }
    }
    0
}

/// Датчик: снять факт с репозитория и подать серверу.
///
/// Что снимать, объявлено в базе: где искать, чем вынимать, как назвать. Пять
/// датчиков жили пятью скриптами на другом языке — общего у них ровно это, а
/// различались они только тремя строками объявления.
pub fn sense(door: &Door, only: Option<&str>) -> Result<Value, String> {
    let root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or("не в репозитории: датчику нечего читать")?;
    // Чем снят факт: голова и чистота дерева (заявка 18). Датчик читает
    // рабочий каталог, а не `HEAD`, и подача обязана сказать, с какого
    // состояния мира она рассказывает: грязное дерево читается гейтом как
    // «неизвестно», а не как «пройдено».
    let head = std::process::Command::new("git")
        .args(["-C", &root, "rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    let dirty = std::process::Command::new("git")
        .args(["-C", &root, "status", "--porcelain"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(true);
    let (specs, _) = door.call("sensor-specs", &json!({}))?;
    let empty = vec![];
    let mut done = Vec::new();
    for spec in specs["specs"].as_array().unwrap_or(&empty) {
        let fact = spec["fact"].as_str().unwrap_or("");
        if let Some(one) = only {
            if one != fact {
                continue;
            }
        }
        let reads = spec["reads"].as_str().unwrap_or("");
        let re = spec["extract"].as_str().unwrap_or("");
        if fact.is_empty() || reads.is_empty() {
            continue;
        }
        let how = spec["how"].as_str().unwrap_or("extract");
        let retired;
        let (how, re) = if how == "retired-terms" {
            let (terms, refused) = door.call("retired-terms", &json!({}))?;
            if refused {
                return Err(format!("{fact}: сервер не отдал снятые термины, подавать пустоту нельзя: {terms}"));
            }
            let words: Vec<String> = terms["rows"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(|r| r["term"].as_str())
                .filter(|t| !t.trim().is_empty())
                .map(regex::escape)
                .collect();
            retired = if words.is_empty() { String::new() } else { format!(r"(?:^|\W)({})(?:\W|$)", words.join("|")) };
            ("lines", retired.as_str())
        } else {
            (how, re)
        };
        // Строка, которую датчик не считает находкой, и места, где правило не
        // действует. Без первого правило ловит себя же в объяснении, без
        // второго — запрещает шкале называть размеры, а часам — форматировать
        // время: то есть запрещает единственному месту быть этим местом.
        let skip = spec["skip"].as_str().unwrap_or("");
        let allow: Vec<&str> = spec["allow"].as_str().unwrap_or("").split_whitespace().collect();
        // Объявленное дерево спрашивается у сервера, а не обходится образцом:
        // объявлены в основном КАТАЛОГИ, и половина их — «описано вперёд», то
        // есть на диске их нет. Обход по диску такую строку не найдёт вовсе и
        // промолчит — а молчание тут читается как «сошлось».
        // Строки по объявленным адресам: спрашиваем набор, что он называет,
        // и читаем ровно эти строки. Полный указатель дерева тут не нужен.
        // Замороженное дерево: хэш каталога у git и его чистота. Донора
        // заморозили решением, и всякое его движение — либо правка того, что
        // править нельзя, либо незакоммиченный мусор, который однажды уедет.
        // СОСТОЯНИЯ ЗАДАЧ ИЗ ЗАКРЫВАЮЩИХ ТРЕЙЛЕРОВ. Дверь `task-state-push`
        // описана словами «принять состояния, ВЫВЕДЕННЫЕ ХАРНЕСОМ из закрывающих
        // трейлеров», а выводить их было некому: среди двадцати восьми видов
        // фактов состояний задач не было.
        //
        // Цена измерена: в истории myack девяносто девять трейлеров, в проекции
        // лежало тридцать четыре. `next-task` отказывал «барьер красной фазы
        // держит» и называл шестьдесят пять имён, закрытых месяц назад; доска
        // при этом говорила «Закрыто 82 из 82» — документ был прав, а проекция
        // нет, ровно наоборот тому, ради чего состояние из документа и вынесли.
        //
        // Образец трейлера ОБЪЯВЛЕН спецификацией, а не зашит: как набор
        // помечает закрытие, знает набор.
        // ДЕРЖАТЕЛЬ НЕ МОЖЕТ БЫТЬ ЗАГЛУШКОЙ, и это проверяет машина, а не
        // внимание. Правило `requirement-lands-on-surface` спрашивает «назван ли
        // держатель», а не «настоящий ли он»: разбор объявил одиннадцать
        // инвариантов держащимися, ревью открыло адреса и нашло, что СЕМЬ из
        // одиннадцати — заглушки. Счёт недоделанного был занижен вдвое.
        //
        // Три формы заглушки, и все три названы набором:
        //   пусто    — в файле нет ни одного элемента;
        //   шапка    — требование помянуто только шапкой модуля `//!`;
        //   `todo!`  — тело элемента не написано.
        if how == "holder-stub" {
            let (declared, _) = door.call("holders", &json!({}))?;
            let empty5 = Vec::new();
            let mut facts: Vec<Value> = Vec::new();
            for h in declared["holders"].as_array().unwrap_or(&empty5) {
                let req = h["requirement"].as_str().unwrap_or("").to_owned();
                let path = h["path"].as_str().unwrap_or("").trim().to_owned();
                if path.is_empty() {
                    continue;
                }
                let full = format!("{root}/{path}");
                // Файл читается ЗДЕСЬ, судит `holder_verdict`: «пути нет» — такая
                // же форма заглушки, как пустой файл, и решать её надо там же,
                // где решаются остальные три, иначе проверить её будет негде.
                let text = std::fs::read_to_string(&full).ok();
                if let Some(form) = holder_verdict(text.as_deref(), &req) {
                    facts.push(json!({ "name": format!("{req} → {path}"),
                        "detail": format!("держатель — заглушка: {form}") }));
                }
            }
            let read_n = declared["holders"].as_array().map(|a| a.len()).unwrap_or(0);
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": declared["holders"].as_array().map(|a| a.len()).unwrap_or(0),
                              "found": facts.len(), "was": out["was"], "now": out["now"] }));
            continue;
        }

        if how == "task-trailers" {
            let log = std::process::Command::new("git")
                // ВРЕМЯ КОММИТА ПОДАЁТСЯ ВМЕСТЕ С ТРЕЙЛЕРОМ. Без него сервер
                // знает лишь «когда увидел», а этим порядок не судится: набор,
                // проработавший год и подключённый вчера, показал бы всю историю
                // вчерашним днём, и правило «план записан до закрытия» не
                // отличило бы сделанного до себя от сделанного после.
                .args(["-C", &root, "log", "--all", "--pretty=%H%x00%ct%x00%B%x01"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            // ЗАКРЫТО — ЗНАЧИТ В ПРОДУКТЕ, а не «где-то в ветке». `--all` выше
            // берёт трейлеры отовсюду, и это правильно: иначе работу, ведомую
            // на ветке, не видно вовсе. Но состояние `closed` по трейлеру с
            // невлитой ветки — неправда: доска говорит «сделано», а в стволе
            // этого нет.
            //
            // Померено 19 сентября: у `tot-ade` три коммита закрытия жили
            // только в ветке — обе задачи волны 2, PR не влит. Доска
            // показывала владельцу «закрыто 3», в стволе была одна.
            //
            // Трейлер с невлитой ветки даёт `claimed`: работа есть, в продукт
            // не попала. Вольётся — станет `closed` следующей же подачей.
            let mainline = ["origin/HEAD", "origin/main", "main", "origin/master", "master"]
                .iter()
                .find(|r| {
                    std::process::Command::new("git")
                        .args(["-C", &root, "rev-parse", "--verify", "--quiet", r])
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
                .copied();
            // Ствола нет — судить «в продукте ли» нечем, и молча считать всё
            // закрытым нельзя: это ровно та ложь, от которой здесь уходим.
            let Some(mainline) = mainline else {
                return Err(format!(
                    "{fact}: ствола не найдено ни под одним из имён origin/HEAD · main · master.                      Закрыта ли задача В ПРОДУКТЕ, судить нечем, а считать закрытым всё подряд —                      это доска, которая врёт вверх."
                ));
            };
            let in_product: std::collections::HashSet<String> = std::process::Command::new("git")
                .args(["-C", &root, "log", mainline, "--pretty=%H"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim().to_owned())
                .collect();
            let rex = regex::Regex::new(re).map_err(|e| format!("{fact}: образец трейлера не разбирается: {e}"))?;
            let states = trailer_states(&log, &in_product, &rex);
            if states.is_empty() {
                return Err(format!(
                    "{fact}: закрывающих трейлеров в истории нет ни одного. Пустая подача \
                     стёрла бы состояния, выведенные прежде, — это отказ, а не ноль."
                ));
            }
            let (out, _) = door.call("task-state-push", &json!({ "states": states }))?;
            done.push(json!({ "fact": fact, "files": 0, "found": states.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }

        if how == "frozen-tree" {
            let (declared, _) = door.call("frozen-trees", &json!({}))?;
            let empty4 = Vec::new();
            let mut names: Vec<(String, String)> = Vec::new();
            for row in declared["trees"].as_array().unwrap_or(&empty4) {
                let path = row["path"].as_str().unwrap_or("").trim().to_owned();
                let want = row["hash"].as_str().unwrap_or("").trim().to_owned();
                if path.is_empty() {
                    continue;
                }
                let now = std::process::Command::new("git")
                    .args(["-C", &root, "rev-parse", &format!("HEAD:{path}")])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default();
                // Локальное имя не `dirty`: флаг чистоты дерева целиком
                // объявлен выше (head/dirty подаются с фактом), и тень здесь
                // подставила бы строку вместо признака.
                let uncommitted = std::process::Command::new("git")
                    .args(["-C", &root, "status", "--porcelain", "--", &path])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default();
                let detail = if now.is_empty() {
                    "каталога нет в дереве git".to_owned()
                } else if !want.is_empty() && now != want {
                    format!("хэш дерева {now}, а заморожен {want}")
                } else if !uncommitted.is_empty() {
                    format!("незакоммиченные правки: {}", uncommitted.lines().count())
                } else {
                    format!("стоит на {now}")
                };
                names.push((path, detail));
            }
            let facts: Vec<Value> = names.iter()
                .map(|(n, d)| json!({ "name": n, "detail": d }))
                .collect();
            let read_n = facts.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": facts.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        if how == "declared-lines" {
            let (declared, _) = door.call("addresses-declared", &json!({}))?;
            let empty3 = Vec::new();
            let mut names: Vec<(String, String)> = Vec::new();
            for row in declared["addresses"].as_array().unwrap_or(&empty3) {
                let path = row["path"].as_str().unwrap_or("").trim().to_owned();
                let n: usize = row["line"].as_str().unwrap_or("0").parse().unwrap_or(0);
                if path.is_empty() || n == 0 {
                    continue;
                }
                let key = format!("{path}:{n}");
                // Набор часто зовёт файл голым именем: `0001_core.sql:145`. Такой
                // адрес разрешается поиском по дереву — но ТОЛЬКО если имя одно.
                // Два одинаковых имени значат, что адрес двусмыслен, и это своя
                // находка, а не повод выбрать первое попавшееся.
                let text = match std::fs::read_to_string(format!("{root}/{path}")) {
                    Ok(t) => t,
                    Err(_) => {
                        let hits = find_by_name(&root, &path);
                        match hits.len() {
                            0 => {
                                names.push((key, "файла нет в репозитории".to_owned()));
                                continue;
                            }
                            1 => std::fs::read_to_string(&hits[0]).unwrap_or_default(),
                            _ => {
                                names.push((key, format!("имя неоднозначно: {}", hits.join(" · "))));
                                continue;
                            }
                        }
                    }
                };
                match text.lines().nth(n - 1) {
                    Some(l) => names.push((key, l.trim().chars().take(200).collect())),
                    None => names.push((key, format!("в файле строк {}", text.lines().count()))),
                }
            }
            let facts: Vec<Value> = names.iter()
                .map(|(n, d)| json!({ "name": n, "detail": d }))
                .collect();
            let read_n = facts.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": facts.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        if how == "declared-paths" {
            let (declared, _) = door.call("tree-declared", &json!({}))?;
            let empty2 = Vec::new();
            let mut names: Vec<(String, String)> = Vec::new();
            for row in declared["paths"].as_array().unwrap_or(&empty2) {
                let path = row["path"].as_str().unwrap_or("").trim().to_owned();
                if path.is_empty() {
                    continue;
                }
                let state = row["state"].as_str().unwrap_or("").trim().to_lowercase();
                let there = std::path::Path::new(&format!("{root}/{path}")).exists();
                names.push((path, format!("объявлено {state}, на диске {}",
                                          if there { "есть" } else { "нет" })));
            }
            let facts: Vec<Value> = names.iter()
                .map(|(n, note)| json!({ "name": n, "detail": note }))
                .collect();
            let read_n = facts.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": facts.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        // Корней у датчика бывает несколько: правила именования смотрят и на
        // `backend/crates`, и на `backend/bin`, и на `frontend/src`. Три датчика
        // ради одного правила развели бы одно правило по трём объявлениям.
        let mut files: Vec<String> = Vec::new();
        for one in reads.split_whitespace() {
            files.extend(walk(&root, one));
        }
        files.sort();
        files.dedup();

        if how == "contract-head" {
            let mut pairs = Vec::new();
            for f in &files {
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                let doc = crate::yaml::parse(&text);
                pairs.extend(crate::repo_corpus::contract_head(&text, &doc));
                // Величины требований считаются здесь же: телом и границей
                // делится ОДИН текст, и разрезать его дважды значит однажды
                // разрезать по-разному.
                let (in_body, border) = crate::repo_corpus::contract_requirement_split(&text);
                pairs.extend(crate::repo_corpus::contract_head_echo(
                    &text, in_body, border, &crate::repo_corpus::contract_sizes(&doc)));
                // Заявленное шапкой ВСЕГО уезжает фактом: сверять его с числом
                // требований — дело базы, там оно связью, а не пересказом.
                pairs.extend(crate::repo_corpus::contract_head_total(&text));
            }
            let facts: Vec<Value> = pairs.iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        if how == "contract-marks" {
            let mut pairs = Vec::new();
            for f in &files {
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                pairs.extend(crate::repo_corpus::contract_marks(&crate::yaml::parse(&text)));
            }
            let facts: Vec<Value> = pairs.iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }

        // Требование, названное телом контракта: поверхность «контракт HTTP».
        if how == "contract-body" {
            let mut pairs = Vec::new();
            for f in &files {
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                pairs.extend(crate::repo_corpus::requirements_in_body(&text));
            }
            if pairs.is_empty() {
                return Err(format!("{fact}: тело контракта не назвало ни одного требования"));
            }
            let facts: Vec<Value> = pairs.iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }

        // Требование против операции контракта: пары, которые больше никто не
        // считает. Пустой ответ здесь — отказ, а не ноль.
        if how == "contract-ops" {
            let mut pairs = Vec::new();
            for f in &files {
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                pairs.extend(crate::repo_corpus::requirement_ops(&text));
            }
            if pairs.is_empty() {
                return Err(format!("{fact}: ни одной пары «требование · операция» — сверять нечем"));
            }
            let facts: Vec<Value> = pairs
                .iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }

        // Контракт против схемы. Отказ вместо тихого нуля: сломанная ветка
        // YAML уносит схему из корпуса, обе стороны показывают ноль, и правило
        // выглядит зелёным ровно потому, что ему нечего сказать.
        if how == "contract-vs-schema" {
            let mut doc = serde_json::Value::Null;
            // Схема собирается ЦЕЛИКОМ и по порядку: `CREATE TABLE` плюс всё,
            // что доехало `ALTER`-ами. Список отсортирован, а миграции нумерованы,
            // так что порядок имён — он же порядок применения.
            let mut sql: Vec<String> = Vec::new();
            for f in &files {
                let short = f.strip_prefix(&format!("{root}/")).unwrap_or(f).to_owned();
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                if short.ends_with(".yaml") || short.ends_with(".yml") {
                    doc = crate::yaml::parse(&text);
                } else if crate::repo_corpus::migration_up(&short) {
                    sql.push(text);
                }
            }
            let tables = crate::repo_corpus::schema_of(&sql);
            let schemas = doc
                .get("components")
                .and_then(|c| c.get("schemas"))
                .and_then(|s| s.as_object())
                .map(|o| o.len())
                .unwrap_or(0);
            if schemas == 0 || tables.is_empty() {
                return Err(format!(
                    "{fact}: контракт дал {schemas} схем, схема {} таблиц — сверять нечем",
                    tables.len()
                ));
            }
            let forced: Vec<(&str, &str)> = re
                .split(';')
                .filter_map(|p| p.split_once('='))
                .map(|(a, b)| (a.trim(), b.trim()))
                .collect();
            let facts = facts_of(&crate::repo_corpus::contract_vs_schema(&doc, &tables, &forced));
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }

        // Домен против схемы: спаривание — само наблюдение над репозиторием,
        // а не суд над ним. Датчик подаёт пары и НАЗЫВАЕТ несопоставленное:
        // правило, молча пропускающее непарное, вырождается в зелёное.
        if how == "domain-vs-check" {
            let mut enums = Vec::new();
            let mut checks = Vec::new();
            for f in &files {
                let short = f.strip_prefix(&format!("{root}/")).unwrap_or(f).to_owned();
                let Ok(text) = std::fs::read_to_string(f) else { continue };
                if short.ends_with(".rs") {
                    enums.extend(crate::repo_corpus::enums_of(&text, &short));
                } else if crate::repo_corpus::migration_up(&short) {
                    checks.extend(crate::repo_corpus::checks_of(&text, &short));
                }
            }
            // Отказ вместо тихого нуля: пропавший каталог обнулил бы оба
            // перечня и показал бы «сверили, сошлось».
            if enums.is_empty() || checks.is_empty() {
                return Err(format!(
                    "{fact}: домен дал {} перечислений, схема {} множеств — сверять нечем",
                    enums.len(),
                    checks.len()
                ));
            }
            let forced: Vec<(&str, &str)> = re
                .split(';')
                .filter_map(|p| p.split_once('='))
                .map(|(a, b)| (a.trim(), b.trim()))
                .collect();
            let contract_enums = files
                .iter()
                .filter(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
                .filter_map(|f| std::fs::read_to_string(f).ok())
                .map(|t| crate::repo_corpus::contract_enums(&crate::yaml::parse(&t)))
                .fold(Vec::new(), |mut a, b| {
                    a.extend(b);
                    a
                });
            let checks_tables: Vec<crate::repo_corpus::Table> = crate::repo_corpus::schema_of(
                &files
                    .iter()
                    .filter(|f| crate::repo_corpus::migration_up(f))
                    .filter_map(|f| std::fs::read_to_string(f).ok())
                    .collect::<Vec<String>>(),
            );
            let mut pairs =
                crate::repo_corpus::pair(&enums, &checks, &contract_enums, &forced, &checks_tables);
            // Полнота разбора считается тем же проходом: корпус, разобранный
            // наполовину, даёт зелёное всем правилам разом.
            for f in files.iter().filter(|f| f.ends_with(".yaml") || f.ends_with(".yml")) {
                if let Ok(text) = std::fs::read_to_string(f) {
                    let doc = crate::yaml::parse(&text);
                    pairs.extend(crate::repo_corpus::corpus_reach(&text, &doc, &enums, &checks_tables));
                }
            }
            // Тем же проходом — значения без пути: они опираются на ту же пару,
            // и считать их отдельно значило бы завести вторую правду о паре.
            pairs.extend(crate::repo_corpus::check_values_without_path(
                &enums, &checks, &contract_enums, &forced, &checks_tables));
            let facts = facts_of(&pairs);
            let read_n = files.len();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        let mut names: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let declared: Vec<String> = if how == "secret-fields" {
            files.iter().filter_map(|f| std::fs::read_to_string(f).ok()).flat_map(|t| declared_types(&t)).collect()
        } else {
            Vec::new()
        };
        for f in &files {
            let short = f.strip_prefix(&format!("{root}/")).unwrap_or(f).to_owned();
            if how == "files" {
                // Дисковая сторона сверки: файл есть. Сравнит сервер — у него
                // объявленная сторона лежит таблицей документа.
                if seen.insert(short.clone()) {
                    names.push((short.clone(), "есть на диске".to_owned()));
                }
                continue;
            }
            // ФАЙЛ, В КОТОРОМ ЧТО-ТО НАЙДЕНО, — один факт на файл, а не на строку.
            // Датчик `db-test-file` объявлен и спрашивается живым пунктом `G3`, а
            // подать его было нечем: пункт держался на подаче двухдневной
            // давности и говорил «неизвестно», едва она протухала.
            //
            // От `lines` отличается тем, что находок ровно столько, сколько
            // файлов: правило считает ФАЙЛЫ, и сто строк одного файла сказали бы
            // «сто интеграционных проверок» там, где она одна.
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            if how == "file-matches" {
                let hit = regex::Regex::new(re).ok().map(|r| r.is_match(&text)).unwrap_or(false);
                if !re.is_empty() && hit && seen.insert(short.clone()) {
                    names.push((short.clone(), spec["note"].as_str().unwrap_or("совпало").to_owned()));
                }
                continue;
            }
            if how == "lines" {
                // Построчный датчик: имя факта — путь и НОМЕР строки, пояснение —
                // сама строка. Оговорки («это донор», «отменено») правилом не
                // разбираются: датчик подаёт наблюдение, а решает гейт.
                // Место, где правило объявлено недействующим, пропускается
                // целиком: скажет оно ровно то, ради чего это место и заведено.
                if allow.iter().any(|a| short == *a || short.ends_with(a)) {
                    continue;
                }
                for (n, line) in text.split('\n').enumerate() {
                    if re.is_empty() || !suspect_line(re, line) {
                        continue;
                    }
                    if !skip.is_empty() && suspect_line(skip, line) {
                        continue;
                    }
                    let key = format!("{short}:{}", n + 1);
                    if seen.insert(key.clone()) {
                        // Обрезка по ЗНАКАМ, а не по байтам: русская строка
                        // рвётся посреди буквы, и обрезка по длине валит датчик
                        // целиком — паникой, а не отказом.
                        let body: String = line.trim().chars().take(200).collect();
                        names.push((key, body));
                    }
                }
                continue;
            }
            if how == "secret-fields" {
                for (decl, field, ty) in secret_fields(&text, re, &declared) {
                    // Ключ несёт путь: `Target.url` встречается в двух файлах
                    // разными типами, и одно исключение сняло бы оба разом.
                    let key = format!("{short}#{decl}.{field}");
                    if seen.insert(key.clone()) {
                        names.push((key, format!("голым под derive(Debug), тип {ty}")));
                    }
                }
                continue;
            }
            if re.trim().is_empty() {
                // Пустой образец — факт о самом файле: `.sqlx` снят или нет.
                if seen.insert(short.clone()) {
                    names.push((short.clone(), format!("есть в {reads}")));
                }
                continue;
            }
            let Ok(rx) = regex::Regex::new(re) else { continue };
            for c in rx.captures_iter(&text) {
                let name = c.get(1).or_else(|| c.get(0)).map(|m| m.as_str().to_owned());
                let Some(name) = name else { continue };
                if seen.insert(name.clone()) {
                    names.push((name, format!("названо в {short}")));
                }
            }
        }
        let facts: Vec<Value> = names.iter()
            .map(|(n, note)| json!({ "name": n, "detail": note }))
            .collect();
        let read_n = files.len();
        let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts, "commit": head, "dirty": dirty, "read": read_n }))?;
        done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                          "was": out["was"], "now": out["now"] }));
    }
    if done.is_empty() {
        return Err(match only {
            Some(f) => format!("датчик {f} не объявлен: чем его снимать — не сказано"),
            None => "ни один датчик не объявлен".to_owned(),
        });
    }
    Ok(json!({ "sensed": done }))
}

/// Секрето-подобное поле, стоящее ГОЛЫМ под `#[derive(Debug)]`.
///
/// Не образец, а обход: между `derive` и объявлением стоят другие атрибуты и
/// комментарии, а поле надо смотреть внутри тела — regex такого не выражает.
/// `field_re` — чем узнаётся секрето-подобное имя; подозрение снимает только
/// `Secret<…>` или тип, объявленный в самом корпусе (`StoredToken`, `PublicUrl`),
/// потому что он и есть ответ на него. Тип корпуса узнаётся голым именем или
/// путём через `crate::`/`self::`/`super::`: `reqwest::Url` — не `Url` корпуса.
/// Всякий иной тип подозрителен.
/// Строка подходит под образец. Ошибка в образце не молчит: она отвечает «нет»
/// на каждую строку, и датчик подал бы пустоту как «ничего не найдено».
fn suspect_line(re: &str, line: &str) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<regex::Regex>>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock().expect("замок образцов");
    let entry = map.entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok());
    entry.as_ref().map(|r| r.is_match(line)).unwrap_or(false)
}

fn bare_type(ty: &str) -> String {
    ty.split("//").next().unwrap_or("").trim().trim_end_matches(',').trim().to_owned()
}

fn answered(ty: &str, declared: &[String], imported: &[String]) -> bool {
    let mut t = ty.trim();
    while let Some(inner) = ["Option<", "Box<", "Vec<"]
        .iter()
        .find_map(|w| t.strip_prefix(w)?.strip_suffix('>'))
    {
        t = inner.trim();
    }
    let scalar = ty.trim().strip_prefix("Option<").and_then(|x| x.strip_suffix('>')).unwrap_or(ty.trim()).trim();
    if scalar == "bool" {
        return true;
    }
    let local = match t.split_once("::") {
        None => Some(t).filter(|name| !imported.iter().any(|i| i == name)),
        Some(("crate" | "self" | "super", _)) => t.rsplit("::").next(),
        Some(_) => None,
    };
    ty.contains("Secret<") || local.is_some_and(|name| declared.iter().any(|d| d == name))
}

fn declared_types(text: &str) -> Vec<String> {
    regex::Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)")
        .expect("образец объявления")
        .captures_iter(text)
        .map(|c| c[1].to_owned())
        .collect()
}

fn imported_types(text: &str) -> Vec<String> {
    regex::Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([^;]+);")
        .expect("образец use")
        .captures_iter(text)
        .filter(|c| !matches!(c[1].trim().trim_start_matches("::").split("::").next(), Some("crate" | "self" | "super")))
        .flat_map(|c| {
            c[1].replace(['{', '}'], ",")
                .split(',')
                .filter_map(|piece| {
                    let name = match piece.split_once(" as ") {
                        Some((_, alias)) => alias.trim(),
                        None => piece.rsplit("::").next().unwrap_or("").trim(),
                    };
                    (!name.is_empty() && name != "self" && name != "*").then(|| name.to_owned())
                })
                .collect::<Vec<String>>()
        })
        .collect()
}

fn facts_of(pairs: &[crate::repo_corpus::Pair]) -> Vec<Value> {
    let marked = |p: &crate::repo_corpus::Pair| p.detail.starts_with("помечено ");
    pairs
        .iter()
        .filter(|p| !marked(p) || !pairs.iter().any(|q| q.name == p.name && !marked(q)))
        .map(|p| json!({ "name": p.name, "detail": p.detail }))
        .collect()
}

fn secret_fields(text: &str, field_re: &str, declared: &[String]) -> Vec<(String, String, String)> {
    let Ok(suspect) = regex::Regex::new(if field_re.trim().is_empty() {
        r"(?i)\b(token|secret|password|passwd|api_?key|credential|private_?key)\b"
    } else {
        field_re
    }) else {
        return Vec::new();
    };
    let derive = regex::Regex::new(r"^\s*#\[derive\(([^)]*)\)\]").expect("образец derive");
    let decl = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)")
        .expect("образец объявления");
    let field = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:\s*(.+?),?\s*$")
        .expect("образец поля");
    let variant = regex::Regex::new(r"^\s*\w+\s*\{(.+)\}\s*,?\s*$").expect("образец ветки");
    let imported = imported_types(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(d) = derive.captures(lines[i]) else { i += 1; continue };
        if !d[1].split(',').any(|t| t.trim() == "Debug") {
            i += 1;
            continue;
        }
        // Между `derive` и объявлением стоят другие атрибуты и комментарии.
        let mut j = i + 1;
        while j < lines.len()
            && (lines[j].trim_start().starts_with("#[")
                || lines[j].trim_start().starts_with("//")
                || lines[j].trim().is_empty())
        {
            j += 1;
        }
        let Some(name) = decl.captures(lines.get(j).copied().unwrap_or("")) else { i += 1; continue };
        let owner = name[1].to_owned();
        // `pub struct Tier(pub u8);` — тело в круглых скобках и кончается точкой
        // с запятой на той же строке. Обход, не заметив этого, читал дальше и
        // приписывал кортежной структуре поля СЛЕДУЮЩЕЙ за ней: `Tier.url`
        // вместо `RepoRef.url` — находка врала именем в обе стороны сразу.
        if lines[j].trim_end().ends_with(';') {
            i = j + 1;
            continue;
        }
        // Тело — до закрывающей скобки на своём уровне отступа.
        let mut k = j + 1;
        while k < lines.len() && !lines[k].starts_with('}') {
            // `Provider { url: String },` — поле ветки перечисления стоит в
            // одной строке с ней, и построчный образец поля его не видит.
            if let Some(v) = variant.captures(lines[k]) {
                for part in v[1].split(',') {
                    if let Some(f) = field.captures(part) {
                        let (fname, ftype) = (f[1].to_owned(), bare_type(&f[2]));
                        if suspect.is_match(&fname) && !answered(&ftype, declared, &imported) {
                            out.push((owner.clone(), fname, ftype));
                        }
                    }
                }
            } else if let Some(f) = field.captures(lines[k]) {
                let (fname, ftype) = (f[1].to_owned(), bare_type(&f[2]));
                if suspect.is_match(&fname) && !answered(&ftype, declared, &imported) {
                    out.push((owner.clone(), fname, ftype));
                }
            }
            k += 1;
        }
        i = k.max(i + 1);
    }
    out
}

/// Обход по простому образцу: `backend/**/*.rs`, `backend/api/openapi.yaml`.
///
/// Свой, а не библиотекой: клиента носят по чужим машинам, и одна зависимость
/// ради двух звёздочек дороже той пользы, что она даёт.
/// Файлы дерева с таким путём-хвостом. Голое имя из набора разрешается им.
fn find_by_name(root: &str, tail: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    let want = format!("/{tail}");
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "node_modules" | "target" | "dist") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.display().to_string().ends_with(&want) {
                out.push(p.display().to_string());
            }
        }
    }
    out.sort();
    out
}

fn walk(root: &str, pattern: &str) -> Vec<String> {
    let full = format!("{root}/{pattern}");
    let (dir, rest) = match full.find('*') {
        Some(i) => {
            let cut = full[..i].rfind('/').map(|j| j + 1).unwrap_or(0);
            (full[..cut].to_owned(), full[cut..].to_owned())
        }
        None => return if std::path::Path::new(&full).is_file() { vec![full] } else { vec![] },
    };
    let suffix = rest.rsplit('*').next().unwrap_or("").to_owned();
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Пропускается ТО, ЧТО НЕ ЧАСТЬ дерева, а не всё точечное:
            // `.github` и `.sqlx` — такие же каталоги проекта, как остальные, и
            // задача вправе называть их местом для кода. Прежний пропуск всего
            // точечного делал их невидимыми, и правило «нет даже каталога»
            // краснело на существующем.
            if matches!(name, ".git" | "node_modules" | "target" | "dist" | ".venv") {
                continue;
            }
            if p.is_dir() {
                // Спуск нужен не только при `**`: у образца `crates/*/Cargo.toml`
                // искомое лежит уровнем ниже, и без спуска обход возвращал ноль
                // — а ноль читался как «таких файлов нет».
                if rest.contains("**") || rest.contains('/') {
                    stack.push(p);
                }
            } else if suffix.is_empty() || p.display().to_string().ends_with(&suffix) {
                // Хвост сверяется с ПУТЁМ, а не с именем файла: у образца
                // `backend/crates/*/Cargo.toml` хвост — `/Cargo.toml`, и ни одно
                // имя файла им не кончается. Датчик отвечал нулём, и ноль
                // читался как «таких файлов нет».
                out.push(p.display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Поставлен ли файл установкой: её шапка несёт имя и описание в кавычках.
fn installed_head(text: &str, name: &str) -> bool {
    text.starts_with(&format!("---\nname: {name}\ndescription: \""))
}

pub fn install(door: &Door, into: &str) -> Result<Value, String> {
    use std::io::Write;
    let mut written = Vec::new();
    let mut same = 0usize;

    // Репозиторий, уже назвавший СВОЙ проект, чужим не переписывается.
    //
    // Проект берётся из окружения, и одна установка, запущенная в цикле по двум
    // репозиториям с выставленной переменной, вписала во второй проект первого:
    // после неё `mh call next-step` в одном каталоге отвечал про другой набор.
    // Молча — потому что оба ответа выглядят настоящими.
    let declared = std::path::Path::new(into).join(".harness/mh.json");
    if let Ok(text) = std::fs::read_to_string(&declared) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(was) = v["env"]["MH_PROJECT"].as_str() {
                if !was.is_empty() && was != door.project {
                    return Err(format!(
                        "{into} уже объявлен проектом {was}, а ставится {}. \
                         Если проект правда сменился — снимите {} и повторите",
                        door.project,
                        declared.display()
                    ));
                }
            }
        }
    }

    // Чей это репозиторий, знает сервер. Оставшийся от другой работы
    // `MH_PROJECT` поставил бы сюда чужой набор — а теперь установка ещё и
    // снимает поставленное: чужой набор стёр бы свой.
    if let Ok(target) = std::fs::canonicalize(into) {
        if let Some(owner) = Door::ask_whose_at(&door.url, &door.secret, &door.principal, &target) {
            if owner != door.project {
                return Err(format!(
                    "{} принадлежит проекту {owner}, а ставится {}: снимите MH_PROJECT или поставьте из своего дерева",
                    target.display(),
                    door.project
                ));
            }
        }
    }

    let put = |path: &std::path::Path, text: &str| -> Result<bool, String> {
        if let Ok(old) = std::fs::read_to_string(path) {
            if old == text {
                return Ok(false);
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("каталог {dir:?} не заводится: {e}"))?;
        }
        let mut f = std::fs::File::create(path).map_err(|e| format!("{path:?} не пишется: {e}"))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("{path:?} не дописан: {e}"))?;
        Ok(true)
    };
    // Значение шапки бывает с двоеточием и переводом строки; кавычки и переводы
    // экранируются, иначе шапка перестанет разбираться на первом же описании.
    let quoted = |v: &str| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " "));

    let skills = door.call("skills", &json!({ "body": true }))?.0;
    for sk in skills.get("skills").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        let name = sk["name"].as_str().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        // Шапка несёт ВСЁ объявленное, а не только имя с описанием. Чем умению
        // разрешено пользоваться — часть объявления: установка, которая это
        // теряет, снимает ограничение молча.
        let mut head = format!("---\nname: {name}\ndescription: {}\n",
                               quoted(sk["description"].as_str().unwrap_or("")));
        let tools = sk["allowedTools"].as_str().unwrap_or("");
        if !tools.is_empty() {
            head.push_str(&format!("allowed-tools: {tools}\n"));
        }
        if let Some(flag) = sk["disableModelInvocation"].as_bool() {
            head.push_str(&format!("disable-model-invocation: {flag}\n"));
        }
        let text = format!("{head}---\n\n{}", sk["body"].as_str().unwrap_or(""));
        let path = std::path::Path::new(into).join(".claude/skills").join(name).join("SKILL.md");
        if put(&path, &text)? { written.push(format!("умение {name}")) } else { same += 1 }
    }

    let agents = door.call("agents", &json!({ "body": true }))?.0;
    for a in agents.get("agents").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        let name = a["name"].as_str().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let mut head = format!("---\nname: {name}\ndescription: {}\n", quoted(a["description"].as_str().unwrap_or("")));
        for (field, key) in [("tools", "tools"), ("model", "model")] {
            let v = a[key].as_str().unwrap_or("");
            if !v.is_empty() {
                head.push_str(&format!("{field}: {v}\n"));
            }
        }
        let text = format!("{head}---\n\n{}", a["body"].as_str().unwrap_or(""));
        let path = std::path::Path::new(into).join(".claude/agents").join(format!("{name}.md"));
        if put(&path, &text)? { written.push(format!("субагент {name}")) } else { same += 1 }
    }

    // Снятое на сервере снимается и здесь. Установка писала и не стирала: скилл,
    // убранный из набора, оставался в `.claude/skills` и звался дальше — копия
    // без источника отстаёт навсегда. Трогается только поставленное установкой:
    // его шапка несёт описание в кавычках, как пишет `quoted`; свой файл проекта
    // так не выглядит, и его не касаемся. Пустой перечень с сервера не снимает
    // ничего: сбой ответа не должен стереть всё поставленное.
    let names_of = |list: &Value, key: &str| -> std::collections::HashSet<String> {
        list.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    let ours = |path: &std::path::Path, name: &str| {
        std::fs::read_to_string(path).map(|t| installed_head(&t, name)).unwrap_or(false)
    };
    let kept_skills = names_of(&skills, "skills");
    if !kept_skills.is_empty() {
        if let Ok(dirs) = std::fs::read_dir(std::path::Path::new(into).join(".claude/skills")) {
            for dir in dirs.flatten() {
                // Ссылка — не каталог установки: файл за ней лежит где угодно, и
                // снимать его значило бы стирать вне репозитория.
                if dir.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
                    continue;
                }
                let name = dir.file_name().to_string_lossy().into_owned();
                let file = dir.path().join("SKILL.md");
                if kept_skills.contains(&name) || !ours(&file, &name) {
                    continue;
                }
                // Установке принадлежит только её файл. Каталог снимается, лишь
                // если опустел: чужой файл рядом — не наш, и стирать его нельзя.
                std::fs::remove_file(&file).map_err(|e| format!("{file:?} не снимается: {e}"))?;
                if std::fs::remove_dir(dir.path()).is_ok() {
                    written.push(format!("снято умение {name}"));
                } else {
                    written.push(format!("снято умение {name}; каталог оставлен — в нём чужие файлы"));
                }
            }
        }
    }
    let kept_agents = names_of(&agents, "agents");
    if !kept_agents.is_empty() {
        if let Ok(files) = std::fs::read_dir(std::path::Path::new(into).join(".claude/agents")) {
            for file in files.flatten() {
                if file.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
                    continue;
                }
                let path = file.path();
                let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
                if path.extension().and_then(|e| e.to_str()) != Some("md") || kept_agents.contains(&name) || !ours(&path, &name) {
                    continue;
                }
                std::fs::remove_file(&path).map_err(|e| format!("{path:?} не снимается: {e}"))?;
                written.push(format!("снят субагент {name}"));
            }
        }
    }

    // Сторож подключается хуком: `mh guard` перехватывает вызов инструмента до
    // его исполнения и отвечает отказом по правилам, объявленным в сервере.
    // Файл маленький и в репозитории законен: это НЕ данные, а связка с
    // машиной — тем же, чем `.mcp.json` был для MCP.
    let hooks = format!(
        "{{\n  \"hooks\": {{\n    \"PreToolUse\": [\n      {{\n        \"matcher\": \"Write|Edit|MultiEdit|NotebookEdit|Bash\",\n        \"hooks\": [{{ \"type\": \"command\", \"command\": {} }}]\n      }}\n    ]\n  }}\n}}\n",
        quoted(&format!("{} guard",
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "mh".into())))
    );
    let hook_path = std::path::Path::new(into).join(".claude/settings.json");
    if put(&hook_path, &hooks)? { written.push(".claude/settings.json — сторож".into()) } else { same += 1 }

    // Настройки в репозитории БОЛЬШЕ НЕТ.
    //
    // Проект спрашивается у сервера по дереву: `GET /api/whose?path=…`. Файл
    // `.harness/mh.json` держал тот же ответ внутри самого дерева — вторую
    // запись о том, что сервер и так знает. Клиент читает её по-прежнему, если
    // она есть: у чужой машины, где сервера ещё нет, иначе не начать.

    // Секрет — в личный файл пользователя, один на все репозитории. Оболочку
    // править больше не нужно: клиент возьмёт его отсюда, когда переменной нет.
    let keep = Door::secret_file();
    if std::fs::read_to_string(&keep).map(|v| v.trim() != door.secret).unwrap_or(true) {
        if let Some(dir) = keep.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("каталог {dir:?} не заводится: {e}"))?;
        }
        std::fs::write(&keep, format!("{}\n", door.secret))
            .map_err(|e| format!("{keep:?} не пишется: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&keep, std::fs::Permissions::from_mode(0o600));
        }
        written.push(format!("секрет края → {}", keep.display()));
    } else {
        same += 1;
    }

    // Раздел в AGENTS.md — между метками, чтобы обновление не трогало чужого
    // текста. Без него сессия, открывшая репозиторий, о наборе не знает вовсе:
    // умения лежат, дверь настроена, а сказать о них некому.
    let head = "<!-- mh:harness начало — раздел пишет `mh install`, руками не править -->";
    let tail = "<!-- mh:harness конец -->";
    // Строки перечислены поимённо, а не склеены переносом строки в исходнике:
    // склейка тащит в текст отступ самого исходника, и первая же установка
    // положила в AGENTS.md блок, сдвинутый на девять пробелов.
    let block = [
        head, "\n",
        "## Набор документов живёт в сервере\n",
        "\n",
        "Документы этого проекта — не файлы репозитория. Они в сервере харнеса, и ходят\n",
        "к ним клиентом `mh`: `mh call <ручка> ключ=значение`. Что он умеет — `mh tools`.\n",
        "Адрес сервера и проект он берёт из `.harness/mh.json` рядом, секрет — из личного\n",
        "файла; переменных окружения задавать не надо. Заводить в репозитории копии\n",
        "документов набора не надо: копия отстанет молча.\n", "",
        "\n",
        "**С чего начинать любую работу.** Спроси `next-step`: сервер назовёт ступень, что\n",
        "её держит и чьё это умение. Умения лежат в `.claude/skills/`, вход — `/godzy`.\n",
        "Их источник — сервер; обновляются командой `mh install .`, руками не правятся\n",
        "и в git не коммитятся: `.claude/skills/` и `.claude/agents/` — в `.gitignore`.\n",
        "\n",
        "**Что чем отвечается:**\n",
        "\n",
        "| Вопрос | Чем спросить |\n",
        "|---|---|\n",
        "| где мы и что держит | `next-step` |\n",
        "| что не прошло в гейтах | `gate` |\n",
        "| какую задачу брать | `next-task`, порядок — `waves` |\n",
        "| у каких задач нет вердикта предполёта | `preflight-queue` |\n",
        "| где документ и история расходятся | `state-disagreements` |\n",
        "\n",
        "**Состояние задач сервер не читает сам.** Оно выводится из закрывающих трейлеров\n",
        "`Task: <id> closed` и подаётся `task-state-push`. Отметка в документе — заявление,\n",
        "трейлер — свидетельство, и расхождение видно отдельным запросом.\n",
        tail, "\n",
    ].concat();
    let agents = std::path::Path::new(into).join("AGENTS.md");
    let old_text = std::fs::read_to_string(&agents).unwrap_or_default();
    let next = match (old_text.find(head), old_text.find(tail)) {
        (Some(a), Some(b)) => format!("{}{}{}", &old_text[..a], block, &old_text[b + tail.len() + 1..]),
        _ if old_text.trim().is_empty() => format!("# AGENTS.md\n\n{block}"),
        _ => format!("{}\n\n{block}", old_text.trim_end()),
    };
    if put(&agents, &next)? { written.push("AGENTS.md".into()) } else { same += 1 }

    Ok(json!({
        "into": into,
        "written": written.len(), "files": written,
        "unchanged": same,
        "why": "источник один — сервер; копий в репозитории не остаётся, и отставать нечему",
    }))
}

/// Объявление без приставки видимости: `pub`, `pub(crate)`, `pub(super)`,
/// `pub(in …)`. Перечислять написания — значит отставать от кода при первой же
/// правке видимости.
pub(crate) fn without_visibility(line: &str) -> &str {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix("pub") else { return t };
    let rest = match rest.strip_prefix('(') {
        Some(inner) => match inner.find(')') {
            Some(at) => &inner[at + 1..],
            None => return t,
        },
        None => rest,
    };
    if rest.starts_with(char::is_whitespace) || rest.is_empty() {
        rest.trim_start()
    } else {
        t
    }
}

/// Слово объявления: `fn`, `struct`, `enum`, `type`, `const`, `static`.
/// `async fn` и `unsafe fn` — то же объявление, только с оговоркой впереди.
fn head_of(line: &str) -> Option<&'static str> {
    let mut t = without_visibility(line);
    for prefix in ["async ", "unsafe ", "extern ", "default "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
        }
    }
    ["fn ", "struct ", "enum ", "type ", "const ", "static "]
        .into_iter()
        .find(|word| t.starts_with(*word))
}

/// Настоящий ли держатель: разбор ПОЭЛЕМЕНТНО, а не по файлу.
///
/// Первая редакция решала не тот вопрос дважды.
///
/// `"///".starts_with("//")` — истина, и докблоки выпадали из тела вместе с
/// обычными комментариями. А в этом наборе докблок — ЕДИНСТВЕННЫЙ способ назвать
/// требование в коде: так написан весь домен, так же говорит и правило,
/// вводящее держателя. Семь находок из восьми получали вердикт «помянуто только
/// шапкой модуля», который просто неверен.
///
/// `todo!` считался по ВСЕМУ файлу, а правило говорит «ЭЛЕМЕНТ ПОД ИМЕНЕМ, чьё
/// тело — `todo!`». Разница решающая: порядок этого проекта — проверки до кода,
/// и пока идёт фаза тестов, файл с проверкой ОБЯЗАН содержать `todo!` — это и
/// есть её краснота. Прошли ровно те держатели, у которых проверка лежит в
/// отдельном файле: правило требовало раскладки, которой не требует ни один
/// документ набора.
///
/// Проверка заглушкой не бывает: держатель держится типом ИЛИ проверкой, и
/// элемент под `#[test]`, назвавший требование, — законный держатель, даже если
/// в соседней функции стоит `todo!`.
///
/// `None` — держатель настоящий.
fn holder_verdict(text: Option<&str>, req: &str) -> Option<String> {
    // ПУТИ НЕТ — самая дешёвая форма и самая частая: держатель объявлен, а файла
    // в дереве нет. Прежде она решалась в стороне от остальных трёх, и проверить
    // её было негде.
    let Some(text) = text else {
        return Some("пути нет: держатель объявлен, а файла в дереве не существует".to_owned());
    };
    #[derive(Default)]
    struct Element {
        docs: String,
        attrs: String,
        body: String,
    }
    let mut elements: Vec<Element> = Vec::new();
    let mut cur = Element::default();
    let mut depth = 0i32;
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim_start();
        if !inside {
            // Докблок и пометки копятся ПЕРЕД элементом и принадлежат ему.
            if t.starts_with("///") || t.starts_with("/**") || t.starts_with("*") {
                cur.docs.push_str(line);
                cur.docs.push('\n');
                continue;
            }
            if t.starts_with("#[") {
                cur.attrs.push_str(line);
                cur.attrs.push('\n');
                continue;
            }
            // Шапка модуля объясняет файл и НИЧЕГО не держит.
            if t.starts_with("//!") || t.starts_with("//") || t.is_empty() {
                continue;
            }
            // КОНТЕЙНЕР НЕ ЭЛЕМЕНТ. `impl`, `mod`, `trait` держат в себе другие
            // элементы, и приняв их за один, разбор проглатывал чужой `todo!`:
            // метод с настоящим телом получал вердикт «тело не написано», потому
            // что где-то в том же блоке стоял `todo!` соседа. Спускаемся внутрь,
            // а докблок контейнера ничего не держит — он объясняет блок.
            let container = ["impl ", "impl<", "mod ", "trait "];
            if container.iter().any(|h| without_visibility(t).starts_with(h)) {
                cur = Element::default();
                continue;
            }
            // ВИДИМОСТЬ СНИМАЕТСЯ, А НЕ ПЕРЕЧИСЛЯЕТСЯ. Перечень написаний
            // («pub fn», «pub(crate) fn», «pub(super) fn», …) отстаёт от кода:
            // стоило закрыть видимость в библиотеке, и `pub(crate) fn` выпал из
            // перечня — файл с настоящей работой читался как «без единого
            // элемента», а заглушка соседа доставалась ему в вердикт. Здесь
            // снимается приставка видимости, а дальше смотрится само слово.
            if head_of(t).is_some() {
                inside = true;
                depth = 0;
            } else {
                cur = Element::default();
                continue;
            }
        }
        cur.body.push_str(line);
        cur.body.push('\n');
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        // Элемент без тела (`type`, `const`, объявление в трейте) кончается точкой
        // с запятой на нулевой глубине.
        if inside && (depth <= 0 && (line.contains('}') || line.trim_end().ends_with(';'))) {
            elements.push(std::mem::take(&mut cur));
            inside = false;
        }
    }
    if inside {
        elements.push(cur);
    }
    let named: Vec<&Element> = elements
        .iter()
        .filter(|e| e.docs.contains(req) || e.body.contains(req))
        .collect();
    if elements.is_empty() {
        return Some("файл без единого элемента".to_owned());
    }
    if named.is_empty() {
        // ФОРМ ТРИ, И ВСЕ ТРИ НАЗВАНЫ ПРАВИЛОМ: файл без элементов; требование,
        // названное только шапкой; элемент, чьё тело — `todo!`.
        //
        // «Имени нет в файле вовсе» — четвёртая, и правило её не объявляло.
        // Это слабость самого объявления держателя, а не заглушка, и судить её
        // здесь значило бы завести проверку, которой никто не просил.
        if text.lines().any(|l| l.trim_start().starts_with("//!") && l.contains(req)) {
            return Some("требование помянуто только шапкой модуля `//!`".to_owned());
        }
        return None;
    }
    // Проверка заглушкой не бывает.
    if named.iter().any(|e| e.attrs.contains("#[test]") || e.attrs.contains("#[tokio::test]")) {
        return None;
    }
    if named.iter().all(|e| e.body.contains("todo!") || e.body.contains("unimplemented!")) {
        return Some(format!(
            "тело не написано: все {} элементов, назвавших требование, — `todo!`",
            named.len()
        ));
    }
    None
}

#[cfg(test)]
mod holder {
    use super::holder_verdict;

    /// Порядок этого проекта — проверки до кода: пока идёт фаза тестов, файл с
    /// проверкой ОБЯЗАН содержать `todo!`, и это её краснота, а не заглушка.
    const WITH_WITH_CHECK: &str = r#"
//! Доставка.

/// `TC-ESC-07` (`FR-ESC-06`) — намерение и попытка это разные записи.
#[test]
fn an_intent_and_an_attempt_are_two_records() {
    assert!(true);
}

/// Соседняя функция, кода ещё нет.
pub fn attempt() -> u8 {
    todo!("TC-ESC-09")
}
"#;

    const WITHOUT_CHECKS: &str = r#"
//! Доставка.

/// Здесь держится `FR-ESC-06`.
pub fn attempt() -> u8 {
    todo!("TC-ESC-09")
}
"#;

    const ONLY_HEADER: &str = r#"
//! `FR-ESC-06` — намерение и попытка разные записи.

pub fn unrelated() -> u8 { 1 }
"#;

    #[test]
    fn doc_block_during_check_holds() {
        assert_eq!(holder_verdict(Some(WITH_WITH_CHECK), "FR-ESC-06"), None,
                   "проверка заглушкой не бывает, а чужой `todo!` рядом ничего не значит");
    }

    #[test]
    fn that_same_file_without_checks_red() {
        assert!(holder_verdict(Some(WITHOUT_CHECKS), "FR-ESC-06").is_some());
    }

    #[test]
    fn header_module_nothing_not_holds() {
        let v = holder_verdict(Some(ONLY_HEADER), "FR-ESC-06");
        assert!(v.as_deref().map(|s| s.contains("шапкой")).unwrap_or(false), "{v:?}");
    }

    #[test]
    fn path_missing_is_stub() {
        // Проба из просьбы: держатель на `crates/tot-nowhere/src/lib.rs`.
        let v = holder_verdict(None, "FR-116");
        assert!(v.as_deref().map(|s| s.contains("пути нет")).unwrap_or(false), "{v:?}");
    }

    #[test]
    fn empty_file_is_stub() {
        let v = holder_verdict(Some("//! Только шапка.\n"), "FR-116");
        assert!(v.is_some(), "файл без единого элемента прошёл");
    }

    #[test]
    fn doc_block_not_falls_out_together_with_comment() {
        // Первая редакция роняла `///` фильтром `starts_with("//")`, и требование,
        // названное докблоком, считалось не названным нигде.
        let v = holder_verdict(Some(WITH_WITH_CHECK), "FR-ESC-06");
        assert!(v.is_none(), "докблок снова не читается: {v:?}");
    }
}

#[cfg(test)]
mod secrets {
    use super::{declared_types, facts_of, secret_fields};
    use crate::repo_corpus::Pair;

    const RS: &str = r#"
#[derive(Debug)]
pub struct Target {
    pub url: Option<String>, // адрес вебхука
    pub public_url: PublicUrl,
}

#[derive(Debug)]
pub struct PublicUrl(String);

#[derive(Debug)]
pub struct Issued {
    pub token: Option<crate::StoredToken>,
    pub token_hash: Secret<String>,
}

#[derive(Debug)]
pub struct StoredToken {
    pub token_hash: Secret<String>,
}

#[derive(Debug)]
pub struct Keys {
    pub signing_key: Vec<u8>,
    pub private_key: [u8; 32],
    pub token: Uuid,
    pub api_key: Arc<str>,
    pub password: std::string::String,
    pub secret: HashMap<String, String>,
    pub webhook_url: reqwest::Url,
    pub credential: serde_json::Value,
}
"#;

    const NAMES: &str = r"(?i)(^|_)(password|token|secret|url|credential)($|_)|(^|_)(api|private|secret|signing)_key($|_)";

    #[test]
    fn suspicious_everything_except_secret_and_type_corpus() {
        let found: Vec<String> = secret_fields(RS, NAMES, &declared_types(RS))
            .into_iter()
            .map(|(decl, field, ty)| format!("{decl}.{field}: {ty}"))
            .collect();
        assert_eq!(found, vec![
            "Target.url: Option<String>",
            "Keys.signing_key: Vec<u8>",
            "Keys.private_key: [u8; 32]",
            "Keys.token: Uuid",
            "Keys.api_key: Arc<str>",
            "Keys.password: std::string::String",
            "Keys.secret: HashMap<String, String>",
            "Keys.webhook_url: reqwest::Url",
            "Keys.credential: serde_json::Value",
        ]);
    }

    #[test]
    fn type_corpus_recognised_bare_name_or_path_crate() {
        let rs = "#[derive(Debug)]\npub struct Url(pub String);\n\n#[derive(Debug)]\npub struct Hook {\n    pub webhook_url: reqwest::Url,\n    pub callback_url: Option<Url>,\n    pub public_url: crate::Url,\n}\n";
        let found: Vec<String> = secret_fields(rs, NAMES, &declared_types(rs)).into_iter().map(|(_, f, _)| f).collect();
        assert_eq!(found, vec!["webhook_url"]);
        let declared = vec!["Url".to_owned(), "Link".to_owned(), "PublicUrl".to_owned()];
        for rs in ["use reqwest::Url;\n\n#[derive(Debug)]\npub struct Hook {\n    pub webhook_url: Url,\n    pub public_url: PublicUrl,\n}\n",
                   "use reqwest::{Client, Url as Link};\n\n#[derive(Debug)]\npub struct Hook {\n    pub webhook_url: Link,\n    pub public_url: crate::PublicUrl,\n}\n"] {
            let found: Vec<String> = secret_fields(rs, NAMES, &declared).into_iter().map(|(_, f, _)| f).collect();
            assert_eq!(found, vec!["webhook_url"], "{rs}");
        }
    }

    #[test]
    fn type_not_from_corpus_suspicion_not_clears() {
        let found = secret_fields(RS, NAMES, &[]);
        assert!(found.iter().any(|(d, f, _)| d == "Issued" && f == "token"), "{found:?}");
        assert!(found.iter().any(|(d, f, _)| d == "Target" && f == "public_url"), "{found:?}");
    }

    #[test]
    fn field_bool_secret_not_carries() {
        let rs = "#[derive(Debug)]\npub enum Platform {\n    Windows { restricted_token: bool },\n}\n\n#[derive(Debug)]\npub struct Keys {\n    pub token_ttl: u64,\n    pub token: String,\n}\n";
        let found: Vec<String> = secret_fields(rs, NAMES, &[]).into_iter().map(|(_, f, _)| f).collect();
        assert_eq!(found, vec!["token_ttl", "token"]);
    }

    #[test]
    fn marked_not_displaces_finding_that_same_name() {
        let p = |name: &str, detail: &str| Pair { name: name.into(), detail: detail.into() };
        let facts = facts_of(&[
            p("Target", "перечисление без множества CHECK (a.rs), значений 2"),
            p("Target", "помечено x-derived: computed"),
            p("Operator", "помечено x-stored-in: monitors.condition"),
        ]);
        let got: Vec<(&str, &str)> =
            facts.iter().map(|f| (f["name"].as_str().unwrap_or(""), f["detail"].as_str().unwrap_or(""))).collect();
        assert_eq!(got, vec![
            ("Target", "перечисление без множества CHECK (a.rs), значений 2"),
            ("Operator", "помечено x-stored-in: monitors.condition"),
        ]);
    }
}

#[cfg(test)]
mod install_tests {
    use super::installed_head;

    #[test]
    fn only_installer_heads_are_ours() {
        assert!(installed_head("---\nname: godzy-gate\ndescription: \"Check a gate\"\n---\n", "godzy-gate"));
        assert!(!installed_head("---\nname: godzy-gate\ndescription: Check a gate\n---\n", "godzy-gate"), "свой файл проекта без кавычек");
        assert!(!installed_head("---\nname: other\ndescription: \"x\"\n---\n", "godzy-gate"), "имя в шапке — другого скилла");
        assert!(!installed_head("# notes", "godzy-gate"));
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    /// Код ответа — часть смысла: «занято» и «дверь отказала» приходят разными
    /// телами, и цикл повторов у агента по телу их не различал.
    #[test]
    fn busyness_is_marked_from_the_code() {
        let busy = parse("{\"error\":\"busy\",\"message\":\"сервер занят\"}\n503".as_bytes()).expect("тело разобрано");
        assert!(crate::door::busy_said(&busy));
        let refusal = parse("{\"error\":\"upstream_error\"}\n502".as_bytes()).expect("тело разобрано");
        assert!(!crate::door::busy_said(&refusal), "отказ по существу занятостью не помечается");
        // Край отвечает своей строкой, и она не предмет: пометка по такому
        // телу роняла клиента ровно в час перегрузки.
        let edge = parse("\"service unavailable\"\n503".as_bytes()).expect("тело разобрано");
        assert!(crate::door::busy_said(&edge));
        assert_eq!(edge["message"], serde_json::json!("service unavailable"));
        let answer = parse("{\"count\":3}\n200".as_bytes()).expect("тело разобрано");
        assert_eq!(answer["count"], serde_json::json!(3));
        assert!(!crate::door::busy_said(&answer));
    }
}

#[cfg(test)]
mod heads {
    use super::{head_of, holder_verdict, without_visibility};

    /// Приставка видимости СНИМАЕТСЯ. Перечень написаний отставал от кода: как
    /// только библиотека закрыла видимость, `pub(crate) fn` перестал быть
    /// элементом — файл с настоящей работой читался «без единого элемента», а
    /// заглушка соседа доставалась ему в вердикт. Оба вранья из одной строки.
    #[test]
    fn visibility_is_stripped_not_listed() {
        assert_eq!(without_visibility("pub(crate) fn a() {"), "fn a() {");
        assert_eq!(without_visibility("    pub(super) async fn b() {"), "async fn b() {");
        assert_eq!(without_visibility("pub(in crate::corpus) struct C {"), "struct C {");
        assert_eq!(without_visibility("pub fn d() {"), "fn d() {");
        assert_eq!(without_visibility("fn e() {"), "fn e() {");
        // `publisher` — не видимость: имя, начинающееся на «pub».
        assert_eq!(without_visibility("publisher() {"), "publisher() {");
        assert_eq!(head_of("pub(crate) async fn f() {"), Some("fn "));
        assert_eq!(head_of("pub(crate) const G: u8 = 1;"), Some("const "));
        assert_eq!(head_of("let x = 1;"), None);
    }

    #[test]
    fn a_closed_visibility_element_is_still_an_element() {
        let has_work = "\
/// Держит FR-X-01.
pub(crate) fn real(a: u8) -> u8 {
    a + 1
}
";
        assert_eq!(holder_verdict(Some(has_work), "FR-X-01"), None, "работа есть — вердикта нет");
        let stub_body = "\
/// Держит FR-X-04.
pub(crate) fn stub() {
    todo!()
}
";
        let verdict = holder_verdict(Some(stub_body), "FR-X-04").expect("заглушка называется");
        assert!(verdict.contains("тело не написано"), "{verdict}");
    }
}

/// Состояния задач из летописи: что сказал самый свежий коммит, назвавший её.
///
/// ЗАКРЫТА ЛИ ЗАДАЧА В СТВОЛЕ — вопрос о ЗАДАЧЕ, а не о коммите. Сначала здесь
/// стояло «трейлер не в стволе — значит `claimed`», и это отвечало на другой
/// вопрос: лежит ли в стволе ИМЕННО ЭТОТ коммит.
///
/// У набора `tot-ade` 19 сентября ветку по умолчанию на двадцать минут
/// переключили на `night/gates-f4`, туда уехал PR, и его слияние стало самым
/// свежим коммитом, назвавшим десять задач. Работа при этом давно лежала в
/// стволе — `M1-T5` закрыт коммитом `82ec5f6` в 12:45, — но свежим оказался
/// `ada913f` из чужой ветки, и все десять получили `claimed`. Отсюда же 78
/// находок пункта `task-tree-op-matches-disk`: он судит НЕзакрытые задачи, а
/// они перестали числиться закрытыми, и с ними встала фаза Ф4.
///
/// Поэтому `closed` понижается до `claimed` только тогда, когда задачу не
/// закрывает НИ ОДИН коммит ствола.
fn trailer_states(
    log: &str,
    in_product: &std::collections::HashSet<String>,
    rex: &regex::Regex,
) -> Vec<Value> {
    let split = |e: &str| {
        let mut parts = e.splitn(3, '\u{0}');
        let commit = parts.next().unwrap_or("").trim().to_owned();
        let at: i64 = parts.next().unwrap_or("").trim().parse::<i64>().unwrap_or(0) * 1000;
        let body = parts.next().unwrap_or("").to_owned();
        (commit, at, body)
    };
    let closed_in_product: std::collections::HashSet<String> = log
        .split('\u{1}')
        .map(&split)
        .filter(|(commit, _, _)| in_product.contains(commit))
        .flat_map(|(_, _, body)| {
            rex.captures_iter(&body)
                .filter_map(|c| {
                    let id = c.get(1).map(|m| m.as_str().trim().to_owned())?;
                    let state = c.get(2).map(|m| m.as_str().trim()).unwrap_or("closed");
                    (state == "closed" && !id.is_empty()).then_some(id)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    // Состояние берётся у САМОГО СВЕЖЕГО коммита, назвавшего задачу: `git log`
    // идёт от новых к старым, и первый ответ — последнее слово.
    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut states: Vec<Value> = Vec::new();
    for e in log.split('\u{1}') {
        let (commit, at, body) = split(e);
        for c in rex.captures_iter(&body) {
            let id = c.get(1).map(|m| m.as_str().trim().to_owned()).unwrap_or_default();
            let state = c.get(2).map(|m| m.as_str().trim().to_owned())
                .unwrap_or_else(|| "closed".to_owned());
            if id.is_empty() || !seen.insert(id.clone()) {
                continue;
            }
            let state = if state == "closed" && !closed_in_product.contains(&id) {
                "claimed".to_owned()
            } else {
                state
            };
            states.push(json!({ "id": id, "state": state, "commit": commit, "at": at }));
        }
    }
    states
}

#[cfg(test)]
mod trailers {
    use super::trailer_states;

    /// Летопись в том же виде, в каком её отдаёт `git log`: записи через \u{1},
    /// поля внутри записи через \u{0}.
    fn log(rows: &[(&str, i64, &str)]) -> String {
        rows.iter()
            .map(|(commit, at, body)| format!("{commit}\u{0}{at}\u{0}{body}"))
            .collect::<Vec<_>>()
            .join("\u{1}")
    }

    fn rex() -> regex::Regex {
        regex::Regex::new(r"Task:\s*([A-Za-z0-9-]+)\s+(\w+)").expect("образец трейлера")
    }

    fn state_of(states: &[serde_json::Value], id: &str) -> String {
        states
            .iter()
            .find(|s| s["id"] == id)
            .map(|s| s["state"].as_str().unwrap_or("").to_owned())
            .unwrap_or_else(|| "нет".to_owned())
    }

    /// Случай `tot-ade` 19 сентября: работа лежит в стволе с обеда, а самым
    /// свежим коммитом, назвавшим задачу, оказалось слияние в чужую ветку —
    /// ветку по умолчанию на двадцать минут переключили. Задача закрыта, и
    /// свежесть чужого коммита этого не отменяет.
    #[test]
    fn a_newer_commit_off_the_mainline_does_not_unclose_a_task() {
        let l = log(&[
            ("ada913f", 1789833840, "M1-T9 (#36)\n\nTask: M1-T5 closed"),
            ("82ec5f6", 1789822716, "M1-T5 · блоки\n\nTask: M1-T5 closed"),
        ]);
        let product = ["82ec5f6".to_owned()].into_iter().collect();
        assert_eq!(state_of(&trailer_states(&l, &product, &rex()), "M1-T5"), "closed");
    }

    /// Обратное держится: работы в стволе нет вовсе, и доска не имеет права
    /// говорить «сделано». Это тот случай, ради которого `claimed` и заведён.
    #[test]
    fn a_task_closed_only_on_a_branch_stays_claimed() {
        let l = log(&[("beefbee", 1789833840, "черновик\n\nTask: M3-T7 closed")]);
        let product = ["82ec5f6".to_owned()].into_iter().collect();
        assert_eq!(state_of(&trailer_states(&l, &product, &rex()), "M3-T7"), "claimed");
    }

    /// Последнее слово остаётся за самым свежим коммитом: переоткрытая задача
    /// не становится закрытой оттого, что в стволе лежит её прежнее закрытие.
    #[test]
    fn the_newest_commit_still_has_the_last_word() {
        let l = log(&[
            ("cafe777", 1789833840, "вернули в работу\n\nTask: M1-T5 reopened"),
            ("82ec5f6", 1789822716, "M1-T5 · блоки\n\nTask: M1-T5 closed"),
        ]);
        let product = ["82ec5f6".to_owned(), "cafe777".to_owned()].into_iter().collect();
        assert_eq!(state_of(&trailer_states(&l, &product, &rex()), "M1-T5"), "reopened");
    }
}
