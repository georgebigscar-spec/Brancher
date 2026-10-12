//! Interactive rebase без редактора: todo-список готовит Brancher, git берёт его через
//! `GIT_SEQUENCE_EDITOR`, который просто копирует готовый файл поверх своего.

use brancher_core::{RebaseAction, RebaseStep};

/// Файл с готовым todo-списком в каталоге git (`git rev-parse --git-path`).
pub const TODO_FILE: &str = "brancher-todo";
/// Каталог с новыми сообщениями reword: `brancher-reword/<номер шага>`.
pub const REWORD_DIR: &str = "brancher-reword";

/// Команда «редактора» todo. git запускает её через shell с путём к своему todo в `$1`;
/// путь к нашему файлу вычисляется там же, поэтому его не нужно экранировать.
pub fn sequence_editor() -> String {
    format!("cp \"$(git rev-parse --git-path {TODO_FILE})\"")
}

/// Начать rebase на `base` (родитель первого коммита) или с корня истории.
pub fn start_args(base: Option<&str>) -> Vec<String> {
    let mut a = vec!["rebase".to_owned(), "-i".to_owned()];
    a.push(base.unwrap_or("--root").to_owned());
    a
}

/// Todo-список git. Reword — это `pick` и сразу `exec git commit --amend` с сообщением
/// из файла: так новое сообщение не нужно передавать через редактор.
pub fn todo(steps: &[RebaseStep]) -> String {
    let mut out = String::new();
    for (i, s) in steps.iter().enumerate() {
        out.push_str(&format!("{} {}\n", s.action.command(), s.commit));
        if s.action == RebaseAction::Reword {
            out.push_str(&format!(
                "exec git commit --amend --allow-empty --cleanup=strip -F \"$(git rev-parse --git-path {REWORD_DIR}/{i})\"\n"
            ));
        }
    }
    out
}

/// Полное сообщение коммита.
pub fn message_args(commit: &str) -> Vec<String> {
    vec![
        "log".into(),
        "-1".into(),
        "--format=%B".into(),
        commit.into(),
        "--".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(action: RebaseAction, commit: &str) -> RebaseStep {
        RebaseStep {
            action,
            commit: commit.into(),
            subject: "s".into(),
            message: Some("m".into()),
        }
    }

    #[test]
    fn builds_todo_with_reword_exec() {
        let t = todo(&[
            step(RebaseAction::Pick, "a"),
            step(RebaseAction::Reword, "b"),
            step(RebaseAction::Fixup, "c"),
            step(RebaseAction::Drop, "d"),
        ]);
        assert_eq!(
            t,
            "pick a\npick b\n\
             exec git commit --amend --allow-empty --cleanup=strip -F \"$(git rev-parse --git-path brancher-reword/1)\"\n\
             fixup c\ndrop d\n"
        );
        assert_eq!(start_args(None), ["rebase", "-i", "--root"]);
        assert_eq!(start_args(Some("abc")), ["rebase", "-i", "abc"]);
    }
}
