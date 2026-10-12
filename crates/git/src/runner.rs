use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::GitVersion;

/// Флаг отмены, общий для UI и worker-а.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
pub enum GitError {
    /// Не удалось запустить `git` (не найден, нет прав).
    Spawn(std::io::Error),
    /// git завершился с ненулевым кодом.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    Cancelled,
    Parse(String),
    Unsupported(GitVersion),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "не удалось запустить git: {e}"),
            Self::Failed { code: Some(c), .. } => write!(f, "git завершился с кодом {c}"),
            Self::Failed { code: None, .. } => write!(f, "git завершён сигналом"),
            Self::Cancelled => f.write_str("операция отменена"),
            Self::Parse(m) => write!(f, "не удалось разобрать вывод git: {m}"),
            Self::Unsupported(v) => write!(
                f,
                "git {v} не поддерживается, нужен {} или новее",
                crate::MIN_GIT_VERSION
            ),
        }
    }
}

impl std::error::Error for GitError {}

impl From<GitError> for brancher_core::JobError {
    fn from(e: GitError) -> Self {
        let stderr = match &e {
            GitError::Failed { stderr, .. } => stderr.clone(),
            _ => String::new(),
        };
        Self {
            message: e.to_string(),
            stderr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Запускает git-процессы. Никогда не использует shell (NFR-10).
#[derive(Debug, Clone)]
pub struct Runner {
    pub git: PathBuf,
    /// Дополнительные переменные окружения (например, `GIT_ASKPASS` helper-режима, NFR-09).
    pub env: Vec<(OsString, OsString)>,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            git: PathBuf::from("git"),
            env: vec![],
        }
    }
}

/// Аргументы, которые добавляются к каждому вызову.
const GLOBAL_ARGS: [&str; 6] = [
    "-c",
    "core.quotepath=off",
    "-c",
    "color.ui=false",
    "-c",
    "log.showSignature=false",
];

impl Runner {
    pub fn with_git(git: impl Into<PathBuf>) -> Self {
        Self {
            git: git.into(),
            ..Self::default()
        }
    }

    /// Полная командная строка — для журнала команд (принцип прозрачности, раздел 1).
    pub fn command_line(&self, args: &[&str]) -> String {
        std::iter::once("git")
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn run(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        stdin: Option<&[u8]>,
        cancel: &CancelToken,
    ) -> Result<Output, GitError> {
        self.run_accepting(cwd, args, stdin, cancel, &[0])
    }

    /// Как [`Runner::run`], но успехом считаются и другие коды выхода
    /// (например, 1 у `git diff --no-index`, когда файлы различаются).
    pub fn run_accepting(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        stdin: Option<&[u8]>,
        cancel: &CancelToken,
        ok_codes: &[i32],
    ) -> Result<Output, GitError> {
        if cancel.is_cancelled() {
            return Err(GitError::Cancelled);
        }
        let mut cmd = Command::new(&self.git);
        cmd.args(GLOBAL_ARGS)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(GitError::Spawn)?;

        // stdin, stdout и stderr обслуживаются отдельными потоками, чтобы git не встал на полном пайпе.
        let input = stdin.map(<[u8]>::to_vec);
        let mut child_in = child.stdin.take();
        let writer = thread::spawn(move || {
            if let (Some(pipe), Some(data)) = (child_in.as_mut(), input) {
                let _ = pipe.write_all(&data);
            }
            drop(child_in);
        });
        let mut out_pipe = child.stdout.take().expect("stdout piped");
        let mut err_pipe = child.stderr.take().expect("stderr piped");
        let out_reader = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out_pipe.read_to_end(&mut buf);
            buf
        });
        let err_reader = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err_pipe.read_to_end(&mut buf);
            buf
        });

        let status = loop {
            if let Some(status) = child.try_wait().map_err(GitError::Spawn)? {
                break status;
            }
            if cancel.is_cancelled() {
                // TODO: убивать дерево процессов (command-group), а не только git.
                let _ = child.kill();
                let _ = child.wait();
                let _ = (writer.join(), out_reader.join(), err_reader.join());
                return Err(GitError::Cancelled);
            }
            thread::sleep(Duration::from_millis(5));
        };
        let _ = writer.join();
        let stdout = out_reader.join().unwrap_or_default();
        let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();

        if status.code().is_some_and(|c| ok_codes.contains(&c)) {
            Ok(Output { stdout, stderr })
        } else {
            Err(GitError::Failed {
                code: status.code(),
                stderr,
            })
        }
    }
}
