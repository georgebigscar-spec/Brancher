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

use std::path::Path;

use brancher_core::{Commit, DiffTarget, FileDiff, GitRef, RepoStatus};

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
        commands::status::parse(&out.stdout)
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
