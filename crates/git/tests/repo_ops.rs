//! Интеграционные тесты: операции на временном репозитории (раздел 13).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use std::collections::BTreeSet;

use brancher_core::patch;
use brancher_core::{ChangeKind, DiffTarget, LineKind, RefKind};
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
        // На Windows-раннерах autocrlf часто включён глобально, а тесты проверяют байты.
        git(&["config", "core.autocrlf", "false"]);
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
fn diffs_for_commits_and_working_tree() {
    let repo = TempRepo::new("diff");
    let c = CancelToken::new();
    let g = git();
    let change = |path: &str, staged, untracked| DiffTarget::Change {
        path: path.into(),
        staged,
        untracked,
    };

    // Untracked: сравнение с пустым файлом через --no-index (код выхода 1 — не ошибка).
    repo.write("файл с пробелом.txt", "один\nдва\n");
    let d = g
        .diff(repo.path(), &change("файл с пробелом.txt", false, true), &c)
        .unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path(), "файл с пробелом.txt");
    assert_eq!(d[0].old_path, None);
    assert_eq!(d[0].stats(), (2, 0));

    g.stage(repo.path(), &["файл с пробелом.txt".into()], &c).unwrap();
    let d = g
        .diff(repo.path(), &change("файл с пробелом.txt", true, false), &c)
        .unwrap();
    assert_eq!(d[0].stats(), (2, 0));
    g.commit(repo.path(), "root", false, &c).unwrap();

    // Корневой коммит сравнивается с пустым деревом.
    let root = g.log(repo.path(), 1, &c).unwrap().remove(0);
    let d = g
        .diff(
            repo.path(),
            &DiffTarget::Commit {
                id: root.id.clone(),
                parent: None,
            },
            &c,
        )
        .unwrap();
    assert_eq!((d.len(), d[0].stats()), (1, (2, 0)));

    // Изменение в рабочем дереве и затем в коммите.
    repo.write("файл с пробелом.txt", "один\nтри\n");
    let d = g
        .diff(repo.path(), &change("файл с пробелом.txt", false, false), &c)
        .unwrap();
    let kinds: Vec<_> = d[0].hunks[0].lines.iter().map(|l| l.kind).collect();
    assert_eq!(kinds, [LineKind::Context, LineKind::Removed, LineKind::Added]);
    assert!(g
        .diff(repo.path(), &change("файл с пробелом.txt", true, false), &c)
        .unwrap()
        .is_empty());

    g.stage(repo.path(), &["файл с пробелом.txt".into()], &c).unwrap();
    g.commit(repo.path(), "second", false, &c).unwrap();
    let head = g.log(repo.path(), 1, &c).unwrap().remove(0);
    let d = g
        .diff(
            repo.path(),
            &DiffTarget::Commit {
                id: head.id,
                parent: Some(root.id),
            },
            &c,
        )
        .unwrap();
    assert_eq!(d[0].stats(), (1, 1));
    assert_eq!(d[0].hunks[0].lines[2].text, "три");
}

#[test]
fn partial_stage_and_unstage_by_lines() {
    let repo = TempRepo::new("lines");
    let c = CancelToken::new();
    let g = git();
    let target = |staged| DiffTarget::Change {
        path: "f.txt".into(),
        staged,
        untracked: false,
    };
    let cached = |g: &Git| {
        let out = g
            .runner
            .run(Some(repo.path()), &["show", ":f.txt"], None, &c)
            .unwrap();
        out.stdout_lossy()
    };

    repo.write("f.txt", "a\r\nb\r\nc\r\nd\r\n");
    g.stage(repo.path(), &["f.txt".into()], &c).unwrap();
    g.commit(repo.path(), "base", false, &c).unwrap();
    // Две правки в одном ханке: b → B и новая строка e в конце (CRLF сохраняется).
    repo.write("f.txt", "a\r\nB\r\nc\r\nd\r\ne\r\n");

    let d = g.diff(repo.path(), &target(false), &c).unwrap();
    let lines = &d[0].hunks[0].lines;
    let e = lines
        .iter()
        .position(|l| l.kind == LineKind::Added && l.text == "e")
        .unwrap();
    let p = patch::build(&d[0], &BTreeSet::from([(0, e)]), false).unwrap();
    g.apply(repo.path(), &p, true, false, &c).unwrap();
    assert_eq!(cached(&g), "a\r\nb\r\nc\r\nd\r\ne\r\n");

    // Теперь индексируем b → B ханком целиком и снимаем из индекса только e.
    let d = g.diff(repo.path(), &target(false), &c).unwrap();
    let p = patch::build(&d[0], &patch::hunk_lines(&d[0], 0), false).unwrap();
    g.apply(repo.path(), &p, true, false, &c).unwrap();
    assert_eq!(cached(&g), "a\r\nB\r\nc\r\nd\r\ne\r\n");

    let d = g.diff(repo.path(), &target(true), &c).unwrap();
    let lines = &d[0].hunks[0].lines;
    let e = lines
        .iter()
        .position(|l| l.kind == LineKind::Added && l.text == "e")
        .unwrap();
    let p = patch::build(&d[0], &BTreeSet::from([(0, e)]), true).unwrap();
    g.apply(repo.path(), &p, true, true, &c).unwrap();
    assert_eq!(cached(&g), "a\r\nB\r\nc\r\nd\r\n");

    // Discard: в индексе a B c d, в рабочем дереве ещё и e — откатываем e в рабочем дереве.
    let d = g.diff(repo.path(), &target(false), &c).unwrap();
    let lines = &d[0].hunks[0].lines;
    let e = lines
        .iter()
        .position(|l| l.kind == LineKind::Added && l.text == "e")
        .unwrap();
    let p = patch::build(&d[0], &BTreeSet::from([(0, e)]), true).unwrap();
    g.apply(repo.path(), &p, false, true, &c).unwrap();
    let work = std::fs::read_to_string(repo.path().join("f.txt")).unwrap();
    assert_eq!(work, "a\r\nB\r\nc\r\nd\r\n");
    assert!(g.diff(repo.path(), &target(false), &c).unwrap().is_empty());

    // Untracked: в индекс попадает только выбранная строка нового файла.
    repo.write("new.txt", "one\ntwo\n");
    let untracked = DiffTarget::Change {
        path: "new.txt".into(),
        staged: false,
        untracked: true,
    };
    let d = g.diff(repo.path(), &untracked, &c).unwrap();
    let p = patch::build(&d[0], &BTreeSet::from([(0, 1)]), false).unwrap();
    g.apply(repo.path(), &p, true, false, &c).unwrap();
    let out = g
        .runner
        .run(Some(repo.path()), &["show", ":new.txt"], None, &c)
        .unwrap();
    assert_eq!(out.stdout_lossy(), "two\n");
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
