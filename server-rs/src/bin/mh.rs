//! `mh` — клиент харнеса. Отдельная сборка, а не подкоманда сервера.
//!
//! Разделение не косметическое. Клиента носят по чужим машинам: он едет туда,
//! где лежит репозиторий, и запускается скиллами. Тащить с ним весь сервер —
//! значит возить сетевой слой, пул к Postgres и все проекции туда, где ничего
//! этого не нужно и где ничего этого быть не должно.
//!
//! Здесь нет ни базы, ни схемы, ни разбора набора. Есть адрес, секрет края и
//! имя — и умение спросить сервер.
//!
//! Три способа звать:
//!   mh call <ручка> [имя=значение …]   спросить ручку
//!   mh tools                            перечень ручек — у сервера, не свой
//!   mh mcp                              тот же разговор по stdio, для агента

#[path = "../reproject/ids.rs"]
mod ids;
#[path = "../yaml.rs"]
mod yaml;
#[path = "../repo_corpus.rs"]
mod repo_corpus;
#[path = "../client.rs"]
mod client;

fn main() {
    let mut args = std::env::args().skip(1);
    let what = args.next().unwrap_or_default();
    if what.is_empty() || what == "--help" || what == "-h" {
        eprintln!(
            "mh — клиент харнеса\n\n\
             \x20 mh call <ручка> [имя=значение …]   спросить ручку\n\
             \x20 mh tools                           какие ручки есть\n\
             \x20 mh mcp                             разговор по stdio для агента\n\
             \x20 mh install [куда]                  умения, субагенты и связка — с сервера\n\
             \x20 mh guard                           сторож: наряд хука со входа, отказ — выходом 2\n\
             \x20 mh sense [что]                     снять факты с репозитория и подать серверу\n\n\
             Доводы: имя=значение · имя=@файл (строкой) · имя:=<json> · имя:=@файл (json)\n\
             Окружение: MH_URL (по умолчанию http://127.0.0.1:8096), MH_PROJECT,\n\
             \x20           MH_PRINCIPAL, MH_EDGE_SECRET\n"
        );
        std::process::exit(2);
    }
    let door = match client::Door::from_env() {
        Ok(d) => d,
        Err(why) => {
            eprintln!("mh {what}: {why}");
            std::process::exit(2);
        }
    };
    match what.as_str() {
        "mcp" => client::serve_mcp(door),
        "guard" => std::process::exit(client::guard(&door)),
        "sense" => {
            let only = args.next();
            match client::sense(&door, only.as_deref()) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(why) => {
                    eprintln!("mh sense: {why}");
                    std::process::exit(2);
                }
            }
        }
        "install" => {
            let into = args.next().unwrap_or_else(|| ".".to_owned());
            match client::install(&door, &into) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(why) => {
                    eprintln!("mh install: {why}");
                    std::process::exit(2);
                }
            }
        }
        "tools" => match door.tools() {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
            Err(why) => {
                eprintln!("mh tools: {why}");
                std::process::exit(2);
            }
        },
        "call" => {
            let Some(name) = args.next() else {
                eprintln!("mh call: не названа ручка");
                std::process::exit(2);
            };
            let rest: Vec<String> = args.collect();
            let parsed = match client::args_of(&rest) {
                Ok(a) => a,
                Err(why) => {
                    eprintln!("mh call: {why}");
                    std::process::exit(2);
                }
            };
            match door.call(&name, &parsed) {
                Ok((v, refused)) => {
                    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                    // Отказ — не успех. Оболочка обязана его различать: скрипт на
                    // `set -e` иначе пройдёт мимо и понесёт отказ дальше как ответ.
                    if refused {
                        std::process::exit(1);
                    }
                }
                Err(why) => {
                    eprintln!("mh call: {why}");
                    std::process::exit(2);
                }
            }
        }
        other => {
            eprintln!("mh: не знаю такого действия: {other}");
            std::process::exit(2);
        }
    }
}
