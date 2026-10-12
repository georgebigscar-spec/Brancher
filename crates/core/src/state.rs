//! Состояние приложения и переходы: `Action` → `reduce` → `Effect`, `JobResult` → `apply_result`.

use std::path::{Path, PathBuf};

use crate::graph::CommitGraph;
use crate::model::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoId(pub u64);

/// Фоновая задача для одного репозитория. Выполняется worker-ом из `brancher-services`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Статус, ссылки и история.
    Refresh {
        history_limit: usize,
    },
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Commit {
        message: String,
        amend: bool,
    },
}

impl Job {
    /// Изменяет ли задача репозиторий (такие задачи идут строго по одной, раздел 3).
    pub fn is_write(&self) -> bool {
        !matches!(self, Job::Refresh { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Job::Refresh { .. } => "refresh",
            Job::Stage(_) => "stage",
            Job::Unstage(_) => "unstage",
            Job::Commit { .. } => "commit",
        }
    }
}

/// История коммитов вместе с раскладкой графа, посчитанной в фоне.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    pub commits: Vec<Commit>,
    pub graph: CommitGraph,
}

impl History {
    pub fn new(commits: Vec<Commit>) -> Self {
        let graph = CommitGraph::build(&commits);
        Self { commits, graph }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    Status(Result<RepoStatus, JobError>),
    Refs(Result<Vec<GitRef>, JobError>),
    History(Result<History, JobError>),
    /// Задача завершена (после этого счётчик занятости уменьшается).
    Finished {
        job: &'static str,
        result: Result<(), JobError>,
    },
}

/// Намерение пользователя, полученное из UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenRepo(PathBuf),
    /// Показать стартовую страницу, не закрывая вкладки.
    ShowWelcome,
    SelectTab(usize),
    CloseTab(usize),
    Refresh,
    SelectCommit(Option<Oid>),
    Stage(Vec<String>),
    StageAll,
    Unstage(Vec<String>),
    UnstageAll,
    SetCommitMessage(String),
    SetAmend(bool),
    Commit,
    DismissNotification(usize),
}

/// Побочный эффект, который должен выполнить слой services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Создать очередь задач для нового репозитория.
    StartRepo {
        id: RepoId,
        path: PathBuf,
    },
    StopRepo(RepoId),
    Spawn {
        repo: RepoId,
        job: Job,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub severity: Severity,
    pub title: String,
    pub details: String,
}

pub const DEFAULT_HISTORY_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Default)]
pub struct RepoState {
    pub id: Option<RepoId>,
    pub path: PathBuf,
    pub name: String,
    pub status: Option<RepoStatus>,
    pub refs: Vec<GitRef>,
    pub history: Vec<Commit>,
    /// Раскладка графа для `history`, строка в строку.
    pub graph: CommitGraph,
    pub selected_commit: Option<Oid>,
    pub commit_message: String,
    pub amend: bool,
    /// Сколько задач сейчас выполняется.
    pub busy: u32,
    /// Растёт при каждом изменении данных — ключ кешей UI (раздел 3).
    pub version: u64,
}

impl RepoState {
    fn id(&self) -> RepoId {
        self.id.expect("RepoState без id")
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub tabs: Vec<RepoState>,
    /// `None` — открыта стартовая страница.
    pub active: Option<usize>,
    pub recent: Vec<RepoEntry>,
    pub notifications: Vec<Notification>,
    pub history_limit: usize,
    next_id: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
            ..Default::default()
        }
    }

    pub fn active_repo(&self) -> Option<&RepoState> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    pub fn active_repo_mut(&mut self) -> Option<&mut RepoState> {
        self.active.and_then(|i| self.tabs.get_mut(i))
    }

    pub fn repo_mut(&mut self, id: RepoId) -> Option<&mut RepoState> {
        self.tabs.iter_mut().find(|r| r.id == Some(id))
    }

    fn notify_error(&mut self, title: impl Into<String>, err: &JobError) {
        self.notifications.push(Notification {
            severity: Severity::Error,
            title: title.into(),
            details: err.to_string(),
        });
    }
}

fn repo_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Применяет действие пользователя к состоянию и возвращает эффекты.
pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    let limit = if state.history_limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        state.history_limit
    };
    let refresh = |id| Effect::Spawn {
        repo: id,
        job: Job::Refresh { history_limit: limit },
    };

    match action {
        Action::OpenRepo(path) => {
            if let Some(i) = state.tabs.iter().position(|r| r.path == path) {
                state.active = Some(i);
                return vec![];
            }
            state.next_id += 1;
            let id = RepoId(state.next_id);
            let name = repo_name(&path);
            state.recent.retain(|e| e.path != path);
            state.recent.insert(
                0,
                RepoEntry {
                    path: path.clone(),
                    name: name.clone(),
                },
            );
            state.tabs.push(RepoState {
                id: Some(id),
                path: path.clone(),
                name,
                ..Default::default()
            });
            state.active = Some(state.tabs.len() - 1);
            vec![Effect::StartRepo { id, path }, refresh(id)]
        }
        Action::ShowWelcome => {
            state.active = None;
            vec![]
        }
        Action::SelectTab(i) => {
            if i < state.tabs.len() {
                state.active = Some(i);
            }
            vec![]
        }
        Action::CloseTab(i) => {
            if i >= state.tabs.len() {
                return vec![];
            }
            let repo = state.tabs.remove(i);
            state.active = match state.active {
                _ if state.tabs.is_empty() => None,
                Some(a) if a > i => Some(a - 1),
                Some(a) if a == i => Some(a.min(state.tabs.len() - 1)),
                other => other,
            };
            vec![Effect::StopRepo(repo.id())]
        }
        Action::DismissNotification(i) => {
            if i < state.notifications.len() {
                state.notifications.remove(i);
            }
            vec![]
        }
        action => {
            let Some(repo) = state.active_repo_mut() else {
                return vec![];
            };
            let id = repo.id();
            let spawn = |job| vec![Effect::Spawn { repo: id, job }];
            match action {
                Action::Refresh => vec![refresh(id)],
                Action::SelectCommit(oid) => {
                    repo.selected_commit = oid;
                    vec![]
                }
                Action::Stage(paths) if !paths.is_empty() => spawn(Job::Stage(paths)),
                Action::Unstage(paths) if !paths.is_empty() => spawn(Job::Unstage(paths)),
                Action::StageAll => {
                    let paths: Vec<_> = repo
                        .status
                        .iter()
                        .flat_map(|s| s.unstaged())
                        .map(|c| c.path.clone())
                        .collect();
                    if paths.is_empty() {
                        vec![]
                    } else {
                        spawn(Job::Stage(paths))
                    }
                }
                Action::UnstageAll => {
                    let paths: Vec<_> = repo
                        .status
                        .iter()
                        .flat_map(|s| s.staged())
                        .map(|c| c.path.clone())
                        .collect();
                    if paths.is_empty() {
                        vec![]
                    } else {
                        spawn(Job::Unstage(paths))
                    }
                }
                Action::SetCommitMessage(m) => {
                    repo.commit_message = m;
                    vec![]
                }
                Action::SetAmend(a) => {
                    repo.amend = a;
                    vec![]
                }
                Action::Commit => {
                    let message = repo.commit_message.trim().to_owned();
                    if message.is_empty() {
                        return vec![];
                    }
                    spawn(Job::Commit {
                        message,
                        amend: repo.amend,
                    })
                }
                _ => vec![],
            }
        }
    }
}

/// Отмечает, что задача отправлена в очередь (для индикатора занятости).
pub fn job_started(state: &mut AppState, repo: RepoId) {
    if let Some(r) = state.repo_mut(repo) {
        r.busy += 1;
    }
}

/// Применяет результат фоновой задачи. Возвращает эффекты-продолжения
/// (например, обновить статус после stage).
pub fn apply_result(state: &mut AppState, repo: RepoId, result: JobResult) -> Vec<Effect> {
    let limit = if state.history_limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        state.history_limit
    };
    let Some(r) = state.repo_mut(repo) else {
        return vec![];
    }; // вкладку уже закрыли
    let mut error = None;
    let mut effects = vec![];
    match result {
        JobResult::Status(Ok(s)) => r.status = Some(s),
        JobResult::Refs(Ok(refs)) => r.refs = refs,
        JobResult::History(Ok(h)) => {
            r.history = h.commits;
            r.graph = h.graph;
        }
        JobResult::Status(Err(e)) | JobResult::Refs(Err(e)) | JobResult::History(Err(e)) => {
            error = Some(("Не удалось прочитать репозиторий".to_owned(), e))
        }
        JobResult::Finished { job, result } => {
            r.busy = r.busy.saturating_sub(1);
            match result {
                Ok(()) => {
                    if job == "commit" {
                        r.commit_message.clear();
                        r.amend = false;
                    }
                    if job != "refresh" {
                        effects.push(Effect::Spawn {
                            repo,
                            job: Job::Refresh { history_limit: limit },
                        });
                    }
                }
                Err(e) => error = Some((format!("Ошибка: git {job}"), e)),
            }
        }
    }
    r.version += 1;
    if let Some((title, e)) = error {
        state.notify_error(title, &e);
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(state: &mut AppState, p: &str) -> RepoId {
        let fx = reduce(state, Action::OpenRepo(PathBuf::from(p)));
        match &fx[0] {
            Effect::StartRepo { id, .. } => *id,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn open_repo_creates_tab_and_refreshes() {
        let mut s = AppState::new();
        let fx = reduce(&mut s, Action::OpenRepo("/work/demo".into()));
        assert_eq!(s.tabs.len(), 1);
        assert_eq!(s.active, Some(0));
        assert_eq!(s.tabs[0].name, "demo");
        assert_eq!(fx.len(), 2);
        assert!(matches!(
            fx[1],
            Effect::Spawn {
                job: Job::Refresh { .. },
                ..
            }
        ));
    }

    #[test]
    fn opening_same_repo_twice_switches_tab() {
        let mut s = AppState::new();
        open(&mut s, "/a");
        open(&mut s, "/b");
        let fx = reduce(&mut s, Action::OpenRepo("/a".into()));
        assert!(fx.is_empty());
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.active, Some(0));
        assert_eq!(s.recent[0].path, PathBuf::from("/b"));
    }

    #[test]
    fn close_tab_keeps_valid_active_index() {
        let mut s = AppState::new();
        open(&mut s, "/a");
        open(&mut s, "/b");
        let c = open(&mut s, "/c");
        assert_eq!(reduce(&mut s, Action::CloseTab(2)), vec![Effect::StopRepo(c)]);
        assert_eq!(s.active, Some(1));
        reduce(&mut s, Action::SelectTab(1));
        reduce(&mut s, Action::CloseTab(0));
        assert_eq!(s.active, Some(0));
        assert_eq!(s.tabs[0].name, "b");
        reduce(&mut s, Action::CloseTab(0));
        assert_eq!(s.active, None);
    }

    #[test]
    fn empty_commit_message_does_nothing() {
        let mut s = AppState::new();
        open(&mut s, "/a");
        reduce(&mut s, Action::SetCommitMessage("   ".into()));
        assert!(reduce(&mut s, Action::Commit).is_empty());
    }

    #[test]
    fn successful_commit_clears_message_and_refreshes() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        reduce(&mut s, Action::SetCommitMessage(" fix: typo \n".into()));
        let fx = reduce(&mut s, Action::Commit);
        assert_eq!(
            fx,
            vec![Effect::Spawn {
                repo: id,
                job: Job::Commit {
                    message: "fix: typo".into(),
                    amend: false
                }
            }]
        );
        job_started(&mut s, id);
        let fx = apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "commit",
                result: Ok(()),
            },
        );
        assert!(matches!(
            fx[0],
            Effect::Spawn {
                job: Job::Refresh { .. },
                ..
            }
        ));
        let r = s.active_repo().unwrap();
        assert!(r.commit_message.is_empty());
        assert_eq!(r.busy, 0);
    }

    #[test]
    fn failed_job_creates_notification_with_stderr() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let err = JobError {
            message: "git exited with 128".into(),
            stderr: "fatal: bad".into(),
        };
        apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "stage",
                result: Err(err),
            },
        );
        assert_eq!(s.notifications.len(), 1);
        assert!(s.notifications[0].details.contains("fatal: bad"));
    }

    #[test]
    fn results_for_closed_tab_are_ignored() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        reduce(&mut s, Action::CloseTab(0));
        assert!(apply_result(&mut s, id, JobResult::History(Ok(History::default()))).is_empty());
    }

    #[test]
    fn stage_all_collects_unstaged_paths() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let change = |p: &str, st, un| Change {
            path: p.into(),
            orig_path: None,
            staged: st,
            unstaged: un,
        };
        let status = RepoStatus {
            changes: vec![
                change("a.txt", None, Some(ChangeKind::Modified)),
                change("b.txt", Some(ChangeKind::Added), None),
                change("c.txt", None, Some(ChangeKind::Untracked)),
            ],
            ..Default::default()
        };
        apply_result(&mut s, id, JobResult::Status(Ok(status)));
        let fx = reduce(&mut s, Action::StageAll);
        assert_eq!(
            fx,
            vec![Effect::Spawn {
                repo: id,
                job: Job::Stage(vec!["a.txt".into(), "c.txt".into()])
            }]
        );
    }
}
