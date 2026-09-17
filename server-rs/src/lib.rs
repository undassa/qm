//! Харнес: общий код сервера и клиента.
//!
//! Библиотекой, а не включением файлов по пути. Клиент `mh` подключал
//! `ids.rs`, `yaml.rs`, `repo_corpus.rs` и `door.rs` через `#[path]`, и каждая
//! функция, нужная только серверу, значилась в его сборке мёртвой: разбор имён
//! ругался тремя предупреждениями, которые нечем было погасить, кроме запрета
//! проверки. Один общий ящик — и то, что не зовёт клиент, зовёт сервер.

pub mod api;
pub mod client;
pub mod corpus;
pub mod db;
pub mod documents;
pub mod door;
pub mod entities;
pub mod identity;
pub mod kinds;
pub mod mcp;
pub mod parse;
pub mod projector;
pub mod projects;
pub mod repo_corpus;
pub mod reproject;
pub mod scheme;
pub mod store;
pub mod watch;
pub mod yaml;
