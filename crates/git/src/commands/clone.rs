//! `git clone --progress`; строки прогресса разбирает [`super::progress`].

pub fn args(url: &str, dest: &str) -> Vec<String> {
    // `--` — чтобы URL, начинающийся с `-`, не стал опцией (NFR-10).
    ["clone", "--progress", "--", url, dest]
        .map(str::to_owned)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::progress::parse_progress;

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
