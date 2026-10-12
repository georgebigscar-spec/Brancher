//! Merge, rebase и разрешение конфликтов. Имена веток проверены core и с `-` не начинаются.

use std::path::Path;

use brancher_core::Operation;

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Слить `target` в текущую ветку; перемотка, если возможна, иначе merge-коммит.
pub fn merge_args(target: &str) -> Vec<String> {
    owned(&["merge", "--no-edit", target])
}

pub fn rebase_args(onto: &str) -> Vec<String> {
    owned(&["rebase", onto])
}

/// Продолжить после разрешения конфликтов (сообщение — по умолчанию, `GIT_EDITOR=true`).
pub fn continue_args(op: Operation) -> Vec<String> {
    match op {
        Operation::Merge => owned(&["commit", "--no-edit"]),
        op => owned(&[op.name(), "--continue"]),
    }
}

pub fn abort_args(op: Operation) -> Vec<String> {
    owned(&[op.name(), "--abort"])
}

/// Взять версию файла одной из сторон конфликта.
pub fn checkout_side_args(path: &str, ours: bool) -> Vec<String> {
    let side = if ours { "--ours" } else { "--theirs" };
    owned(&["checkout", side, "--", path])
}

pub fn git_dir_args() -> Vec<String> {
    owned(&["rev-parse", "--absolute-git-dir"])
}

/// Какая операция идёт, по служебным файлам в `.git`, и шаг rebase.
pub fn detect(git_dir: &Path) -> (Option<Operation>, Option<(u32, u32)>) {
    let read = |p: &Path| -> Option<u32> { std::fs::read_to_string(p).ok()?.trim().parse().ok() };
    let merge_dir = git_dir.join("rebase-merge");
    let apply_dir = git_dir.join("rebase-apply");
    if merge_dir.is_dir() {
        let step = read(&merge_dir.join("msgnum")).zip(read(&merge_dir.join("end")));
        return (Some(Operation::Rebase), step);
    }
    // rebase-apply без `applying` — rebase; с ним — `git am`, его Brancher не ведёт.
    if apply_dir.is_dir() && !apply_dir.join("applying").exists() {
        let step = read(&apply_dir.join("next")).zip(read(&apply_dir.join("last")));
        return (Some(Operation::Rebase), step);
    }
    let op = [
        ("MERGE_HEAD", Operation::Merge),
        ("CHERRY_PICK_HEAD", Operation::CherryPick),
        ("REVERT_HEAD", Operation::Revert),
    ]
    .into_iter()
    .find(|(f, _)| git_dir.join(f).is_file())
    .map(|(_, op)| op);
    (op, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_commands() {
        assert_eq!(merge_args("dev"), ["merge", "--no-edit", "dev"]);
        assert_eq!(rebase_args("main"), ["rebase", "main"]);
        assert_eq!(continue_args(Operation::Merge), ["commit", "--no-edit"]);
        assert_eq!(continue_args(Operation::Rebase), ["rebase", "--continue"]);
        assert_eq!(abort_args(Operation::CherryPick), ["cherry-pick", "--abort"]);
        assert_eq!(
            checkout_side_args("a b", false),
            ["checkout", "--theirs", "--", "a b"]
        );
    }

    #[test]
    fn detects_operations() {
        let dir = std::env::temp_dir().join(format!("brancher-op-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect(&dir), (None, None));
        std::fs::write(dir.join("MERGE_HEAD"), "x").unwrap();
        assert_eq!(detect(&dir), (Some(Operation::Merge), None));
        std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();
        std::fs::write(dir.join("rebase-merge/msgnum"), "2\n").unwrap();
        std::fs::write(dir.join("rebase-merge/end"), "5\n").unwrap();
        assert_eq!(detect(&dir), (Some(Operation::Rebase), Some((2, 5))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
