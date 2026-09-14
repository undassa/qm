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
    pub fn secret_file() -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join(".config/mh/edge-secret")
    }

    fn body(&self, path: &str, args: Option<&Value>) -> Result<Value, String> {
        // Разговор ведёт `curl`, а не своя реализация HTTP: одна зависимость на
        // клиента, которого и так носят по чужим машинам, дороже той пользы,
        // что она даёт. Тело подаётся через stdin — доводы бывают длиннее, чем
        // выдерживает список аргументов, и подача фактов на этом уже спотыкалась.
        let mut cmd = std::process::Command::new("curl");
        cmd.arg("-sS")
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
        let inner = envelope
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .and_then(|t| serde_json::from_str::<Value>(t).ok()
                .or_else(|| Some(json!({ "why": t }))));
        Ok((inner.unwrap_or(envelope), refused))
    }

    /// Перечень ручек — у сервера, а не свой.
    pub fn tools(&self) -> Result<Value, String> {
        let path = format!("/api/projects/{}/tools", self.project);
        self.body(&path, None)
    }
}

fn parse(bytes: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        // Пустой ответ — не пустой набор. Сервер мог не подняться, край мог не
        // пустить; назвать это «ничего не нашлось» значит соврать в ту сторону,
        // в которую врать нельзя.
        return Err("сервер ответил пустотой: он поднят и край пускает?".into());
    }
    serde_json::from_str(&text).map_err(|e| format!("ответ не разбирается: {e}; было: {}",
                                                    text.chars().take(200).collect::<String>()))
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let rex = regex::Regex::new(re).map_err(|e| format!("{fact}: образец трейлера не разбирается: {e}"))?;
            // Состояние берётся у САМОГО СВЕЖЕГО коммита, назвавшего задачу:
            // `git log` идёт от новых к старым, и первый ответ — последнее слово.
            let mut seen: std::collections::HashSet<String> = Default::default();
            let mut states: Vec<Value> = Vec::new();
            for entry in log.split('\u{1}') {
                let mut parts = entry.splitn(3, '\u{0}');
                let commit = parts.next().unwrap_or("").trim().to_owned();
                let at: i64 = parts.next().unwrap_or("").trim().parse::<i64>().unwrap_or(0) * 1000;
                let body = parts.next().unwrap_or("");
                for c in rex.captures_iter(body) {
                    let id = c.get(1).map(|m| m.as_str().trim().to_owned()).unwrap_or_default();
                    let state = c.get(2).map(|m| m.as_str().trim().to_owned())
                        .unwrap_or_else(|| "closed".to_owned());
                    if id.is_empty() || !seen.insert(id.clone()) {
                        continue;
                    }
                    states.push(json!({ "id": id, "state": state, "commit": commit, "at": at }));
                }
            }
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
                let dirty = std::process::Command::new("git")
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
                } else if !dirty.is_empty() {
                    format!("незакоммиченные правки: {}", dirty.lines().count())
                } else {
                    format!("стоит на {now}")
                };
                names.push((path, detail));
            }
            let facts: Vec<Value> = names.iter()
                .map(|(n, d)| json!({ "name": n, "detail": d }))
                .collect();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
                } else if short.ends_with(".sql") {
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
            let pairs = crate::repo_corpus::contract_vs_schema(&doc, &tables, &forced);
            let facts: Vec<Value> = pairs
                .iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
                } else if short.ends_with(".sql") {
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
                    .filter(|f| f.ends_with(".sql"))
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
            let facts: Vec<Value> = pairs
                .iter()
                .map(|p| json!({ "name": p.name, "detail": p.detail }))
                .collect();
            let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
            done.push(json!({ "fact": fact, "files": files.len(), "found": facts.len(),
                              "was": out["was"], "now": out["now"] }));
            continue;
        }
        let mut names: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
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
                for (decl, field, ty) in secret_fields(&text, re) {
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
        let (out, _) = door.call("code-facts-push", &json!({ "kind": fact, "facts": facts }))?;
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
/// `field_re` — чем узнаётся секрето-подобное имя; подозрение снимает тип, не
/// являющийся сырой строкой (`Secret<…>`, `StoredToken`, newtype), потому что он
/// и есть ответ на него.
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

fn raw_string(ty: &str) -> bool {
    let mut t = ty.trim();
    while let Some(inner) = ["Option<", "Box<", "Vec<"]
        .iter()
        .find_map(|w| t.strip_prefix(w)?.strip_suffix('>'))
    {
        t = inner.trim();
    }
    if let Some(r) = t.strip_prefix('&') {
        t = r.trim_start();
        if t.starts_with('\'') {
            t = t.split_once(' ').map(|(_, r)| r.trim()).unwrap_or("");
        }
    }
    matches!(t, "String" | "str" | "Url" | "url::Url" | "Uri") || (t.starts_with("Cow<") && t.ends_with("str>"))
}

fn secret_fields(text: &str, field_re: &str) -> Vec<(String, String, String)> {
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
                        if suspect.is_match(&fname) && raw_string(&ftype) {
                            out.push((owner.clone(), fname, ftype));
                        }
                    }
                }
            } else if let Some(f) = field.captures(lines[k]) {
                let (fname, ftype) = (f[1].to_owned(), bare_type(&f[2]));
                if suspect.is_match(&fname) && raw_string(&ftype) {
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
        "Их источник — сервер; обновляются командой `mh install .`, руками не правятся.\n",
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
    struct Элемент {
        docs: String,
        attrs: String,
        body: String,
    }
    let mut elements: Vec<Элемент> = Vec::new();
    let mut cur = Элемент::default();
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
            let container = ["impl ", "impl<", "mod ", "pub mod ", "trait ", "pub trait "];
            if container.iter().any(|h| t.starts_with(h)) {
                cur = Элемент::default();
                continue;
            }
            let head = ["fn ", "pub fn ", "struct ", "pub struct ", "enum ", "pub enum ",
                        "type ", "pub type ", "const ", "static ",
                        "async fn ", "pub async fn ", "pub(crate) fn "];
            if head.iter().any(|h| t.starts_with(h)) {
                inside = true;
                depth = 0;
            } else {
                cur = Элемент::default();
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
    let named: Vec<&Элемент> = elements
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
mod держатель {
    use super::holder_verdict;

    /// Порядок этого проекта — проверки до кода: пока идёт фаза тестов, файл с
    /// проверкой ОБЯЗАН содержать `todo!`, и это её краснота, а не заглушка.
    const С_ПРОВЕРКОЙ: &str = r#"
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

    const БЕЗ_ПРОВЕРКИ: &str = r#"
//! Доставка.

/// Здесь держится `FR-ESC-06`.
pub fn attempt() -> u8 {
    todo!("TC-ESC-09")
}
"#;

    const ТОЛЬКО_ШАПКА: &str = r#"
//! `FR-ESC-06` — намерение и попытка разные записи.

pub fn unrelated() -> u8 { 1 }
"#;

    #[test]
    fn докблок_при_проверке_держит() {
        assert_eq!(holder_verdict(Some(С_ПРОВЕРКОЙ), "FR-ESC-06"), None,
                   "проверка заглушкой не бывает, а чужой `todo!` рядом ничего не значит");
    }

    #[test]
    fn тот_же_файл_без_проверки_красен() {
        assert!(holder_verdict(Some(БЕЗ_ПРОВЕРКИ), "FR-ESC-06").is_some());
    }

    #[test]
    fn шапка_модуля_ничего_не_держит() {
        let v = holder_verdict(Some(ТОЛЬКО_ШАПКА), "FR-ESC-06");
        assert!(v.as_deref().map(|s| s.contains("шапкой")).unwrap_or(false), "{v:?}");
    }

    #[test]
    fn пути_нет_это_заглушка() {
        // Проба из просьбы: держатель на `crates/tot-nowhere/src/lib.rs`.
        let v = holder_verdict(None, "FR-116");
        assert!(v.as_deref().map(|s| s.contains("пути нет")).unwrap_or(false), "{v:?}");
    }

    #[test]
    fn пустой_файл_это_заглушка() {
        let v = holder_verdict(Some("//! Только шапка.\n"), "FR-116");
        assert!(v.is_some(), "файл без единого элемента прошёл");
    }

    #[test]
    fn докблок_не_выпадает_вместе_с_комментарием() {
        // Первая редакция роняла `///` фильтром `starts_with("//")`, и требование,
        // названное докблоком, считалось не названным нигде.
        let v = holder_verdict(Some(С_ПРОВЕРКОЙ), "FR-ESC-06");
        assert!(v.is_none(), "докблок снова не читается: {v:?}");
    }
}

#[cfg(test)]
mod секреты {
    use super::secret_fields;

    const RS: &str = r#"
#[derive(Debug)]
pub struct Target {
    pub url: Option<String>, // адрес вебхука
    pub public_url: PublicUrl,
}

#[derive(Debug)]
pub struct Issued {
    pub token: StoredToken,
    pub token_hash: Secret<String>,
}
"#;

    #[test]
    fn подозрительна_только_сырая_строка() {
        let found = secret_fields(RS, r"(?i)(^|_)(url|token)($|_)");
        assert_eq!(found, vec![("Target".to_owned(), "url".to_owned(), "Option<String>".to_owned())]);
    }
}
