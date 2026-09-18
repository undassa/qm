//! Пускатель сессий: единственное место, которое говорит с docker.
//!
//! Право на docker — это право root: сокет docker позволяет поднять контейнер с
//! примонтированным корнем машины. Сегодня им владеет каждая агентская сессия —
//! `undassa` состоит в группе `docker`, — и потому «сессия без sudo» на этой
//! машине значит ровно ничего.
//!
//! Пускатель эту связь разрывает: команда docker собирается ЗДЕСЬ и целиком,
//! а зовущий передаёт только имя сессии, набор и рабочее дерево. Ни довода, ни
//! флага, ни образа снаружи не приходит: словарь из четырёх слов читается
//! глазами целиком, и это единственная его защита, которая не зависит от того,
//! кто сегодня умеет писать в сокет.
//!
//! Решение владельца по #19: пускатель — отдельный процесс, сеть — прокси со
//! списком, ключ модели задаётся проекту, при отсутствии — общий на машину.

use serde::{Deserialize, Serialize};

/// Что пускателю велено сделать. Больше ничего он не умеет.
#[derive(Debug, Deserialize)]
#[serde(tag = "делать", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Ask {
    /// Поднять сессию: имя, набор, рабочее дерево, секрет.
    Up {
        session: String,
        project: String,
        worktree: String,
        secret: String,
        /// Ключ модели этого набора; пусто — общий на машину.
        #[serde(default)]
        key: String,
    },
    /// Снести сессию вместе с контейнером.
    Down { session: String },
    /// Состояние: одна сессия либо все.
    State {
        #[serde(default)]
        session: String,
    },
    /// Последние строки журнала сессии.
    Log {
        session: String,
        #[serde(default = "hundred")]
        lines: u32,
    },
}

fn hundred() -> u32 {
    100
}

#[derive(Debug, Serialize)]
pub struct Said {
    pub ok: bool,
    pub said: String,
}

/// Имя контейнера сессии. Одно место, которое его знает: по нему пускатель
/// находит СВОИ контейнеры и не трогает чужие — на этой машине их полтора
/// десятка, от базы набора до личного помощника владельца.
pub fn container_of(session: &str) -> String {
    format!("mh-session-{session}")
}

/// Имя сессии годится, если по нему нельзя ничего подставить: оно едет в имя
/// контейнера и в метку. Латиница, цифры и дефис — всё.
pub fn name_is_plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !name.starts_with('-')
}

/// Путь рабочего дерева годится, если он абсолютный, без «..» и лежит под
/// объявленным корнем: монтировать чужое — то же самое, что отдать машину.
pub fn worktree_is_allowed(path: &str, root: &str) -> bool {
    let clean = std::path::Path::new(path);
    clean.is_absolute()
        && !path.contains("..")
        && clean.starts_with(root)
        && path.chars().all(|c| !c.is_control() && c != '"' && c != '\'' && c != '$')
}

/// Доводы `docker run` для сессии — собранные ЗДЕСЬ и целиком.
///
/// Каждый запрет назван доводом, потому что снять его однажды «на пять минут»
/// будет некому объяснить:
///   · не root и без прибавления прав — внутри сессия не станет никем другим;
///   · сокета docker нет — иначе граница снимается одной командой изнутри;
///   · сеть только своя, с прокси по списку, — наружу мимо списка хода нет;
///   · рабочее дерево и только оно; корень машины не виден;
///   · память и процессы ограничены: сессия, забывшая себя, не уносит машину.
pub fn run_args(up: &Up) -> Vec<String> {
    let mut a: Vec<String> = ["run", "--detach", "--init"].iter().map(|s| (*s).to_owned()).collect();
    a.extend(["--name".into(), container_of(&up.session)]);
    a.extend(["--label".into(), format!("mh.session={}", up.session)]);
    a.extend(["--label".into(), format!("mh.project={}", up.project)]);
    a.extend(["--user".into(), "10001:10001".into()]);
    a.extend(["--security-opt".into(), "no-new-privileges".into()]);
    a.extend(["--cap-drop".into(), "ALL".into()]);
    a.extend(["--network".into(), up.network.clone()]);
    a.extend(["--memory".into(), "4g".into()]);
    a.extend(["--pids-limit".into(), "512".into()]);
    a.extend(["--volume".into(), format!("{}:/work:rw", up.worktree)]);
    a.extend(["--workdir".into(), "/work".into()]);
    a.extend(["--env".into(), format!("MH_SESSION_SECRET={}", up.secret)]);
    a.extend(["--env".into(), format!("MH_PROJECT={}", up.project)]);
    a.extend(["--env".into(), format!("MH_URL={}", up.harness_url)]);
    a.extend(["--env".into(), format!("ANTHROPIC_API_KEY={}", up.key)]);
    a.extend(["--env".into(), format!("HTTPS_PROXY={}", up.proxy)]);
    a.extend(["--env".into(), format!("HTTP_PROXY={}", up.proxy)]);
    a.push(up.image.clone());
    a
}

/// Всё, что нужно для подъёма: часть приходит от зовущего, часть — настройка
/// самого пускателя, и перепутать их нельзя.
#[derive(Debug)]
pub struct Up {
    pub session: String,
    pub project: String,
    pub worktree: String,
    pub secret: String,
    pub key: String,
    pub image: String,
    pub network: String,
    pub proxy: String,
    pub harness_url: String,
}

#[cfg(test)]
mod dictionary {
    use super::*;

    #[test]
    fn a_name_that_could_slip_into_a_command_is_refused() {
        assert!(name_is_plain("tot-ade-7d"));
        assert!(!name_is_plain(""));
        assert!(!name_is_plain("-начало-с-дефиса"));
        assert!(!name_is_plain("имя с пробелом"));
        assert!(!name_is_plain("a;rm -rf /"));
        assert!(!name_is_plain("a$(whoami)"));
        assert!(!name_is_plain(&"д".repeat(65)));
    }

    #[test]
    fn a_worktree_outside_the_declared_root_is_refused() {
        let root = "/opt/src/github.com";
        assert!(worktree_is_allowed("/opt/src/github.com/tot-space/tot-ade", root));
        assert!(!worktree_is_allowed("/etc", root), "чужой каталог");
        assert!(!worktree_is_allowed("relative/path", root), "не абсолютный");
        assert!(!worktree_is_allowed("/opt/src/github.com/../../etc", root), "выход наверх");
        assert!(!worktree_is_allowed("/opt/src/github.com/a\"b", root), "кавычка в пути");
    }

    /// Запреты перечислены здесь, а не только в доводе: снятый запрет обязан
    /// уронить пробу, а не пройти ревью «мелкой правкой аргументов».
    #[test]
    fn the_run_carries_every_boundary() {
        let args = run_args(&Up {
            session: "tot-ade-7d".into(),
            project: "ae7ec7fa".into(),
            worktree: "/opt/src/github.com/tot-space/tot-ade".into(),
            secret: "тайна".into(),
            key: "ключ".into(),
            image: "mh-session:latest".into(),
            network: "mh-agents".into(),
            proxy: "http://egress:3128".into(),
            harness_url: "http://mh:8096".into(),
        });
        let line = args.join(" ");
        for must in [
            "--user 10001:10001",
            "--security-opt no-new-privileges",
            "--cap-drop ALL",
            "--network mh-agents",
            "--pids-limit 512",
            "--name mh-session-tot-ade-7d",
            "/opt/src/github.com/tot-space/tot-ade:/work:rw",
        ] {
            assert!(line.contains(must), "в запуске нет «{must}»: {line}");
        }
        assert!(!line.contains("/var/run/docker.sock"), "сокет docker внутрь не едет");
        assert!(!line.contains("--privileged"), "привилегий не бывает");
        assert!(!line.contains("--volume /:"), "корень машины внутрь не едет");
    }
}
