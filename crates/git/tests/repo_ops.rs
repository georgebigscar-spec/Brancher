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
        conflicted: false,
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
        conflicted: false,
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
        conflicted: false,
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

#[test]
fn create_switch_track_and_delete_branches() {
    let repo = TempRepo::new("branches");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    repo.write("a.txt", "a\n");
    g.stage(p, &["a.txt".into()], &c).unwrap();
    g.commit(p, "init", false, &c).unwrap();
    // Remote без сети: достаточно настроенного origin и его ссылки.
    let run = |args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(p)
            .status()
            .unwrap()
            .success());
    };
    run(&["remote", "add", "origin", "https://example.invalid/r.git"]);
    run(&["update-ref", "refs/remotes/origin/feature", "HEAD"]);

    let head = |g: &Git| g.status(p, &c).unwrap().branch;
    g.create_branch(p, "dev", "main", false, &c).unwrap();
    assert_eq!(head(&g).as_deref(), Some("main"));
    g.create_branch(p, "topic/x", "main", true, &c).unwrap();
    assert_eq!(head(&g).as_deref(), Some("topic/x"));
    g.switch(p, "dev", &c).unwrap();
    assert_eq!(head(&g).as_deref(), Some("dev"));

    g.switch_track(p, "origin/feature", &c).unwrap();
    let refs = g.refs(p, &c).unwrap();
    let feature = refs
        .iter()
        .find(|r| r.kind == RefKind::LocalBranch && r.name == "feature")
        .expect("локальная feature создана");
    assert!(feature.is_head);
    assert_eq!(feature.upstream.as_deref(), Some("refs/remotes/origin/feature"));

    // --no-guess: несуществующая локальная ветка не создаётся из remote неявно.
    assert!(matches!(g.switch(p, "nope", &c), Err(GitError::Failed { .. })));
    g.delete_branch(p, "topic/x", true, &c).unwrap();
    let names: Vec<_> = g.refs(p, &c).unwrap().into_iter().map(|r| r.name).collect();
    assert!(!names.contains(&"topic/x".to_owned()), "{names:?}");
}

#[test]
fn fetch_pull_and_push_with_local_remote() {
    let (g, c) = (git(), CancelToken::new());
    let origin = TempRepo::new("origin-src");
    origin.write("a.txt", "1\n");
    g.stage(origin.path(), &["a.txt".into()], &c).unwrap();
    g.commit(origin.path(), "one", false, &c).unwrap();
    let bare = std::env::temp_dir().join(format!("brancher-test-bare-{}", std::process::id()));
    let _ = fs::remove_dir_all(&bare);
    let url = bare.display().to_string();
    let run = |dir: &Path, args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
    };
    run(origin.path(), &["clone", "-q", "--bare", ".", &url]);

    // Клон, коммит и первый push новой ветки с -u.
    let work = TempRepo::new("work");
    let w = work.path();
    run(w, &["remote", "add", "origin", &url]);
    g.fetch(w, &c, &mut |_| {}).unwrap();
    run(w, &["checkout", "-q", "-B", "main", "origin/main"]);
    work.write("b.txt", "2\n");
    g.stage(w, &["b.txt".into()], &c).unwrap();
    g.commit(w, "two", false, &c).unwrap();
    g.push(w, None, &c, &mut |_| {}).unwrap();
    g.create_branch(w, "topic", "main", true, &c).unwrap();
    g.push(w, Some(("origin", "topic")), &c, &mut |_| {}).unwrap();
    let topic = g
        .refs(w, &c)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "topic")
        .unwrap();
    assert_eq!(topic.upstream.as_deref(), Some("refs/remotes/origin/topic"));

    // Второй клон забирает изменения перемоткой.
    run(origin.path(), &["remote", "add", "up", &url]);
    g.fetch(origin.path(), &c, &mut |_| {}).unwrap();
    run(origin.path(), &["branch", "-q", "--set-upstream-to=up/main"]);
    g.pull(origin.path(), &c, &mut |_| {}).unwrap();
    assert!(origin.path().join("b.txt").exists());
    let _ = fs::remove_dir_all(&bare);
}

#[test]
fn stash_push_list_apply_pop_and_drop() {
    let repo = TempRepo::new("stash");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    repo.write("a.txt", "a\n");
    g.stage(p, &["a.txt".into()], &c).unwrap();
    g.commit(p, "init", false, &c).unwrap();
    let head = g.status(p, &c).unwrap().head.unwrap();
    assert!(g.stashes(p, &c).unwrap().is_empty());

    repo.write("a.txt", "a\nb\n");
    g.stash_push(p, "первый", false, &c).unwrap();
    repo.write("new.txt", "n\n");
    g.stash_push(p, "", true, &c).unwrap();
    assert!(g.status(p, &c).unwrap().changes.is_empty());

    let list = g.stashes(p, &c).unwrap();
    assert_eq!(list.len(), 2);
    assert!(
        list[0].message.starts_with("WIP on main"),
        "{:?}",
        list[0].message
    );
    assert_eq!(list[1].message, "On main: первый");
    assert_eq!(list[1].base.as_deref(), Some(head.as_str()));
    // Коммиты stash не попадают в историю.
    assert_eq!(g.log(p, 100, &c).unwrap().len(), 1);

    // Diff записи — изменения относительно базового коммита.
    let diff = g
        .diff(
            p,
            &DiffTarget::Commit {
                id: list[1].id.clone(),
                parent: list[1].base.clone(),
            },
            &c,
        )
        .unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path(), "a.txt");

    // Номер с чужим коммитом — отказ без изменений.
    assert!(matches!(
        g.stash_apply(p, 0, &list[1].id, false, &c),
        Err(GitError::Stale(_))
    ));
    g.stash_apply(p, 0, &list[0].id, true, &c).unwrap();
    assert!(p.join("new.txt").exists());
    let list = g.stashes(p, &c).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].message, "On main: первый");

    g.stash_apply(p, 0, &list[0].id, false, &c).unwrap();
    assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "a\nb\n");
    assert_eq!(g.stashes(p, &c).unwrap().len(), 1);
    g.stash_drop(p, 0, &list[0].id, &c).unwrap();
    assert!(g.stashes(p, &c).unwrap().is_empty());
}

#[test]
fn merge_and_rebase_with_conflicts() {
    use brancher_core::Operation;
    let repo = TempRepo::new("merge");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
    };
    commit("a.txt", "base\n", "init");
    g.create_branch(p, "dev", "main", true, &c).unwrap();
    commit("a.txt", "dev\n", "dev");
    commit("b.txt", "b\n", "dev b");
    g.switch(p, "main", &c).unwrap();
    commit("a.txt", "main\n", "main");

    // Merge останавливается на конфликте, но это не ошибка.
    g.merge(p, "dev", &c).unwrap();
    let s = g.status(p, &c).unwrap();
    assert_eq!(s.operation, Some(Operation::Merge));
    let conflicts: Vec<_> = s.conflicts().map(|c| c.path.as_str()).collect();
    assert_eq!(conflicts, ["a.txt"]);

    g.resolve_with(p, "a.txt", false, &c).unwrap();
    assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "dev\n");
    g.continue_operation(p, Operation::Merge, &c).unwrap();
    let s = g.status(p, &c).unwrap();
    assert_eq!(s.operation, None);
    assert!(s.changes.is_empty());
    assert!(g.log(p, 10, &c).unwrap()[0].is_merge());

    // Rebase с конфликтом и отмена.
    g.create_branch(p, "topic", "main~1", true, &c).unwrap();
    commit("a.txt", "topic\n", "topic");
    let before = g.status(p, &c).unwrap().head;
    g.rebase(p, "main", &c).unwrap();
    let s = g.status(p, &c).unwrap();
    assert_eq!(s.operation, Some(Operation::Rebase));
    assert_eq!(s.rebase_step, Some((1, 1)));
    g.abort_operation(p, Operation::Rebase, &c).unwrap();
    let s = g.status(p, &c).unwrap();
    assert_eq!((s.operation, s.head), (None, before));

    // Ошибка, не связанная с конфликтами, остаётся ошибкой.
    assert!(g.merge(p, "nope", &c).is_err());
}

#[test]
fn blame_and_file_history() {
    let repo = TempRepo::new("blame");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
    };
    commit("a.txt", "один\nдва\n", "first");
    commit("b.txt", "x\n", "other file");
    commit("a.txt", "один\nдва\nтри\n", "second");
    let first = g.log(p, 10, &c).unwrap()[2].id.clone();

    let history = g.file_log(p, "a.txt", 100, &c).unwrap();
    let subjects: Vec<_> = history.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["second", "first"]);

    repo.write("a.txt", "один\nдва\nтри\nчетыре\n");
    let blame = g.blame(p, "a.txt", None, &c).unwrap();
    let who: Vec<_> = blame
        .lines
        .iter()
        .map(|(i, _)| {
            let c = &blame.commits[*i];
            if c.is_uncommitted() {
                "—"
            } else {
                c.summary.as_str()
            }
        })
        .collect();
    assert_eq!(who, ["first", "first", "second", "—"]);
    assert_eq!(blame.lines[3].1, "четыре");

    let old = g.blame(p, "a.txt", Some(&first), &c).unwrap();
    assert_eq!(old.lines.len(), 2);
    assert_eq!(old.commits.len(), 1);
}
