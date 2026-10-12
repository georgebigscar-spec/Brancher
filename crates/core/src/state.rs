//! Состояние приложения и переходы: `Action` → `reduce` → `Effect`, `JobResult` → `apply_result`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::diff::{Diff, DiffRow, DiffTarget, LineKind};
use crate::graph::CommitGraph;
use crate::model::*;
use crate::patch::{self, LineRef};

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
    Diff(DiffTarget),
    /// `git apply`: частичный stage (`cached`), unstage (`cached` + `reverse`)
    /// или discard в рабочем дереве (только `reverse`).
    ApplyPatch {
        patch: String,
        cached: bool,
        reverse: bool,
    },
}

impl Job {
    /// Изменяет ли задача репозиторий (такие задачи идут строго по одной, раздел 3).
    pub fn is_write(&self) -> bool {
        !matches!(self, Job::Refresh { .. } | Job::Diff(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Job::Refresh { .. } => "refresh",
            Job::Stage(_) => "stage",
            Job::Unstage(_) => "unstage",
            Job::Commit { .. } => "commit",
            Job::Diff(_) => "diff",
            Job::ApplyPatch { cached: false, .. } => "discard-lines",
            Job::ApplyPatch { reverse: false, .. } => "stage-lines",
            Job::ApplyPatch { reverse: true, .. } => "unstage-lines",
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
    Diff {
        target: DiffTarget,
        result: Result<Diff, JobError>,
    },
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
    /// Выбрать коммит и показать его diff; `None` снимает выбор.
    SelectCommit(Option<Oid>),
    /// Показать diff файла рабочей копии (индексированную или нет сторону).
    SelectChange {
        path: String,
        staged: bool,
    },
    CloseDiff,
    /// Клик по изменённой строке diff: `extend` (Shift) — выделить диапазон от прошлого
    /// клика, `toggle` (Ctrl/Cmd) — добавить или убрать строку, иначе выбрать только её.
    SelectDiffLine {
        file: usize,
        hunk: usize,
        line: usize,
        extend: bool,
        toggle: bool,
    },
    /// Stage (или unstage для индекса) выбранных строк.
    ApplyDiffSelection,
    /// Stage (или unstage) ханка целиком.
    ApplyHunk {
        file: usize,
        hunk: usize,
    },
    /// Отменить выбранные строки в рабочем дереве (после подтверждения).
    DiscardDiffSelection,
    /// Отменить ханк в рабочем дереве (после подтверждения).
    DiscardHunk {
        file: usize,
        hunk: usize,
    },
    /// Ответ на диалог подтверждения: `true` — выполнить.
    Confirm(bool),
    Stage(Vec<String>),
    StageAll,
    Unstage(Vec<String>),
    UnstageAll,
    SetCommitMessage(String),
    SetAmend(bool),
    Commit,
    DismissNotification(usize),
    SetSideBySide(bool),
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

/// Вопрос перед необратимым действием (раздел 1: потеря данных — только с подтверждением).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    pub description: String,
    pub action: String,
    pub job: Job,
}

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
    /// Что показывает панель diff; `None` — панель закрыта.
    pub diff_target: Option<DiffTarget>,
    /// Загруженный diff для `diff_target`; `None`, пока идёт загрузка.
    pub diff: Option<Diff>,
    /// Выбранные строки diff: `(файл, ханк, строка)`. Сбрасывается при каждой загрузке diff.
    pub diff_selection: BTreeSet<(usize, usize, usize)>,
    /// Строка последнего клика — начало диапазона для Shift+клик.
    pub diff_anchor: Option<(usize, usize, usize)>,
    /// Необратимое действие, которое ждёт подтверждения в диалоге.
    pub confirm: Option<Confirm>,
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

    /// Открывает панель diff и возвращает задачу загрузки.
    fn show_diff(&mut self, target: DiffTarget) -> Vec<Effect> {
        if self.diff_target.as_ref() == Some(&target) {
            return vec![];
        }
        self.diff_target = Some(target.clone());
        self.diff = None;
        self.clear_selection();
        vec![Effect::Spawn {
            repo: self.id(),
            job: Job::Diff(target),
        }]
    }

    fn clear_selection(&mut self) {
        self.diff_selection.clear();
        self.diff_anchor = None;
    }

    /// Можно ли выбирать строки: только у diff файла рабочей копии, не у коммита.
    pub fn can_select_lines(&self) -> bool {
        matches!(self.diff_target, Some(DiffTarget::Change { .. })) && self.diff.is_some()
    }

    fn select_line(&mut self, at: (usize, usize, usize), extend: bool, toggle: bool) {
        let Some(diff) = self.diff.as_ref().filter(|_| self.can_select_lines()) else {
            return;
        };
        let is_change = |&(f, h, l): &(usize, usize, usize)| {
            let line = diff
                .files
                .get(f)
                .and_then(|x| x.hunks.get(h))
                .and_then(|x| x.lines.get(l));
            line.is_some_and(|l| matches!(l.kind, LineKind::Added | LineKind::Removed))
        };
        if !is_change(&at) {
            return;
        }
        match (extend, self.diff_anchor) {
            (true, Some(anchor)) => {
                let (a, b) = if anchor <= at { (anchor, at) } else { (at, anchor) };
                let range = diff.rows.iter().filter_map(|r| match *r {
                    DiffRow::Line(f, h, l) if (f, h, l) >= a && (f, h, l) <= b => Some((f, h, l)),
                    _ => None,
                });
                let range: Vec<_> = range.filter(is_change).collect();
                if !toggle {
                    self.diff_selection.clear();
                }
                self.diff_selection.extend(range);
                return;
            }
            _ if toggle => {
                if !self.diff_selection.remove(&at) {
                    self.diff_selection.insert(at);
                }
            }
            _ => {
                self.diff_selection.clear();
                self.diff_selection.insert(at);
            }
        }
        self.diff_anchor = Some(at);
    }

    /// Patch для выбранных строк; направление — по стороне diff (индекс → unstage).
    /// `discard` — откатить выбранное в рабочем дереве (только для неиндексированного diff).
    fn apply_lines(
        &self,
        selection: &BTreeSet<(usize, usize, usize)>,
        discard: bool,
    ) -> Result<Job, patch::PatchError> {
        let (Some(diff), Some(DiffTarget::Change { staged, .. })) = (&self.diff, &self.diff_target) else {
            return Err(patch::PatchError::NothingSelected);
        };
        if discard && *staged {
            return Err(patch::PatchError::NothingSelected);
        }
        // Discard откатывает изменения рабочего дерева так же, как unstage — индекса.
        let reverse = *staged || discard;
        let mut text = String::new();
        for (f, file) in diff.files.iter().enumerate() {
            let lines: BTreeSet<LineRef> = selection
                .iter()
                .filter(|(sf, ..)| *sf == f)
                .map(|&(_, h, l)| (h, l))
                .collect();
            if lines.is_empty() {
                continue;
            }
            text.push_str(&patch::build(file, &lines, reverse)?);
        }
        if text.is_empty() {
            return Err(patch::PatchError::NothingSelected);
        }
        Ok(Job::ApplyPatch {
            patch: text,
            cached: !discard,
            reverse,
        })
    }

    fn close_diff(&mut self) {
        self.diff_target = None;
        self.diff = None;
        self.clear_selection();
    }

    /// Цель diff для файла рабочей копии, если он всё ещё изменён на этой стороне.
    fn change_target(&self, path: &str, staged: bool) -> Option<DiffTarget> {
        let status = self.status.as_ref()?;
        let change = if staged {
            status.staged().find(|c| c.path == path)?
        } else {
            status.unstaged().find(|c| c.path == path)?
        };
        Some(DiffTarget::Change {
            path: path.to_owned(),
            staged,
            untracked: !staged && change.unstaged == Some(ChangeKind::Untracked),
        })
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
    pub settings: Settings,
    next_id: u64,
}

/// Настройки пользователя, общие для всех вкладок.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// Diff в две колонки вместо одной.
    pub side_by_side: bool,
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
        Action::SetSideBySide(on) => {
            state.settings.side_by_side = on;
            vec![]
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
                Action::SelectCommit(None) => {
                    repo.selected_commit = None;
                    if matches!(repo.diff_target, Some(DiffTarget::Commit { .. })) {
                        repo.close_diff();
                    }
                    vec![]
                }
                Action::SelectCommit(Some(oid)) => {
                    let parent = repo
                        .history
                        .iter()
                        .find(|c| c.id == oid)
                        .and_then(|c| c.parents.first().cloned());
                    repo.selected_commit = Some(oid.clone());
                    repo.show_diff(DiffTarget::Commit { id: oid, parent })
                }
                Action::SelectChange { path, staged } => match repo.change_target(&path, staged) {
                    Some(target) => {
                        repo.selected_commit = None;
                        repo.show_diff(target)
                    }
                    None => vec![],
                },
                Action::SelectDiffLine {
                    file,
                    hunk,
                    line,
                    extend,
                    toggle,
                } => {
                    repo.select_line((file, hunk, line), extend, toggle);
                    vec![]
                }
                Action::ApplyDiffSelection
                | Action::ApplyHunk { .. }
                | Action::DiscardDiffSelection
                | Action::DiscardHunk { .. } => {
                    let discard = matches!(action, Action::DiscardDiffSelection | Action::DiscardHunk { .. });
                    let selection = match action {
                        Action::ApplyHunk { file, hunk } | Action::DiscardHunk { file, hunk } => {
                            match repo.diff.as_ref().and_then(|d| d.files.get(file)) {
                                Some(f) if hunk < f.hunks.len() => patch::hunk_lines(f, hunk)
                                    .into_iter()
                                    .map(|(h, l)| (file, h, l))
                                    .collect(),
                                _ => BTreeSet::new(),
                            }
                        }
                        _ => repo.diff_selection.clone(),
                    };
                    match repo.apply_lines(&selection, discard) {
                        Ok(job) if discard => {
                            let path = match &repo.diff_target {
                                Some(DiffTarget::Change { path, .. }) => path.clone(),
                                _ => String::new(),
                            };
                            let n = selection.len();
                            repo.confirm = Some(Confirm {
                                title: "Отменить изменения?".into(),
                                description: format!(
                                    "Изменённых строк: {n} в {path}. Они будут удалены из рабочей копии, \
                                     вернуть их через Git не получится."
                                ),
                                action: "Отменить изменения".into(),
                                job,
                            });
                            vec![]
                        }
                        Ok(job) => {
                            repo.clear_selection();
                            spawn(job)
                        }
                        Err(e) => {
                            let title = if discard {
                                "Отмена строк недоступна"
                            } else {
                                "Частичный stage недоступен"
                            };
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: title.into(),
                                details: e.to_string(),
                            });
                            vec![]
                        }
                    }
                }
                Action::Confirm(yes) => match repo.confirm.take() {
                    Some(c) if yes => {
                        repo.clear_selection();
                        spawn(c.job)
                    }
                    _ => vec![],
                },
                Action::CloseDiff => {
                    repo.close_diff();
                    repo.selected_commit = None;
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
        JobResult::Diff { target, result } => {
            // Результат для уже закрытой или сменённой цели не нужен.
            if r.diff_target.as_ref() == Some(&target) {
                match result {
                    Ok(d) => {
                        // Номера строк в новом diff другие — выделение больше не валидно.
                        r.diff = Some(d);
                        r.clear_selection();
                    }
                    Err(e) => {
                        r.close_diff();
                        error = Some(("Не удалось получить diff".to_owned(), e));
                    }
                }
            }
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
                    if job == "refresh" {
                        effects.extend(refresh_change_diff(r));
                    } else if job != "diff" {
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

/// После обновления статуса diff файла рабочей копии мог устареть: перечитываем его,
/// а если файл на этой стороне больше не изменён (например, его проиндексировали) — закрываем.
fn refresh_change_diff(r: &mut RepoState) -> Vec<Effect> {
    let Some(DiffTarget::Change { path, staged, .. }) = r.diff_target.clone() else {
        return vec![];
    };
    match r.change_target(&path, staged) {
        Some(target) => {
            r.diff_target = Some(target.clone());
            vec![Effect::Spawn {
                repo: r.id(),
                job: Job::Diff(target),
            }]
        }
        None => {
            r.close_diff();
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, FileDiff, Hunk};

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

    fn change(p: &str, st: Option<ChangeKind>, un: Option<ChangeKind>) -> Change {
        Change {
            path: p.into(),
            orig_path: None,
            staged: st,
            unstaged: un,
        }
    }

    fn set_status(s: &mut AppState, id: RepoId, changes: Vec<Change>) {
        let status = RepoStatus {
            changes,
            ..Default::default()
        };
        apply_result(s, id, JobResult::Status(Ok(status)));
    }

    #[test]
    fn selecting_commit_loads_diff_against_first_parent() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let commits = vec![
            Commit {
                id: "m".into(),
                parents: vec!["p1".into(), "p2".into()],
                ..Default::default()
            },
            Commit {
                id: "p1".into(),
                ..Default::default()
            },
        ];
        apply_result(&mut s, id, JobResult::History(Ok(History::new(commits))));
        let target = DiffTarget::Commit {
            id: "m".into(),
            parent: Some("p1".into()),
        };
        assert_eq!(
            reduce(&mut s, Action::SelectCommit(Some("m".into()))),
            vec![Effect::Spawn {
                repo: id,
                job: Job::Diff(target.clone())
            }]
        );
        // Повторный выбор того же коммита не перезапускает загрузку.
        assert!(reduce(&mut s, Action::SelectCommit(Some("m".into()))).is_empty());

        // Результат для другой цели игнорируется, для текущей — показывается.
        let stale = DiffTarget::Commit {
            id: "x".into(),
            parent: None,
        };
        apply_result(
            &mut s,
            id,
            JobResult::Diff {
                target: stale,
                result: Ok(Diff::default()),
            },
        );
        assert_eq!(s.active_repo().unwrap().diff, None);
        apply_result(
            &mut s,
            id,
            JobResult::Diff {
                target,
                result: Ok(Diff::default()),
            },
        );
        assert_eq!(s.active_repo().unwrap().diff, Some(Diff::default()));
        let fx = apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "diff",
                result: Ok(()),
            },
        );
        assert!(fx.is_empty(), "diff не должен вызывать обновление репозитория");

        reduce(&mut s, Action::CloseDiff);
        let r = s.active_repo().unwrap();
        assert_eq!((r.diff_target.as_ref(), r.selected_commit.as_ref()), (None, None));
    }

    #[test]
    fn change_diff_marks_untracked_and_follows_refresh() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        set_status(
            &mut s,
            id,
            vec![change("new.txt", None, Some(ChangeKind::Untracked))],
        );
        let fx = reduce(
            &mut s,
            Action::SelectChange {
                path: "new.txt".into(),
                staged: false,
            },
        );
        assert_eq!(
            fx,
            vec![Effect::Spawn {
                repo: id,
                job: Job::Diff(DiffTarget::Change {
                    path: "new.txt".into(),
                    staged: false,
                    untracked: true
                })
            }]
        );
        let refresh_done = |s: &mut AppState| {
            apply_result(
                s,
                id,
                JobResult::Finished {
                    job: "refresh",
                    result: Ok(()),
                },
            )
        };
        // Файл всё ещё не проиндексирован — diff перечитывается.
        assert_eq!(refresh_done(&mut s).len(), 1);
        // Файл проиндексировали — неиндексированного diff больше нет, панель закрывается.
        set_status(&mut s, id, vec![change("new.txt", Some(ChangeKind::Added), None)]);
        assert!(refresh_done(&mut s).is_empty());
        assert_eq!(s.active_repo().unwrap().diff_target, None);
        // Неизвестный путь ничего не открывает.
        let fx = reduce(
            &mut s,
            Action::SelectChange {
                path: "nope".into(),
                staged: true,
            },
        );
        assert!(fx.is_empty());
    }

    /// Репозиторий с открытым diff файла `f.txt` (рабочая копия или индекс).
    fn with_change_diff(staged: bool) -> (AppState, RepoId) {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let kind = Some(ChangeKind::Modified);
        let (st, un) = if staged { (kind, None) } else { (None, kind) };
        set_status(&mut s, id, vec![change("f.txt", st, un)]);
        reduce(
            &mut s,
            Action::SelectChange {
                path: "f.txt".into(),
                staged,
            },
        );
        let line = |kind, text: &str| DiffLine {
            kind,
            old_no: None,
            new_no: None,
            text: text.into(),
            cr: false,
        };
        let file = FileDiff {
            old_path: Some("f.txt".into()),
            new_path: Some("f.txt".into()),
            hunks: vec![Hunk {
                header: "@@ -1,2 +1,3 @@".into(),
                old_start: 1,
                old_count: 2,
                new_start: 1,
                new_count: 3,
                lines: vec![
                    line(LineKind::Context, "a"),
                    line(LineKind::Removed, "b"),
                    line(LineKind::Added, "B"),
                    line(LineKind::Added, "c"),
                ],
            }],
            ..Default::default()
        };
        let target = s.active_repo().unwrap().diff_target.clone().unwrap();
        apply_result(
            &mut s,
            id,
            JobResult::Diff {
                target,
                result: Ok(Diff::new(vec![file])),
            },
        );
        (s, id)
    }

    fn click(s: &mut AppState, line: usize, extend: bool, toggle: bool) {
        reduce(
            s,
            Action::SelectDiffLine {
                file: 0,
                hunk: 0,
                line,
                extend,
                toggle,
            },
        );
    }

    #[test]
    fn line_selection_click_shift_and_toggle() {
        let (mut s, _) = with_change_diff(false);
        let selection = |s: &AppState| -> Vec<usize> {
            s.active_repo()
                .unwrap()
                .diff_selection
                .iter()
                .map(|&(_, _, l)| l)
                .collect()
        };
        click(&mut s, 0, false, false); // контекст не выбирается
        assert!(selection(&s).is_empty());
        click(&mut s, 1, false, false);
        click(&mut s, 3, true, false);
        assert_eq!(selection(&s), [1, 2, 3]);
        click(&mut s, 2, false, true);
        assert_eq!(selection(&s), [1, 3]);
        click(&mut s, 3, false, false);
        assert_eq!(selection(&s), [3]);
    }

    #[test]
    fn applying_selection_spawns_patch_in_right_direction() {
        let (mut s, id) = with_change_diff(false);
        click(&mut s, 3, false, false);
        let fx = reduce(&mut s, Action::ApplyDiffSelection);
        let Effect::Spawn {
            job:
                Job::ApplyPatch {
                    patch,
                    cached: true,
                    reverse,
                },
            ..
        } = &fx[0]
        else {
            panic!("{fx:?}")
        };
        assert!(!reverse);
        assert!(patch.ends_with("@@ -1,2 +1,3 @@\n a\n b\n+c\n"), "{patch}");
        assert!(s.active_repo().unwrap().diff_selection.is_empty());
        // После stage — обычное обновление, а затем перечитывание diff.
        let fx = apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "stage-lines",
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

        let (mut s, _) = with_change_diff(true);
        let fx = reduce(&mut s, Action::ApplyHunk { file: 0, hunk: 0 });
        assert!(matches!(
            &fx[0],
            Effect::Spawn {
                job: Job::ApplyPatch { reverse: true, .. },
                ..
            }
        ));
    }

    #[test]
    fn discard_asks_for_confirmation_first() {
        let (mut s, id) = with_change_diff(false);
        click(&mut s, 2, false, false);
        assert!(reduce(&mut s, Action::DiscardDiffSelection).is_empty());
        let confirm = s
            .active_repo()
            .unwrap()
            .confirm
            .clone()
            .expect("диалог подтверждения");
        assert!(confirm.description.contains("f.txt"));
        let Job::ApplyPatch {
            cached,
            reverse,
            ref patch,
        } = confirm.job
        else {
            panic!()
        };
        assert_eq!((cached, reverse), (false, true));
        // Невыбранная строка `c` есть в рабочем дереве — остаётся контекстом.
        assert!(patch.ends_with("@@ -1,2 +1,3 @@\n a\n+B\n c\n"), "{patch}");

        // Отказ ничего не запускает, согласие — запускает ровно этот patch.
        assert!(reduce(&mut s, Action::Confirm(false)).is_empty());
        assert!(s.active_repo().unwrap().confirm.is_none());
        reduce(&mut s, Action::DiscardHunk { file: 0, hunk: 0 });
        let job = s.active_repo().unwrap().confirm.clone().unwrap().job;
        assert_eq!(
            reduce(&mut s, Action::Confirm(true)),
            vec![Effect::Spawn { repo: id, job }]
        );

        // В diff индекса discard недоступен.
        let (mut s, _) = with_change_diff(true);
        reduce(&mut s, Action::DiscardHunk { file: 0, hunk: 0 });
        assert!(s.active_repo().unwrap().confirm.is_none());
    }

    #[test]
    fn empty_selection_notifies_instead_of_running_git() {
        let (mut s, _) = with_change_diff(false);
        assert!(reduce(&mut s, Action::ApplyDiffSelection).is_empty());
        assert_eq!(s.notifications.len(), 1);
        assert_eq!(s.notifications[0].severity, Severity::Info);
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
