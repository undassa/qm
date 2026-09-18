//! Пускатель сессий: слушает свой сокет и говорит с docker за всех.
//!
//! Запускается своей службой под пользователем, которому docker разрешён, и
//! больше никем: сервер харнеса смотрит наружу через портал, и держать в одном
//! процессе поверхность и право root — значит объявить границу и не построить
//! её. Словарь — четыре слова (`up`, `down`, `state`, `log`), они в
//! `launcher.rs` и там же проверены пробами.
//!
//! Разговор: одна строка JSON — один ответ строкой JSON. Ни потока, ни сессии,
//! ни состояния в памяти: состояние спрашивается у docker, который его и
//! держит. Права на сокет — единственный вход: кто может писать в него, тот и
//! зовёт, и это решается правами файла, а не проверкой внутри.

use mh_server::launcher::{container_of, name_is_plain, run_args, worktree_is_allowed, Ask, Said, Up};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Настройка пускателя: то, чего зовущий не выбирает. Образ, сеть, прокси и
/// корень рабочих деревьев — граница, а не довод вызова.
struct Rules {
    image: String,
    network: String,
    proxy: String,
    harness_url: String,
    root: String,
    key: String,
    docker: String,
}

fn required(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("mh-launcher: не задано {name}; без этого граница не описана, и пускатель не поднимается");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let rules = Rules {
        image: required("MH_SESSION_IMAGE"),
        network: required("MH_SESSION_NETWORK"),
        proxy: required("MH_SESSION_PROXY"),
        harness_url: required("MH_URL"),
        root: required("MH_WORKTREE_ROOT"),
        // Ключ модели общий на машину; набор задаёт свой вызовом — решение
        // владельца. Пустой здесь законен: набор со своим ключом обойдётся.
        key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        docker: std::env::var("MH_DOCKER").unwrap_or_else(|_| "docker".into()),
    };
    let path = std::env::var("MH_LAUNCHER_SOCKET").unwrap_or_else(|_| "/run/mh/launcher.sock".into());
    // Старый сокет снимается: файл переживает падение процесса, и без этого
    // пускатель не поднимется после нечистой остановки — «address in use» на
    // пустом месте.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mh-launcher: сокет {path} не занять: {e}");
            std::process::exit(2);
        }
    };
    // ПРАВА НА СОКЕТ — ЕДИНСТВЕННЫЙ ВХОД. Читает и пишет владелец и его группа;
    // все прочие не пишут ничего. Проверки «кто там» внутри нет намеренно: она
    // была бы вторым описанием того же права и разошлась бы с первым.
    if let Err(e) = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o660)) {
        eprintln!("mh-launcher: права на {path} не выставлены: {e}");
        std::process::exit(2);
    }
    eprintln!("mh-launcher: слушает {path}");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let rules = Rules {
                    image: rules.image.clone(),
                    network: rules.network.clone(),
                    proxy: rules.proxy.clone(),
                    harness_url: rules.harness_url.clone(),
                    root: rules.root.clone(),
                    key: rules.key.clone(),
                    docker: rules.docker.clone(),
                };
                tokio::spawn(async move { serve(stream, &rules).await });
            }
            Err(e) => eprintln!("mh-launcher: соединение не принято: {e}"),
        }
    }
}

async fn serve(stream: UnixStream, rules: &Rules) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).await.is_err() || line.trim().is_empty() {
        return;
    }
    let said = match serde_json::from_str::<Ask>(&line) {
        Ok(ask) => answer(ask, rules).await,
        // Неизвестное слово — отказ с перечнем: молчаливое «ничего не сделал»
        // читается зовущим как «сделал».
        Err(e) => Said {
            ok: false,
            said: format!("не разобрано: {e}. Слова: up · down · state · log"),
        },
    };
    let mut out = serde_json::to_string(&said).unwrap_or_else(|_| "{\"ok\":false}".into());
    out.push('\n');
    let _ = reader.into_inner().write_all(out.as_bytes()).await;
}

async fn answer(ask: Ask, rules: &Rules) -> Said {
    match ask {
        Ask::Up { session, project, worktree, secret, key } => {
            if !name_is_plain(&session) || !name_is_plain(&project) {
                return refused("имя сессии или набора не годится: латиница, цифры и дефис");
            }
            if !worktree_is_allowed(&worktree, &rules.root) {
                return refused(&format!("рабочее дерево вне {}: смонтировать чужое нельзя", rules.root));
            }
            if secret.trim().is_empty() {
                return refused("сессия без секрета не поднимается: ей нечем себя назвать");
            }
            let up = Up {
                session,
                project,
                worktree,
                secret,
                key: if key.trim().is_empty() { rules.key.clone() } else { key },
                image: rules.image.clone(),
                network: rules.network.clone(),
                proxy: rules.proxy.clone(),
                harness_url: rules.harness_url.clone(),
            };
            docker(rules, run_args(&up)).await
        }
        Ask::Down { session } => {
            if !name_is_plain(&session) {
                return refused("имя сессии не годится");
            }
            docker(rules, vec!["rm".into(), "--force".into(), container_of(&session)]).await
        }
        Ask::State { session } => {
            // Свои контейнеры узнаются МЕТКОЙ, а не именем: на машине их
            // полтора десятка, от базы набора до личного помощника владельца, и
            // трогать чужое пускатель не должен даже чтением.
            let mut args: Vec<String> = ["ps", "--all", "--filter", "label=mh.session"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
            if !session.is_empty() {
                if !name_is_plain(&session) {
                    return refused("имя сессии не годится");
                }
                args.extend(["--filter".into(), format!("name={}", container_of(&session))]);
            }
            args.extend(["--format".into(), "{{.Names}}\t{{.Status}}".into()]);
            docker(rules, args).await
        }
        Ask::Log { session, lines } => {
            if !name_is_plain(&session) {
                return refused("имя сессии не годится");
            }
            docker(
                rules,
                vec!["logs".into(), "--tail".into(), lines.min(2000).to_string(), container_of(&session)],
            )
            .await
        }
    }
}

fn refused(why: &str) -> Said {
    Said { ok: false, said: why.to_owned() }
}

/// Позвать docker и отдать сказанное как есть.
///
/// Доводы приходят ВЕКТОРОМ и уезжают вектором: оболочки между пускателем и
/// docker нет, и подставить в неё нечего.
async fn docker(rules: &Rules, args: Vec<String>) -> Said {
    match tokio::process::Command::new(&rules.docker).args(&args).output().await {
        Ok(out) => {
            let said = String::from_utf8_lossy(if out.status.success() { &out.stdout } else { &out.stderr })
                .trim()
                .to_owned();
            Said { ok: out.status.success(), said }
        }
        Err(e) => Said { ok: false, said: format!("docker не позвался: {e}") },
    }
}
