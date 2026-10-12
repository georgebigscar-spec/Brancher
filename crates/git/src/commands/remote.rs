//! Fetch, pull, push и список remotes. Прогресс идёт в stderr (`--progress` нужен,
//! потому что stderr — не терминал).

use brancher_core::Remote;

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Все remotes; удалённые на сервере ветки убираются и локально.
pub fn fetch_args() -> Vec<String> {
    owned(&["fetch", "--all", "--prune", "--progress"])
}

/// Один remote; как и у `fetch --all`, удалённые на сервере ветки убираются.
pub fn fetch_remote_args(remote: &str) -> Vec<String> {
    let mut a = owned(&["fetch", "--prune", "--progress"]);
    a.push(remote.into());
    a
}

pub fn list_args() -> Vec<String> {
    owned(&["remote", "-v"])
}

/// Разбирает `git remote -v`: `имя\tURL (fetch)` и `имя\tURL (push)` на каждый remote.
pub fn parse_list(out: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = vec![];
    for line in out.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let (url, push) = if let Some(u) = rest.strip_suffix(" (push)") {
            (u, true)
        } else {
            (rest.strip_suffix(" (fetch)").unwrap_or(rest), false)
        };
        let i = match remotes.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                remotes.push(Remote {
                    name: name.to_owned(),
                    ..Default::default()
                });
                remotes.len() - 1
            }
        };
        let r = &mut remotes[i];
        if push {
            r.push_url = url.to_owned();
        } else {
            r.url = url.to_owned();
        }
    }
    remotes
}

/// `--` перед именем и URL: значения от пользователя не станут опциями git.
pub fn add_args(name: &str, url: &str) -> Vec<String> {
    vec![
        "remote".into(),
        "add".into(),
        "--".into(),
        name.into(),
        url.into(),
    ]
}

pub fn remove_args(name: &str) -> Vec<String> {
    vec!["remote".into(), "remove".into(), "--".into(), name.into()]
}

pub fn rename_args(old: &str, new: &str) -> Vec<String> {
    vec![
        "remote".into(),
        "rename".into(),
        "--".into(),
        old.into(),
        new.into(),
    ]
}

pub fn set_url_args(name: &str, url: &str) -> Vec<String> {
    vec![
        "remote".into(),
        "set-url".into(),
        "--".into(),
        name.into(),
        url.into(),
    ]
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
        assert_eq!(fetch_remote_args("up"), ["fetch", "--prune", "--progress", "up"]);
        assert_eq!(set_url_args("o", "-x"), ["remote", "set-url", "--", "o", "-x"]);
    }

    #[test]
    fn parses_remote_list() {
        let out = "origin\thttps://example.com/a.git (fetch)\n\
origin\thttps://example.com/a.git (push)\n\
fork\tgit@host:me/a.git (fetch)\n\
fork\tssh://other/a.git (push)\n";
        let r = parse_list(out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].name, "origin");
        assert_eq!(r[0].url, "https://example.com/a.git");
        assert_eq!(r[0].push_url, r[0].url);
        assert_eq!(r[1].url, "git@host:me/a.git");
        assert_eq!(r[1].push_url, "ssh://other/a.git");
        assert!(parse_list("").is_empty());
    }
}
