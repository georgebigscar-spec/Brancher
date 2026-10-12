//! Работа с репозиторием через установленный `git` (раздел 3 спецификации).
//!
//! * Аргументы и парсеры — чистые функции в [`commands`], тестируются на фикстурах.
//! * [`Runner`] запускает процесс без shell, аргументы массивом (NFR-10),
//!   с `core.quotepath=off` (NFR-15), без интерактивных запросов, с отменой (NFR-06).
//! * Парсеры опираются только на porcelain- и format-вывод.

pub mod commands;
mod runner;
mod version;

pub use runner::{CancelToken, GitError, Output, Runner};
pub use version::{GitVersion, MIN_GIT_VERSION};

use std::path::{Path, PathBuf};

use brancher_core::{
    AbsorbReport, Blame, Commit, DiffTarget, FileDiff, GitRef, Operation, Progress, RebaseAction, RebaseStep,
    Remote, RepoStatus, Snapshot, Stash,
};

/// Высокоуровневые операции поверх [`Runner`].
#[derive(Debug, Clone)]
pub struct Git {
    pub runner: Runner,
}

impl Git {
    pub fn new(runner: Runner) -> Self {
        Self { runner }
    }

    /// Ищет `git` в PATH и проверяет минимальную версию (NFR-14).
    pub fn detect() -> Result<(Self, GitVersion), GitError> {
        let runner = Runner::default();
        let out = runner.run(None, &["--version"], None, &CancelToken::new())?;
        let version = GitVersion::parse(&out.stdout_lossy())
            .ok_or_else(|| GitError::Parse(format!("unknown git version: {}", out.stdout_lossy())))?;
        if version < MIN_GIT_VERSION {
            return Err(GitError::Unsupported(version));
        }
        Ok((Self { runner }, version))
    }

    pub fn status(&self, repo: &Path, cancel: &CancelToken) -> Result<RepoStatus, GitError> {
        let out = self
            .runner
            .run(Some(repo), &commands::status::args(), None, cancel)?;
        let mut status = commands::status::parse(&out.stdout)?;
        let git_dir = self.git_dir(repo, cancel)?;
        (status.operation, status.rebase_step) = commands::merge::detect(&git_dir);
        Ok(status)
    }

    fn git_dir(&self, repo: &Path, cancel: &CancelToken) -> Result<PathBuf, GitError> {
        let args = commands::merge::git_dir_args();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        Ok(PathBuf::from(out.stdout_lossy().trim_end_matches(['\r', '\n'])))
    }

    /// Merge или rebase. Остановка на конфликтах — не ошибка: операция остаётся идти,
    /// конфликты видны в статусе.
    pub fn merge(&self, repo: &Path, target: &str, cancel: &CancelToken) -> Result<(), GitError> {
        self.start_operation(repo, &commands::merge::merge_args(target), cancel)
    }

    /// Файлы, в которых слияние HEAD с `target` даст конфликты; рабочая копия не меняется.
    /// Требует git 2.38+ (`merge-tree --write-tree`), на старом git — ошибка.
    pub fn merge_conflicts(
        &self,
        repo: &Path,
        target: &str,
        cancel: &CancelToken,
    ) -> Result<Vec<String>, GitError> {
        let args = commands::merge::preview_args(target);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self
            .runner
            .run_accepting(Some(repo), &args, None, cancel, &[0, 1])?;
        Ok(commands::merge::parse_preview(&out.stdout))
    }

    pub fn rebase(&self, repo: &Path, onto: &str, cancel: &CancelToken) -> Result<(), GitError> {
        self.start_operation(repo, &commands::merge::rebase_args(onto), cancel)
    }

    /// Сохраняет снимок для ленты отмены перед операцией `title`.
    pub fn snapshot(&self, repo: &Path, title: &str, cancel: &CancelToken) -> Result<(), GitError> {
        use commands::oplog;
        let run = |args: &[&str], stdin: Option<&[u8]>, ok: &[i32]| {
            self.runner.run_accepting(Some(repo), args, stdin, cancel, ok)
        };
        let refs = run(&strs(&oplog::refs_args()), None, &[0])?.stdout_lossy();
        let (refs, mut keep) = oplog::parse_refs(&refs);
        let stashes: Vec<(String, String)> = self
            .stashes(repo, cancel)?
            .into_iter()
            .map(|s| (s.id, s.message))
            .collect();
        keep.extend(stashes.iter().map(|(id, _)| id.clone()));
        let head = match run(&["symbolic-ref", "-q", "HEAD"], None, &[0, 1])?
            .stdout_lossy()
            .trim()
        {
            "" => {
                let out = run(&["rev-parse", "--verify", "--quiet", "HEAD"], None, &[0, 1])?;
                Some(out.stdout_lossy().trim().to_owned()).filter(|h| !h.is_empty())
            }
            sym => Some(format!("ref: {sym}")),
        };
        // В репозитории без коммитов `stash create` не работает — незакоммиченного там не сохранить.
        let wip = run(&["stash", "create"], None, &[0, 1, 128])
            .map(|o| o.stdout_lossy().trim().to_owned())
            .ok()
            .filter(|w| !w.is_empty());
        if let Some(h) = head.as_deref().filter(|h| !h.starts_with("ref: ")) {
            keep.push(h.to_owned());
        }
        let state = oplog::State {
            head,
            wip: wip.clone(),
            refs,
            stashes,
        };
        let prev = run(
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{}^{{commit}}", oplog::REF),
            ],
            None,
            &[0, 1],
        )?
        .stdout_lossy()
        .trim()
        .to_owned();
        let tree = run(&["mktree"], Some(b""), &[0])?
            .stdout_lossy()
            .trim()
            .to_owned();
        let mut parents: Vec<String> = vec![];
        for p in std::iter::once(prev).chain(wip).chain(keep) {
            if !p.is_empty() && !parents.contains(&p) {
                parents.push(p);
            }
        }
        let mut args = vec!["commit-tree".to_owned(), "--no-gpg-sign".to_owned(), tree];
        for p in parents {
            args.extend(["-p".to_owned(), p]);
        }
        args.extend(["-F".to_owned(), "-".to_owned()]);
        let mut git = self.clone();
        for (k, v) in [
            ("GIT_AUTHOR_NAME", "Brancher"),
            ("GIT_AUTHOR_EMAIL", "brancher@localhost"),
            ("GIT_COMMITTER_NAME", "Brancher"),
            ("GIT_COMMITTER_EMAIL", "brancher@localhost"),
        ] {
            git.runner.env.push((k.into(), v.into()));
        }
        let message = oplog::message(title, &state);
        let out = git
            .runner
            .run(Some(repo), &strs(&args), Some(message.as_bytes()), cancel)?;
        let id = out.stdout_lossy().trim().to_owned();
        run(
            &["update-ref", "-m", "brancher snapshot", oplog::REF, &id],
            None,
            &[0],
        )
        .map(|_| ())
    }

    /// Снимки ленты отмены, новые сверху.
    pub fn snapshots(
        &self,
        repo: &Path,
        limit: usize,
        cancel: &CancelToken,
    ) -> Result<Vec<Snapshot>, GitError> {
        use commands::oplog;
        let exists = self.runner.run_accepting(
            Some(repo),
            &["rev-parse", "--verify", "--quiet", oplog::REF],
            None,
            cancel,
            &[0, 1],
        )?;
        if exists.stdout_lossy().trim().is_empty() {
            return Ok(vec![]);
        }
        let out = self
            .runner
            .run(Some(repo), &strs(&oplog::list_args(limit)), None, cancel)?;
        oplog::parse_list(&out.stdout)
    }

    /// Возвращает репозиторий к снимку `id`. Текущее состояние перед этим само попадает
    /// в ленту, так что откат тоже можно отменить.
    pub fn restore(&self, repo: &Path, id: &str, cancel: &CancelToken) -> Result<(), GitError> {
        use commands::oplog;
        if let Some(op) = self.status(repo, cancel)?.operation {
            return Err(GitError::Stale(format!(
                "Идёт {}: сначала завершите или прервите его",
                op.name()
            )));
        }
        let run = |args: &[&str], stdin: Option<&[u8]>| self.runner.run(Some(repo), args, stdin, cancel);
        let msg = run(&["log", "-1", "--format=%B", id, "--"], None)?.stdout_lossy();
        let state =
            oplog::parse_message(&msg).ok_or_else(|| GitError::Parse(format!("{id} не снимок Brancher")))?;
        let title = msg
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches("Brancher: ");
        self.snapshot(repo, &format!("Перед откатом к «{title}»"), cancel)?;

        // Ветки и теги: как в снимке, лишние — удалить.
        let (current, _) = oplog::parse_refs(&run(&strs(&oplog::refs_args()), None)?.stdout_lossy());
        let mut script = String::new();
        for (sha, name) in &state.refs {
            script.push_str(&format!("update {name} {sha}\n"));
        }
        for (_, name) in current
            .iter()
            .filter(|(_, n)| !state.refs.iter().any(|(_, s)| s == n))
        {
            script.push_str(&format!("delete {name}\n"));
        }
        run(&["update-ref", "--stdin"], Some(script.as_bytes()))?;

        if let Some(h) = state.head.as_deref() {
            match h.strip_prefix("ref: ") {
                Some(r) => run(&["symbolic-ref", "HEAD", r], None)?,
                None => run(&["update-ref", "--no-deref", "HEAD", h], None)?,
            };
        }
        let has_head = run(&["rev-parse", "--verify", "--quiet", "HEAD"], None).is_ok();
        if has_head {
            run(&["reset", "--hard", "-q"], None)?;
        }

        // Stash: записи, которых не было в снимке, убираются (их содержимое есть в снимке
        // «Перед откатом…»), недостающие возвращаются.
        let now = self.stashes(repo, cancel)?;
        for st in now
            .iter()
            .rev()
            .filter(|st| !state.stashes.iter().any(|(id, _)| *id == st.id))
        {
            run(&strs(&commands::stash::drop_args(st.index)), None)?;
        }
        let now: Vec<String> = self.stashes(repo, cancel)?.into_iter().map(|s| s.id).collect();
        for (sha, m) in state.stashes.iter().rev().filter(|(id, _)| !now.contains(id)) {
            run(&["stash", "store", "-m", m, sha], None)?;
        }

        if let Some(wip) = &state.wip {
            run(&["stash", "apply", "--index", wip], None)?;
        }
        Ok(())
    }

    /// Interactive rebase по готовому плану; `base` — `None` для rebase с корня истории.
    /// Остановка на конфликте — не ошибка, как у обычного rebase.
    pub fn interactive_rebase(
        &self,
        repo: &Path,
        base: Option<&str>,
        steps: &[RebaseStep],
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        use commands::rebase::{REWORD_DIR, TODO_FILE};
        let git_dir = self.git_dir(repo, cancel)?;
        let io = |e: std::io::Error| GitError::Parse(format!("не удалось записать план rebase: {e}"));
        let reword = git_dir.join(REWORD_DIR);
        let _ = std::fs::remove_dir_all(&reword);
        std::fs::create_dir_all(&reword).map_err(io)?;
        for (i, s) in steps.iter().enumerate() {
            if let (RebaseAction::Reword, Some(m)) = (s.action, &s.message) {
                std::fs::write(reword.join(i.to_string()), m).map_err(io)?;
            }
        }
        std::fs::write(git_dir.join(TODO_FILE), commands::rebase::todo(steps)).map_err(io)?;
        let mut git = self.clone();
        git.runner.env.push((
            "GIT_SEQUENCE_EDITOR".into(),
            commands::rebase::sequence_editor().into(),
        ));
        git.start_operation(repo, &commands::rebase::start_args(base), cancel)
    }

    /// Отменяет коммит, оставляя его изменения незакоммиченными (в индексе).
    /// `rebase` — план без этого коммита, если он не последний: `(база, шаги)`.
    pub fn uncommit(
        &self,
        repo: &Path,
        commit: &str,
        rebase: Option<(Option<&str>, &[RebaseStep])>,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let run = |args: &[&str]| self.runner.run(Some(repo), args, None, cancel).map(|_| ());
        match rebase {
            None if self.has_parent(repo, "HEAD", cancel)? => run(&["reset", "--soft", "-q", "HEAD~1"]),
            // Единственный коммит: ветка снова пустая, файлы остаются в индексе.
            None => run(&["update-ref", "-d", "HEAD"]),
            Some((base, steps)) => {
                self.interactive_rebase(repo, base, steps, cancel)?;
                if self.status(repo, cancel)?.operation.is_some() {
                    return Err(GitError::Stale(
                        "Rebase без этого коммита остановился на конфликте. Прервите rebase \
                         или разрешите конфликт; всё вернуть можно через «История»"
                            .into(),
                    ));
                }
                self.start_operation(repo, &strs_owned(&["cherry-pick", "--no-commit", commit]), cancel)
            }
        }
    }

    /// Добавляет проиндексированные изменения в коммит `commit` (ниже HEAD): fixup-коммит
    /// и interactive rebase, где он сразу после `commit`.
    pub fn amend_into(
        &self,
        repo: &Path,
        commit: &str,
        base: Option<&str>,
        steps: &[RebaseStep],
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let msg = format!("fixup! {commit}");
        self.runner.run(
            Some(repo),
            &["commit", "--no-verify", "-F", "-"],
            Some(msg.as_bytes()),
            cancel,
        )?;
        let fixup = self.status(repo, cancel)?.head.unwrap_or_default();
        let mut plan = steps.to_vec();
        let at = plan
            .iter()
            .position(|s| s.commit == commit)
            .map_or(plan.len(), |i| i + 1);
        plan.insert(
            at,
            RebaseStep {
                action: RebaseAction::Fixup,
                commit: fixup,
                subject: msg,
                message: None,
            },
        );
        self.interactive_rebase(repo, base, &plan, cancel)
    }

    /// Раскладывает проиндексированные фрагменты по коммитам текущей ветки, которые последними
    /// меняли эти строки: fixup-коммит на каждый такой коммит и один interactive rebase.
    /// Фрагменты, которые не удалось отнести к одному коммиту ветки, остаются в индексе.
    pub fn absorb(&self, repo: &Path, cancel: &CancelToken) -> Result<AbsorbReport, GitError> {
        use commands::absorb;
        let run = |args: &[String], input: Option<&[u8]>| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            self.runner.run(Some(repo), &args, input, cancel)
        };
        let status = self.status(repo, cancel)?;
        let Some(head) = status.head else {
            return Err(GitError::Stale("В репозитории ещё нет коммитов".into()));
        };
        let refs = run(&absorb::branch_refs_args(), None)?.stdout_lossy();
        let input = absorb::candidates_input(status.branch.as_deref(), &refs);
        let candidates =
            absorb::parse_candidates(&run(&absorb::candidates_args(200), Some(input.as_bytes()))?.stdout);
        let files = absorb::parse_diff(&run(&absorb::staged_diff_args(), None)?.stdout);
        let total: usize = files.iter().map(|f| f.hunks.len()).sum();

        // Владелец каждого фрагмента: индекс в `candidates` (0 — HEAD).
        let mut owners: Vec<Vec<Option<usize>>> = Vec::new();
        for f in &files {
            let blame = absorb::parse_blame(&run(&absorb::blame_args(&f.path), None)?.stdout);
            owners.push(
                f.hunks
                    .iter()
                    .map(|h| {
                        absorb::owner(h, &blame).and_then(|sha| candidates.iter().position(|c| c.id == sha))
                    })
                    .collect(),
            );
        }
        // Цели от старых к новым.
        let mut targets: Vec<usize> = owners.iter().flatten().flatten().copied().collect();
        targets.sort_unstable_by(|a, b| b.cmp(a));
        targets.dedup();
        let hunks = owners.iter().flatten().flatten().count();
        if targets.is_empty() {
            return Err(GitError::Stale(
                "Ни один проиндексированный фрагмент не удалось отнести к коммиту этой ветки: \
                 строки менялись в разных коммитах или в коммитах, которые есть и в других ветках"
                    .into(),
            ));
        }

        // Fixup-коммиты цепочкой поверх HEAD. Дерево k-го — HEAD плюс фрагменты целей 1..=k,
        // применённые одним патчем, чтобы номера строк совпадали с diff.
        let git_dir = self.git_dir(repo, cancel)?;
        let index = git_dir.join("brancher-absorb-index");
        let mut tmp = self.clone();
        tmp.runner
            .env
            .push(("GIT_INDEX_FILE".into(), index.clone().into_os_string()));
        let tmp_run = |args: &[&str], input: Option<&[u8]>| tmp.runner.run(Some(repo), args, input, cancel);
        let mut parent = head.clone();
        let mut fixups = Vec::new();
        let result = (|| {
            tmp_run(&["read-tree", "HEAD"], None)?;
            let mut applied = 0;
            for (k, &t) in targets.iter().enumerate() {
                let patch = absorb::build_patch(&files, |fi, hi| {
                    owners[fi][hi].is_some_and(|o| targets[applied..=k].contains(&o))
                });
                applied = k + 1;
                tmp_run(
                    &["apply", "--cached", "--unidiff-zero", "-"],
                    Some(patch.as_bytes()),
                )?;
                let tree = tmp_run(&["write-tree"], None)?.stdout_lossy().trim().to_owned();
                let msg = format!("fixup! {}", candidates[t].subject);
                let commit = tmp_run(
                    &["commit-tree", &tree, "-p", &parent, "-F", "-"],
                    Some(msg.as_bytes()),
                )?
                .stdout_lossy()
                .trim()
                .to_owned();
                fixups.push((t, commit.clone(), msg));
                parent = commit;
            }
            Ok::<_, GitError>(())
        })();
        let _ = std::fs::remove_file(&index);
        result?;
        self.runner.run(
            Some(repo),
            &["update-ref", "-m", "brancher: absorb", "HEAD", &parent, &head],
            None,
            cancel,
        )?;

        // План: коммиты от самой старой цели до старого HEAD, fixup сразу после своей цели.
        let oldest = targets[0];
        let mut steps = Vec::new();
        for i in (0..=oldest).rev() {
            let c = &candidates[i];
            steps.push(RebaseStep {
                action: RebaseAction::Pick,
                commit: c.id.clone(),
                subject: c.subject.clone(),
                message: None,
            });
            for (_, id, msg) in fixups.iter().filter(|(t, ..)| *t == i) {
                steps.push(RebaseStep {
                    action: RebaseAction::Fixup,
                    commit: id.clone(),
                    subject: msg.clone(),
                    message: None,
                });
            }
        }
        self.interactive_rebase(repo, candidates[oldest].parent.as_deref(), &steps, cancel)?;
        Ok(AbsorbReport {
            hunks,
            commits: targets.len(),
            left: total - hunks,
        })
    }

    /// Убирает файл из последнего коммита; его изменения остаются в рабочей копии.
    pub fn uncommit_file(&self, repo: &Path, path: &str, cancel: &CancelToken) -> Result<(), GitError> {
        let run = |args: &[&str]| self.runner.run(Some(repo), args, None, cancel).map(|_| ());
        if self.has_parent(repo, "HEAD", cancel)? {
            run(&["reset", "-q", "HEAD~1", "--", path])?;
        } else {
            run(&["rm", "--cached", "-q", "--", path])?;
        }
        run(&["commit", "--amend", "--no-edit", "--allow-empty", "--no-verify"])
    }

    fn has_parent(&self, repo: &Path, rev: &str, cancel: &CancelToken) -> Result<bool, GitError> {
        let out = self.runner.run_accepting(
            Some(repo),
            &["rev-parse", "--verify", "--quiet", &format!("{rev}^")],
            None,
            cancel,
            &[0, 1],
        )?;
        Ok(!out.stdout_lossy().trim().is_empty())
    }

    /// Полное сообщение коммита (для reword).
    pub fn commit_message(
        &self,
        repo: &Path,
        commit: &str,
        cancel: &CancelToken,
    ) -> Result<String, GitError> {
        let args = commands::rebase::message_args(commit);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        Ok(out.stdout_lossy().trim_end().to_owned())
    }

    /// `--continue`; новые конфликты на следующем шаге rebase — тоже не ошибка.
    pub fn continue_operation(
        &self,
        repo: &Path,
        op: Operation,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.start_operation(repo, &commands::merge::continue_args(op), cancel)
    }

    pub fn abort_operation(&self, repo: &Path, op: Operation, cancel: &CancelToken) -> Result<(), GitError> {
        self.run_args(repo, &commands::merge::abort_args(op), None, cancel)
    }

    /// Разрешить конфликт версией одной стороны и отметить файл решённым.
    pub fn resolve_with(
        &self,
        repo: &Path,
        path: &str,
        ours: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.run_args(
            repo,
            &commands::merge::checkout_side_args(path, ours),
            None,
            cancel,
        )?;
        self.stage(repo, &[path.to_owned()], cancel)
    }

    fn start_operation(&self, repo: &Path, args: &[String], cancel: &CancelToken) -> Result<(), GitError> {
        match self.run_args(repo, args, None, cancel) {
            Err(GitError::Failed { .. }) if self.has_conflicts(repo, cancel)? => Ok(()),
            other => other,
        }
    }

    fn has_conflicts(&self, repo: &Path, cancel: &CancelToken) -> Result<bool, GitError> {
        let status = self.status(repo, cancel)?;
        Ok(status.operation.is_some() && status.conflicts().next().is_some())
    }

    pub fn refs(&self, repo: &Path, cancel: &CancelToken) -> Result<Vec<GitRef>, GitError> {
        let out = self
            .runner
            .run(Some(repo), &commands::refs::args(), None, cancel)?;
        commands::refs::parse(&out.stdout)
    }

    /// История всех веток (`--all`); в пустом репозитории — пустой список.
    pub fn log(&self, repo: &Path, limit: usize, cancel: &CancelToken) -> Result<Vec<Commit>, GitError> {
        let args = commands::log::args(limit);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        commands::log::parse(&out.stdout)
    }

    /// Коммиты `to`, которых нет в `from`.
    pub fn log_range(
        &self,
        repo: &Path,
        from: &str,
        to: &str,
        limit: usize,
        cancel: &CancelToken,
    ) -> Result<Vec<Commit>, GitError> {
        let args = commands::log::range_args(from, to, limit);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        commands::log::parse(&out.stdout)
    }

    /// История файла с переименованиями, новые коммиты сверху.
    pub fn file_log(
        &self,
        repo: &Path,
        path: &str,
        limit: usize,
        cancel: &CancelToken,
    ) -> Result<Vec<Commit>, GitError> {
        let args = commands::log::file_args(path, limit);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        commands::log::parse(&out.stdout)
    }

    /// Blame файла в коммите `rev` или в рабочей копии (`None`).
    pub fn blame(
        &self,
        repo: &Path,
        path: &str,
        rev: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Blame, GitError> {
        let args = commands::blame::args(path, rev);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        commands::blame::parse(&out.stdout)
    }

    pub fn diff(
        &self,
        repo: &Path,
        target: &DiffTarget,
        cancel: &CancelToken,
    ) -> Result<Vec<FileDiff>, GitError> {
        let args = commands::diff::args(target);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let ok = commands::diff::ok_codes(target);
        let out = self.runner.run_accepting(Some(repo), &args, None, cancel, ok)?;
        commands::diff::parse(&out.stdout)
    }

    pub fn stage(&self, repo: &Path, paths: &[String], cancel: &CancelToken) -> Result<(), GitError> {
        let args = commands::stage::stage_args(paths);
        self.run_args(repo, &args, None, cancel)
    }

    pub fn unstage(
        &self,
        repo: &Path,
        paths: &[String],
        has_head: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::stage::unstage_args(paths, has_head);
        self.run_args(repo, &args, None, cancel)
    }

    /// Применяет patch к индексу (`cached`) или к рабочему дереву; `reverse` — откатывает его.
    /// Patch идёт через stdin.
    pub fn apply(
        &self,
        repo: &Path,
        patch: &str,
        cached: bool,
        reverse: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::apply::args(cached, reverse);
        self.run_args(repo, &args, Some(patch.as_bytes()), cancel)
    }

    /// Сообщение передаётся через stdin (`-F -`), чтобы не упираться в длину командной строки.
    pub fn commit(
        &self,
        repo: &Path,
        message: &str,
        amend: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::commit::args(amend);
        self.run_args(repo, &args, Some(message.as_bytes()), cancel)
    }

    pub fn switch(&self, repo: &Path, branch: &str, cancel: &CancelToken) -> Result<(), GitError> {
        self.run_args(repo, &commands::branch::switch_args(branch), None, cancel)
    }

    pub fn switch_track(
        &self,
        repo: &Path,
        remote_branch: &str,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::branch::switch_track_args(remote_branch);
        self.run_args(repo, &args, None, cancel)
    }

    pub fn create_branch(
        &self,
        repo: &Path,
        name: &str,
        start: &str,
        checkout: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::branch::create_args(name, start, checkout);
        self.run_args(repo, &args, None, cancel)
    }

    pub fn delete_branch(
        &self,
        repo: &Path,
        name: &str,
        force: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.run_args(repo, &commands::branch::delete_args(name, force), None, cancel)
    }

    pub fn stashes(&self, repo: &Path, cancel: &CancelToken) -> Result<Vec<Stash>, GitError> {
        let args = commands::stash::list_args();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        commands::stash::parse_list(&out.stdout)
    }

    pub fn stash_push(
        &self,
        repo: &Path,
        message: &str,
        untracked: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.run_args(
            repo,
            &commands::stash::push_args(message, untracked),
            None,
            cancel,
        )
    }

    /// Применяет `stash@{index}` (и удаляет его при `pop`), если там всё ещё коммит `id`.
    pub fn stash_apply(
        &self,
        repo: &Path,
        index: usize,
        id: &str,
        pop: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.verify_stash(repo, index, id, cancel)?;
        self.run_args(repo, &commands::stash::apply_args(index, pop), None, cancel)
    }

    pub fn stash_drop(
        &self,
        repo: &Path,
        index: usize,
        id: &str,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.verify_stash(repo, index, id, cancel)?;
        self.run_args(repo, &commands::stash::drop_args(index), None, cancel)
    }

    /// Номера stash сдвигаются, когда прячут или удаляют другие записи (в том числе из терминала):
    /// действуем, только если под номером та самая запись.
    fn verify_stash(
        &self,
        repo: &Path,
        index: usize,
        id: &str,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args = commands::stash::verify_args(index);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self
            .runner
            .run_accepting(Some(repo), &args, None, cancel, &[0, 1])?;
        if out.stdout_lossy().trim() == id {
            Ok(())
        } else {
            Err(GitError::Stale(format!(
                "{} уже другая запись: список stash изменился, проверьте его и повторите",
                commands::stash::reference(index)
            )))
        }
    }

    /// Тег на `target`; непустое `message` делает его аннотированным.
    pub fn create_tag(
        &self,
        repo: &Path,
        name: &str,
        target: &str,
        message: &str,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let annotated = !message.trim().is_empty();
        let args = commands::tag::create_args(name, target, annotated);
        let stdin = annotated.then_some(message.as_bytes());
        self.run_args(repo, &args, stdin, cancel)
    }

    pub fn delete_tag(&self, repo: &Path, name: &str, cancel: &CancelToken) -> Result<(), GitError> {
        self.run_args(repo, &commands::tag::delete_args(name), None, cancel)
    }

    pub fn push_tag(
        &self,
        repo: &Path,
        remote: &str,
        name: &str,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        self.run_progress(repo, &commands::tag::push_args(remote, name), cancel, on_progress)
    }

    /// Cherry-pick и revert; остановка на конфликтах — не ошибка, как у merge.
    pub fn cherry_pick(
        &self,
        repo: &Path,
        commit: &str,
        mainline: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.start_operation(repo, &commands::tag::cherry_pick_args(commit, mainline), cancel)
    }

    pub fn revert(
        &self,
        repo: &Path,
        commit: &str,
        mainline: bool,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.start_operation(repo, &commands::tag::revert_args(commit, mainline), cancel)
    }

    pub fn fetch(
        &self,
        repo: &Path,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        self.run_progress(repo, &commands::remote::fetch_args(), cancel, on_progress)
    }

    pub fn fetch_remote(
        &self,
        repo: &Path,
        remote: &str,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        self.run_progress(
            repo,
            &commands::remote::fetch_remote_args(remote),
            cancel,
            on_progress,
        )
    }

    pub fn remotes(&self, repo: &Path, cancel: &CancelToken) -> Result<Vec<Remote>, GitError> {
        let args = commands::remote::list_args();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.runner.run(Some(repo), &args, None, cancel)?;
        Ok(commands::remote::parse_list(&out.stdout_lossy()))
    }

    pub fn add_remote(
        &self,
        repo: &Path,
        name: &str,
        url: &str,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        self.run_args(repo, &commands::remote::add_args(name, url), None, cancel)
    }

    /// Удаляет remote вместе с его remote-ветками и настройками upstream.
    pub fn remove_remote(&self, repo: &Path, name: &str, cancel: &CancelToken) -> Result<(), GitError> {
        self.run_args(repo, &commands::remote::remove_args(name), None, cancel)
    }

    /// Переименование (`new_name`) и новый URL (`url`); `None` — не менять.
    pub fn edit_remote(
        &self,
        repo: &Path,
        name: &str,
        new_name: Option<&str>,
        url: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let mut name = name;
        if let Some(new) = new_name {
            self.run_args(repo, &commands::remote::rename_args(name, new), None, cancel)?;
            name = new;
        }
        if let Some(url) = url {
            self.run_args(repo, &commands::remote::set_url_args(name, url), None, cancel)?;
        }
        Ok(())
    }

    pub fn pull(
        &self,
        repo: &Path,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        self.run_progress(repo, &commands::remote::pull_args(), cancel, on_progress)
    }

    pub fn push(
        &self,
        repo: &Path,
        set_upstream: Option<(&str, &str)>,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        let args = commands::remote::push_args(set_upstream);
        self.run_progress(repo, &args, cancel, on_progress)
    }

    fn run_progress(
        &self,
        repo: &Path,
        args: &[String],
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.runner
            .run_with_progress(Some(repo), &args, cancel, &mut |line| {
                if let Some(p) = commands::progress::parse_progress(line) {
                    on_progress(p);
                }
            })
            .map(|_| ())
    }

    /// Клонирует `url` в `dest`, передавая этапы прогресса в `on_progress`.
    pub fn clone_repo(
        &self,
        url: &str,
        dest: &Path,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), GitError> {
        let dest = dest.to_string_lossy();
        let args = commands::clone::args(url, &dest);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.runner
            .run_with_progress(None, &args, cancel, &mut |line| {
                if let Some(p) = commands::progress::parse_progress(line) {
                    on_progress(p);
                }
            })
            .map(|_| ())
    }

    fn run_args(
        &self,
        repo: &Path,
        args: &[String],
        stdin: Option<&[u8]>,
        cancel: &CancelToken,
    ) -> Result<(), GitError> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.runner.run(Some(repo), &args, stdin, cancel).map(|_| ())
    }
}

fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

fn strs_owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}
