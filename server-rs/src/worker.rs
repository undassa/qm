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
/// Как часто харнес ЗАГЛЯДЫВАЕТ в ствол (заявка 20).
///
/// СНИМОК ИДЁТ ЗА СТВОЛОМ, А НЕ ПО ЧАСАМ, И ЭТО НЕ НАСТРОЙКА ТЕМПА.
///
/// Час между заглядываниями создавал окно, в котором прибор НЕ МОГ увидеть
/// того, что обязан. Красная фаза доказывается прогоном, где зеркало падало;
/// зеркало и тело садятся за минуты. Замер 21.09: `V5-T32` влита в 05:16,
/// `M5-T32` в 06:20, прохода между ними не было; `V4-T8` в 15:51, `M4-T8` в
/// 16:07, проходы стояли в 15:20 и 16:16. Шесть находок «не наблюдалась
/// красной» не снимались НИКАКОЙ работой набора — прибор требовал
/// наблюдения, которого сам не имел возможности сделать.
///
/// Пять минут стоят почти ничего: заход, у которого вершина уже измерена,
/// кончается на `git rev-parse` и одном счёте в базе. Прогон идёт только на
/// НОВОЙ вершине.
pub const TEST_TICK_S: u64 = 300;
/// Через сколько вершину перемеряют, даже если она не менялась.
///
/// Не украшение: прогон срывается и по причинам, к дереву отношения не
/// имеющим. 21.09 три падения подряд дал переполненный диск, а не код. Без
/// повтора такой срыв стоял бы записью о вершине навсегда, и отличить «тут
/// правда красно» от «в тот раз не собралось» было бы нечем.
pub const TEST_REDO_S: u64 = 6 * 3600;
const TEST_TIMEOUT_S: u64 = 1800;
const LIMIT_S: u64 = 1800;
const SILENCE_LIMIT: i32 = 3;
const CIRCLES: i32 = 5; // потолок кругов починки — правило харнеса

static GH_STAMP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z ").unwrap());

static ANSI: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap());

static MARKER: Lazy<Regex> = Lazy::new(|| {
    // Маркеры контракта дословны (заглавными). Порядок — по тяжести: DRIFT
    // встаёт всегда, NEEDS FIX важнее случайного слова PLAN в прозе.
    Regex::new(r"\b(DRIFT|NEEDS FIX|RETHINK|PLAN|BLOCKED|NEEDS CONTEXT|CLOSED)\b").unwrap()
});


/// Голова дерева из события «одобрение» — к какому состоянию дерева привязано
/// одобрение владельца. Чистая функция: привязку проверяют тесты без базы.
pub fn approval_head(events: &[Value]) -> Option<String> {
    for e in events {
        if e["kind"].as_str() == Some("одобрение") {
            if let Some(rest) = e["text"].as_str().and_then(|t| t.strip_prefix("head ")) {
                return Some(rest.trim().to_owned());
            }
        }
    }
    None
}

/// Обрезка по границе символа: срез по байтам падает на кириллице, а ответы
/// сессий и заметки — кириллические. Паника здесь уносила бы воркера на
/// первом же длинном отчёте.
pub fn cut(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Clone, Deserialize, Default)]
pub struct TestSpec {
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub dir: String,
    /// Красный профиль: команда, которой набор гонит зеркала. Пусто — прежнее
    /// поведение, один обычный прогон.
    #[serde(default)]
    pub red: String,
}

#[derive(Clone, Deserialize)]
pub struct Bundle {
    pub name: String,
    pub project: String,
    pub repo: String,
    /// Чем харнес гоняет тесты набора (заявка 20): команда — из конфигурации
    /// набора, а не из угла сессии. Без команды прогонов нет — и пункты
    /// про ствол честно отвечают «неизвестно».
    #[serde(default)]
    pub test: Option<TestSpec>,
    /// Вести ли прогоны задач автоматом. Набор, чьи задачи ведут живые сессии
    /// разработки, отказывается: иначе снятая остановка конвейера подняла бы
    /// второго исполнителя на каждую задачу, уже взятую сессией (tot-ade,
    /// 2026-09-30: семь прогонов в «running» ждали только снятия DRIFT). Беседы,
    /// прогоны тестов и журналы CI от этого не зависят.
    #[serde(default = "drives_by_default")]
    pub drive: bool,
}

fn drives_by_default() -> bool {
    true
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

/// Поле ответа двери — и ОТСУТСТВИЕ ЕГО НАЗЫВАЕТСЯ ВСЛУХ.
///
/// `serde_json` на несуществующий ключ отдаёт `Null`, а `Null` послушно
/// становится нулём, пустым перечнем и пустой строкой. Читатель, написанный
/// по памяти о форме ответа, молчит вместо отказа.
///
/// Трижды за одну смену: `fresh` вместо `stale` у датчиков, `was`/`now`
/// вместо `accepted` у подачи состояний, `done` вместо `sensed` у съёма.
/// Последний написал в журнал «снято 0» при тридцати снятых — и это читалось
/// как «механизм не работает».
///
/// Потолок: ловит только там, где позвали. Типизировать ответы дверей — та
/// правка, которая закрыла бы класс целиком, и она больше этой.
fn fld<'a>(v: &'a Value, key: &str, who: &str) -> &'a Value {
    if v.get(key).is_none() {
        println!("{who}: в ответе двери нет поля `{key}` — читают не то, что отвечают");
    }
    &v[key]
}

/// Строки прогона — из текста, который напечатал `cargo test` либо nextest.
///
/// ОДНА ФУНКЦИЯ НА ВСЕ ИСТОЧНИКИ, И ЭТО НЕ ЭКОНОМИЯ. Прогон харнеса и журнал
/// работы GitHub Actions печатают одно и то же, и разбор, списанный копией,
/// однажды разошёлся бы с оригиналом: `red-observed-failing` судит по имени
/// проверки, и имя, вынутое иначе, — это другая проверка.
///
/// Журнал работы GitHub предваряет каждую строку временем
/// (`2026-09-29T22:56:47.5896990Z test … ... FAILED`), а первую ещё и BOM;
/// без их снятия `strip_prefix("test ")` не узнаёт ни одной строки, и
/// разбор отвечает «проверок ноль» — неотличимо от несобравшегося. Цвет
/// снимается по той же причине: задание с `CARGO_TERM_COLOR=always` печатает
/// `test x ... \e[31mFAILED\e[0m`, и вердикт не узнавался.
pub fn test_lines(text: &str) -> Vec<(String, String, String)> {
    let plain = ANSI.replace_all(text, "");
    let lines: Vec<&str> = plain
        .lines()
        .map(|l| {
            let l = l.trim_start_matches('\u{feff}');
            GH_STAMP.find(l).map_or(l, |m| &l[m.end()..])
        })
        .collect();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    // ДВА ФОРМАТА, И ГЛАВНЫЙ — NEXTEST.
    //
    // «Упало на стволе» определяет не харнес, а сам набор своим рецептом:
    // `just test` у `tot-ade` — это `cargo nextest ... -E 'not
    // binary(/^mirror_/)'`. Прогонщик, меряющий другой командой, изобретает
    // красноту, которой у набора нет: под `cargo test` проверки делят один
    // процесс, и `the_door_counts_what_is_born_deeper_in_the_tree` ложно
    // краснела — её собственная шапка это и говорит, с замером.
    //
    // У nextest связь «проверка ↔ бинарь» — ОДНА СТРОКА, а не порядок строк:
    //     PASS [0.002s] (1/313) tot-core::mirror_foo s8_ac_6_имя
    // Склеить её нечем, и разбору нечего терять между потоками.
    for line in &lines {
        let t = line.trim_start();
        let Some((head, rest)) = t.split_once(char::is_whitespace) else { continue };
        let verdict = match head {
            "PASS" => "passed",
            "FAIL" => "failed",
            "SKIP" => "ignored",
            _ => continue,
        };
        // Хвост после `[время]` и необязательного `(n/N)`: два слова —
        // «крейт::бинарь» и имя проверки.
        let tail = rest.rsplit(']').next().unwrap_or("").trim_start();
        let tail = tail.strip_prefix('(').map_or(tail, |x| {
            x.split_once(')').map_or(tail, |(_, after)| after.trim_start())
        });
        let mut parts = tail.split_whitespace();
        let (Some(binary), Some(name)) = (parts.next(), parts.next()) else { continue };
        if parts.next().is_some() {
            continue;
        }
        rows.push((
            name.to_owned(),
            verdict.to_owned(),
            binary.rsplit("::").next().unwrap_or(binary).to_owned(),
        ));
    }
    // ЧЕМ ШЛА ПРОВЕРКА. `cargo test` печатает `Running tests/<файл>.rs (…)`
    // перед блоком каждого бинаря. Прежде разбор эту строку пропускал, и
    // «упала» становилось неотличимо от «обязана падать»: зеркало красной
    // фазы падает по построению, а пункт про ствол звал это поломкой.
    //
    // Раннер Windows печатает путь обратной чертой — `Running
    // tests\mirror_revoke_takes_the_refusals_off.rs` (прогон `36642293742`), —
    // и по одной `/` бинарём стал бы весь путь: граница зеркал `mirror_`
    // сравнивает начало имени и такой бинарь не узнала бы.
    let mut binary = String::new();
    for line in &lines {
        if let Some(rest) = line.trim_start().strip_prefix("Running ") {
            binary = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_owned();
            continue;
        }
        let rest = line.strip_prefix("test ").unwrap_or("");
        let Some((name, tail)) = rest.split_once(" ... ") else { continue };
        let name = name.trim();
        if name.is_empty() || name == "result:" {
            continue;
        }
        let verdict = if tail.starts_with("ok") {
            "passed"
        } else if tail.starts_with("FAILED") {
            "failed"
        } else if tail.starts_with("ignored") {
            "ignored"
        } else {
            continue;
        };
        rows.push((name.to_owned(), verdict.to_owned(), binary.clone()));
    }
    rows
}

/// Сколько группа просроченной команды получает на выход после TERM, прежде
/// чем её добьют KILL.
///
/// СНАЧАЛА TERM, И ЭТО НЕ ВЕЖЛИВОСТЬ. nextest держит каждый тест в своей
/// группе процессов и гасит их сам, получив TERM; KILL ему этого не даёт, и
/// зависший тест оставался бы сиротой вне нашей группы — с тем же замком
/// каталога сборки, ради которого команду и убивают. Пятнадцать секунд —
/// потому что nextest, получив TERM, сам ждёт свои тесты около десяти, прежде
/// чем убить их; добей мы его раньше, его тесты остались бы сиротами.
const TERM_GRACE_S: u64 = 15;

/// Вывод команды с потолком времени; `None` — потолок вышел. `input` уходит
/// потомку на ввод; пустой — ввода нет вовсе.
///
/// ПО ИСТЕЧЕНИИ ГАСИТСЯ ВСЯ ГРУППА ПРОЦЕССОВ, А НЕ ТОЛЬКО ЗАПУЩЕННЫЙ.
/// Брошенный `tokio::time::timeout` будущий вывод процесса не убивает: cargo
/// жил бы дальше и держал замок каталога сборки, и следующий прогон ждал бы
/// его. Одного `kill_on_drop` мало: профиль идёт через `bash -c "( … )"`, и
/// убит был бы только bash, а cargo под ним осиротел бы. Поэтому своя группа
/// и `put_out` по ней.
///
/// Без ввода — `Stdio::null()`: `spawn()` наследует ввод воркера, и при
/// запуске из терминала потомок в фоновой группе ловил бы SIGTTIN.
///
/// Отказ чтения вывода — отказ, а не пустой вывод: обрезанный вывод лёг бы
/// записью как обычный прогон.
async fn output_within(
    mut cmd: tokio::process::Command,
    limit_s: u64,
    input: &[u8],
) -> std::io::Result<Option<std::process::Output>> {
    let stdin = if input.is_empty() { Stdio::null() } else { Stdio::piped() };
    cmd.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).kill_on_drop(true);
    let mut child = cmd.spawn()?;
    // Номер группы — сейчас, до ожидания: после `wait()` tokio номера
    // потомка не отдаёт, а вышедший вожак может оставить в группе тех, кто
    // держит его вывод (claude кончился, его фоновая команда жива).
    let group = child.id();
    let (mut out, mut err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let mut feed = child.stdin.take();
    let done = async {
        if let Some(pipe) = feed.as_mut() {
            pipe.write_all(input).await?;
        }
        // Ввод закрывается сразу: потомок, читающий его до конца, иначе ждал бы.
        drop(feed.take());
        let status = child.wait().await?;
        Ok(std::process::Output {
            status,
            stdout: (&mut out).await.map_err(std::io::Error::other)??,
            stderr: (&mut err).await.map_err(std::io::Error::other)??,
        })
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(limit_s), done).await;
    // Трубы читаются, пока их держит хоть кто-то, а держать их может и
    // процесс, ушедший из группы: без `abort` воркер ждал бы его вечно.
    out.abort();
    err.abort();
    match result {
        Ok(Ok(output)) => Ok(Some(output)),
        Ok(Err(e)) => {
            put_out(&mut child, group, std::time::Duration::from_secs(TERM_GRACE_S)).await;
            Err(e)
        }
        Err(_) => {
            put_out(&mut child, group, std::time::Duration::from_secs(TERM_GRACE_S)).await;
            Ok(None)
        }
    }
}

/// Погасить группу `group`, которую потомок завёл при запуске
/// (`process_group(0)`): TERM, а если через `grace` она жива — KILL.
/// `child` — её вожак, его пожинаем по ходу.
async fn put_out(child: &mut tokio::process::Child, group: Option<u32>, grace: std::time::Duration) {
    let Some(group) = group else { return };
    let signal = |sig: &str| {
        std::process::Command::new("kill")
            .args([sig, "--", &format!("-{group}")])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    signal("-TERM");
    let deadline = std::time::Instant::now() + grace;
    // Вожака пожинаем сами: зомби числится в группе, и без этого группа
    // выглядела бы живой до конца отсрочки.
    while child.try_wait().is_ok() && signal("-0") {
        if std::time::Instant::now() >= deadline {
            if !signal("-KILL") {
                println!("группа {group} после потолка не добита");
            }
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Прочесть трубу потомка до конца в своей задаче.
fn drain<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    pipe: Option<R>,
) -> tokio::task::JoinHandle<std::io::Result<Vec<u8>>> {
    tokio::spawn(async move {
        let mut b = Vec::new();
        if let Some(mut p) = pipe {
            tokio::io::AsyncReadExt::read_to_end(&mut p, &mut b).await?;
        }
        Ok(b)
    })
}

/// `gh api <путь>` — с отказом словом, а не пустой строкой: пустой ответ
/// разобрался бы в «проверок ноль» и лёг отметкой «прогон взят».
async fn gh(path: &str) -> Result<String, String> {
    let mut run = tokio::process::Command::new("gh");
    run.args(["api", path]);
    let out = match output_within(run, 120, b"").await {
        Ok(Some(o)) => o,
        Ok(None) => return Err(format!("gh api {path} не уложился в 120 с")),
        Err(e) => return Err(format!("gh не завёлся: {e}")),
    };
    if !out.status.success() {
        return Err(format!("gh api {path} отказал: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn gh_json(path: &str) -> Result<Value, String> {
    serde_json::from_str(&gh(path).await?).map_err(|e| format!("gh api {path}: ответ не разобран: {e}"))
}

/// `владелец/имя` из адреса `origin`, если он на GitHub.
pub fn github_slug(url: &str) -> Option<String> {
    let url = url.trim();
    let path = ["git@github.com:", "ssh://git@github.com/", "https://github.com/", "http://github.com/"]
        .iter()
        .find_map(|p| url.strip_prefix(p))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then(|| format!("{owner}/{name}"))
}

/// Каталоги профиля сборки, где cargo копит выход каждой сборки и ничего не
/// убирает сам.
///
/// `incremental` НЕ В СПИСКЕ: имена там несут не хеш единицы, а свой
/// (`app-0pqlvgfuleagk` при `deps/app-c23c86af9c2c3f98`, замер 2026-09-30),
/// и перечень cargo не может за них поручиться — уборка снесла бы кэш живой
/// единицы. Старые сеансы в нём убирает сам rustc.
const SWEPT: &[&str] = &["deps", ".fingerprint", "build"];

/// Команда перечня: то, что прогон собрал, называет сам cargo.
///
/// СВОЙ ЗАПРОС, А НЕ ВЫВОД РЕЦЕПТА НАБОРА. Рецепты печатают что хотят: у
/// tot-ade `just test` идёт через nextest без json, и уборка, ждавшая json от
/// рецепта, не шла бы там никогда. nextest собирает тестовые единицы тем же
/// `cargo test --no-run`, а после прогона всё уже собрано, так что запрос
/// идёт секунды и ничего не строит. `--all-features` — потому что с ним
/// собирают рецепты tot-ade (`M5-T45`); у крейта без признаков он ничего не
/// меняет. Набор, собирающий иначе, увидит здесь сборку и отказ уборки.
const CARGO_LIST: &[&str] = &["test", "--workspace", "--all-features", "--no-run", "--message-format=json"];

/// Пути выхода, которые назвал запрос перечня: `filenames` сообщений
/// `compiler-artifact` и `out_dir` у `build-script-executed`.
///
/// ПЕРЕЧЕНЬ, А НЕ ВРЕМЯ ФАЙЛА, ГОВОРИТ, ЧТО ЖИВО. Единицу, которую сборка
/// сочла свежей, cargo не переписывает, и её время остаётся старым (замер
/// 2026-09-30 на пробном крейте: после пересборки приложения rlib
/// зависимости сохранил прежнее время). Уборка по одному времени снимала бы
/// все ~330 сторонних крейтов tot-ade после каждого прогона, и каждый прогон
/// собирал бы их заново. В перечень свежие единицы входят (`"fresh":true`).
///
/// ЕДИНИЦА, СОБРАННАЯ ЗАПРОСОМ (`"fresh":false`), — ОТКАЗ. Значит, запрос
/// собирает не то, что прогон, или дерево сдвинулось, и перечень не говорит
/// о прогоне. Пустой перечень — тоже отказ: судить нечем.
///
/// Читается только stdout: json cargo пишет туда, и строка, разрезанная
/// выводом stderr в общей трубе, выпала бы из перечня.
fn cargo_listing(stdout: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let mut out = Vec::new();
    for line in stdout.lines().filter(|l| l.starts_with('{')) {
        let msg = serde_json::from_str::<Value>(line).map_err(|e| format!("строка перечня не разобрана: {e}"))?;
        match msg["reason"].as_str() {
            Some("compiler-artifact") => {
                if msg["fresh"].as_bool() != Some(true) {
                    return Err(format!(
                        "запрос перечня собрал `{}` — перечень не о прогоне",
                        msg["package_id"].as_str().unwrap_or("?")
                    ));
                }
                out.extend(msg["filenames"].as_array().into_iter().flatten().filter_map(Value::as_str).map(Into::into));
            }
            Some("build-script-executed") => out.extend(msg["out_dir"].as_str().map(Into::into)),
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("перечень cargo пуст".into());
    }
    Ok(out)
}

/// Хеш единицы в имени записи сборки: `libdep-c05783f808fda616.rlib`,
/// `.fingerprint/dep-c05783f808fda616`, `build/dep-0469e4080376578f` — один
/// род имени во всех трёх каталогах.
fn unit_key(name: &std::ffi::OsStr) -> Option<&str> {
    let key = name.to_str()?.split('.').next()?.rsplit('-').next()?;
    (key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit())).then_some(key)
}

/// Самое позднее изменение внутри записи и её вес — без хода по ссылкам.
///
/// ВОЗРАСТ КАТАЛОГА — ЭТО ВОЗРАСТ НОВЕЙШЕГО ФАЙЛА В НЁМ, А НЕ ЕГО СОБСТВЕННЫЙ.
/// Cargo переписывает файлы `.fingerprint/<единица>/` на месте, и время самого
/// каталога от этого не двигается: замер 2026-09-30 на пробном крейте —
/// после пересборки файлы внутри новые, каталог старый.
///
/// Нечитаемое внутри — отказ, а не ноль: запись неизвестного возраста могла
/// быть написана этим прогоном.
fn newest_and_size(path: &std::path::Path) -> Result<(std::time::SystemTime, u64), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut newest = meta.modified().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut size = if meta.is_file() { meta.len() } else { 0 };
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
            let (n, s) = newest_and_size(&entry.path())?;
            newest = newest.max(n);
            size += s;
        }
    }
    Ok((newest, size))
}

/// Что в каталоге сборки дерева прогона не названо cargo за прогон и старше
/// `cutoff`: путь и вес. Только выбор — удаляет `sweep_runner_target`.
///
/// Живой считается единица, чей хеш стоит в пути из перечня, или чей файл —
/// та же inode, что файл из перечня: у бинаря cargo называет выложенную копию
/// `target/debug/app`, а не `deps/app-<хеш>`, и хеш виден только через
/// жёсткую ссылку. Запись без хеша в имени не выбирается: перечень не может
/// сказать о ней ни да, ни нет.
///
/// НИЧЕГО ВНЕ `<дерево>/target` ВЫБРАНО НЕ БУДЕТ, И ЭТО ПРОВЕРЯЕТСЯ ПО
/// РАЗРЕШЁННОМУ ПУТИ. `target` бывает ссылкой в общий каталог сборки, а
/// профиль или `deps` — ссылкой куда угодно; удаление по такому пути снесло бы
/// чужую сборку. Ссылки не выбираются и не обходятся ни на каком уровне.
///
/// Пустой выбор по причине — отказ словом, а не «удалено 0»: иначе уборка,
/// не нашедшая дерева, в журнале неотличима от уборки, которой нечего убрать.
fn stale_artefacts(
    tree: &std::path::Path,
    target: &std::path::Path,
    cutoff: std::time::SystemTime,
    listed: &[std::path::PathBuf],
) -> Result<Vec<(std::path::PathBuf, u64)>, String> {
    use std::os::unix::fs::MetadataExt;
    let resolve = |p: &std::path::Path| p.canonicalize().map_err(|e| format!("{} не разрешается: {e}", p.display()));
    let (tree, root) = (resolve(tree)?, resolve(target)?);
    if !root.starts_with(&tree) || root == tree {
        return Err(format!("{} лежит вне дерева прогона {}", root.display(), tree.display()));
    }
    let read = |p: &std::path::Path| -> Result<Vec<std::path::PathBuf>, String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(p).map_err(|e| format!("{} не читается: {e}", p.display()))? {
            out.push(entry.map_err(|e| format!("{} не читается: {e}", p.display()))?.path());
        }
        Ok(out)
    };
    let real_dir = |p: &std::path::Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    let mut entries = Vec::new();
    for profile in read(&root)?.into_iter().filter(|p| real_dir(p)) {
        for kind in SWEPT {
            let dir = profile.join(kind);
            if !real_dir(&dir) {
                continue;
            }
            if !resolve(&dir)?.starts_with(&root) {
                return Err(format!("{} лежит вне {}", dir.display(), root.display()));
            }
            entries.extend(read(&dir)?);
        }
    }
    let inodes: std::collections::HashSet<(u64, u64)> =
        listed.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| (m.dev(), m.ino())).collect();
    let mut live: std::collections::HashSet<String> =
        listed.iter().flat_map(|p| p.iter()).filter_map(unit_key).map(str::to_owned).collect();
    for path in &entries {
        let same = std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && inodes.contains(&(m.dev(), m.ino())));
        if let (true, Some(key)) = (same, path.file_name().and_then(unit_key)) {
            live.insert(key.to_owned());
        }
    }
    let mut out = Vec::new();
    for path in entries {
        let Some(key) = path.file_name().and_then(unit_key) else { continue };
        if live.contains(key) || std::fs::symlink_metadata(&path).map_or(true, |m| m.file_type().is_symlink()) {
            continue;
        }
        let (newest, size) = newest_and_size(&path)?;
        if newest < cutoff {
            out.push((path, size));
        }
    }
    out.sort();
    Ok(out)
}

/// Убирать ли после прогона, и по какому перечню. `listing` — ответ запроса
/// перечня, `None` — запрос не задавали.
///
/// Грязное дерево — отказ при любом перечне: в дереве мог работать чужой
/// cargo (замер 2026-09-21 у проверки чистоты после прогона), и удаление шло
/// бы из-под его сборки. Отказ запроса — отказ уборки с его причиной.
fn sweep_list(
    dirty_after: bool,
    listing: Option<Result<Vec<std::path::PathBuf>, String>>,
) -> Result<Vec<std::path::PathBuf>, String> {
    if dirty_after {
        return Err("в дереве во время прогона работал кто-то ещё".into());
    }
    listing.unwrap_or_else(|| Err("перечень cargo не спрошен".into()))
}

/// Убрать из каталога сборки дерева прогона выход, которого прогон, начатый
/// в `started`, не назвал и не коснулся.
///
/// ЗАЧЕМ. Cargo не убирает выход прежних сборок, и дерево прогона копило его
/// с каждой вершины ствола: заявка 151 — у tot-ade 60 ГБ, затем 56 ГБ за
/// полдня 2026-09-30, `/opt` заполнен до 96 %; удаление файлов старше шести
/// часов освобождало 43–48 ГБ за раз, и cargo пересобирал недостающее.
///
/// ЗОВЁТСЯ ПОСЛЕ ПРОГОНА И В ТОЙ ЖЕ ЗАДАЧЕ — своя сборка к этому времени
/// кончилась. Чужая может идти: дерево прогона наше по имени, но не по замку
/// (замер 2026-09-21 у проверки чистоты после прогона). Поэтому зовущий не
/// убирает, когда дерево испачкали во время прогона. Отдельный цикл уборки
/// удалял бы и из-под нашей же сборки.
///
/// Отказ уборки прогона не роняет: замер уже записан, а место — забота
/// следующего захода. Всё сказанное — в журнал, с освобождённым объёмом.
fn sweep_runner_target(
    name: &str,
    tree: &str,
    cwd: &str,
    started: std::time::SystemTime,
    listed: &[std::path::PathBuf],
) {
    // Секунда запаса: время файла ставится грубыми часами ядра и может
    // отставать от `SystemTime::now()` на тик, а тогда файл, записанный сразу
    // после начала, выглядел бы старше него.
    let cutoff = started - std::time::Duration::from_secs(1);
    let target = std::path::Path::new(cwd).join("target");
    let chosen = match stale_artefacts(std::path::Path::new(tree), &target, cutoff, listed) {
        Ok(chosen) => chosen,
        Err(why) => return println!("{name} · уборка сборки не идёт: {why}"),
    };
    let (mut freed, mut removed, mut failed) = (0u64, 0usize, 0usize);
    for (path, size) in chosen {
        let gone = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match gone {
            Ok(()) => {
                freed += size;
                removed += 1;
            }
            Err(e) => {
                failed += 1;
                println!("{name} · уборка сборки: {} не удалён: {e}", path.display());
            }
        }
    }
    println!(
        "{name} · уборка сборки: удалено {removed}, освобождено {} МБ, отказов {failed}",
        freed / (1024 * 1024)
    );
}

/// Прогон работы, годный в факт о стволе набора.
#[derive(Debug, PartialEq)]
pub struct TrunkRun {
    pub id: i64,
    pub attempt: i64,
    pub sha: String,
    pub started: String,
}

/// Прогон ствола — ветка по умолчанию, коммит из САМОГО репозитория набора и
/// событие не от запроса на слияние; иначе `None`.
///
/// Имя ветки само по себе ничего не доказывает: форк держит свой `main`, и
/// его прогон приходит в тот же список с `head_branch = main`. Прогон
/// `pull_request_target` исполняется в контексте ствола, но по чужому коду.
/// Любой из них закрыл бы красную фазу падением, которого в наборе не было.
pub fn trunk_run(run: &Value, repo: &str, trunk: &str) -> Option<TrunkRun> {
    if run["head_branch"].as_str() != Some(trunk)
        || matches!(run["event"].as_str(), None | Some("pull_request" | "pull_request_target"))
        || !run["head_repository"]["full_name"].as_str()?.eq_ignore_ascii_case(repo)
    {
        return None;
    }
    Some(TrunkRun {
        id: run["id"].as_i64()?,
        attempt: run["run_attempt"].as_i64()?,
        sha: run["head_sha"].as_str()?.to_owned(),
        started: run["run_started_at"].as_str()?.to_owned(),
    })
}

/// Лежит ли коммит на `origin/<ствол>` дерева набора. Неизвестный коммит —
/// «нет»: доказать нечем.
fn on_trunk(root: &str, sha: &str, trunk: &str) -> bool {
    Worker::git(root, &["merge-base", "--is-ancestor", sha, &format!("origin/{trunk}")]).is_ok()
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
            return Err(format!("{name}: {}", cut(why, 400)));
        }
        let text = out["content"][0]["text"].as_str().unwrap_or("");
        serde_json::from_str(text)
            .map_err(|e| format!("{name}: ответ не разбирается: {e}: {}", cut(text, 200)))
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
        cmd.current_dir(&bundle.repo).env("MH_PROJECT", &bundle.project);
        // Через `output_within`: потолок обязан погасить claude вместе с его
        // командами — иначе осиротевший claude или его cargo продолжили бы
        // править дерево и держать замок сборки, а воркер завёл бы вторую
        // сессию рядом (mh#154).
        let out = match output_within(cmd, LIMIT_S, prompt.as_bytes()).await {
            Ok(Some(o)) => (o.status, o.stdout, o.stderr),
            Ok(None) => return (format!("сессия не уложилась в {LIMIT_S} с"), session.to_owned(), true),
            Err(e) => return (format!("сессия не ответила: {e}"), session.to_owned(), true),
        };
        let (status, stdout, stderr) = out;
        {
            let text = String::from_utf8_lossy(&stdout);
            let err_tail = String::from_utf8_lossy(&stderr);
            if !status.success() {
                let tail = format!("{text}{err_tail}");
                return (format!("сессия не ответила: {}", cut(&tail, 600)), session.to_owned(), true);
            }
            match serde_json::from_str::<Value>(&text) {
                Err(_) => (format!("ответ не разобран: {}", cut(&text, 600)), session.to_owned(), true),
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

    /// Статус прогонов тестов: когда последний, что упало на чистом дереве.
    async fn test_status(&self, project: &str) -> Value {
        crate::projector::test_status(&self.pool, project, "").await.unwrap_or(json!({ "runs": 0 }))
    }

    /// Наблюдатель прогонов (заявка 20): раз в TEST_TICK гоняет тесты набора,
    /// но только с чистого дерева и подписывая коммит. Грязное дерево — не
    /// «подождать», а «не снимать»: факт о другом состоянии мира харнес не
    /// пишет (заявка 18 научила, чем это кончается).
    pub async fn test_watch(self: std::sync::Arc<Self>, bundle: Bundle) {
        let Some(spec) = bundle.test.clone() else { return };
        loop {
            match self.run_tests_once(&bundle, &spec).await {
                Ok(Some(v)) => println!(
                    "{} · тесты: {} проверок, упало {} · {}",
                    bundle.name,
                    fld(&v, "runs", &bundle.name), fld(&v, "failed", &bundle.name).as_array().map(|a| a.len()).unwrap_or(0),
                    fld(&v, "cleanAt", &bundle.name).as_i64().map(|t| format!("{t}")).unwrap_or_default(),
                ),
                Ok(None) => {}
                Err(e) => println!("{} · тесты: {e}", bundle.name),
            }
            tokio::time::sleep(std::time::Duration::from_secs(TEST_TICK_S)).await;
        }
    }

    /// Журналы заданий CI, объявленных набором (`ci-job-add`).
    ///
    /// ПАДЕНИЕ СНИМАЕТСЯ ТАМ, ГДЕ ОНО ВОЗМОЖНО. Проверка под `#![cfg(windows)]`
    /// на этой машине собирается пустой, и `red-observed-failing` держал пару
    /// `V5-T135` без выхода: падение `m5_t135_revoke_takes_the_refusals_off_too`
    /// видел лишь раннер Windows (прогон `36642293742`), а записать его было
    /// некому. Журнал харнес качает и разбирает сам — `test_lines`, как свой.
    ///
    /// СВОЙ ЦИКЛ, А НЕ ХВОСТ НАБЛЮДАТЕЛЯ ПРОГОНОВ. Наблюдатель заводится лишь у
    /// набора с разделом `test`; у `myack` в `scripts/runner.json` его нет, и
    /// дверь отвечала бы «объявлено», а взятия не случалось бы никогда.
    pub async fn ci_watch(self: std::sync::Arc<Self>, bundle: Bundle) {
        loop {
            self.take_ci_jobs(&bundle).await;
            tokio::time::sleep(std::time::Duration::from_secs(TEST_TICK_S)).await;
        }
    }

    async fn take_ci_jobs(&self, bundle: &Bundle) {
        let jobs = match crate::db::conn(&self.pool).await {
            Ok(c) => c
                .query("SELECT workflow, job, platform FROM project_ci_job WHERE project_id = $1 ORDER BY 1, 2",
                       &[&bundle.project])
                .await
                .map_err(|e| format!("{e}")),
            Err(e) => Err(format!("{e:?}")),
        };
        let jobs = match jobs {
            Ok(j) if j.is_empty() => return,
            Ok(j) => j,
            Err(why) => return println!("{} · CI: объявления не читаются: {why}", bundle.name),
        };
        // Репозиторий — из `origin` дерева набора, того же, по которому ниже
        // сверяется ствол; объявить его рукой нельзя (см. `project_ci_job`).
        let repo = match Self::git(&bundle.repo, &["remote", "get-url", "origin"]) {
            Ok(url) => match github_slug(&url) {
                Some(r) => r,
                None => return println!("{} · CI: `origin` не на GitHub: {}", bundle.name, url.trim()),
            },
            Err(why) => return println!("{} · CI: {why}", bundle.name),
        };
        for r in jobs {
            let (workflow, job, platform): (String, String, String) = (r.get(0), r.get(1), r.get(2));
            let actor = format!("ci:{workflow}/{job}");
            match self.take_ci_job(bundle, &repo, &workflow, &job, &platform, &actor).await {
                Ok(0) => {}
                Ok(n) => println!("{} · CI {actor}: взято прогонов {n}", bundle.name),
                Err(why) => println!("{} · CI {actor}: {why}", bundle.name),
            }
        }
    }

    async fn take_ci_job(
        &self, bundle: &Bundle, repo: &str, workflow: &str, job: &str, platform: &str, actor: &str,
    ) -> Result<usize, String> {
        let about = gh_json(&format!("repos/{repo}")).await?;
        let trunk = about["default_branch"].as_str().filter(|b| !b.is_empty())
            .ok_or_else(|| format!("у {repo} не названа ветка по умолчанию"))?
            .to_owned();
        let runs = gh_json(&format!(
            "repos/{repo}/actions/workflows/{workflow}/runs?branch={trunk}&status=completed&per_page=20"
        ))
        .await?;
        // Ответ без перечня — отказ, а не «прогонов нет»: иначе опечатка в
        // имени файла работы молчала бы вечно, как пустой список.
        let list = runs["workflow_runs"].as_array()
            .ok_or_else(|| format!("в ответе о прогонах {workflow} нет `workflow_runs`: {}", cut(&runs.to_string(), 300)))?;
        let client = crate::db::conn(&self.pool).await.map_err(|e| format!("{e:?}"))?;
        let (mut taken, mut fetched) = (0, false);
        for run in list {
            let Some(r) = trunk_run(run, repo, &trunk) else { continue };
            let seen: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM ci_run_taken
                                     WHERE project_id = $1 AND run_id = $2 AND run_attempt = $3 AND actor = $4)",
                    &[&bundle.project, &r.id, &r.attempt, &actor],
                )
                .await
                .map_err(|e| format!("{e}"))?
                .get(0);
            if seen {
                continue;
            }
            // КОММИТ ОБЯЗАН ЛЕЖАТЬ НА СТВОЛЕ НАБОРА, а не только называться им:
            // имя ветки в ответе GitHub — слова прогона. Докачивается ствол раз
            // за заход и лишь когда коммита не нашлось: свежий прогон обычно
            // новее, чем знает общий чекаут. Не на стволе — не берётся и не
            // отмечается: после докачки он может там оказаться.
            if !on_trunk(&bundle.repo, &r.sha, &trunk) && !fetched {
                fetched = true;
                if let Err(why) = Self::git(&bundle.repo, &["fetch", "origin", "--quiet"]) {
                    println!("{} · CI: ствол не докачан: {why}", bundle.name);
                }
            }
            if !on_trunk(&bundle.repo, &r.sha, &trunk) {
                println!("{} · CI {actor}: прогон {} не взят: коммит {} не на origin/{trunk}", bundle.name, r.id, r.sha);
                continue;
            }
            // Отказ одного прогона не держит остальные: журнал старше девяноста
            // дней GitHub уже не отдаёт, и `?` здесь запер бы за ним все новые.
            let rows = match Self::ci_job_rows(repo, r.id, job).await {
                Ok(rows) => rows,
                Err(why) => {
                    println!("{} · CI {actor}: прогон {} не взят: {why}", bundle.name, r.id);
                    continue;
                }
            };
            // Взятый без строк прогон больше не перечитывается — и молчать об
            // этом нельзя: «задание не собралось» и «разбор не узнал журнал»
            // выглядят одинаково, пока их не назвали.
            if rows.is_empty() {
                println!("{} · CI {actor}: прогон {} взят без проверок: в журнале задания «{job}» \
                          нет строк прогона (не собралось, не запускалось либо формат не узнан)",
                         bundle.name, r.id);
            }
            let ci = crate::projector::CiRun { platform, run: r.id, attempt: r.attempt, started: &r.started };
            crate::projector::record_test_runs(&self.pool, &bundle.project, &r.sha, &rows, actor, false, Some(ci))
                .await
                .map_err(|e| format!("прогон {} не записан: {e:?}", r.id))?;
            taken += 1;
        }
        Ok(taken)
    }

    /// Строки прогона из журнала одного задания. Задания с таким именем в
    /// прогоне нет — строк нет: его не запускали, и мерить нечего.
    async fn ci_job_rows(repo: &str, run: i64, job: &str) -> Result<Vec<(String, String, String)>, String> {
        let jobs = gh_json(&format!("repos/{repo}/actions/runs/{run}/jobs?per_page=100")).await?;
        let list = jobs["jobs"].as_array()
            .ok_or_else(|| format!("в ответе о заданиях прогона {run} нет `jobs`"))?;
        let Some(id) = list.iter().find(|j| j["name"].as_str() == Some(job)).and_then(|j| j["id"].as_i64()) else {
            return Ok(Vec::new());
        };
        Ok(test_lines(&gh(&format!("repos/{repo}/actions/jobs/{id}/logs")).await?))
    }

    /// Своё дерево прогона: `<репозиторий>-mh-runner`, рядом, а не внутри.
    ///
    /// **Ствол мерят на стволе, а не на чужом рабочем столе.** Прежде прогон
    /// шёл в `bundle.repo` — в каталоге, где работает сессия набора. Он грязен
    /// почти всегда: замер 2026-09-21 дал 39 файлов в индексе и ветку `pr-70`,
    /// и наблюдатель отказывался каждый час двенадцать часов подряд. Отказ был
    /// верен — факт с грязного дерева хуже отсутствия факта (заявка 18), — но
    /// неверен был выбор предмета: запись прогона замерла на снимке двухдневной
    /// давности, и пункт `test-trunk-green` звал поломкой ствола 68 имён, из
    /// которых почти все с тех пор позеленели.
    fn runner_tree(repo: &str) -> String {
        format!("{repo}-mh-runner")
    }

    /// Завести либо обновить дерево прогона до вершины ствола.
    ///
    /// Отказ — словом, и тогда прогона не будет: **мерить не то хуже, чем не
    /// мерить**. Своего в дереве не остаётся между заходами: `reset --hard` и
    /// `clean -fd` снимают всё, кроме игнорируемого, — каталог сборки переживает
    /// и не собирается заново каждый час, пока не перерос порог `trim_target`.
    fn prepare_runner_tree(repo: &str) -> Result<String, String> {
        let tree = Self::runner_tree(repo);
        // ОТКАЗ `fetch` НЕ ОТМЕНЯЕТ ЗАМЕР, а отказ `reset` — отменяет.
        //
        // Разница в том, чем кончается каждый. Не докачав ствол, мы померим
        // известный коммит — на один позади, и он записан в партии; не
        // сбросив дерево, мы померим НЕИЗВЕСТНО ЧТО. Отказать здесь значило бы
        // терять целый час замера из-за замка в общем чекауте, где весь день
        // работают чужие сессии, — а пункты гейта продолжали бы отвечать по
        // позавчерашней партии, ничем не показывая пропуск.
        if let Err(why) = Self::git(repo, &["fetch", "origin", "--quiet"]) {
            println!("ствол не докачан, мерим известную вершину: {why}");
        }
        if !std::path::Path::new(&tree).is_dir() {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["worktree", "add", "--detach", &tree, "origin/main"])
                .output()
                .map_err(|e| format!("дерево прогона не завелось: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "дерево прогона не завелось: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        Self::git(&tree, &["reset", "--hard", "origin/main"])?;
        Self::git(&tree, &["clean", "-qfd"])?;
        Self::trim_target(&tree);
        Ok(tree)
    }

    /// Сборка дерева прогона растёт без предела, и предел ставится здесь.
    ///
    /// Каждая новая вершина пересобирает изменённые крейты под новыми хешами, а
    /// прежние бинари проверок остаются в `target/debug/deps` навсегда. Замер
    /// 2026-10-01: каталог дорос до 24 ГБ (22 ГБ — `deps`) за день прогонов, и
    /// диск `/opt`, общий с CI набора, дважды падал ниже 9 ГБ — раннеры CI
    /// кончались на «No space left». Почистить выборочно нечем: ни время
    /// доступа (relatime), ни время правки (cargo не трогает свежие артефакты)
    /// не отличают нужное от брошенного. Поэтому сборка сносится целиком, когда
    /// переросла порог: следующий прогон соберёт всё с нуля, и это цена раз в
    /// несколько вершин, а не на каждой.
    fn trim_target(tree: &str) {
        const LIMIT_KB: u64 = 18 * 1024 * 1024;
        let target = format!("{tree}/target");
        if !std::path::Path::new(&target).is_dir() {
            return;
        }
        let kb = std::process::Command::new("du")
            .args(["-sk", &target])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().and_then(|n| n.parse::<u64>().ok()));
        // Размер не прочёлся — предел молча снят и диск снова кончится; это
        // говорится вслух, иначе отказ `du` неотличим от «меньше порога».
        let Some(kb) = kb else {
            println!("размер сборки дерева прогона не прочёлся — предел 18 ГБ не проверен");
            return;
        };
        if kb > LIMIT_KB {
            match std::fs::remove_dir_all(&target) {
                Ok(()) => println!("сборка дерева прогона снесена: {} ГБ при пороге 18", kb / 1024 / 1024),
                Err(e) => println!("сборка дерева прогона не снесена ({} ГБ): {e}", kb / 1024 / 1024),
            }
        }
    }

    /// Перечень выхода сборки из дерева прогона для уборки.
    ///
    /// Отказ здесь — отказ уборки, а не прогона: прогон уже записан, и
    /// без перечня судить, что живо, нечем (довод у `cargo_listing`).
    async fn cargo_list(cwd: &str) -> Result<Vec<std::path::PathBuf>, String> {
        let mut cmd = tokio::process::Command::new("cargo");
        cmd.args(CARGO_LIST).current_dir(cwd).env("CARGO_TERM_COLOR", "never");
        let out = match output_within(cmd, TEST_TIMEOUT_S, b"").await {
            Ok(Some(o)) => o,
            Ok(None) => return Err(format!("запрос перечня не уложился в {TEST_TIMEOUT_S} с")),
            Err(e) => return Err(format!("запрос перечня не завёлся: {e}")),
        };
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("запрос перечня отказал: {}", cut(err.trim(), 400)));
        }
        cargo_listing(&String::from_utf8_lossy(&out.stdout))
    }

    /// Один профиль прогона: команда, её вывод и разобранные строки.
    ///
    /// ПРОФИЛЕЙ ДВА, И ВТОРОЙ ОБЯЗАТЕЛЕН. `just test` исключает зеркала, поэтому
    /// у проверок красной фазы НИКОГДА не появлялось записи с вердиктом
    /// `failed` — а `corpus · red-observed-failing` требует ровно её: у каждой
    /// проверки ЗАКРЫТОЙ красной задачи обязан найтись хотя бы один упавший
    /// прогон. Замер 2026-09-21: пункт держал 201 находку, и ни одна из них не
    /// была долгом набора — это прибор не снимал того, чего сам же требует.
    /// Набор принёс это заявкой 239.
    ///
    /// Ненулевой код выхода у красного профиля — норма: `just red` обязан
    /// упасть. Поэтому «не собралось» здесь распознаётся по ПУСТОМУ разбору, а
    /// не по коду выхода.
    async fn profile(cwd: &str, cmd: &str) -> Result<(Vec<(String, String, String)>, bool), String> {
        // ОДИН ПОТОК, А НЕ ДВА СКЛЕЕННЫХ. `cargo` печатает `Running tests/<файл>`
        // в stderr, а `test <имя> ... ok` — в stdout. Прежде оба читались
        // порознь и склеивались подряд: все имена бинарей оказывались ПОСЛЕ
        // всех имён проверок, и разбор, ведущий текущий бинарь, не видел ни
        // одного вовремя. Порядок здесь и есть связь, и рвался он склейкой.
        let joined = format!("( {} ) 2>&1", cmd);
        let mut run = tokio::process::Command::new("bash");
        run.args(["-c", &joined]).current_dir(cwd).env("CARGO_TERM_COLOR", "never");
        let out = match output_within(run, TEST_TIMEOUT_S, b"").await {
            Ok(Some(o)) => o,
            Ok(None) => return Err(format!("прогон не уложился в {TEST_TIMEOUT_S} с")),
            Err(e) => return Err(format!("прогон не завёлся: {e}")),
        };
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok((test_lines(&text), out.status.success()))
    }

    /// Состояния задач — со ствола, каждый час, а не когда сессия вспомнит.
    ///
    /// Дверь `task-state-push` описана словами «принять состояния, ВЫВЕДЕННЫЕ
    /// харнесом из закрывающих трейлеров», а звать её было некому: выводил их
    /// `mh sense`, и гоняют его руками. `HARNESS-PLAN-7` записал этот пробел
    /// дословно — «скриптов харнеса, зовущих `task-state-push`: 0, постоянного
    /// механизма нет».
    ///
    /// Цена измерена 21.09: ствол закрыл `M4-T9` трейлером, а прибор два с
    /// половиной часа показывал её взятой в работу. Владелец смотрел на доску и
    /// спрашивал, почему за смену не закрыто ни одной задачи, — закрыто было
    /// шесть.
    ///
    /// Место выбрано не случайно: здесь уже лежит ЧИСТОЕ дерево ствола с
    /// прочитанным `HEAD` — ровно те два условия, без которых `fact_fresh`
    /// отвечает «снят БЕЗ КОММИТА» и факт не считается свежим никогда.
    ///
    /// Вывод берётся общей функцией с `mh sense`, а не повторяется здесь:
    /// правило «закрыто — значит в продукте» должно жить в одном месте, иначе
    /// два звавших однажды покажут разное.
    ///
    /// ПОТОЛОК, И ОН НЕ МЕЛКИЙ: НАБОР БЕЗ РАЗДЕЛА `test` СЮДА НЕ ЗАХОДИТ.
    /// `mh-worker` заводит наблюдателя по `bundle.test.is_some()`, и в
    /// `scripts/runner.json` у `myack` ключа `test` нет вовсе — состояний он
    /// не получит, их ему по-прежнему подаёт только `mh sense` руками.
    ///
    /// Это ровно тот набор, чьим замером и доказан вред: у `myack` в летописи
    /// девяносто девять трейлеров, а в проекции лежало тридцать четыре.
    /// Починка до него не достаёт, и молчать об этом нельзя.
    async fn push_task_states(&self, bundle: &Bundle, root: &str, head: &str, dirty: bool) {
        let specs = match self.door(&bundle.project, "sensor-specs", json!({})).await {
            Ok(v) => v,
            Err(why) => {
                println!("{} · состояния задач: {why}", bundle.name);
                return;
            }
        };
        let empty = Vec::new();
        let re = specs["specs"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .find(|s| s["how"].as_str() == Some("task-trailers"))
            .and_then(|s| s["extract"].as_str())
            .unwrap_or("");
        // Образец трейлера ОБЪЯВЛЯЕТ НАБОР. Не объявлен — молчим и говорим об
        // этом: подать пустое значило бы стереть состояния, выведенные прежде.
        if re.is_empty() {
            println!("{} · состояния задач: датчик «task-trailers» не объявлен — не подаю", bundle.name);
            return;
        }
        match crate::client::task_states_from_repo(root, re) {
            Ok(states) => {
                let n = states.len();
                match self
                    .door(
                        &bundle.project,
                        "task-state-push",
                        json!({ "states": states, "commit": head, "dirty": dirty }),
                    )
                    .await
                {
                    // ЧИСЛА БЕРУТСЯ ИЗ ОТВЕТА ДВЕРИ, А НЕ ИЗ ПАМЯТИ О НЁМ.
                    // Первая же подача написала «было null стало null»: дверь
                    // отдаёт `accepted`, а я спросил у неё поля, которых у неё
                    // нет. Строка в журнале повторяется каждый час — молчать
                    // ей нельзя.
                    Ok(out) => println!(
                        "{} · состояния задач: {n} из трейлеров, принято {}",
                        bundle.name, fld(&out, "accepted", &bundle.name)
                    ),
                    Err(why) => println!("{} · состояния задач: {why}", bundle.name),
                }
            }
            Err(why) => println!("{} · состояния задач: {why}", bundle.name),
        }
    }

    /// Съём датчиков набора с дерева обходчика.
    ///
    /// Дверь берётся из окружения службы — та же, что у `mh sense`, — и ей
    /// подставляется набор этого прохода. Съём блокирующий: он читает файлы и
    /// ходит в сеть, и держать на нём исполнителя задач нельзя.
    ///
    /// Отказ не отменяет прогона: несвежий датчик хуже свежего, но прогон,
    /// не снятый из-за датчика, хуже обоих.
    async fn sense_tree(&self, bundle: &Bundle, root: &str) {
        let door = match crate::client::Door::from_env_for(Some(&bundle.project)) {
            Ok(d) => d,
            Err(why) => {
                println!("{} · датчики: двери нет — {why}", bundle.name);
                return;
            }
        };
        let (name, root) = (bundle.name.clone(), root.to_owned());
        match tokio::task::spawn_blocking(move || crate::client::sense(&door, None, &root)).await {
            Ok(Ok(v)) => println!(
                "{} · датчики: снято {}",
                name,
                // КЛЮЧ ОТВЕТА — `sensed`, И ЭТО ТРЕТИЙ ЗА СМЕНУ СЛУЧАЙ, КОГДА Я
                // ПРОЧЁЛ ПОЛЕ ПО ПАМЯТИ О ФОРМЕ, А НЕ ПО ОТВЕЧАЮЩЕМУ.
                // Первый съём написал «снято 0» при тридцати снятых датчиках,
                // и это читалось как «механизм не работает». Строка в журнале
                // повторяется на каждой новой вершине — молчать ей нельзя.
                fld(&v, "sensed", &name).as_array().map(|a| a.len()).unwrap_or(0)
            ),
            Ok(Err(why)) => println!("{name} · датчики: {why}"),
            Err(e) => println!("{name} · датчики: съём не завершился: {e}"),
        }
    }

    async fn run_tests_once(&self, bundle: &Bundle, spec: &TestSpec) -> Result<Option<Value>, String> {
        let root = Self::prepare_runner_tree(&bundle.repo)?;
        let cwd = if spec.dir.is_empty() { root.clone() } else { format!("{root}/{}", spec.dir) };
        if !std::path::Path::new(&cwd).is_dir() {
            return Err(format!("каталог прогона {cwd} нет"));
        }
        // Дерево своё и только что сброшено — грязным ему быть неоткуда. Но
        // проверка остаётся: она стережёт не сессию, а нас самих, и её молчание
        // — единственное, чем «прогон снят со ствола» отличается от обещания.
        let dirty = !Self::git(&cwd, &["status", "--porcelain"])?.trim().is_empty();
        // КОРЕНЬ, А НЕ КАТАЛОГ ПРОГОНА. Сегодня они совпадают — ни один набор
        // не объявил подкаталога, — но `cwd` собирается из объявленного
        // `dir`, и `git -C` привязался бы к вложенному репозиторию, случись
        // там такой. Подача полная: чужая история заменила бы доску целиком.
        let head = Self::git(&root, &["rev-parse", "HEAD"])?.trim().to_owned();
        if head.is_empty() {
            return Err("HEAD не читается".into());
        }
        // СОСТОЯНИЯ ЗАДАЧ ПОДАЮТСЯ ДО ОБЕИХ РАЗВИЛОК, И ЭТО НЕ ПОРЯДОК СТРОК.
        //
        // «Перепрогон не нужен» — тесты на той же голове мерить нечего, а
        // трейлеры есть: ветку могли влить без единой правки кода.
        //
        // «Дерево грязное» — вывод состояний читает ЛЕТОПИСЬ и рабочего дерева
        // не касается вовсе. Стоя за этой проверкой, подача пропускала бы
        // целый час из-за постороннего файла. Довод не выдуман: доккомментарий
        // дерева прогона хранит замер, где наблюдатель отказывался по этой
        // причине двенадцать часов подряд.
        //
        // Грязнота при этом ПОДАЁТСЯ измеренной, а не пишется нулём: пусть
        // свежесть судит `fact_fresh`, а не мы за неё.
        self.push_task_states(bundle, &root, &head, dirty).await;
        if dirty {
            println!("{} · тесты: дерево грязное — прогон не снимается", bundle.name);
            return Ok(None);
        }
        // Перепрогон той же головы не нужен: свежий замер есть. Окно СВОЁ, а не
        // шаг заглядывания: шаг теперь пять минут, и мерить вершину заново
        // каждые пять минут значило бы жечь машину впустую.
        //
        // ПОТОЛОК НАЗВАН: прогон, упавший на сборке, тоже «свежий замер». На той
        // же голове сборка упадёт снова — это и довод; но случайное падение
        // (сборку убил systemd-oomd, кончился диск) не перемеряется до
        // `TEST_REDO_S`, и всё это время читатели вердиктов судят по прошлому
        // измерившему прогону (`test_run_last`). Партия CI замером вершины не
        // считается — довод у `head_measured_since`.
        let since = crate::projector::now_ms() - (TEST_REDO_S as i64) * 1000;
        let fresh = match crate::db::conn(&self.pool).await {
            Ok(c) => crate::projector::head_measured_since(&*c, &bundle.project, &head, since).await.unwrap_or(false),
            Err(_) => false,
        };
        if fresh {
            return Ok(None);
        }
        // ДАТЧИКИ СНИМАЮТСЯ С ТОЙ ЖЕ ВЕРШИНЫ, ЧТО И ПРОГОН.
        //
        // Съём запускался только руками (`mh sense`), и постоянного механизма
        // не было — `HARNESS-PLAN-7` записал это дословно. Цена измерена
        // 21.09: факты о коде отстали на четыре часа, проверка
        // `m4_t9_change_outside_the_border_is_drift_not_moved` была зелена в
        // прогоне и ОТСУТСТВОВАЛА у датчика, а пункт гейта «проверка не
        // написана» судит именно по датчику. Отставания при этом не видно:
        // срок объявлен неделей, и четыре часа прибор честно числит свежестью.
        //
        // Место то же, что у прогона, и по той же причине: здесь лежит чистое
        // дерево ствола с прочитанной вершиной — два условия, без которых
        // факт не считается свежим никогда. И снимается только на НОВОЙ
        // вершине: на прежней снимать нечего.
        self.sense_tree(bundle, &root).await;
        println!("{} · тесты: прогон на {head}", bundle.name);
        let started = std::time::SystemTime::now();
        let (mut rows, ok) = Self::profile(&cwd, &spec.cmd).await?;
        if rows.is_empty() && !ok {
            // Не собралось: ни одной проверки не увидели — факт об этом тоже
            // факт, иначе красное сборки неотличимо от «не гоняли».
            rows.push(("(build)".to_owned(), "build-failed".to_owned(), String::new()));
        }
        // Красный профиль идёт ТОЙ ЖЕ ПАРТИЕЙ: у записи одно время, и «последний
        // прогон» по max(at) видит оба. Пункт про ствол зеркала отсеивает сам —
        // по имени бинаря, а не по принадлежности партии.
        if !spec.red.trim().is_empty() {
            match Self::profile(&cwd, &spec.red).await {
                // СКОЛЬКО СНЯЛОСЬ — В ЖУРНАЛ, И ЭТО НЕ УКРАШЕНИЕ. Разбор, не
                // узнавший формата, отдаёт пустой перечень, и запись молча
                // остаётся прежней: пункт про красную фазу как держал свои
                // находки, так и держит, а причина выглядит как «набор не
                // сделал». Число рядом с именем профиля отличает «снято ноль»
                // от «не звали».
                Ok((red, _)) => {
                    println!("{} · красный профиль: {} проверок", bundle.name, red.len());
                    rows.extend(red);
                }
                // Красный профиль не роняет запись обычного: половина замера
                // лучше, чем ни одной, и молчание о ней хуже обеих.
                Err(why) => println!("{} · красный профиль не снят: {why}", bundle.name),
            }
        }
        if rows.is_empty() {
            return Ok(None);
        }
        // ЧИСТОТА МЕРЯЕТСЯ ПОСЛЕ ПРОГОНА, А НЕ ТОЛЬКО ДО НЕГО.
        //
        // Дерево прогона — наше по имени, но не по замку: соседняя сессия,
        // открывшая его как рабочее, пачкает дерево ПОКА идёт сборка, и
        // проверка «до» этого не видит. Замер 2026-09-21: в дереве лежали
        // правка `tot-core/src/lib.rs` и четыре удалённых зеркала, сборка на
        // них не собралась, и `build-failed` лёг записью с ЧИСТОГО дерева —
        // последним словом о стволе для каждого пункта, читающего `test_run`.
        //
        // Грязный прогон не выбрасывается: он факт о том, что мерили. Он лишь
        // перестаёт выдавать себя за ствол — отметкой `dirty`, которую все
        // читатели уже спрашивают.
        // Сверяется и ГОЛОВА: чужой `checkout` посреди прогона оставляет дерево
        // чистым, но на другом коммите, и строки легли бы под прежний `head`.
        let moved = Self::git(&cwd, &["rev-parse", "HEAD"]).map_or(true, |now| now.trim() != head);
        let dirty_after = moved
            || Self::git(&cwd, &["status", "--porcelain"]).map_or(true, |out| !out.trim().is_empty());
        if dirty_after {
            println!("{} · тесты: дерево испачкали во время прогона — запись помечена грязной", bundle.name);
        }
        let recorded = rows.len() as i64;
        crate::projector::record_test_runs(&self.pool, &bundle.project, &head, &rows, "mh-runner", dirty_after, None)
            .await
            // Отказ здесь значит ЛИБО незаписанный прогон, ЛИБО записанный без
            // отметки «пересчитать»; различать их сообщением было бы враньём в
            // одну из двух сторон. Названо обоими: у обоих исходов одно
            // следствие — пульт показывает прежний замер.
            .map_err(|e| format!("прогон не дошёл до пульта (запись или отметка): {e:?}"))?;
        let listing = if dirty_after { None } else { Some(Self::cargo_list(&cwd).await) };
        match sweep_list(dirty_after, listing) {
            Err(why) => println!("{} · уборка сборки не идёт: {why}", bundle.name),
            Ok(listed) => {
                let (name, tree, dir) = (bundle.name.clone(), root.clone(), cwd.clone());
                let sweep = move || sweep_runner_target(&name, &tree, &dir, started, &listed);
                if let Err(e) = tokio::task::spawn_blocking(sweep).await {
                    println!("{} · уборка сборки не завершилась: {e}", bundle.name);
                }
            }
        }
        let mut v = self.test_status(&bundle.project).await;
        v["recorded"] = json!(recorded);
        v["commit"] = json!(head);
        Ok(Some(v))
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

    /// Git в дереве — С ОТВЕТОМ ОБ ОТКАЗЕ, а не пустой строкой.
    ///
    /// Прежде отказ и «команда напечатала пусто» были одним значением. Замер
    /// 2026-09-21: `fetch` в общем чекауте молчал час, `origin/main` стоял на
    /// позавчерашней вершине, `reset --hard` целился в неё же — и прогон,
    /// снятый с чужого дерева, лёг в `test_run` как правда о стволе. Дверь
    /// `test-run` после этого отвечала «build-failed: 1» на весь набор, а
    /// прежний замер на 756 проверок переставал быть последним.
    fn git(repo: &str, args: &[&str]) -> Result<String, String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .map_err(|e| format!("git {} не завёлся: {e}", args.join(" ")))?;
        if !out.status.success() {
            return Err(format!(
                "git {} отказал: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Голова дерева и приборные следы в нём — с чем сверять дерево после хода.
    /// След — путь с временем правки: правка файла, грязного ещё на снимке,
    /// иначе неотличима от тишины.
    fn tree_snapshot(repo: &str) -> (String, HashMap<String, u128>) {
        let mut marks = HashMap::new();
        for line in Self::git(repo, &["status", "--porcelain"]).unwrap_or_default().lines() {
            let path = &line[3.min(line.len())..];
            if INSTRUMENT_PREFIXES.iter().any(|p| path.starts_with(p)) {
                let mtime = std::fs::metadata(std::path::Path::new(repo).join(path))
                    .and_then(|m| m.modified())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis())
                    .unwrap_or(0);
                marks.insert(path.to_owned(), mtime);
            }
        }
        (Self::git(repo, &["rev-parse", "HEAD"]).unwrap_or_default().trim().to_owned(), marks)
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
                .unwrap_or_default()
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
            // Хвост про коммит — только при реальном одобрении: отклонённый
            // approval с хвостом «Одобрено» закрыл бы задачу без коммита,
            // прямо следуя приказу, которому противоречит решение владельца.
            let approved = decisions
                .iter()
                .any(|d| d["kind"].as_str() == Some("approval") && d["state"].as_str() == Some("approved"));
            let rejected = decisions
                .iter()
                .any(|d| d["state"].as_str() == Some("rejected") || d["state"].as_str() == Some("declined"));
            let tail = if step == "closing" && approved {
                "\n\nОдобрено: коммить с трейлерами закрытия задачи и закончи ответ словом CLOSED."
            } else if step == "closing" && rejected {
                "\n\nВладелец отклонил. Переделай правки с учётом довода и позови approval-ask снова."
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
            "closing" => {
                // Чек-лист приёмки — прогон по пунктам: закрытие обязано
                // пройти каждый открытый пункт и отметить его в документе.
                let ready = self.door(project, "ready", json!({ "task": task })).await.unwrap_or(json!({}));
                let mut lines: Vec<String> = ready["items"].as_array().cloned().unwrap_or_default()
                    .iter().filter(|i| !i["done"].as_bool().unwrap_or(true))
                    .map(|i| format!("- [ ] {} {}", i["check"].as_str().unwrap_or(""), i["text"].as_str().unwrap_or("")))
                    .collect();
                lines.dedup();
                let list = if lines.is_empty() {
                    "открытых пунктов приёмки нет".to_owned()
                } else {
                    lines.join("\n")
                };
                (Some(fill(CLOSING, &[("{ready}", &list)])), session, Stage::Work)
            }
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
                            println!("{0} · прогон {task} ({1}) — полоса закрыта, ждёт фазы «{band_name}»",
                                bundle.name, run["state"].as_str().unwrap_or(""));
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
                    // ask-inbox уже пометил слово владельца прочитанным: молча
                    // continue потеряло бы его. Слово называем, владелец
                    // повторит.
                    println!("{0}: {e}", bundle.name);
                    if !decisions.is_empty() {
                        let _ = self.door(project, "run-state", json!({
                            "runId": rid, "state": "waiting",
                            "note": format!("слово владельца получено, но не прочитано ({e}); повторите решение"),
                            "session": run["sessionId"].as_str().unwrap_or("") })).await;
                    }
                    continue;
                }
            };
            if let Some(halt) = am["halt"].as_str().filter(|h| !h.is_empty()) {
                // Вставший автомат ждёт слова владельца (resume=1), а не круга.
                // Слово, пришедшее без resume, не двигает автомат — но и не
                // должно молча исчезать: называем, что оно ждёт своего часа.
                let note = if decisions.is_empty() {
                    format!("автомат встал: {halt}")
                } else {
                    format!("автомат встал: {halt}; слово владельца есть — снимите остановку resume=1")
                };
                if run["note"].as_str() != Some(&note) {
                    println!("{0} · прогон {task} — {note}", bundle.name);
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
            if approved {
                // Одобрение привязано к дереву: коммитить разрешается только то,
                // что владелец видел. Голова на момент одобрения лежит в событии
                // «одобрение»; дерево, изменившееся после, требует нового слова.
                let recorded = approval_head(&run["events"].as_array().cloned().unwrap_or_default());
                // У ГОЛОВЫ ТРИ СОСТОЯНИЯ, А НЕ ДВА: та же, другая и НЕ ПРОЧИТАНА.
                //
                // Отказ `rev-parse` давал пустую строку, она не равна
                // записанной, и сессия получала «дерево уехало после
                // одобрения» на неподвижном дереве. Но молча пропускать
                // сверку нельзя тем более: дальше одобрение снимает запрет на
                // коммит, и право коммитить выдавалось бы по одобрению,
                // которое не с чем сверить. Непрочитанная голова паркует
                // прогон так же, как уехавшая, — и говорит об этом своими
                // словами.
                let now = Self::git(&bundle.repo, &["rev-parse", "HEAD"]);
                let note = match (&recorded, &now) {
                    (Some(was), Err(why)) if !was.is_empty() => {
                        Some(format!("голова дерева не читается ({why}) — сверить одобрение не с чем"))
                    }
                    (Some(was), Ok(now)) if !was.is_empty() && was != now.trim() => {
                        Some(format!("дерево изменилось после одобрения ({was} → {}) — повторите approval", now.trim()))
                    }
                    _ => None,
                };
                if let Some(note) = note {
                    let _ = self.door(project, "question-ask", json!({
                        "runId": rid, "title": format!("{task}: одобрение не сходится с деревом"),
                        "body": note })).await;
                    let _ = self.door(project, "run-state", json!({
                        "runId": rid, "state": "waiting", "note": note,
                        "session": run["sessionId"].as_str().unwrap_or("") })).await;
                    continue;
                }
            }
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
                    if !decisions.is_empty() {
                        let _ = self.door(project, "run-state", json!({
                            "runId": rid, "state": "waiting",
                            "note": format!("слово владельца получено, но гейт не прочитан ({e}); повторите решение"),
                            "session": run["sessionId"].as_str().unwrap_or("") })).await;
                    }
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
                // молча. Прогон остаётся running — следующий круг зовёт ту же
                // сессию снова; счёт молчаний в событиях базы переживает
                // перезапуск воркера, и третье подряд падает. Состояние
                // waiting здесь было бы ловушкой: воркер пропускает waiting
                // без слова владельца, и повтор никогда бы не случился.
                let n = 1 + run["events"].as_array().cloned().unwrap_or_default()
                    .iter().filter(|e| e["kind"].as_str() == Some("молчание")).count() as i32;
                let _ = self.door(project, "run-event",
                    json!({ "runId": rid, "kind": "молчание", "text": cut(&answer, 4000) })).await;
                let (state, note) = if n >= SILENCE_LIMIT {
                    note_if("failed", format!("сессия молчит {n} раза подряд: {}", cut(&answer, 200)))
                } else {
                    note_if("running", format!("сессия молчит, попытка {n}: {}", cut(&answer, 200)))
                };
                let _ = self.door(project, "run-state",
                    json!({ "runId": rid, "state": state, "note": note, "session": kept_session })).await;
                return;
            }
            let answer_full = answer.clone();
            let answer = cut(&answer_full, 4000);
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
            let adv = if step == "closed" {
                am.clone() // автомат закрыт: вердиктов не ждёт, судит конвейер
            } else {
                let verdict = verdict_of(&answer_full);
                if matches!(verdict.as_str(), "BLOCKED" | "NEEDS CONTEXT") {
                    let _ = self.door(project, "question-ask", json!({
                        "runId": rid, "title": format!("{task}: {verdict}"),
                        "body": cut(&answer_full, 2000) })).await;
                    let _ = self.door(project, "run-state", json!({
                        "runId": rid, "state": "waiting",
                        "note": format!("ждёт владельца: {verdict} — {}", cut(&answer_full, 150)),
                        "session": kept_session })).await;
                    return;
                }
                if matches!(verdict.as_str(), "RETHINK" | "NEEDS FIX") {
                    // Довод ревью — в событиях: следующий шаг читает его оттуда
                    // и переживает перезапуск воркера. Пустые события не пишутся:
                    // они вытесняли бы «план» из окна в последние события.
                    let _ = self.door(project, "run-event",
                        json!({ "runId": rid, "kind": "находки", "text": answer })).await;
                }
                match self.door(project, "run-automaton",
                    json!({ "runId": rid, "status": verdict, "note": cut(&answer_full, 300) })).await
                {
                    Ok(v) => v,
                    Err(e) => {
                        println!("{0}: {e}", bundle.name);
                        return;
                    }
                }
            };
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
                    "body": format!("Прогон {rid} задачи {task}: {halt}.\n\nСнять остановку конвейера может владелец: mh call run-automaton runId=… resume=1; задачу вести дальше — новым прогоном (run-start): этот уже failed.") })).await;
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
            if open_asks.iter().any(|a| a["kind"].as_str() == Some("approval")) {
                // Голова дерева на момент просьбы об одобрении: одобрение ниже
                // привяжется к этому состоянию, и коммит чужого дерева откажет.
                let _ = self.door(project, "run-event", json!({
                    "runId": rid, "kind": "одобрение", "text": format!("head {}", snapshot.0) })).await;
            }
            let now_gate = self.gate_items(project).await.unwrap_or_default();
            // Покраснение — поимённо: пункт, который был зелёным и перестал,
            // плюс пункт, объявившийся красным посреди прогона.
            let regression = regression(&was_gate, &now_gate);
            let (status_after, now_reached) = self.task_conveyor(project, task, &ladder).await.unwrap_or((json!({}), Vec::new()));
            let terminal_hit = terminal.iter().any(|t| now_reached.contains(t));
            let adv_step = adv["step"].as_str().unwrap_or("").to_owned();
            let adv_circle = adv["circle"].as_i64().unwrap_or(0);
            let current = status_after["current"].as_str().unwrap_or("");
            let (state, note) = if !open_asks.is_empty() {
                note_if("waiting", format!("ждёт решения владельца: {}", open_asks[0]["title"].as_str().unwrap_or("")))
            } else if !regression.is_empty() {
                let names: Vec<String> = regression.iter().take(4).map(|i| {
                    let title = now_gate.get(i).map(|(_, t)| t.as_str()).unwrap_or("");
                    format!("{} ({})", i, cut(title, 60))
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

const CLOSING: &str = r#"Ревью задачи {task} чистое. Осталось закрыть, и это прогон по чек-листу приёмки:
{ready}

Пункт приёмки считается проверенным, когда его проверка живёт тестом в дереве (пункт гейта видит это сам — `[x]` проставлять не нужно, закрытие это трейлер). Пройди список: у каждого пункта либо тест существует и проходит, либо его нет — тогда напиши тест или назови причину словом. Когда каждый пункт либо покрыт, либо объяснён — позови `mh call approval-ask runId={run} title="что коммитим" body="дифф коротко"` и остановись. Коммит с трейлерами закрытия — после одобрения владельца."#;

#[cfg(test)]
mod tests {
    use super::{cut, regression, verdict_of, Config, Worker};

    /// Набор без поля drive ведётся как прежде; false выключает только прогоны.
    #[test]
    fn a_bundle_drives_unless_it_says_otherwise() {
        let c: Config = serde_json::from_str(
            r#"{"projects":[{"name":"a","project":"p","repo":"/r"},
                            {"name":"b","project":"q","repo":"/s","drive":false}]}"#,
        )
        .expect("разбор runner.json");
        assert!(c.projects[0].drive, "набор без поля drive обязан вестись, как до поля");
        assert!(!c.projects[1].drive, "drive: false обязан выключать ведение прогонов");
    }
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

    /// Обрезка по границе символа: срез по байтам падает на кириллице, а
    /// ответы сессий длинные и кириллические. Падение тут вешало бы воркера
    /// в цикле перезапусков на первом же длинном отчёте.
    #[test]
    fn cut_never_splits_a_multibyte_char() {
        let s = "абвгд".repeat(500); // 5000 байт, 2500 символов
        for n in [200, 4000, 4095, 4096, 4097] {
            let c = cut(&s, n);
            assert!(s.starts_with(c), "префикс не порчен");
            assert!(c.len() <= n, "не длиннее просимого");
        }
        assert_eq!(cut("коротко", 4000), "коротко");
        assert_eq!(cut("", 10), "");
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

    /// Отказ git — ОТКАЗ, а не пустой вывод. На этом различии стоит вся
    /// запись прогона: не отличив их, прогонщик мерил чужое дерево и
    /// записывал замер как правду о стволе.
    #[test]
    fn a_refusing_git_is_not_an_empty_answer() {
        // Каталог — СВОЙ у каждого прогона. Общее имя в `/tmp` делало пробу
        // ложно красной, когда два прогона шли разом: чужая уборка попадала
        // между заведением каталога и `git init`.
        let tmp = std::env::temp_dir().join(format!("mh-not-a-repo-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let path = tmp.to_string_lossy().into_owned();
        assert!(Worker::git(&path, &["init", "--quiet"]).is_ok(), "успешный вызов остаётся успешным");
        // Репозиторий без коммитов: `rev-parse HEAD` отказывает здесь всегда —
        // и не зависит от того, лежит ли `TMPDIR` внутри чужого репозитория,
        // где «не репозиторий» перестаёт быть правдой.
        let out = Worker::git(&path, &["rev-parse", "HEAD"]);
        assert!(out.is_err(), "без коммитов головы нет, и это отказ словом, а вышло {out:?}");
        let status = Worker::git(&path, &["status", "--porcelain"]).expect("чистый статус читается");
        assert_eq!(status.trim(), "", "пустой вывод — это по-прежнему успех, а не отказ");
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Уборка выбирает ровно выход, которого прогон не назвал и не коснулся,
    /// и ничего вне `<дерево>/target`.
    #[test]
    fn the_sweep_chooses_exactly_the_unlisted_old_and_nothing_outside_target() {
        use std::path::PathBuf;
        use std::time::{Duration, SystemTime};
        let tmp = std::env::temp_dir().join(format!("mh-sweep-{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        let (tree, outside) = (tmp.join("tree"), tmp.join("outside"));
        let debug = tree.join("target/debug");
        let (deps, prints) = (debug.join("deps"), debug.join(".fingerprint"));
        let (old_unit, rewritten, fresh, bin) =
            ("old-1111111111111111", "rewritten-2222222222222222", "fresh-3333333333333333", "app-4444444444444444");
        for d in [&deps, &outside.join("debug/deps")] {
            std::fs::create_dir_all(d).expect("каталог заводится");
        }
        for u in [old_unit, rewritten, fresh, bin] {
            std::fs::create_dir_all(prints.join(u)).expect("каталог заводится");
            std::fs::write(prints.join(u).join("lib"), b"x").expect("файл пишется");
        }
        let files = [
            deps.join(format!("lib{old_unit}.rlib")),
            deps.join("libnew-5555555555555555.rlib"),
            deps.join(format!("lib{fresh}.rlib")),
            deps.join(bin),
            deps.join("keyless.txt"),
            outside.join("stale"),
            outside.join("debug/deps/libold-6666666666666666.rlib"),
        ];
        for f in &files {
            std::fs::write(f, b"x").expect("файл пишется");
        }
        // Бинарь cargo называет выложенной копией — жёсткой ссылкой на `deps`.
        std::fs::hard_link(deps.join(bin), debug.join("app")).expect("ссылка заводится");
        let old = SystemTime::now() - Duration::from_secs(2 * 3600);
        let age = |p: &std::path::Path| {
            std::fs::File::open(p).and_then(|f| f.set_modified(old)).expect("время ставится");
        };
        for f in files.iter().filter(|f| !f.ends_with("libnew-5555555555555555.rlib")) {
            age(f);
        }
        for u in [old_unit, fresh, bin] {
            age(&prints.join(u).join("lib"));
        }
        for u in [old_unit, rewritten, fresh, bin] {
            age(&prints.join(u));
        }
        age(&outside.join("debug/deps"));
        std::os::unix::fs::symlink(outside.join("stale"), deps.join("link-out-7777777777777777")).expect("ссылка");
        std::os::unix::fs::symlink(outside.join("debug"), tree.join("target/linked-profile")).expect("ссылка");
        // Перечень прогона: свежая единица и бинарь — старые по времени, но
        // названные cargo.
        let listed = vec![deps.join(format!("lib{fresh}.rlib")), debug.join("app")];
        let cutoff = SystemTime::now() - Duration::from_secs(3600);

        let base = tree.canonicalize().expect("дерево разрешается");
        let chosen: Vec<PathBuf> = super::stale_artefacts(&tree, &tree.join("target"), cutoff, &listed)
            .expect("выбор идёт")
            .into_iter()
            .map(|(p, _)| p.strip_prefix(&base).expect("выбор внутри дерева").to_owned())
            .collect();
        assert_eq!(
            chosen,
            vec![
                PathBuf::from(format!("target/debug/.fingerprint/{old_unit}")),
                PathBuf::from(format!("target/debug/deps/lib{old_unit}.rlib")),
            ],
            "выбрано ровно старое и неназванное: не новое, не переписанный отпечаток, не свежее из \
             перечня, не бинарь по жёсткой ссылке, не запись без хеша, не ссылки наружу"
        );

        let other = tmp.join("other-tree");
        std::fs::create_dir_all(&other).expect("каталог заводится");
        std::os::unix::fs::symlink(&outside, other.join("target")).expect("ссылка заводится");
        let refused = super::stale_artefacts(&other, &other.join("target"), cutoff, &listed);
        assert!(
            refused.as_ref().is_err_and(|why| why.contains("вне дерева")),
            "`target`, ведущий вон из дерева, — отказ с причиной, а вышло {refused:?}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Перечень — из stdout запроса cargo; свежие единицы в нём есть, а
    /// собранная запросом единица или пустой вывод — отказ.
    #[test]
    fn the_cargo_listing_names_fresh_artefacts_and_refuses_a_build() {
        let fresh = concat!(
            r#"{"reason":"compiler-artifact","package_id":"dep 0.1.0","fresh":true,"filenames":["/t/debug/build/dep-5da9ee4087ba1b89/build-script-build"]}"#, "\n",
            r#"{"reason":"build-script-executed","package_id":"dep 0.1.0","out_dir":"/t/debug/build/dep-0469e4080376578f/out"}"#, "\n",
            r#"{"reason":"compiler-artifact","package_id":"dep 0.1.0","fresh":true,"filenames":["/t/debug/deps/libdep-c05783f808fda616.rlib","/t/debug/deps/libdep-c05783f808fda616.rmeta"]}"#, "\n",
            r#"{"reason":"compiler-artifact","package_id":"app 0.1.0","fresh":true,"filenames":["/t/debug/deps/app-c23c86af9c2c3f98"],"executable":"/t/debug/deps/app-c23c86af9c2c3f98"}"#, "\n",
            r#"{"reason":"build-finished","success":true}"#, "\n",
        );
        let paths: Vec<String> = super::cargo_listing(fresh)
            .expect("свежий перечень читается")
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        assert_eq!(
            paths,
            [
                "/t/debug/build/dep-5da9ee4087ba1b89/build-script-build",
                "/t/debug/build/dep-0469e4080376578f/out",
                "/t/debug/deps/libdep-c05783f808fda616.rlib",
                "/t/debug/deps/libdep-c05783f808fda616.rmeta",
                "/t/debug/deps/app-c23c86af9c2c3f98",
            ]
        );
        let built = fresh.replacen(r#""package_id":"app 0.1.0","fresh":true"#, r#""package_id":"app 0.1.0","fresh":false"#, 1);
        let refused = super::cargo_listing(&built);
        assert!(
            refused.as_ref().is_err_and(|why| why.contains("app 0.1.0")),
            "единица, собранная запросом, — отказ с её именем, а вышло {refused:?}"
        );
        assert!(super::cargo_listing("").is_err(), "пустой вывод — отказ, а не пустой перечень");
    }

    /// Уборка идёт только по чистому дереву и ответившему запросу перечня;
    /// каждый отказ называет свою причину.
    #[test]
    fn the_sweep_runs_only_on_a_clean_tree_with_a_listing() {
        let list = || vec![std::path::PathBuf::from("/t/debug/deps/libdep-c05783f808fda616.rlib")];
        assert_eq!(super::sweep_list(false, Some(Ok(list()))), Ok(list()));
        let dirty = super::sweep_list(true, Some(Ok(list())));
        assert!(
            dirty.as_ref().is_err_and(|why| why.contains("кто-то ещё")),
            "грязное дерево — отказ даже с перечнем, а вышло {dirty:?}"
        );
        for why in ["запрос перечня не уложился в 1800 с", "запрос перечня отказал: error"] {
            assert_eq!(super::sweep_list(false, Some(Err(why.into()))), Err(why.into()), "причина запроса доходит");
        }
        assert!(super::sweep_list(false, None).is_err(), "неспрошенный перечень — отказ");
    }

    /// Потолок времени убивает и внуков: cargo под `bash -c "( … )"` не
    /// переживает прогон, упавший по времени.
    #[tokio::test]
    async fn a_timed_out_command_takes_its_grandchildren_along() {
        let tmp = std::env::temp_dir().join(format!("mh-within-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let pidfile = tmp.join("pid");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &format!("( sleep 60 & echo $! > {} ; wait )", pidfile.display())]);
        assert!(super::output_within(cmd, 1, b"").await.expect("команда заводится").is_none(), "потолок вышел");
        let pid = std::fs::read_to_string(&pidfile).expect("внук записал себя");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let alive = std::path::Path::new(&format!("/proc/{}", pid.trim())).exists();
        if alive {
            std::process::Command::new("kill").args(["-KILL", pid.trim()]).status().ok();
        }
        std::fs::remove_dir_all(&tmp).ok();
        assert!(!alive, "внук {} пережил потолок", pid.trim());
    }

    /// Обычный путь отдаёт вывод обоих потоков и код выхода целиком.
    #[tokio::test]
    async fn a_command_within_its_limit_gives_its_output_and_code() {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", "echo o; echo e >&2; exit 3"]);
        let out = super::output_within(cmd, 5, b"").await.expect("команда заводится").expect("уложилась");
        assert_eq!((out.stdout.as_slice(), out.stderr.as_slice(), out.status.code()), (&b"o\n"[..], &b"e\n"[..], Some(3)));
    }

    /// Ввод доходит до потомка и закрывается: `cat` кончается сам.
    #[tokio::test]
    async fn the_input_reaches_the_command_and_ends() {
        let cmd = tokio::process::Command::new("cat");
        let prompt = "просьба сессии".as_bytes();
        let out = super::output_within(cmd, 5, prompt).await.expect("команда заводится").expect("уложилась");
        assert_eq!(out.stdout, prompt);
    }

    /// Трубу держит процесс, ушедший из группы: потолок всё равно кончает
    /// ожидание, а не ждёт конца чужого процесса.
    #[tokio::test]
    async fn a_pipe_held_outside_the_group_does_not_hold_the_worker() {
        let tmp = std::env::temp_dir().join(format!("mh-held-pipe-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let pidfile = tmp.join("pid");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &format!("setsid sleep 20 & echo $! > {}; exit 0", pidfile.display())]);
        let begun = std::time::Instant::now();
        let out = super::output_within(cmd, 1, b"").await.expect("команда заводится");
        let took = begun.elapsed();
        if let Ok(pid) = std::fs::read_to_string(&pidfile) {
            std::process::Command::new("kill").args(["-KILL", pid.trim()]).status().ok();
        }
        std::fs::remove_dir_all(&tmp).ok();
        assert!(out.is_none(), "вывод не дочитан — это потолок, а не ответ");
        assert!(took < std::time::Duration::from_secs(5), "ожидание кончилось через {took:?}");
    }

    /// Вожак вышел, а его внук в той же группе держит вывод: потолок гасит
    /// внука. Номер группы снимается до ожидания — после `wait()` tokio
    /// номера потомка уже не отдаёт.
    #[tokio::test]
    async fn a_grandchild_outliving_its_leader_is_put_out() {
        let tmp = std::env::temp_dir().join(format!("mh-outlived-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let pidfile = tmp.join("pid");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &format!("sleep 30 & echo $! > {}; exit 0", pidfile.display())]);
        assert!(super::output_within(cmd, 1, b"").await.expect("команда заводится").is_none(), "потолок вышел");
        let pid = std::fs::read_to_string(&pidfile).expect("внук записал себя");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let alive = std::path::Path::new(&format!("/proc/{}", pid.trim())).exists();
        if alive {
            std::process::Command::new("kill").args(["-KILL", pid.trim()]).status().ok();
        }
        std::fs::remove_dir_all(&tmp).ok();
        assert!(!alive, "внук {} пережил вышедшего вожака и потолок", pid.trim());
    }

    /// Группа, глухая к TERM, добивается KILL по истечении отсрочки.
    #[tokio::test]
    async fn a_group_deaf_to_term_is_killed_after_the_grace() {
        let tmp = std::env::temp_dir().join(format!("mh-deaf-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let pidfile = tmp.join("pid");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &format!("trap '' TERM; sleep 60 & echo $! > {}; wait", pidfile.display())]);
        let mut child = cmd.process_group(0).kill_on_drop(true).spawn().expect("команда заводится");
        while std::fs::read_to_string(&pidfile).map_or(true, |p| !p.ends_with('\n')) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let group = child.id();
        super::put_out(&mut child, group, std::time::Duration::from_secs(1)).await;
        let pid = std::fs::read_to_string(&pidfile).expect("внук записал себя");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let alive = std::path::Path::new(&format!("/proc/{}", pid.trim())).exists();
        if alive {
            std::process::Command::new("kill").args(["-KILL", pid.trim()]).status().ok();
        }
        std::fs::remove_dir_all(&tmp).ok();
        assert!(!alive, "глухой к TERM внук {} пережил отсрочку", pid.trim());
    }

    /// Внук в СВОЕЙ группе гасится через TERM нашего вожака — так nextest
    /// гасит свои тесты. Вожак ловит TERM и передаёт его группе внука; KILL
    /// первым сигналом не дал бы ему этого сделать.
    #[tokio::test]
    async fn a_grandchild_in_its_own_group_is_put_out_through_the_leader() {
        let tmp = std::env::temp_dir().join(format!("mh-own-group-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let pidfile = tmp.join("pid");
        let script = format!(
            "trap 'kill -TERM -- -$p; exit' TERM; setsid sleep 60 & p=$!; echo $p > {}; wait",
            pidfile.display()
        );
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &script]);
        assert!(super::output_within(cmd, 1, b"").await.expect("команда заводится").is_none(), "потолок вышел");
        let pid = std::fs::read_to_string(&pidfile).expect("внук записал себя");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let alive = std::path::Path::new(&format!("/proc/{}", pid.trim())).exists();
        if alive {
            std::process::Command::new("kill").args(["-KILL", pid.trim()]).status().ok();
        }
        std::fs::remove_dir_all(&tmp).ok();
        assert!(!alive, "внук {} в своей группе пережил потолок", pid.trim());
    }

    fn row(name: &str, verdict: &str, binary: &str) -> (String, String, String) {
        (name.to_owned(), verdict.to_owned(), binary.to_owned())
    }

    /// Nextest: вердикт, бинарь и имя — одной строкой, `крейт::` снимается.
    #[test]
    fn nextest_lines_carry_their_binary() {
        let text = "        PASS [   0.002s] (1/313) tot-core::mirror_foo s8_ac_6_имя\n\
                    \x20       FAIL [   0.010s] (2/313) tot-core::lib save::tests::bom_survives\n\
                    \x20       SKIP [   0.000s] (3/313) tot-core::slow slow_one\n\
                    \x20       PASS [   0.002s] лишнее слово здесь\n";
        assert_eq!(super::test_lines(text), vec![
            row("s8_ac_6_имя", "passed", "mirror_foo"),
            row("save::tests::bom_survives", "failed", "lib"),
            row("slow_one", "ignored", "slow"),
        ]);
    }

    /// `cargo test`: бинарь — из предшествующей `Running`, итог блока не
    /// проверка.
    #[test]
    fn cargo_test_lines_take_the_binary_from_running() {
        let text = "     Running tests/mirror_x.rs (target/debug/deps/mirror_x-1a2b)\n\
                    test a_red_one ... FAILED\n\
                    test a_green_one ... ok\n\
                    test result: FAILED. 1 passed; 1 failed\n\
                    \x20    Running tests/plain.rs (target/debug/deps/plain-3c4d)\n\
                    test skipped_one ... ignored, нужна сеть\n";
        assert_eq!(super::test_lines(text), vec![
            row("a_red_one", "failed", "mirror_x.rs"),
            row("a_green_one", "passed", "mirror_x.rs"),
            row("skipped_one", "ignored", "plain.rs"),
        ]);
    }

    /// Журнал работы GitHub: BOM, время перед каждой строкой, CRLF и путь
    /// Windows. Строки — дословно из прогона `36642293742` пары `V5-T135`;
    /// без снятия времени разбор отвечал «проверок ноль».
    #[test]
    fn a_github_job_log_parses_like_a_local_run() {
        let text = "\u{feff}2026-09-29T22:54:42.7396393Z Current runner version: '2.328.0'\r\n\
                    2026-09-29T22:56:47.5592055Z      Running tests\\mirror_revoke_takes_the_refusals_off.rs (target\\debug\\deps\\mirror_revoke_takes_the_refusals_off-1c368eaaebb7b3a5.exe)\r\n\
                    2026-09-29T22:56:47.5656672Z running 1 test\r\n\
                    2026-09-29T22:56:47.5896990Z test m5_t135_revoke_takes_the_refusals_off_too ... FAILED\r\n\
                    2026-09-29T22:56:47.5901204Z     m5_t135_revoke_takes_the_refusals_off_too\r\n\
                    2026-09-29T22:56:47.5903487Z test result: FAILED. 0 passed; 1 failed; 0 ignored\r\n\
                    2026-09-29T22:56:48.0000000Z         PASS [   0.002s] (1/2) tot-core::mirror_foo s8_ac_6_имя\r\n";
        assert_eq!(super::test_lines(text), vec![
            row("s8_ac_6_имя", "passed", "mirror_foo"),
            row("m5_t135_revoke_takes_the_refusals_off_too", "failed", "mirror_revoke_takes_the_refusals_off.rs"),
        ]);
        // Строка, где время стоит не в начале, — не журнал GitHub: её не трогают.
        assert!(super::test_lines("note 2026-09-29T22:56:47.5Z test x ... ok\n").is_empty());
    }

    /// Цветной вывод (`CARGO_TERM_COLOR=always`): без снятия цвета вердикт не
    /// узнавался, и прогон отмечался взятым без единой строки.
    #[test]
    fn colored_output_is_parsed_like_plain() {
        let text = "2026-09-29T22:56:47.5592055Z \x1b[1m\x1b[92m     Running\x1b[0m tests\\mirror_x.rs (x.exe)\n\
                    2026-09-29T22:56:47.5896990Z test red_one ... \x1b[31mFAILED\x1b[0m\n\
                    2026-09-29T22:56:47.6000000Z         \x1b[32;1mPASS\x1b[0m [   0.002s] tot-core::plain green_one\n";
        assert_eq!(super::test_lines(text), vec![
            row("green_one", "passed", "plain"),
            row("red_one", "failed", "mirror_x.rs"),
        ]);
    }

    #[test]
    fn origin_names_its_github_repo() {
        for url in ["git@github.com:tot-space/tot-ade.git", "https://github.com/tot-space/tot-ade",
                    "https://github.com/tot-space/tot-ade.git\n", "ssh://git@github.com/tot-space/tot-ade.git"] {
            assert_eq!(super::github_slug(url).as_deref(), Some("tot-space/tot-ade"), "{url}");
        }
        for url in ["/srv/git/tot-ade.git", "git@gitlab.com:tot-space/tot-ade.git", "https://github.com/tot-space"] {
            assert_eq!(super::github_slug(url), None, "{url}");
        }
    }

    /// Прогон годится в факт о стволе, только если он со ствола САМОГО набора.
    /// Порча, которую ловит проверка: форк с веткой `main` или
    /// `pull_request_target` закрывают красную фазу падением не из набора.
    #[test]
    fn only_a_trunk_run_of_the_project_itself_is_taken() {
        let run = |branch: &str, event: &str, from: &str| serde_json::json!({
            "id": 36642293742_i64, "run_attempt": 2, "head_sha": "6009f67", "run_started_at": "2026-09-29T22:54:37Z",
            "head_branch": branch, "event": event, "head_repository": { "full_name": from },
        });
        assert_eq!(
            super::trunk_run(&run("main", "workflow_dispatch", "tot-space/tot-ade"), "tot-space/tot-ade", "main"),
            Some(super::TrunkRun { id: 36642293742, attempt: 2, sha: "6009f67".into(),
                                   started: "2026-09-29T22:54:37Z".into() }),
        );
        for (branch, event, from) in [("main", "push", "someone/tot-ade"),
                                      ("main", "pull_request_target", "tot-space/tot-ade"),
                                      ("main", "pull_request", "tot-space/tot-ade"),
                                      ("v5-t135-revoke-takes-refusals", "workflow_dispatch", "tot-space/tot-ade")] {
            assert_eq!(super::trunk_run(&run(branch, event, from), "tot-space/tot-ade", "main"), None,
                       "{branch} · {event} · {from}");
        }
    }

    /// Коммит прогона должен лежать на `origin/<ствол>`: коммит с ветки, как
    /// у прогона `36642293742`, не берётся, даже если GitHub назвал ветку стволом.
    #[test]
    fn a_commit_off_the_trunk_is_not_on_it() {
        let tmp = std::env::temp_dir().join(format!("mh-on-trunk-{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).expect("каталог заводится");
        let path = tmp.to_string_lossy().into_owned();
        let git = |args: &[&str]| Worker::git(&path, args).expect("git в пробе");
        git(&["init", "--quiet", "-b", "main"]);
        let commit = |msg: &str| {
            git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "--allow-empty", "-m", msg]);
            git(&["rev-parse", "HEAD"]).trim().to_owned()
        };
        let trunk = commit("ствол");
        git(&["update-ref", "refs/remotes/origin/main", &trunk]);
        git(&["checkout", "--quiet", "-b", "side"]);
        let side = commit("ветка");
        assert!(super::on_trunk(&path, &trunk, "main"), "коммит ствола на стволе");
        assert!(!super::on_trunk(&path, &side, "main"), "коммит ветки — нет");
        assert!(!super::on_trunk(&path, "0000000000000000000000000000000000000000", "main"),
                "неизвестный коммит — нет: доказать нечем");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
