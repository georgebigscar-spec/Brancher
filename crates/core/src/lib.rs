//! Ядро: модели Git-объектов, состояние приложения и редьюсеры.
//!
//! Крейт не делает ввода-вывода. UI превращает ввод в [`Action`], [`reduce`] меняет
//! [`AppState`] и возвращает [`Effect`]-ы (что запустить в фоне), а результаты фоновых
//! задач применяются через [`apply_result`].

pub mod diff;
pub mod graph;
pub mod model;
pub mod state;

pub use diff::*;
pub use graph::*;
pub use model::*;
pub use state::*;
