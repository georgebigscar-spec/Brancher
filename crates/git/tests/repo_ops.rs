//! Интеграционные тесты: операции на временном репозитории (раздел 13).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use brancher_core::{ChangeKind, RefKind};
use brancher_git::{CancelToken, Git, GitError, Runner};

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("brancher-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success())
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Тест"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "commit.gpgsign", "false"]);
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn write(&self, file: &str, text: &str) {
        let p = self.0.join(file);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, text).unwrap();
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git() -> Git {
    Git::detect().expect("git установлен и не старше 2.25.1").0
}

#[test]
fn empty_repo_has_no_head_and_no_history() {
    let repo = TempRepo::new("empty");
    let c = CancelToken::new();
    let g = git();
    let s = g.status(repo.path(), &c).unwrap();
    assert_eq!(s.head, None);
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert!(g.log(repo.path(), 100, &c).unwrap().is_empty());
    assert!(g.refs(repo.path(), &c).unwrap().is_empty());
}

#[test]
fn stage_commit_unstage_roundtrip() {
    let repo = TempRepo::new("roundtrip");
    let c = CancelToken::new();
    let g = git();

    repo.write("привет мир.txt", "1\n");
    repo.write("src/lib.rs", "fn main() {}\n");
    let s = g.status(repo.path(), &c).unwrap();
    assert_eq!(s.changes.len(), 2);
    assert!(s
        .changes
        .iter()
        .all(|ch| ch.unstaged == Some(ChangeKind::Untracked)));

    // Unstage до первого коммита.
    g.stage(repo.path(), &["привет мир.txt".into()], &c).unwrap();
    g.unstage(repo.path(), &["привет мир.txt".into()], false, &c)
        .unwrap();
    assert_eq!(g.status(repo.path(), &c).unwrap().staged().count(), 0);

    let all: Vec<String> = s.changes.iter().map(|ch| ch.path.clone()).collect();
    g.stage(repo.path(), &all, &c).unwrap();
    assert_eq!(g.status(repo.path(), &c).unwrap().staged().count(), 2);

    g.commit(repo.path(), "Первый коммит\n\nОписание.", false, &c)
        .unwrap();
    let log = g.log(repo.path(), 10, &c).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "Первый коммит");
    assert_eq!(log[0].author.name, "Тест");

    let refs = g.refs(repo.path(), &c).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::LocalBranch);
    assert!(refs[0].is_head);
    assert_eq!(refs[0].target, log[0].id);

    // Изменение, stage, unstage после первого коммита.
    repo.write("src/lib.rs", "fn main() { println!(); }\n");
    g.stage(repo.path(), &["src/lib.rs".into()], &c).unwrap();
    let s = g.status(repo.path(), &c).unwrap();
    assert_eq!(s.changes[0].staged, Some(ChangeKind::Modified));
    g.unstage(repo.path(), &["src/lib.rs".into()], true, &c).unwrap();
    let s = g.status(repo.path(), &c).unwrap();
    assert_eq!(s.changes[0].staged, None);
    assert_eq!(s.changes[0].unstaged, Some(ChangeKind::Modified));

    // Amend.
    g.stage(repo.path(), &["src/lib.rs".into()], &c).unwrap();
    g.commit(repo.path(), "Первый коммит (исправлен)", true, &c)
        .unwrap();
    let log = g.log(repo.path(), 10, &c).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "Первый коммит (исправлен)");
}

#[test]
fn failed_command_reports_stderr() {
    let repo = TempRepo::new("fail");
    let err = git()
        .commit(repo.path(), "nothing to commit", false, &CancelToken::new())
        .unwrap_err();
    match err {
        GitError::Failed { code, .. } => assert_eq!(code, Some(1)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cancelled_token_stops_before_spawn() {
    let c = CancelToken::new();
    c.cancel();
    let err = Runner::default().run(None, &["--version"], None, &c).unwrap_err();
    assert!(matches!(err, GitError::Cancelled));
}

#[test]
fn missing_git_binary_is_spawn_error() {
    let err = Runner::with_git("/nonexistent/git")
        .run(None, &["--version"], None, &CancelToken::new())
        .unwrap_err();
    assert!(matches!(err, GitError::Spawn(_)));
}
