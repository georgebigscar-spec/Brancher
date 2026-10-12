//! `git stash`: список, push, apply/pop и drop. Записи адресуются как `stash@{n}`.

use brancher_core::Stash;

use super::utf8;
use crate::GitError;

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Поля через NUL, записи через RS (0x1e); `%gs` — сообщение stash.
pub fn list_args() -> Vec<String> {
    owned(&["stash", "list", "--format=%H%x00%P%x00%ct%x00%gs%x1e"])
}

pub fn parse_list(out: &[u8]) -> Result<Vec<Stash>, GitError> {
    out.split(|b| *b == 0x1e)
        .map(|r| r.strip_prefix(b"\n").unwrap_or(r))
        .filter(|r| !r.is_empty() && *r != b"\n")
        .enumerate()
        .map(|(index, rec)| {
            let f: Vec<String> = rec.split(|b| *b == 0).map(utf8).collect();
            let [id, parents, time, message] = &f[..] else {
                return Err(GitError::Parse(format!("stash list: {} полей вместо 4", f.len())));
            };
            Ok(Stash {
                index,
                id: id.clone(),
                base: parents.split_whitespace().next().map(str::to_owned),
                time: time
                    .parse()
                    .map_err(|_| GitError::Parse(format!("stash list: время {time:?}")))?,
                message: message.clone(),
            })
        })
        .collect()
}

pub fn reference(index: usize) -> String {
    format!("stash@{{{index}}}")
}

/// Спрятать изменения; `untracked` — вместе с неотслеживаемыми файлами.
pub fn push_args(message: &str, untracked: bool) -> Vec<String> {
    let mut args = owned(&["stash", "push"]);
    if untracked {
        args.push("--include-untracked".into());
    }
    if !message.is_empty() {
        args.extend(["-m".into(), message.to_owned()]);
    }
    args
}

/// Вернуть изменения; `pop` — и удалить запись, если применилось без конфликтов.
/// Без `--index`: он отказывает при любом расхождении индекса, а изменения важнее разметки.
pub fn apply_args(index: usize, pop: bool) -> Vec<String> {
    let cmd = if pop { "pop" } else { "apply" };
    vec!["stash".into(), cmd.into(), reference(index)]
}

pub fn drop_args(index: usize) -> Vec<String> {
    vec!["stash".into(), "drop".into(), reference(index)]
}

/// Какой коммит сейчас под `stash@{n}`: список мог измениться с прошлого обновления.
pub fn verify_args(index: usize) -> Vec<String> {
    vec![
        "rev-parse".into(),
        "--verify".into(),
        "--quiet".into(),
        format!("{}^{{commit}}", reference(index)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list() {
        let out =
            b"aaa\x00base idx\x00100\x00On main: wip\x1e\nbbb\x00b2 i2 u2\x00200\x00WIP on dev: 123 x\x1e\n";
        let list = parse_list(out).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].index, 0);
        assert_eq!(list[0].id, "aaa");
        assert_eq!(list[0].base.as_deref(), Some("base"));
        assert_eq!(list[0].message, "On main: wip");
        assert_eq!(list[1].index, 1);
        assert_eq!(list[1].time, 200);
        assert!(parse_list(b"").unwrap().is_empty());
        assert!(parse_list(b"x\x00y\x1e").is_err());
    }

    #[test]
    fn stash_commands() {
        assert_eq!(push_args("", false), ["stash", "push"]);
        assert_eq!(
            push_args("-m", true),
            ["stash", "push", "--include-untracked", "-m", "-m"]
        );
        assert_eq!(apply_args(2, true), ["stash", "pop", "stash@{2}"]);
        assert_eq!(apply_args(0, false), ["stash", "apply", "stash@{0}"]);
        assert_eq!(drop_args(1), ["stash", "drop", "stash@{1}"]);
        assert_eq!(verify_args(0)[3], "stash@{0}^{commit}");
    }
}
