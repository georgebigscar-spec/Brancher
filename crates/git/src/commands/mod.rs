//! Построители аргументов и парсеры. Всё здесь — чистые функции без запуска процессов.

pub mod apply;
pub mod branch;
pub mod clone;
pub mod commit;
pub mod diff;
pub mod log;
pub mod refs;
pub mod stage;
pub mod status;

/// Разбивает вывод по NUL, отбрасывая пустой хвост.
pub(crate) fn split_nul(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|b| *b == 0).filter(|s| !s.is_empty())
}

pub(crate) fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
