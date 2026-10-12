//! Строки прогресса `git … --progress` (clone, fetch, push) из stderr.

use brancher_core::Progress;

/// `Receiving objects:  45% (450/1000), 1.20 MiB | 1.00 MiB/s` → этап и процент.
/// Строки без процента (`Cloning into 'x'...`) — этап без процента.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").map_or(line, str::trim);
    if line.is_empty() {
        return None;
    }
    let Some((phase, rest)) = line.split_once(':') else {
        return Some(Progress {
            phase: line.trim_end_matches("...").to_owned(),
            percent: None,
        });
    };
    let percent = rest
        .split_once('%')
        .and_then(|(n, _)| n.trim().parse::<u8>().ok())
        .filter(|p| *p <= 100);
    Some(Progress {
        phase: phase.trim().to_owned(),
        percent,
    })
}
