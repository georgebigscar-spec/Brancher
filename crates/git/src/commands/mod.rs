//! Построители аргументов и парсеры. Всё здесь — чистые функции без запуска процессов.

pub mod absorb;
pub mod apply;
pub mod blame;
pub mod branch;
pub mod clone;
pub mod commit;
pub mod diff;
pub mod log;
pub mod merge;
pub mod oplog;
pub mod progress;
pub mod rebase;
pub mod refs;
pub mod remote;
pub mod stage;
pub mod stash;
pub mod status;
pub mod tag;

/// Разбивает вывод по NUL, отбрасывая пустой хвост.
pub(crate) fn split_nul(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|b| *b == 0).filter(|s| !s.is_empty())
}

pub(crate) fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
