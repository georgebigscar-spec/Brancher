//! Fetch, pull и push. Прогресс идёт в stderr (`--progress` нужен, потому что stderr — не терминал).

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Все remotes; удалённые на сервере ветки убираются и локально.
pub fn fetch_args() -> Vec<String> {
    owned(&["fetch", "--all", "--prune", "--progress"])
}

/// Только перемотка вперёд: слияние и rebase — отдельные явные операции,
/// pull не должен неожиданно создавать merge-коммит или конфликт.
pub fn pull_args() -> Vec<String> {
    owned(&["pull", "--ff-only", "--progress"])
}

/// `set_upstream` — `(remote, ветка)` для первого push ветки без upstream.
pub fn push_args(set_upstream: Option<(&str, &str)>) -> Vec<String> {
    let mut a = owned(&["push", "--progress"]);
    if let Some((remote, branch)) = set_upstream {
        a.extend(owned(&["-u", remote, branch]));
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_commands() {
        assert_eq!(fetch_args(), ["fetch", "--all", "--prune", "--progress"]);
        assert_eq!(pull_args(), ["pull", "--ff-only", "--progress"]);
        assert_eq!(push_args(None), ["push", "--progress"]);
        assert_eq!(
            push_args(Some(("origin", "feature/x"))),
            ["push", "--progress", "-u", "origin", "feature/x"]
        );
    }
}
