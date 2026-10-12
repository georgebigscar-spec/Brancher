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

#[test]
fn tags_cherry_pick_and_revert() {
    use brancher_core::Operation;
    let repo = TempRepo::new("tags");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
        g.status(p, &c).unwrap().head.unwrap()
    };
    let init = commit("a.txt", "a\n", "init");
    g.create_tag(p, "v1", &init, "", &c).unwrap();
    g.create_tag(p, "v1-notes", "main", "Первый релиз\n\nподробности", &c)
        .unwrap();
    let tags: Vec<_> = g
        .refs(p, &c)
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == RefKind::Tag)
        .map(|r| (r.name, r.target))
        .collect();
    assert_eq!(
        tags,
        [("v1".into(), init.clone()), ("v1-notes".into(), init.clone())]
    );
    g.delete_tag(p, "v1", &c).unwrap();
    assert_eq!(
        g.refs(p, &c)
            .unwrap()
            .iter()
            .filter(|r| r.kind == RefKind::Tag)
            .count(),
        1
    );

    // Cherry-pick коммита из другой ветки.
    g.create_branch(p, "dev", "main", true, &c).unwrap();
    let feature = commit("b.txt", "b\n", "feature");
    g.switch(p, "main", &c).unwrap();
    g.cherry_pick(p, &feature, false, &c).unwrap();
    assert_eq!(fs::read_to_string(p.join("b.txt")).unwrap(), "b\n");
    assert_eq!(g.log(p, 1, &c).unwrap()[0].subject, "feature");

    // Revert с сообщением по умолчанию.
    let head = g.status(p, &c).unwrap().head.unwrap();
    g.revert(p, &head, false, &c).unwrap();
    assert!(!p.join("b.txt").exists());
    assert!(g.log(p, 1, &c).unwrap()[0].subject.starts_with("Revert"));

    // Конфликт cherry-pick оставляет операцию идти.
    commit("a.txt", "main\n", "main edit");
    g.switch(p, "dev", &c).unwrap();
    let dev_edit = commit("a.txt", "dev\n", "dev edit");
    g.switch(p, "main", &c).unwrap();
    g.cherry_pick(p, &dev_edit, false, &c).unwrap();
    assert_eq!(g.status(p, &c).unwrap().operation, Some(Operation::CherryPick));
    g.abort_operation(p, Operation::CherryPick, &c).unwrap();
    assert_eq!(g.status(p, &c).unwrap().operation, None);
}

#[test]
fn remotes_add_fetch_edit_remove_and_push_tag() {
    let server = TempRepo::new("remote-server");
    let repo = TempRepo::new("remote-client");
    let (g, c) = (git(), CancelToken::new());
    let (s, p) = (server.path(), repo.path());
    server.write("a.txt", "a\n");
    g.stage(s, &["a.txt".into()], &c).unwrap();
    g.commit(s, "init", false, &c).unwrap();

    let url = s.to_string_lossy().into_owned();
    g.add_remote(p, "up", &url, &c).unwrap();
    let remotes = g.remotes(p, &c).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(
        (remotes[0].name.as_str(), remotes[0].url.as_str()),
        ("up", url.as_str())
    );
    assert_eq!(remotes[0].push_url, url);

    g.fetch_remote(p, "up", &c, &mut |_| {}).unwrap();
    let names = |g: &Git| -> Vec<String> {
        g.refs(p, &c)
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == RefKind::RemoteBranch)
            .map(|r| r.name)
            .collect()
    };
    assert_eq!(names(&g), ["up/main"]);

    g.edit_remote(p, "up", Some("origin"), Some("/nowhere"), &c)
        .unwrap();
    let remotes = g.remotes(p, &c).unwrap();
    assert_eq!(
        (remotes[0].name.as_str(), remotes[0].url.as_str()),
        ("origin", "/nowhere")
    );
    assert_eq!(names(&g), ["origin/main"]);
    g.edit_remote(p, "origin", None, Some(&url), &c).unwrap();

    // Тег уходит на remote: у сервера появляется refs/tags/v1.
    g.switch_track(p, "origin/main", &c).unwrap();
    let head = g.status(p, &c).unwrap().head.unwrap();
    g.create_tag(p, "v1", &head, "", &c).unwrap();
    g.push_tag(p, "origin", "v1", &c, &mut |_| {}).unwrap();
    let server_tags: Vec<_> = g
        .refs(s, &c)
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == RefKind::Tag)
        .map(|r| (r.name, r.target))
        .collect();
    assert_eq!(server_tags, [("v1".to_owned(), head)]);

    g.remove_remote(p, "origin", &c).unwrap();
    assert!(g.remotes(p, &c).unwrap().is_empty());
    assert!(names(&g).is_empty());
    // Благодаря `--` имя с дефисом — имя, а не опция git (запрещает его уже core).
    g.add_remote(p, "-v", &url, &c).unwrap();
    assert_eq!(g.remotes(p, &c).unwrap()[0].name, "-v");
}

#[test]
fn interactive_rebase_reword_squash_fixup_drop_and_reorder() {
    use brancher_core::{Operation, RebaseAction as A, RebaseStep};
    let repo = TempRepo::new("irebase");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
        g.status(p, &c).unwrap().head.unwrap()
    };
    let root = commit("base.txt", "base\n", "base");
    let a = commit("a.txt", "a\n", "add a\n\nтело a");
    let b = commit("b.txt", "b\n", "add b");
    let b2 = commit("b.txt", "b2\n", "fix b");
    let d = commit("d.txt", "d\n", "add d");
    let e = commit("e.txt", "e\n", "add e");
    assert_eq!(g.commit_message(p, &a, &c).unwrap(), "add a\n\nтело a");

    let step = |action, commit: &str, message: Option<&str>| RebaseStep {
        action,
        commit: commit.into(),
        subject: String::new(),
        message: message.map(Into::into),
    };
    // e переезжает в начало, a переименовывается, b2 сливается с b, d удаляется.
    g.interactive_rebase(
        p,
        Some(&root),
        &[
            step(A::Pick, &e, None),
            step(A::Reword, &a, Some("Добавить a\n\nновое тело")),
            step(A::Pick, &b, None),
            step(A::Fixup, &b2, None),
            step(A::Drop, &d, None),
        ],
        &c,
    )
    .unwrap();
    let log: Vec<_> = g.log(p, 10, &c).unwrap().into_iter().map(|c| c.subject).collect();
    assert_eq!(log, ["add b", "Добавить a", "add e", "base"]);
    let head = g.status(p, &c).unwrap();
    assert_eq!(head.operation, None);
    assert_eq!(
        g.commit_message(p, "HEAD~1", &c).unwrap(),
        "Добавить a\n\nновое тело"
    );
    assert_eq!(fs::read_to_string(p.join("b.txt")).unwrap(), "b2\n");
    assert!(!p.join("d.txt").exists());

    // Squash объединяет сообщения; с корня истории — `--root`.
    let log = g.log(p, 10, &c).unwrap();
    let ids: Vec<_> = log.iter().rev().map(|c| c.id.clone()).collect();
    g.interactive_rebase(
        p,
        None,
        &[
            step(A::Pick, &ids[0], None),
            step(A::Squash, &ids[1], None),
            step(A::Pick, &ids[2], None),
            step(A::Pick, &ids[3], None),
        ],
        &c,
    )
    .unwrap();
    let log = g.log(p, 10, &c).unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(g.commit_message(p, &log[2].id, &c).unwrap(), "base\n\nadd e");

    // Перестановка с конфликтом останавливает rebase, но не считается ошибкой.
    let x1 = commit("x.txt", "1\n", "x1");
    let x2 = commit("x.txt", "2\n", "x2");
    g.interactive_rebase(
        p,
        Some(&log[0].id),
        &[step(A::Pick, &x2, None), step(A::Pick, &x1, None)],
        &c,
    )
    .unwrap();
    let st = g.status(p, &c).unwrap();
    assert_eq!(st.operation, Some(Operation::Rebase));
    assert_eq!(st.conflicts().count(), 1);
    g.abort_operation(p, Operation::Rebase, &c).unwrap();
    assert_eq!(g.status(p, &c).unwrap().head.unwrap(), x2);
}

#[test]
fn compare_branches_commits_and_range_diff() {
    let repo = TempRepo::new("compare");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
    };
    commit("a.txt", "a\n", "base");
    g.create_branch(p, "feature", "main", false, &c).unwrap();
    commit("main.txt", "m\n", "on main");
    g.switch(p, "feature", &c).unwrap();
    commit("f.txt", "f\n", "feature 1");
    commit("f.txt", "f2\n", "feature 2");
    g.switch(p, "main", &c).unwrap();

    let subjects = |v: Vec<brancher_core::Commit>| v.into_iter().map(|c| c.subject).collect::<Vec<_>>();
    assert_eq!(
        subjects(g.log_range(p, "main", "feature", 100, &c).unwrap()),
        ["feature 2", "feature 1"]
    );
    assert_eq!(
        subjects(g.log_range(p, "feature", "main", 100, &c).unwrap()),
        ["on main"]
    );

    // Diff от общего предка: изменений main в нём нет.
    let target = DiffTarget::Range {
        from: "main".into(),
        to: "feature".into(),
    };
    let files = g.diff(p, &target, &c).unwrap();
    let paths: Vec<_> = files.iter().map(|f| f.path().to_owned()).collect();
    assert_eq!(paths, ["f.txt"]);
}

#[test]
fn undo_timeline_restores_refs_head_stash_and_uncommitted() {
    let repo = TempRepo::new("oplog");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    // Снимок в пустом репозитории не падает.
    g.snapshot(p, "пусто", &c).unwrap();
    let commit = |file: &str, text: &str, msg: &str| {
        repo.write(file, text);
        g.stage(p, &[file.into()], &c).unwrap();
        g.commit(p, msg, false, &c).unwrap();
        g.status(p, &c).unwrap().head.unwrap()
    };
    let first = commit("a.txt", "1\n", "first");
    g.create_branch(p, "topic", "main", false, &c).unwrap();
    g.create_tag(p, "v1", &first, "релиз", &c).unwrap();
    repo.write("a.txt", "незакоммичено\n");
    repo.write("new.txt", "staged\n");
    g.stage(p, &["new.txt".into()], &c).unwrap();
    g.stash_push(p, "в stash", false, &c).unwrap();
    repo.write("a.txt", "незакоммичено\n");

    // Состояние, к которому вернёмся.
    g.snapshot(p, "Коммит «second»", &c).unwrap();
    commit("a.txt", "2\n", "second");
    g.delete_branch(p, "topic", true, &c).unwrap();
    g.delete_tag(p, "v1", &c).unwrap();
    let stash = g.stashes(p, &c).unwrap();
    g.stash_drop(p, 0, &stash[0].id, &c).unwrap();
    g.create_branch(p, "extra", "main", true, &c).unwrap();

    let list = g.snapshots(p, 10, &c).unwrap();
    assert_eq!(
        list.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
        ["Коммит «second»", "пусто"]
    );
    // Снимки не попадают в историю.
    assert!(g
        .log(p, 100, &c)
        .unwrap()
        .iter()
        .all(|c| !c.subject.starts_with("Brancher")));

    g.restore(p, &list[0].id, &c).unwrap();
    let st = g.status(p, &c).unwrap();
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(st.head.as_deref(), Some(first.as_str()));
    let refs: Vec<_> = g.refs(p, &c).unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(refs, ["main", "topic", "v1"]);
    assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "незакоммичено\n");
    let stash = g.stashes(p, &c).unwrap();
    assert_eq!(stash.len(), 1);
    assert!(stash[0].message.ends_with("в stash"));

    // Откат тоже попадает в ленту, и его можно отменить.
    let list = g.snapshots(p, 10, &c).unwrap();
    assert_eq!(list[0].title, "Перед откатом к «Коммит «second»»");
    g.restore(p, &list[0].id, &c).unwrap();
    let st = g.status(p, &c).unwrap();
    assert_eq!(st.branch.as_deref(), Some("extra"));
    let refs: Vec<_> = g.refs(p, &c).unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(refs, ["extra", "main"]);
    assert!(g.stashes(p, &c).unwrap().is_empty());
}

#[test]
fn commit_editing_uncommit_amend_into_and_uncommit_file() {
    use brancher_core::{RebaseAction as A, RebaseStep};
    let repo = TempRepo::new("commit-edit");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let commit = |files: &[(&str, &str)], msg: &str| {
        for (f, t) in files {
            repo.write(f, t);
            g.stage(p, &[(*f).into()], &c).unwrap();
        }
        g.commit(p, msg, false, &c).unwrap();
        g.status(p, &c).unwrap().head.unwrap()
    };
    let subjects = || -> Vec<String> { g.log(p, 20, &c).unwrap().into_iter().map(|c| c.subject).collect() };
    let step = |action, commit: &str| RebaseStep {
        action,
        commit: commit.into(),
        subject: String::new(),
        message: None,
    };
    let base = commit(&[("base.txt", "base\n")], "base");
    let a = commit(&[("a.txt", "a\n")], "add a");
    let b = commit(&[("b.txt", "b\n"), ("c.txt", "c\n")], "add b and c");

    // Последний коммит: файл c.txt уходит из коммита в рабочую копию.
    g.uncommit_file(p, "c.txt", &c).unwrap();
    let st = g.status(p, &c).unwrap();
    assert_eq!(st.changes.len(), 1);
    assert_eq!(st.changes[0].path, "c.txt");
    assert_eq!(subjects(), ["add b and c", "add a", "base"]);
    fs::remove_file(p.join("c.txt")).unwrap();

    // Проиндексированное добавляется в старый коммит, а неиндексированное переживает rebase.
    let head = g.status(p, &c).unwrap().head.unwrap();
    repo.write("a.txt", "a2\n");
    g.stage(p, &["a.txt".into()], &c).unwrap();
    repo.write("b.txt", "b-unstaged\n");
    g.amend_into(p, &a, Some(&base), &[step(A::Pick, &a), step(A::Pick, &head)], &c)
        .unwrap();
    assert_eq!(subjects(), ["add b and c", "add a", "base"]);
    let log = g.log(p, 20, &c).unwrap();
    let files = g
        .diff(
            p,
            &DiffTarget::Commit {
                id: log[1].id.clone(),
                parent: Some(base.clone()),
            },
            &c,
        )
        .unwrap();
    assert_eq!(files[0].hunks[0].lines.last().unwrap().text, "a2");
    assert_eq!(fs::read_to_string(p.join("b.txt")).unwrap(), "b-unstaged\n");
    repo.write("b.txt", "b\n");
    let _ = b;

    // Отменить старый коммит: его изменения возвращаются в индекс.
    let log = g.log(p, 20, &c).unwrap();
    let (a_new, top) = (log[1].id.clone(), log[0].id.clone());
    g.uncommit(
        p,
        &a_new,
        Some((Some(&base), &[step(A::Drop, &a_new), step(A::Pick, &top)])),
        &c,
    )
    .unwrap();
    assert_eq!(subjects(), ["add b and c", "base"]);
    let st = g.status(p, &c).unwrap();
    assert_eq!(
        st.staged().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["a.txt"]
    );

    // Отменить последний коммит.
    g.uncommit(p, &top, None, &c).unwrap();
    assert_eq!(subjects(), ["base"]);
    let st = g.status(p, &c).unwrap();
    let mut staged: Vec<_> = st.staged().map(|c| c.path.clone()).collect();
    staged.sort();
    assert_eq!(staged, ["a.txt", "b.txt"]);
}

#[test]
fn absorb_moves_staged_hunks_into_branch_commits() {
    let repo = TempRepo::new("absorb");
    let (g, c) = (git(), CancelToken::new());
    let p = repo.path();
    let show = |rev: &str| -> String {
        let out = Command::new("git")
            .args(["show", rev])
            .current_dir(p)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let commit = |files: &[(&str, &str)], msg: &str| {
        for (f, t) in files {
            repo.write(f, t);
            g.stage(p, &[(*f).into()], &c).unwrap();
        }
        g.commit(p, msg, false, &c).unwrap();
    };
    commit(&[("f.txt", "1\n2\n3\n4\n5\n6\n"), ("g.txt", "g1\n")], "base");
    g.create_branch(p, "feature", "main", true, &c).unwrap();
    commit(&[("f.txt", "1\nTWO\n3\n4\n5\n6\n")], "change two");
    commit(&[("g.txt", "g1\ng2\n")], "add g2");
    commit(&[("h.txt", "h\n")], "add h");

    // Нечего раскладывать: изменённая строка из коммита main.
    repo.write("f.txt", "1\nTWO\n3\n4\nFIVE\n6\n");
    g.stage(p, &["f.txt".into()], &c).unwrap();
    assert!(g.absorb(p, &c).is_err());

    repo.write("f.txt", "1\nTWO!\n3\n4\nFIVE\n6\n");
    repo.write("g.txt", "g1\ng2\ng3\n");
    g.stage(p, &["f.txt".into(), "g.txt".into()], &c).unwrap();
    repo.write("h.txt", "h-unstaged\n");
    let report = g.absorb(p, &c).unwrap();
    assert_eq!((report.hunks, report.commits, report.left), (2, 2, 1));

    let subjects: Vec<String> = g.log(p, 20, &c).unwrap().into_iter().map(|c| c.subject).collect();
    assert_eq!(subjects, ["add h", "add g2", "change two", "base"]);
    assert!(show("HEAD~2").contains("+TWO!\n"));
    assert!(show("HEAD~1").contains("+g3\n"));
    assert!(!show("HEAD").contains("FIVE"));
    assert_eq!(fs::read_to_string(p.join("h.txt")).unwrap(), "h-unstaged\n");
    assert_eq!(
        fs::read_to_string(p.join("f.txt")).unwrap(),
        "1\nTWO!\n3\n4\nFIVE\n6\n"
    );
    let st = g.status(p, &c).unwrap();
    assert!(st.operation.is_none());
    let mut changed: Vec<_> = st.changes.iter().map(|c| c.path.clone()).collect();
    changed.sort();
    assert_eq!(changed, ["f.txt", "h.txt"]);
}
