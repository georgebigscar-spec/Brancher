//! Теги, cherry-pick и revert. Имена тегов проверяет core, с `-` они не начинаются;
//! цели — полные SHA или имена ссылок.

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Тег на `target`; с сообщением — аннотированный, сообщение идёт через stdin.
pub fn create_args(name: &str, target: &str, annotated: bool) -> Vec<String> {
    if annotated {
        owned(&["tag", "-a", "-F", "-", name, target])
    } else {
        owned(&["tag", name, target])
    }
}

pub fn delete_args(name: &str) -> Vec<String> {
    owned(&["tag", "-d", name])
}

pub fn push_args(remote: &str, name: &str) -> Vec<String> {
    vec![
        "push".into(),
        "--progress".into(),
        remote.into(),
        format!("refs/tags/{name}"),
    ]
}

/// `mainline` — для merge-коммита: изменения относительно первого родителя (`-m 1`).
pub fn cherry_pick_args(commit: &str, mainline: bool) -> Vec<String> {
    let mut a = owned(&["cherry-pick"]);
    if mainline {
        a.extend(owned(&["-m", "1"]));
    }
    a.push(commit.into());
    a
}

pub fn revert_args(commit: &str, mainline: bool) -> Vec<String> {
    let mut a = owned(&["revert", "--no-edit"]);
    if mainline {
        a.extend(owned(&["-m", "1"]));
    }
    a.push(commit.into());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_commands() {
        assert_eq!(create_args("v1", "abc", false), ["tag", "v1", "abc"]);
        assert_eq!(
            create_args("v1", "abc", true),
            ["tag", "-a", "-F", "-", "v1", "abc"]
        );
        assert_eq!(delete_args("v1"), ["tag", "-d", "v1"]);
        assert_eq!(
            push_args("origin", "v1"),
            ["push", "--progress", "origin", "refs/tags/v1"]
        );
        assert_eq!(cherry_pick_args("abc", false), ["cherry-pick", "abc"]);
        assert_eq!(cherry_pick_args("abc", true), ["cherry-pick", "-m", "1", "abc"]);
        assert_eq!(
            revert_args("abc", true),
            ["revert", "--no-edit", "-m", "1", "abc"]
        );
    }
}
