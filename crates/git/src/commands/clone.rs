//! `git clone --progress` и разбор строк прогресса из stderr.

use brancher_core::CloneProgress;

pub fn args(url: &str, dest: &str) -> Vec<String> {
    // `--` — чтобы URL, начинающийся с `-`, не стал опцией (NFR-10).
    ["clone", "--progress", "--", url, dest]
        .map(str::to_owned)
        .to_vec()
}

/// `Receiving objects:  45% (450/1000), 1.20 MiB | 1.00 MiB/s` → этап и процент.
/// Строки без процента (`Cloning into 'x'...`) — этап без процента.
pub fn parse_progress(line: &str) -> Option<CloneProgress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").map_or(line, str::trim);
    if line.is_empty() {
        return None;
    }
    let Some((phase, rest)) = line.split_once(':') else {
        return Some(CloneProgress {
            phase: line.trim_end_matches("...").to_owned(),
            percent: None,
        });
    };
    let percent = rest
        .split_once('%')
        .and_then(|(n, _)| n.trim().parse::<u8>().ok())
        .filter(|p| *p <= 100);
    Some(CloneProgress {
        phase: phase.trim().to_owned(),
        percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Option<(String, Option<u8>)> {
        parse_progress(line).map(|p| (p.phase, p.percent))
    }

    #[test]
    fn parses_git_progress_lines() {
        assert_eq!(
            p("Receiving objects:  45% (450/1000), 1.20 MiB | 1.00 MiB/s"),
            Some(("Receiving objects".into(), Some(45)))
        );
        assert_eq!(
            p("remote: Counting objects: 100% (12/12), done."),
            Some(("Counting objects".into(), Some(100)))
        );
        assert_eq!(
            p("remote: Enumerating objects: 12, done."),
            Some(("Enumerating objects".into(), None))
        );
        assert_eq!(
            p("Cloning into 'brancher'..."),
            Some(("Cloning into 'brancher'".into(), None))
        );
        assert_eq!(p("   "), None);
    }

    #[test]
    fn url_cannot_become_an_option() {
        assert_eq!(args("-u x", "d"), ["clone", "--progress", "--", "-u x", "d"]);
    }
}
