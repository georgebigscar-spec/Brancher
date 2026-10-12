//! Stage и unstage целых файлов. Частичный stage (по строкам) — через генератор patch, позже.

/// `git add -A -- <paths>`: `-A` учитывает и удалённые файлы.
pub fn stage_args(paths: &[String]) -> Vec<String> {
    with_paths(&["add", "-A"], paths)
}

/// `git reset -q -- <paths>`; до первого коммита — `git rm --cached` (HEAD ещё нет).
pub fn unstage_args(paths: &[String], has_head: bool) -> Vec<String> {
    if has_head {
        with_paths(&["reset", "-q"], paths)
    } else {
        with_paths(&["rm", "--cached", "-r", "-q"], paths)
    }
}

/// Пути всегда идут после `--` (NFR-10).
fn with_paths(cmd: &[&str], paths: &[String]) -> Vec<String> {
    cmd.iter()
        .map(|s| (*s).to_owned())
        .chain(["--".to_owned()])
        .chain(paths.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_double_dash() {
        let a = stage_args(&["-rf".into(), "a b.txt".into()]);
        assert_eq!(a, ["add", "-A", "--", "-rf", "a b.txt"]);
        assert_eq!(unstage_args(&["x".into()], true), ["reset", "-q", "--", "x"]);
        assert_eq!(
            unstage_args(&["x".into()], false),
            ["rm", "--cached", "-r", "-q", "--", "x"]
        );
    }
}
