//! Состояние приложения и переходы: `Action` → `reduce` → `Effect`, `JobResult` → `apply_result`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::diff::{Diff, DiffRow, DiffTarget, LineKind};
use crate::graph::CommitGraph;
use crate::model::*;
use crate::patch::{self, LineRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoId(pub u64);

impl RepoId {
    /// Адрес результатов, которые не относятся к вкладке (клонирование).
    pub const APP: RepoId = RepoId(0);
}

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
    /// Переключиться на локальную ветку.
    Switch(String),
    /// Создать локальную ветку, следящую за remote-веткой (`origin/x`), и переключиться.
    SwitchTrack(String),
    CreateBranch {
        name: String,
        /// Коммит или ссылка, от которой создаётся ветка.
        start: String,
        checkout: bool,
    },
    /// Удалить локальную ветку, даже не слитую (после подтверждения).
    DeleteBranch(String),
    /// Удалить несколько веток (слитые в основную).
    DeleteBranches(Vec<String>),
    Fetch,
    /// Fetch одного remote.
    FetchRemote(String),
    AddRemote {
        name: String,
        url: String,
    },
    /// Удалить remote вместе с его remote-ветками (после подтверждения).
    RemoveRemote(String),
    /// Переименовать remote и/или сменить URL; `None` — не менять.
    EditRemote {
        name: String,
        new_name: Option<String>,
        url: Option<String>,
    },
    /// Pull текущей ветки, только перемотка вперёд.
    Pull,
    /// Push текущей ветки; `set_upstream` — `(remote, ветка)`, если upstream ещё нет.
    Push {
        set_upstream: Option<(String, String)>,
    },
    /// Спрятать изменения рабочей копии; `untracked` — и неотслеживаемые файлы.
    StashPush {
        message: String,
        untracked: bool,
    },
    /// Применить `stash@{index}`, если там всё ещё коммит `id`; `pop` — затем удалить запись.
    StashApply {
        index: usize,
        id: Oid,
        pop: bool,
    },
    StashDrop {
        index: usize,
        id: Oid,
    },
    /// Blame файла в коммите или в рабочей копии (`rev: None`).
    Blame {
        path: String,
        rev: Option<Oid>,
    },
    /// История коммитов одного файла.
    FileLog(String),
    /// Коммиты, которые есть только в одной из двух ссылок.
    Compare {
        base: String,
        other: String,
    },
    /// Тег на `target`; непустое `message` — аннотированный тег.
    CreateTag {
        name: String,
        target: String,
        message: String,
    },
    DeleteTag(String),
    PushTag {
        remote: String,
        name: String,
    },
    /// `mainline` — коммит слияния: изменения относительно первого родителя.
    CherryPick {
        commit: Oid,
        mainline: bool,
    },
    Revert {
        commit: Oid,
        mainline: bool,
    },
    /// Слить ветку (ссылку) в текущую.
    Merge(String),
    /// Перебазировать текущую ветку на ссылку.
    Rebase(String),
    /// Пробное слияние с `target` перед merge или rebase: в каких файлах будут конфликты.
    PreviewConflicts {
        target: String,
        rebase: bool,
    },
    /// Interactive rebase по плану; `base` — `None` для rebase с корня истории.
    InteractiveRebase {
        base: Option<Oid>,
        steps: Vec<RebaseStep>,
    },
    /// Полное сообщение коммита (для reword).
    CommitMessage(Oid),
    /// Отменить коммит, оставив его изменения в индексе; `rebase` — `(база, шаги)` без него,
    /// если коммит не последний.
    Uncommit {
        commit: Oid,
        rebase: Option<(Option<Oid>, Vec<RebaseStep>)>,
    },
    /// Добавить проиндексированное в коммит: план — от него до HEAD.
    AmendInto {
        commit: Oid,
        base: Option<Oid>,
        steps: Vec<RebaseStep>,
    },
    /// Убрать файл из последнего коммита в рабочую копию.
    UncommitFile(String),
    /// Разложить проиндексированное по коммитам ветки, которые меняли эти строки.
    Absorb,
    /// Список снимков ленты отмены.
    Snapshots,
    /// Вернуть репозиторий к снимку.
    Restore {
        id: Oid,
        title: String,
    },
    Continue(Operation),
    Abort(Operation),
    /// Разрешить конфликт версией одной из сторон и отметить файл решённым.
    ResolveConflict {
        path: String,
        ours: bool,
    },
}

impl Job {
    /// Изменяет ли задача репозиторий (такие задачи идут строго по одной, раздел 3).
    pub fn is_write(&self) -> bool {
        !self.is_read()
    }

    /// Только читает репозиторий: после неё обновляться не нужно.
    pub fn is_read(&self) -> bool {
        matches!(
            self,
            Job::Refresh { .. }
                | Job::Diff(_)
                | Job::Blame { .. }
                | Job::FileLog(_)
                | Job::CommitMessage(_)
                | Job::Compare { .. }
                | Job::Snapshots
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Job::Refresh { .. } => "refresh",
            Job::Stage(_) => "stage",
            Job::Unstage(_) => "unstage",
            Job::Commit { .. } => "commit",
            Job::Diff(_) => "diff",
            Job::Blame { .. } => "blame",
            Job::FileLog(_) | Job::Compare { .. } => "log",
            Job::ApplyPatch { cached: false, .. } => "discard-lines",
            Job::ApplyPatch { reverse: false, .. } => "stage-lines",
            Job::ApplyPatch { reverse: true, .. } => "unstage-lines",
            Job::Switch(_) | Job::SwitchTrack(_) => "switch",
            Job::CreateBranch { .. } => "branch",
            Job::DeleteBranch(_) | Job::DeleteBranches(_) => "delete-branch",
            Job::Fetch | Job::FetchRemote(_) => "fetch",
            Job::AddRemote { .. } => "remote add",
            Job::RemoveRemote(_) => "remote remove",
            Job::EditRemote { .. } => "remote edit",
            Job::Pull => "pull",
            Job::Push { .. } => "push",
            Job::StashPush { .. } => "stash",
            Job::StashApply { pop: false, .. } => "stash apply",
            Job::StashApply { pop: true, .. } => "stash pop",
            Job::StashDrop { .. } => "stash drop",
            Job::CreateTag { .. } => "tag",
            Job::DeleteTag(_) => "delete-tag",
            Job::PushTag { .. } => "push tag",
            Job::CherryPick { .. } => "cherry-pick",
            Job::Revert { .. } => "revert",
            Job::Merge(_) => "merge",
            Job::PreviewConflicts { .. } => "preview",
            Job::Rebase(_) => "rebase",
            Job::InteractiveRebase { .. } => "rebase -i",
            Job::CommitMessage(_) => "message",
            Job::Snapshots => "oplog",
            Job::Uncommit { .. } => "uncommit",
            Job::AmendInto { .. } => "amend into",
            Job::UncommitFile(_) => "uncommit file",
            Job::Absorb => "absorb",
            Job::Restore { .. } => "restore",
            Job::Continue(op) => match op {
                Operation::Merge => "merge --continue",
                Operation::Rebase => "rebase --continue",
                Operation::CherryPick => "cherry-pick --continue",
                Operation::Revert => "revert --continue",
            },
            Job::Abort(op) => match op {
                Operation::Merge => "merge --abort",
                Operation::Rebase => "rebase --abort",
                Operation::CherryPick => "cherry-pick --abort",
                Operation::Revert => "revert --abort",
            },
            Job::ResolveConflict { .. } => "resolve",
        }
    }

    /// Название снимка ленты отмены, который делается перед задачей; `None` — без снимка
    /// (только индекс, сеть, настройки remotes, чтение).
    pub fn undo_title(&self) -> Option<String> {
        let short = |id: &str| id.chars().take(10).collect::<String>();
        Some(match self {
            Job::Commit { message, amend } => {
                let subject: String = message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(60)
                    .collect();
                if *amend {
                    format!("Amend «{subject}»")
                } else {
                    format!("Коммит «{subject}»")
                }
            }
            Job::ApplyPatch { cached: false, .. } => "Отмена строк в рабочей копии".into(),
            Job::Switch(b) | Job::SwitchTrack(b) => format!("Переключение на {b}"),
            Job::CreateBranch { name, .. } => format!("Ветка {name}"),
            Job::DeleteBranch(b) => format!("Удаление ветки {b}"),
            Job::DeleteBranches(b) => format!("Удаление слитых веток: {}", b.join(", ")),
            Job::Pull => "Pull".into(),
            Job::StashPush { .. } => "Stash".into(),
            Job::StashApply { pop: true, .. } => "Stash pop".into(),
            Job::StashApply { pop: false, .. } => "Stash apply".into(),
            Job::StashDrop { index, .. } => format!("Удаление stash@{{{index}}}"),
            Job::CreateTag { name, .. } => format!("Тег {name}"),
            Job::DeleteTag(name) => format!("Удаление тега {name}"),
            Job::CherryPick { commit, .. } => format!("Cherry-pick {}", short(commit)),
            Job::Revert { commit, .. } => format!("Revert {}", short(commit)),
            Job::Merge(b) => format!("Merge {b}"),
            Job::Rebase(b) => format!("Rebase на {b}"),
            Job::InteractiveRebase { .. } => "Interactive rebase".into(),
            Job::Uncommit { commit, .. } => format!("Отмена коммита {}", short(commit)),
            Job::AmendInto { commit, .. } => format!("Изменения в коммит {}", short(commit)),
            Job::UncommitFile(path) => format!("Файл {path} из коммита"),
            Job::Absorb => "Absorb".into(),
            Job::Continue(op) => format!("Продолжение {}", op.name()),
            Job::Abort(op) => format!("Прерывание {}", op.name()),
            Job::ResolveConflict { path, .. } => format!("Разрешение конфликта {path}"),
            _ => return None,
        })
    }

    /// Сетевая задача: показывает прогресс и её можно отменить.
    pub fn is_network(&self) -> bool {
        matches!(
            self,
            Job::Fetch | Job::FetchRemote(_) | Job::Pull | Job::Push { .. } | Job::PushTag { .. }
        )
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
    Stashes(Result<Vec<Stash>, JobError>),
    Remotes(Result<Vec<Remote>, JobError>),
    History(Result<History, JobError>),
    Diff {
        target: DiffTarget,
        result: Result<Diff, JobError>,
    },
    Blame {
        path: String,
        rev: Option<Oid>,
        result: Result<Blame, JobError>,
    },
    FileLog {
        path: String,
        result: Result<Vec<Commit>, JobError>,
    },
    CommitMessage {
        commit: Oid,
        result: Result<String, JobError>,
    },
    /// Коммиты только в `other` и только в `base`.
    Compare {
        base: String,
        other: String,
        result: Result<(Vec<Commit>, Vec<Commit>), JobError>,
    },
    Snapshots(Result<Vec<Snapshot>, JobError>),
    /// Файлы, где merge или rebase на `target` даст конфликты; ошибка — проверить не удалось
    /// (например, git старше 2.38).
    ConflictPreview {
        target: String,
        rebase: bool,
        result: Result<Vec<String>, JobError>,
    },
    /// Absorb выполнен (ошибка приходит в [`JobResult::Finished`]).
    Absorbed(AbsorbReport),
    /// Файлы репозитория изменились снаружи (редактор, терминал) — нужно обновиться.
    FilesChanged,
    /// Этап сетевой задачи репозитория (fetch, pull, push).
    Progress(Progress),
    /// git или ssh спрашивает логин, пароль или passphrase (адресуется [`RepoId::APP`]).
    Prompt {
        id: u64,
        text: String,
    },
    /// Этап клонирования `id` (адресуется [`RepoId::APP`]).
    CloneProgress {
        id: u64,
        progress: Progress,
    },
    /// Клонирование `id` закончено: путь нового репозитория или ошибка.
    Cloned {
        id: u64,
        result: Result<PathBuf, JobError>,
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
    SetDarkTheme(bool),
    /// Переключиться на локальную ветку по короткому имени.
    SwitchBranch(String),
    /// Переключиться на remote-ветку (`origin/x`): на локальную `x`, а если её нет — создать.
    CheckoutRemote(String),
    CreateBranch {
        name: String,
        start: String,
        checkout: bool,
    },
    /// Удалить локальную ветку (спрашивает подтверждение).
    DeleteBranch(String),
    /// Удалить все локальные ветки, слитые в основную (с подтверждением).
    DeleteMerged,
    Fetch,
    Pull,
    Push,
    FetchRemote(String),
    AddRemote {
        name: String,
        url: String,
    },
    /// Удалить remote (спрашивает подтверждение).
    RemoveRemote(String),
    /// Изменить имя и URL remote `name` (неизменённое не трогается).
    EditRemote {
        name: String,
        new_name: String,
        url: String,
    },
    /// Спрятать все изменения рабочей копии.
    Stash {
        message: String,
        untracked: bool,
    },
    /// Показать diff записи stash по номеру.
    SelectStash(usize),
    ApplyStash(usize),
    /// Применить и удалить запись.
    PopStash(usize),
    /// Удалить запись (спрашивает подтверждение).
    DropStash(usize),
    /// Показать blame файла: в коммите `rev` или в рабочей копии.
    ShowBlame {
        path: String,
        rev: Option<Oid>,
    },
    ShowFileHistory(String),
    /// Сравнить ссылку с текущей веткой: коммиты с каждой стороны.
    CompareWith(String),
    /// Diff сравнения: что принесёт ссылка (`HEAD...ссылка`).
    ShowCompareDiff,
    CloseFileView,
    /// Строка поиска по истории: текст заголовка, автор или начало SHA.
    SetHistoryQuery(String),
    /// Показывать только коммиты, достижимые из HEAD.
    SetHeadOnly(bool),
    CreateTag {
        name: String,
        target: String,
        message: String,
    },
    /// Удалить локальный тег (спрашивает подтверждение).
    DeleteTag(String),
    /// Отправить тег на remote по умолчанию.
    PushTag(String),
    CherryPick(Oid),
    Revert(Oid),
    /// Слить ветку, тег или remote-ветку в текущую.
    Merge(String),
    /// Перебазировать текущую ветку на ссылку (спрашивает подтверждение).
    Rebase(String),
    /// Открыть план interactive rebase текущей ветки, начиная с этого коммита.
    PlanRebase(Oid),
    SetRebaseAction {
        index: usize,
        action: RebaseAction,
    },
    /// Сдвинуть шаг плана на одну позицию вверх (раньше) или вниз.
    MoveRebaseStep {
        index: usize,
        up: bool,
    },
    SetRebaseMessage {
        index: usize,
        message: String,
    },
    CancelRebasePlan,
    RunRebasePlan,
    /// Изменить сообщение коммита текущей ветки: открыть диалог.
    EditMessage(Oid),
    SetEditedMessage(String),
    SaveMessage,
    CloseMessageEditor,
    /// Отменить коммит текущей ветки: изменения вернутся в индекс.
    Uncommit(Oid),
    /// Добавить проиндексированные изменения в коммит текущей ветки.
    AmendInto(Oid),
    /// Убрать файл из последнего коммита.
    UncommitFile(String),
    /// Разложить проиндексированное по коммитам ветки (absorb).
    Absorb,
    /// Открыть ленту отмены.
    ShowOplog,
    CloseOplog,
    /// Вернуться к состоянию перед операцией из ленты (спрашивает подтверждение).
    RestoreSnapshot(Oid),
    /// Продолжить merge или rebase после разрешения конфликтов.
    ContinueOperation,
    /// Прервать merge или rebase (спрашивает подтверждение).
    AbortOperation,
    ResolveConflict {
        path: String,
        ours: bool,
    },
    /// Отменить идущую сетевую задачу активной вкладки.
    CancelJob,
    /// Ответ на запрос git (первый в очереди); `None` — отмена.
    AnswerPrompt(Option<String>),
    /// Восстановить вкладки и настройки прошлого запуска.
    Restore(Session),
    /// Клонировать `url` в папку `dest` (её создаст git) и открыть результат.
    Clone {
        url: String,
        dest: PathBuf,
    },
    CancelClone,
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
    Clone {
        id: u64,
        url: String,
        dest: PathBuf,
    },
    CancelClone,
    CancelJob(RepoId),
    AnswerPrompt {
        id: u64,
        answer: Option<String>,
    },
}

/// Вопрос git или ssh к пользователю.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub id: u64,
    pub text: String,
    /// Пароль, токен или passphrase: поле ввода скрывает символы.
    pub secret: bool,
}

impl Prompt {
    pub fn new(id: u64, text: String) -> Self {
        let lower = text.to_lowercase();
        let secret = ["password", "passphrase", "token", "пароль"]
            .iter()
            .any(|w| lower.contains(w));
        Self { id, text, secret }
    }
}

/// Строка прогресса `git … --progress` (clone, fetch, push): этап и процент, если git его сообщил.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    pub phase: String,
    pub percent: Option<u8>,
}

/// Идущее клонирование.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cloning {
    /// Отличает результаты этого клонирования от отменённого прежде.
    pub id: u64,
    pub url: String,
    pub dest: PathBuf,
    pub progress: Progress,
}

/// Имя папки для клона, как его выбирает git: последний сегмент URL без `.git`.
/// `https://github.com/user/repo.git` → `repo`, `git@host:user/repo` → `repo`.
pub fn clone_dir_name(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let url = url.strip_suffix(".git").unwrap_or(url);
    let name = url.rsplit(['/', '\\', ':']).next()?;
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
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

/// Blame или история файла в нижней половине центральной панели (вместо diff).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileView {
    /// `blame` — `None`, пока идёт загрузка.
    Blame {
        path: String,
        rev: Option<Oid>,
        blame: Option<Blame>,
    },
    History {
        path: String,
        commits: Option<Vec<Commit>>,
    },
    /// Сравнение `other` с текущей веткой `base`; `None` — идёт загрузка.
    Compare {
        base: String,
        other: String,
        /// Коммиты только в `other` (их принесёт merge) и только в `base`.
        only_other: Option<Vec<Commit>>,
        only_base: Option<Vec<Commit>>,
    },
}

impl FileView {
    pub fn path(&self) -> &str {
        match self {
            Self::Blame { path, .. } | Self::History { path, .. } => path,
            Self::Compare { other, .. } => other,
        }
    }
}

/// Совпадает ли коммит с одним словом поиска (уже в нижнем регистре):
/// заголовок или автор содержат его, либо SHA с него начинается.
fn commit_matches(c: &Commit, token: &str) -> bool {
    let sha = token.len() >= 4 && token.bytes().all(|b| b.is_ascii_hexdigit()) && c.id.starts_with(token);
    sha || c.subject.to_lowercase().contains(token)
        || c.author.name.to_lowercase().contains(token)
        || c.author.email.to_lowercase().contains(token)
}

/// План interactive rebase, который редактируется в диалоге.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebasePlan {
    /// Родитель первого коммита; `None` — rebase с корня истории.
    pub base: Option<Oid>,
    pub branch: String,
    /// Шаги в порядке применения: сверху самый старый коммит.
    pub steps: Vec<RebaseStep>,
}

impl RebasePlan {
    /// Почему план нельзя выполнить, или `None`.
    pub fn error(&self) -> Option<&'static str> {
        let mut kept = self.steps.iter().filter(|s| s.action != RebaseAction::Drop);
        match kept.next() {
            None => return Some("Все коммиты удалены: оставьте хотя бы один"),
            Some(s) if s.action.melds() => {
                return Some("Первый оставшийся коммит не может быть squash или fixup: его не с чем слить")
            }
            _ => {}
        }
        let reword = self.steps.iter().filter(|s| s.action == RebaseAction::Reword);
        for s in reword {
            match &s.message {
                None => return Some("Сообщение для reword ещё загружается"),
                Some(m) if m.trim().is_empty() => return Some("Сообщение для reword пустое"),
                _ => {}
            }
        }
        None
    }
}

/// Сколько коммитов показывать в истории файла.
pub const FILE_HISTORY_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Default)]
pub struct RepoState {
    pub id: Option<RepoId>,
    pub path: PathBuf,
    pub name: String,
    pub status: Option<RepoStatus>,
    pub refs: Vec<GitRef>,
    pub stashes: Vec<Stash>,
    pub remotes: Vec<Remote>,
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
    /// Поиск по истории и фильтр «только текущая ветка».
    pub history_query: String,
    pub head_only: bool,
    /// Номера коммитов `history`, прошедших фильтр; `None` — фильтра нет, видна вся история.
    pub history_filter: Option<Vec<usize>>,
    /// Диалог нового сообщения коммита: `(коммит, сообщение)`; `None` — сообщение загружается.
    pub message_editor: Option<(Oid, Option<String>)>,
    /// Открытая лента отмены; внутренний `None` — список ещё загружается.
    pub oplog: Option<Option<Vec<Snapshot>>>,
    /// Открытый диалог interactive rebase.
    pub rebase_plan: Option<RebasePlan>,
    /// Blame или история файла; пока открыт, diff закрыт.
    pub file_view: Option<FileView>,
    /// Необратимое действие, которое ждёт подтверждения в диалоге.
    pub confirm: Option<Confirm>,
    pub commit_message: String,
    pub amend: bool,
    /// Сколько задач сейчас выполняется.
    pub busy: u32,
    /// Файлы менялись, пока шли задачи: обновиться, когда очередь опустеет.
    pub stale: bool,
    /// Прогресс fetch, pull или push; `Some`, пока такая задача в очереди или идёт.
    pub progress: Option<Progress>,
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
        self.file_view = None;
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

    /// Пересчитывает `history_filter` по строке поиска и флагу `head_only`.
    fn refilter(&mut self) {
        let tokens: Vec<String> = self
            .history_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if tokens.is_empty() && !self.head_only {
            self.history_filter = None;
            return;
        }
        let reachable = self.head_only.then(|| self.reachable_from_head());
        let filter = self
            .history
            .iter()
            .enumerate()
            .filter(|(i, _)| reachable.as_ref().is_none_or(|r| r[*i]))
            .filter(|(_, c)| tokens.iter().all(|t| commit_matches(c, t)))
            .map(|(i, _)| i)
            .collect();
        self.history_filter = Some(filter);
    }

    /// Какие коммиты загруженной истории достижимы из HEAD (по родителям).
    fn reachable_from_head(&self) -> Vec<bool> {
        let index: std::collections::HashMap<&str, usize> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();
        let mut seen = vec![false; self.history.len()];
        let head = self.status.as_ref().and_then(|s| s.head.as_deref());
        let mut stack: Vec<usize> = head.and_then(|h| index.get(h)).copied().into_iter().collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(
                self.history[i]
                    .parents
                    .iter()
                    .filter_map(|p| index.get(p.as_str())),
            );
        }
        seen
    }

    /// План interactive rebase от коммита `from` до HEAD по первым родителям.
    fn plan_rebase(&self, from: &str) -> Result<RebasePlan, String> {
        let status = self.status.as_ref().ok_or("Статус ещё не загружен")?;
        let branch = status
            .branch
            .clone()
            .ok_or("HEAD не на ветке (detached): interactive rebase недоступен")?;
        let by_id: std::collections::HashMap<&str, &Commit> =
            self.history.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut chain = vec![];
        let mut cur = status.head.as_deref();
        loop {
            let Some(c) = cur.and_then(|id| by_id.get(id)) else {
                return Err("Коммит не входит в текущую ветку (или старше загруженной истории)".into());
            };
            if c.is_merge() {
                return Err(format!(
                    "Среди коммитов есть слияние {}: interactive rebase через слияния не поддерживается",
                    c.short_id()
                ));
            }
            chain.push(*c);
            if c.id == from {
                break;
            }
            cur = c.parents.first().map(String::as_str);
        }
        let base = chain.last().and_then(|c| c.parents.first().cloned());
        let steps = chain
            .iter()
            .rev()
            .map(|c| RebaseStep {
                action: RebaseAction::Pick,
                commit: c.id.clone(),
                subject: c.subject.clone(),
                message: None,
            })
            .collect();
        Ok(RebasePlan { base, branch, steps })
    }

    /// Идущая операция git (merge, rebase…), если есть.
    pub fn operation(&self) -> Option<Operation> {
        self.status.as_ref().and_then(|s| s.operation)
    }

    /// Можно ли выбирать строки: только у diff файла рабочей копии, не у коммита и не у конфликта.
    pub fn can_select_lines(&self) -> bool {
        // У файла с конфликтом индекс не к чему применять: сначала выбирают версию.
        matches!(
            self.diff_target,
            Some(DiffTarget::Change {
                conflicted: false,
                ..
            })
        ) && self.diff.is_some()
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
            conflicted: change.is_conflicted(),
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
    /// Идущее клонирование (одно за раз).
    pub cloning: Option<Cloning>,
    /// Вопросы git к пользователю; показывается первый.
    pub prompts: std::collections::VecDeque<Prompt>,
    next_id: u64,
}

/// Настройки пользователя, общие для всех вкладок.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// Diff в две колонки вместо одной.
    pub side_by_side: bool,
    pub dark: bool,
}

/// Что сохраняется между запусками: вкладки, недавние и настройки.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    pub tabs: Vec<PathBuf>,
    pub active: Option<usize>,
    pub recent: Vec<PathBuf>,
    pub settings: Settings,
}

/// Почему имя не годится для ветки (упрощённые правила `git check-ref-format`), или `None`.
pub fn branch_name_error(name: &str) -> Option<&'static str> {
    const BAD: [char; 9] = [' ', '~', '^', ':', '?', '*', '[', '\\', '\x7f'];
    if name.is_empty() {
        Some("Введите имя ветки")
    } else if name.starts_with('-') {
        Some("Имя не может начинаться с «-»")
    } else if name == "@" || name == "HEAD" {
        Some("Это имя зарезервировано git")
    } else if name.chars().any(|c| c.is_control() || BAD.contains(&c)) {
        Some("Пробелы и символы ~ ^ : ? * [ \\ недопустимы")
    } else if name.contains("..") || name.contains("@{") || name.contains("//") {
        Some("Недопустимы «..», «@{» и «//»")
    } else if name.starts_with('/') || name.ends_with('/') || name.ends_with('.') || name.ends_with(".lock") {
        Some("Имя не может начинаться с «/» или заканчиваться на «/», «.», «.lock»")
    } else if name.split('/').any(|part| part.starts_with('.')) {
        Some("Части имени не могут начинаться с «.»")
    } else {
        None
    }
}

/// Имя тега: те же правила, что у веток.
pub fn tag_name_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Введите имя тега")
    } else {
        branch_name_error(name)
    }
}

/// Имя remote: те же правила, что у веток.
pub fn remote_name_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Введите имя remote")
    } else {
        branch_name_error(name)
    }
}

/// Remote по умолчанию: `origin`, если есть, иначе первый настроенный
/// (или известный по remote-веткам, пока список remotes не загружен).
fn default_remote(repo: &RepoState) -> Result<&str, &'static str> {
    let from_refs = repo
        .refs
        .iter()
        .filter(|r| r.kind == RefKind::RemoteBranch)
        .filter_map(|r| r.name.split_once('/').map(|(remote, _)| remote));
    let mut remotes: Box<dyn Iterator<Item = &str>> = if repo.remotes.is_empty() {
        Box::new(from_refs)
    } else {
        Box::new(repo.remotes.iter().map(|r| r.name.as_str()))
    };
    let first = remotes
        .next()
        .ok_or("У репозитория нет remotes: добавьте remote в списке слева.")?;
    Ok(if first == "origin" || remotes.any(|r| r == "origin") {
        "origin"
    } else {
        first
    })
}

/// Почему нельзя дать remote имя `name` и URL `url`; `editing` — имя изменяемого remote.
fn remote_edit_error(repo: &RepoState, editing: Option<&str>, name: &str, url: &str) -> Option<String> {
    if let Some(e) = remote_name_error(name) {
        return Some(e.to_owned());
    }
    if editing != Some(name) && repo.remotes.iter().any(|r| r.name == name) {
        return Some(format!("Remote {name} уже есть"));
    }
    if url.is_empty() {
        return Some("Введите URL".into());
    }
    None
}

/// Задача push для текущей ветки: без upstream — `-u` на `origin` (или единственный remote).
fn push_job(repo: &RepoState) -> Result<Job, &'static str> {
    let head = repo
        .refs
        .iter()
        .find(|r| r.kind == RefKind::LocalBranch && r.is_head)
        .ok_or("Нет текущей ветки (detached HEAD или репозиторий без коммитов).")?;
    if head.upstream.is_some() {
        return Ok(Job::Push { set_upstream: None });
    }
    let remote = default_remote(repo)?;
    Ok(Job::Push {
        set_upstream: Some((remote.to_owned(), head.name.clone())),
    })
}

/// Сколько недавних репозиториев помнить.
pub const RECENT_LIMIT: usize = 20;

impl AppState {
    pub fn new() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
            ..Default::default()
        }
    }

    /// Снимок для сохранения между запусками.
    pub fn session(&self) -> Session {
        Session {
            tabs: self.tabs.iter().map(|r| r.path.clone()).collect(),
            active: self.active,
            recent: self.recent.iter().map(|e| e.path.clone()).collect(),
            settings: self.settings.clone(),
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
            state.recent.truncate(RECENT_LIMIT - 1);
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
        Action::Clone { url, dest } => {
            let url = url.trim().to_owned();
            if state.cloning.is_some() || url.is_empty() {
                return vec![];
            }
            state.next_id += 1;
            let id = state.next_id;
            state.cloning = Some(Cloning {
                id,
                url: url.clone(),
                dest: dest.clone(),
                progress: Progress::default(),
            });
            vec![Effect::Clone { id, url, dest }]
        }
        Action::AnswerPrompt(answer) => match state.prompts.pop_front() {
            Some(p) => vec![Effect::AnswerPrompt { id: p.id, answer }],
            None => vec![],
        },
        Action::CancelClone => match state.cloning.take() {
            Some(_) => vec![Effect::CancelClone],
            None => vec![],
        },
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
        Action::SetDarkTheme(on) => {
            state.settings.dark = on;
            vec![]
        }
        Action::Restore(session) => {
            state.settings = session.settings;
            let mut effects = vec![];
            for path in session.tabs {
                effects.extend(reduce(state, Action::OpenRepo(path)));
            }
            // Открытие вкладок двигает их в начало недавних — возвращаем сохранённый порядок.
            state.recent = session
                .recent
                .into_iter()
                .map(|path| RepoEntry {
                    name: repo_name(&path),
                    path,
                })
                .take(RECENT_LIMIT)
                .collect();
            state.active = session.active.filter(|&i| i < state.tabs.len());
            effects
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
                Action::Fetch | Action::Pull | Action::Push if repo.progress.is_some() => vec![],
                Action::Fetch => {
                    repo.progress = Some(Progress::default());
                    spawn(Job::Fetch)
                }
                Action::Pull => {
                    repo.progress = Some(Progress::default());
                    spawn(Job::Pull)
                }
                Action::FetchRemote(_) if repo.progress.is_some() => vec![],
                Action::FetchRemote(name) => {
                    repo.progress = Some(Progress::default());
                    spawn(Job::FetchRemote(name))
                }
                Action::AddRemote { name, url } => {
                    let (name, url) = (name.trim().to_owned(), url.trim().to_owned());
                    match remote_edit_error(repo, None, &name, &url) {
                        Some(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Remote не добавлен".into(),
                                details,
                            });
                            vec![]
                        }
                        None => spawn(Job::AddRemote { name, url }),
                    }
                }
                Action::EditRemote { name, new_name, url } => {
                    let (new_name, url) = (new_name.trim().to_owned(), url.trim().to_owned());
                    let Some(old) = repo.remotes.iter().find(|r| r.name == name) else {
                        return vec![];
                    };
                    let url_changed = url != old.url;
                    match remote_edit_error(repo, Some(&name), &new_name, &url) {
                        Some(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Remote не изменён".into(),
                                details,
                            });
                            vec![]
                        }
                        None if new_name == name && !url_changed => vec![],
                        None => spawn(Job::EditRemote {
                            new_name: (new_name != name).then_some(new_name),
                            url: url_changed.then_some(url),
                            name,
                        }),
                    }
                }
                Action::RemoveRemote(name) => {
                    if repo.remotes.iter().any(|r| r.name == name) {
                        repo.confirm = Some(Confirm {
                            title: format!("Удалить remote {name}?"),
                            description: format!(
                                "Удалятся настройки remote и его ветки {name}/… в этом репозитории. \
                                 На сервере ничего не изменится; локальные ветки останутся."
                            ),
                            action: "Удалить".into(),
                            job: Job::RemoveRemote(name),
                        });
                    }
                    vec![]
                }
                Action::Push => match push_job(repo) {
                    Ok(job) => {
                        repo.progress = Some(Progress::default());
                        spawn(job)
                    }
                    Err(details) => {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Push невозможен".into(),
                            details: details.into(),
                        });
                        vec![]
                    }
                },
                Action::CancelJob if repo.progress.is_some() => vec![Effect::CancelJob(id)],
                Action::SwitchBranch(name) => spawn(Job::Switch(name)),
                Action::CheckoutRemote(remote) => {
                    // `origin/feature` → `feature`; если такая локальная ветка уже есть — на неё.
                    let local = remote.split_once('/').map_or(remote.as_str(), |(_, b)| b);
                    let exists = repo
                        .refs
                        .iter()
                        .any(|r| r.kind == RefKind::LocalBranch && r.name == local);
                    if exists {
                        spawn(Job::Switch(local.to_owned()))
                    } else {
                        spawn(Job::SwitchTrack(remote))
                    }
                }
                Action::CreateBranch {
                    name,
                    start,
                    checkout,
                } => {
                    let name = name.trim().to_owned();
                    let exists = repo
                        .refs
                        .iter()
                        .any(|r| r.kind == RefKind::LocalBranch && r.name == name);
                    let error = branch_name_error(&name)
                        .map(str::to_owned)
                        .or_else(|| exists.then(|| format!("Ветка {name} уже есть")));
                    match error {
                        Some(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Ветка не создана".into(),
                                details,
                            });
                            vec![]
                        }
                        None => spawn(Job::CreateBranch {
                            name,
                            start,
                            checkout,
                        }),
                    }
                }
                Action::DeleteBranch(name) => {
                    let branch = repo
                        .refs
                        .iter()
                        .find(|r| r.kind == RefKind::LocalBranch && r.name == name);
                    if let Some(b) = branch.filter(|b| !b.is_head) {
                        repo.confirm = Some(Confirm {
                            title: format!("Удалить ветку {name}?"),
                            description: "Если её коммитов нет в других ветках, вернуть ветку можно \
                                          через «История», кнопка «Отменить»."
                                .into(),
                            action: "Удалить".into(),
                            job: Job::DeleteBranch(b.name.clone()),
                        });
                    }
                    vec![]
                }
                Action::DeleteMerged => {
                    let names: Vec<String> = repo
                        .refs
                        .iter()
                        .filter(|r| r.kind == RefKind::LocalBranch && r.merged && !r.is_head)
                        .map(|r| r.name.clone())
                        .collect();
                    if names.is_empty() {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Нет слитых веток".into(),
                            details: "Все локальные ветки содержат коммиты, которых нет в основной.".into(),
                        });
                    } else {
                        let trunk = repo
                            .refs
                            .iter()
                            .find(|r| {
                                r.kind == RefKind::LocalBranch && (r.name == "main" || r.name == "master")
                            })
                            .map_or("main", |r| r.name.as_str());
                        repo.confirm = Some(Confirm {
                            title: format!("Удалить слитые ветки ({})?", names.len()),
                            description: format!(
                                "Все их коммиты уже есть в {trunk}:\n{}\n\nВернуть можно через «История», кнопка «Отменить».",
                                bullet_list(&names)
                            ),
                            action: "Удалить".into(),
                            job: Job::DeleteBranches(names),
                        });
                    }
                    vec![]
                }
                Action::Stash { message, untracked } => {
                    let changes = repo.status.as_ref().map_or(0, |s| s.changes.len());
                    if changes == 0 {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Нечего прятать".into(),
                            details: "В рабочей копии нет изменений.".into(),
                        });
                        vec![]
                    } else {
                        let message = message.trim().to_owned();
                        spawn(Job::StashPush { message, untracked })
                    }
                }
                Action::SelectStash(index) => match repo.stashes.get(index) {
                    Some(st) => {
                        let target = DiffTarget::Commit {
                            id: st.id.clone(),
                            parent: st.base.clone(),
                        };
                        repo.selected_commit = None;
                        repo.show_diff(target)
                    }
                    None => vec![],
                },
                Action::ApplyStash(index) | Action::PopStash(index) => match repo.stashes.get(index) {
                    Some(st) => spawn(Job::StashApply {
                        index,
                        id: st.id.clone(),
                        pop: matches!(action, Action::PopStash(_)),
                    }),
                    None => vec![],
                },
                Action::DropStash(index) => {
                    if let Some(st) = repo.stashes.get(index) {
                        repo.confirm = Some(Confirm {
                            title: format!("Удалить stash@{{{index}}}?"),
                            description: format!(
                                "«{}». Вернуть запись можно через «История», кнопка «Отменить».",
                                st.message
                            ),
                            action: "Удалить".into(),
                            job: Job::StashDrop {
                                index,
                                id: st.id.clone(),
                            },
                        });
                    }
                    vec![]
                }
                Action::ShowBlame { path, rev } => {
                    repo.close_diff();
                    repo.selected_commit = None;
                    repo.file_view = Some(FileView::Blame {
                        path: path.clone(),
                        rev: rev.clone(),
                        blame: None,
                    });
                    spawn(Job::Blame { path, rev })
                }
                Action::ShowFileHistory(path) => {
                    repo.close_diff();
                    repo.selected_commit = None;
                    repo.file_view = Some(FileView::History {
                        path: path.clone(),
                        commits: None,
                    });
                    spawn(Job::FileLog(path))
                }
                Action::CompareWith(other) => {
                    let base = repo
                        .status
                        .as_ref()
                        .and_then(|s| s.branch.clone())
                        .unwrap_or_else(|| "HEAD".into());
                    repo.close_diff();
                    repo.selected_commit = None;
                    repo.file_view = Some(FileView::Compare {
                        base: base.clone(),
                        other: other.clone(),
                        only_other: None,
                        only_base: None,
                    });
                    spawn(Job::Compare { base, other })
                }
                Action::ShowCompareDiff => match repo.file_view.take() {
                    Some(FileView::Compare { base, other, .. }) => repo.show_diff(DiffTarget::Range {
                        from: base,
                        to: other,
                    }),
                    other => {
                        repo.file_view = other;
                        vec![]
                    }
                },
                Action::CloseFileView => {
                    repo.file_view = None;
                    vec![]
                }
                Action::SetHistoryQuery(query) => {
                    repo.history_query = query;
                    repo.refilter();
                    vec![]
                }
                Action::SetHeadOnly(on) => {
                    repo.head_only = on;
                    repo.refilter();
                    vec![]
                }
                Action::CreateTag {
                    name,
                    target,
                    message,
                } => {
                    let name = name.trim().to_owned();
                    let exists = repo.refs.iter().any(|r| r.kind == RefKind::Tag && r.name == name);
                    let error = tag_name_error(&name)
                        .map(str::to_owned)
                        .or_else(|| exists.then(|| format!("Тег {name} уже есть")));
                    match error {
                        Some(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Тег не создан".into(),
                                details,
                            });
                            vec![]
                        }
                        None => spawn(Job::CreateTag {
                            name,
                            target,
                            message,
                        }),
                    }
                }
                Action::DeleteTag(name) => {
                    if repo.refs.iter().any(|r| r.kind == RefKind::Tag && r.name == name) {
                        repo.confirm = Some(Confirm {
                            title: format!("Удалить тег {name}?"),
                            description: "Тег удалится только локально; на remote он останется.".into(),
                            action: "Удалить".into(),
                            job: Job::DeleteTag(name),
                        });
                    }
                    vec![]
                }
                Action::PushTag(_) if repo.progress.is_some() => vec![],
                Action::PushTag(name) => match default_remote(repo).map(str::to_owned) {
                    Ok(remote) => {
                        repo.progress = Some(Progress::default());
                        spawn(Job::PushTag { remote, name })
                    }
                    Err(details) => {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Push тега невозможен".into(),
                            details: details.into(),
                        });
                        vec![]
                    }
                },
                Action::Merge(_)
                | Action::Rebase(_)
                | Action::CherryPick(_)
                | Action::Revert(_)
                | Action::PlanRebase(_)
                | Action::SaveMessage
                | Action::Uncommit(_)
                | Action::AmendInto(_)
                | Action::UncommitFile(_)
                | Action::Absorb
                    if repo.operation().is_some() =>
                {
                    let op = repo.operation().map_or("", Operation::name);
                    state.notifications.push(Notification {
                        severity: Severity::Info,
                        title: "Сначала завершите текущую операцию".into(),
                        details: format!("Идёт {op}: продолжите или прервите его."),
                    });
                    vec![]
                }
                Action::Merge(target) => spawn(Job::PreviewConflicts {
                    target,
                    rebase: false,
                }),
                Action::CherryPick(commit) => {
                    let mainline = repo.history.iter().any(|c| c.id == commit && c.is_merge());
                    spawn(Job::CherryPick { commit, mainline })
                }
                Action::Revert(commit) => {
                    let mainline = repo.history.iter().any(|c| c.id == commit && c.is_merge());
                    spawn(Job::Revert { commit, mainline })
                }
                Action::Rebase(onto) => {
                    if repo.status.as_ref().is_some_and(|s| s.branch.is_some()) {
                        spawn(Job::PreviewConflicts {
                            target: onto,
                            rebase: true,
                        })
                    } else {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Rebase недоступен".into(),
                            details: "HEAD не на ветке (detached).".into(),
                        });
                        vec![]
                    }
                }
                Action::PlanRebase(from) => {
                    match repo.plan_rebase(&from) {
                        Ok(plan) => repo.rebase_plan = Some(plan),
                        Err(details) => state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Interactive rebase недоступен".into(),
                            details,
                        }),
                    }
                    vec![]
                }
                Action::SetRebaseAction { index, action } => {
                    let Some(step) = repo.rebase_plan.as_mut().and_then(|p| p.steps.get_mut(index)) else {
                        return vec![];
                    };
                    step.action = action;
                    if action == RebaseAction::Reword && step.message.is_none() {
                        spawn(Job::CommitMessage(step.commit.clone()))
                    } else {
                        vec![]
                    }
                }
                Action::MoveRebaseStep { index, up } => {
                    if let Some(plan) = repo.rebase_plan.as_mut() {
                        let other = if up { index.checked_sub(1) } else { Some(index + 1) };
                        if let Some(j) = other.filter(|&j| j < plan.steps.len() && index < plan.steps.len()) {
                            plan.steps.swap(index, j);
                        }
                    }
                    vec![]
                }
                Action::SetRebaseMessage { index, message } => {
                    if let Some(step) = repo.rebase_plan.as_mut().and_then(|p| p.steps.get_mut(index)) {
                        step.message = Some(message);
                    }
                    vec![]
                }
                Action::EditMessage(id) => {
                    repo.message_editor = Some((id.clone(), None));
                    spawn(Job::CommitMessage(id))
                }
                Action::SetEditedMessage(m) => {
                    if let Some((_, msg)) = &mut repo.message_editor {
                        *msg = Some(m);
                    }
                    vec![]
                }
                Action::CloseMessageEditor => {
                    repo.message_editor = None;
                    vec![]
                }
                Action::SaveMessage => {
                    let Some((id, Some(message))) = repo.message_editor.clone() else {
                        return vec![];
                    };
                    if message.trim().is_empty() {
                        return vec![];
                    }
                    match repo.plan_rebase(&id) {
                        Ok(mut plan) => {
                            repo.message_editor = None;
                            plan.steps[0].action = RebaseAction::Reword;
                            plan.steps[0].message = Some(message);
                            spawn(Job::InteractiveRebase {
                                base: plan.base,
                                steps: plan.steps,
                            })
                        }
                        Err(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Сообщение не изменено".into(),
                                details,
                            });
                            vec![]
                        }
                    }
                }
                Action::Uncommit(id) => {
                    let head = repo.status.as_ref().and_then(|s| s.head.clone());
                    let merge = repo.history.iter().any(|c| c.id == id && c.is_merge());
                    let job = if merge {
                        Err("Это коммит слияния: отменить его так нельзя (используйте Revert)".to_owned())
                    } else if head.as_deref() == Some(id.as_str()) {
                        Ok(Job::Uncommit {
                            commit: id,
                            rebase: None,
                        })
                    } else {
                        repo.plan_rebase(&id).map(|mut plan| {
                            plan.steps[0].action = RebaseAction::Drop;
                            Job::Uncommit {
                                commit: id,
                                rebase: Some((plan.base, plan.steps)),
                            }
                        })
                    };
                    match job {
                        Ok(job) => spawn(job),
                        Err(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Коммит не отменён".into(),
                                details,
                            });
                            vec![]
                        }
                    }
                }
                Action::AmendInto(id) => {
                    let staged = repo.status.as_ref().map_or(0, |s| s.staged().count());
                    let job = if staged == 0 {
                        Err("Нет проиндексированных изменений: сначала сделайте stage нужных файлов или строк"
                            .to_owned())
                    } else if repo
                        .status
                        .as_ref()
                        .is_some_and(|s| s.conflicts().next().is_some())
                    {
                        Err("Сначала разрешите конфликты".to_owned())
                    } else {
                        repo.plan_rebase(&id).map(|plan| Job::AmendInto {
                            commit: id,
                            base: plan.base,
                            steps: plan.steps,
                        })
                    };
                    match job {
                        Ok(job) => spawn(job),
                        Err(details) => {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Изменения не добавлены".into(),
                                details,
                            });
                            vec![]
                        }
                    }
                }
                Action::UncommitFile(path) => spawn(Job::UncommitFile(path)),
                Action::Absorb => {
                    let status = repo.status.as_ref();
                    if status.is_some_and(|s| s.conflicts().next().is_some()) {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Absorb не выполнен".into(),
                            details: "Сначала разрешите конфликты".into(),
                        });
                        vec![]
                    } else if status.map_or(0, |s| s.staged().count()) == 0 {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Absorb не выполнен".into(),
                            details: "Нет проиндексированных изменений: сначала сделайте stage нужных файлов или строк"
                                .into(),
                        });
                        vec![]
                    } else {
                        spawn(Job::Absorb)
                    }
                }
                Action::ShowOplog => {
                    repo.oplog = Some(None);
                    spawn(Job::Snapshots)
                }
                Action::CloseOplog => {
                    repo.oplog = None;
                    vec![]
                }
                Action::RestoreSnapshot(id) => {
                    let snapshot = repo.oplog.iter().flatten().flatten().find(|s| s.id == id);
                    if let Some(s) = snapshot {
                        repo.confirm = Some(Confirm {
                            title: format!("Вернуться к состоянию перед «{}»?", s.title),
                            description: "Ветки, теги, stash, HEAD и незакоммиченные изменения станут такими, \
                                          какими были перед этой операцией. Неотслеживаемые файлы не трогаются. \
                                          Текущее состояние сохранится в ленте, откат можно будет отменить."
                                .into(),
                            action: "Вернуть".into(),
                            job: Job::Restore {
                                id: s.id.clone(),
                                title: s.title.clone(),
                            },
                        });
                    }
                    vec![]
                }
                Action::CancelRebasePlan => {
                    repo.rebase_plan = None;
                    vec![]
                }
                Action::RunRebasePlan => {
                    let Some(plan) = repo.rebase_plan.as_ref() else {
                        return vec![];
                    };
                    if let Some(e) = plan.error() {
                        state.notifications.push(Notification {
                            severity: Severity::Info,
                            title: "Interactive rebase не запущен".into(),
                            details: e.into(),
                        });
                        return vec![];
                    }
                    let plan = repo.rebase_plan.take().expect("план есть");
                    spawn(Job::InteractiveRebase {
                        base: plan.base,
                        steps: plan.steps,
                    })
                }
                Action::ContinueOperation => match repo.operation() {
                    Some(op) => {
                        let conflicts = repo.status.as_ref().map_or(0, |s| s.conflicts().count());
                        if conflicts > 0 {
                            state.notifications.push(Notification {
                                severity: Severity::Info,
                                title: "Остались конфликты".into(),
                                details: format!(
                                    "Неразрешённых файлов: {conflicts}. Выберите версию или \
                                     отметьте файл решённым (stage)."
                                ),
                            });
                            vec![]
                        } else {
                            spawn(Job::Continue(op))
                        }
                    }
                    None => vec![],
                },
                Action::AbortOperation => {
                    if let Some(op) = repo.operation() {
                        repo.confirm = Some(Confirm {
                            title: format!("Прервать {}?", op.name()),
                            description: "Репозиторий вернётся к состоянию до начала операции. \
                                          Уже разрешённые конфликты будут потеряны."
                                .into(),
                            action: "Прервать".into(),
                            job: Job::Abort(op),
                        });
                    }
                    vec![]
                }
                Action::ResolveConflict { path, ours } => spawn(Job::ResolveConflict { path, ours }),
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
    let result = match result {
        JobResult::CloneProgress { id, progress } => {
            if let Some(c) = state.cloning.as_mut().filter(|c| c.id == id) {
                c.progress = progress;
            }
            return vec![];
        }
        JobResult::Prompt { id, text } => {
            state.prompts.push_back(Prompt::new(id, text));
            return vec![];
        }
        JobResult::Absorbed(r) => {
            let mut details = format!("Фрагментов: {}, коммитов: {}.", r.hunks, r.commits);
            if r.left > 0 {
                details.push_str(&format!(
                    " Не удалось отнести к коммиту ветки: {} — они остались в изменениях.",
                    r.left
                ));
            }
            state.notifications.push(Notification {
                severity: Severity::Info,
                title: "Изменения разложены по коммитам".into(),
                details,
            });
            return vec![];
        }
        // Другой `id` или нет `cloning` — клонирование отменили, результат не нужен.
        JobResult::Cloned { id, result } => {
            if !state.cloning.as_ref().is_some_and(|c| c.id == id) {
                return vec![];
            }
            state.cloning = None;
            return match result {
                Ok(path) => reduce(state, Action::OpenRepo(path)),
                Err(e) => {
                    state.notify_error("Не удалось клонировать репозиторий", &e);
                    vec![]
                }
            };
        }
        other => other,
    };
    let Some(r) = state.repo_mut(repo) else {
        return vec![];
    }; // вкладку уже закрыли
    let mut error = None;
    let mut effects = vec![];
    match result {
        JobResult::Status(Ok(s)) => {
            let head_moved = r.status.as_ref().map(|o| &o.head) != Some(&s.head);
            r.status = Some(s);
            if head_moved && r.head_only {
                r.refilter();
            }
        }
        JobResult::Refs(Ok(refs)) => r.refs = refs,
        JobResult::Stashes(Ok(stashes)) => {
            // Удалённая или сдвинутая запись: её diff больше не тот.
            let shown =
                |st: &Stash| matches!(&r.diff_target, Some(DiffTarget::Commit { id, .. }) if *id == st.id);
            if r.stashes.iter().any(shown) && !stashes.iter().any(shown) {
                r.close_diff();
            }
            r.stashes = stashes;
        }
        JobResult::Remotes(Ok(remotes)) => r.remotes = remotes,
        JobResult::History(Ok(h)) => {
            r.history = h.commits;
            r.graph = h.graph;
            r.refilter();
        }
        JobResult::Diff { target, result } => {
            // Результат для уже закрытой или сменённой цели не нужен.
            if r.diff_target.as_ref() == Some(&target) {
                match result {
                    // Номера строк в новом diff другие — выделение больше не валидно.
                    // Тот же diff (обновление без изменений в файле) выделение сохраняет.
                    Ok(d) if r.diff.as_ref() != Some(&d) => {
                        r.diff = Some(d);
                        r.clear_selection();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        r.close_diff();
                        error = Some(("Не удалось получить diff".to_owned(), e));
                    }
                }
            }
        }
        JobResult::Blame { path, rev, result } => {
            let shown = matches!(
                &r.file_view,
                Some(FileView::Blame { path: p, rev: v, .. }) if *p == path && *v == rev
            );
            if shown {
                match result {
                    Ok(b) => {
                        if let Some(FileView::Blame { blame, .. }) = &mut r.file_view {
                            *blame = Some(b);
                        }
                    }
                    Err(e) => {
                        r.file_view = None;
                        error = Some((format!("Не удалось получить blame {path}"), e));
                    }
                }
            }
        }
        JobResult::FileLog { path, result } => {
            let shown = matches!(&r.file_view, Some(FileView::History { path: p, .. }) if *p == path);
            if shown {
                match result {
                    Ok(c) => {
                        if let Some(FileView::History { commits, .. }) = &mut r.file_view {
                            *commits = Some(c);
                        }
                    }
                    Err(e) => {
                        r.file_view = None;
                        error = Some((format!("Не удалось получить историю {path}"), e));
                    }
                }
            }
        }
        JobResult::Compare { base, other, result } => {
            let shown = matches!(
                &r.file_view,
                Some(FileView::Compare { base: b, other: o, .. }) if *b == base && *o == other
            );
            if shown {
                match result {
                    Ok((theirs, ours)) => {
                        if let Some(FileView::Compare {
                            only_other,
                            only_base,
                            ..
                        }) = &mut r.file_view
                        {
                            *only_other = Some(theirs);
                            *only_base = Some(ours);
                        }
                    }
                    Err(e) => {
                        r.file_view = None;
                        error = Some((format!("Не удалось сравнить {other} и {base}"), e));
                    }
                }
            }
        }
        JobResult::ConflictPreview {
            target,
            rebase,
            result,
        } => {
            // Ошибка пробного слияния (старый git) не мешает: операция идёт как раньше.
            let files = result.unwrap_or_default();
            let list = bullet_list(&files);
            if rebase {
                let branch = r
                    .status
                    .as_ref()
                    .and_then(|s| s.branch.clone())
                    .unwrap_or_default();
                let mut description = format!(
                    "Коммиты {branch}, которых нет в {target}, будут пересозданы поверх {target}. \
                     Если ветка уже отправлена, потом понадобится push с force."
                );
                if !files.is_empty() {
                    description.push_str(&format!("\n\nВероятны конфликты в файлах:\n{list}"));
                }
                r.confirm = Some(Confirm {
                    title: format!("Перебазировать {branch} на {target}?"),
                    description,
                    action: "Перебазировать".into(),
                    job: Job::Rebase(target),
                });
            } else if files.is_empty() {
                effects.push(Effect::Spawn {
                    repo,
                    job: Job::Merge(target),
                });
            } else {
                r.confirm = Some(Confirm {
                    title: format!("Слияние {target} даст конфликты"),
                    description: format!(
                        "Конфликты будут в файлах:\n{list}\n\nИх придётся разрешить вручную; \
                         слияние можно прервать или откатить через «История»."
                    ),
                    action: "Всё равно слить".into(),
                    job: Job::Merge(target),
                });
            }
        }
        JobResult::Snapshots(result) => match result {
            Ok(list) => {
                if r.oplog.is_some() {
                    r.oplog = Some(Some(list));
                }
            }
            Err(e) => {
                r.oplog = None;
                error = Some(("Не удалось прочитать ленту отмены".to_owned(), e));
            }
        },
        JobResult::CommitMessage { commit, result } => match result {
            Ok(m) => {
                let steps = r.rebase_plan.iter_mut().flat_map(|p| p.steps.iter_mut());
                for s in steps.filter(|s| s.commit == commit && s.message.is_none()) {
                    s.message = Some(m.clone());
                }
                if let Some((id, msg @ None)) = &mut r.message_editor {
                    if *id == commit {
                        *msg = Some(m);
                    }
                }
            }
            Err(e) => error = Some(("Не удалось прочитать сообщение коммита".to_owned(), e)),
        },
        JobResult::Status(Err(e))
        | JobResult::Refs(Err(e))
        | JobResult::Stashes(Err(e))
        | JobResult::Remotes(Err(e))
        | JobResult::History(Err(e)) => error = Some(("Не удалось прочитать репозиторий".to_owned(), e)),
        JobResult::CloneProgress { .. }
        | JobResult::Cloned { .. }
        | JobResult::Prompt { .. }
        | JobResult::Absorbed(_) => {
            unreachable!("обработано выше")
        }
        JobResult::Progress(p) => r.progress = Some(p),
        JobResult::FilesChanged => {
            // Пока идут задачи, обновление отложено: после них оно и так будет или его
            // запустит флаг `stale`. Так подряд идущие изменения не копят очередь Refresh.
            if r.busy == 0 {
                effects.push(Effect::Spawn {
                    repo,
                    job: Job::Refresh { history_limit: limit },
                });
            } else {
                r.stale = true;
            }
        }
        JobResult::Finished { job, result } => {
            r.busy = r.busy.saturating_sub(1);
            if matches!(job, "fetch" | "pull" | "push" | "push tag") {
                r.progress = None;
            }
            match result {
                Ok(()) => {
                    if job == "commit" {
                        r.commit_message.clear();
                        r.amend = false;
                    }
                    if job == "refresh" {
                        effects.extend(refresh_change_diff(r));
                    } else if !matches!(job, "diff" | "blame" | "log" | "message" | "oplog" | "preview") {
                        effects.push(Effect::Spawn {
                            repo,
                            job: Job::Refresh { history_limit: limit },
                        });
                        // Открытая лента показывает и снимок только что сделанной операции.
                        if r.oplog.is_some() {
                            effects.push(Effect::Spawn {
                                repo,
                                job: Job::Snapshots,
                            });
                        }
                    }
                }
                Err(e) => error = Some((format!("Ошибка: git {job}"), e)),
            }
            let refreshing = effects.iter().any(|e| {
                matches!(
                    e,
                    Effect::Spawn {
                        job: Job::Refresh { .. },
                        ..
                    }
                )
            });
            if r.stale && (refreshing || r.busy == 0) {
                r.stale = false;
                if !refreshing {
                    effects.push(Effect::Spawn {
                        repo,
                        job: Job::Refresh { history_limit: limit },
                    });
                }
            }
        }
    }
    r.version += 1;
    if let Some((title, e)) = error {
        state.notify_error(title, &e);
    }
    effects
}

/// Список для подтверждения: первые 10 пунктов и сколько ещё.
fn bullet_list(files: &[String]) -> String {
    const SHOWN: usize = 10;
    let mut list: Vec<String> = files.iter().take(SHOWN).map(|f| format!("• {f}")).collect();
    if files.len() > SHOWN {
        list.push(format!("и ещё {}", files.len() - SHOWN));
    }
    list.join("\n")
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
                    untracked: true,
                    conflicted: false,
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
            spans: vec![],
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

    fn is_refresh(fx: &[Effect]) -> bool {
        matches!(
            fx,
            [Effect::Spawn {
                job: Job::Refresh { .. },
                ..
            }]
        )
    }

    #[test]
    fn files_changed_refreshes_when_idle_and_waits_for_running_jobs() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        assert!(is_refresh(&apply_result(&mut s, id, JobResult::FilesChanged)));

        // Идёт refresh: второй не ставим, а запоминаем, что данные устарели.
        job_started(&mut s, id);
        assert!(apply_result(&mut s, id, JobResult::FilesChanged).is_empty());
        assert!(apply_result(&mut s, id, JobResult::FilesChanged).is_empty());
        let done = |s: &mut AppState, job| apply_result(s, id, JobResult::Finished { job, result: Ok(()) });
        assert!(is_refresh(&done(&mut s, "refresh")));
        assert!(!s.active_repo().unwrap().stale);

        // После stage refresh будет и так — лишний не добавляется.
        job_started(&mut s, id);
        apply_result(&mut s, id, JobResult::FilesChanged);
        assert!(is_refresh(&done(&mut s, "stage")));
        assert!(!s.active_repo().unwrap().stale);
    }

    #[test]
    fn same_diff_after_refresh_keeps_selection() {
        let (mut s, id) = with_change_diff(false);
        reduce(
            &mut s,
            Action::SelectDiffLine {
                file: 0,
                hunk: 0,
                line: 1,
                extend: false,
                toggle: false,
            },
        );
        let r = s.active_repo().unwrap();
        let (target, diff) = (r.diff_target.clone().unwrap(), r.diff.clone().unwrap());
        let reload = |s: &mut AppState, d: Diff| {
            apply_result(
                s,
                id,
                JobResult::Diff {
                    target: target.clone(),
                    result: Ok(d),
                },
            );
        };
        reload(&mut s, diff.clone());
        assert_eq!(s.active_repo().unwrap().diff_selection.len(), 1);
        let mut changed = diff;
        changed.files[0].hunks[0].lines[0].text = "a2".into();
        reload(&mut s, changed);
        assert!(s.active_repo().unwrap().diff_selection.is_empty());
    }

    #[test]
    fn clone_dir_name_like_git() {
        let n = |u| clone_dir_name(u);
        assert_eq!(n("https://github.com/user/repo.git"), Some("repo".into()));
        assert_eq!(n("https://github.com/user/repo/"), Some("repo".into()));
        assert_eq!(n("git@github.com:user/repo.git"), Some("repo".into()));
        assert_eq!(n("host:repo"), Some("repo".into()));
        assert_eq!(n(r"C:\src\proj"), Some("proj".into()));
        assert_eq!(n(""), None);
        assert_eq!(n("https://x/.."), None);
    }

    fn clone_id(fx: &[Effect]) -> u64 {
        match fx {
            [Effect::Clone { id, .. }] => *id,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn clone_reports_progress_and_opens_result() {
        let mut s = AppState::new();
        let fx = reduce(
            &mut s,
            Action::Clone {
                url: " https://h/r.git ".into(),
                dest: "/w/r".into(),
            },
        );
        let id = clone_id(&fx);
        assert_eq!(
            fx,
            [Effect::Clone {
                id,
                url: "https://h/r.git".into(),
                dest: "/w/r".into()
            }]
        );
        // Второе клонирование, пока идёт первое, не начинается.
        let again = Action::Clone {
            url: "u".into(),
            dest: "/w/u".into(),
        };
        assert!(reduce(&mut s, again).is_empty());

        let progress = Progress {
            phase: "Receiving objects".into(),
            percent: Some(40),
        };
        let fx = apply_result(
            &mut s,
            RepoId::APP,
            JobResult::CloneProgress {
                id,
                progress: progress.clone(),
            },
        );
        assert!(fx.is_empty());
        assert_eq!(s.cloning.as_ref().unwrap().progress, progress);

        let done = JobResult::Cloned {
            id,
            result: Ok("/w/r".into()),
        };
        let fx = apply_result(&mut s, RepoId::APP, done);
        assert!(matches!(fx[0], Effect::StartRepo { .. }));
        assert!(s.cloning.is_none());
        assert_eq!(s.active_repo().unwrap().path, PathBuf::from("/w/r"));
    }

    #[test]
    fn clone_error_notifies_but_cancelled_clone_is_silent() {
        let mut s = AppState::new();
        let clone = || Action::Clone {
            url: "https://h/r".into(),
            dest: "/w/r".into(),
        };
        let err = |id| JobResult::Cloned {
            id,
            result: Err(JobError {
                message: "git завершился с кодом 128".into(),
                stderr: "fatal: repository not found".into(),
            }),
        };
        let first = clone_id(&reduce(&mut s, clone()));
        apply_result(&mut s, RepoId::APP, err(first));
        assert_eq!(s.notifications.len(), 1);
        assert!(s.notifications[0].details.contains("not found"));

        let second = clone_id(&reduce(&mut s, clone()));
        assert_eq!(reduce(&mut s, Action::CancelClone), [Effect::CancelClone]);
        // Сразу новое клонирование: поздний ответ отменённого его не трогает.
        let third = clone_id(&reduce(&mut s, clone()));
        assert!(apply_result(&mut s, RepoId::APP, err(second)).is_empty());
        assert_eq!(s.cloning.as_ref().map(|c| c.id), Some(third));
        assert_eq!(s.notifications.len(), 1);
        assert!(s.tabs.is_empty());
    }

    #[test]
    fn session_round_trip_keeps_tabs_recent_order_and_settings() {
        let mut s = AppState::new();
        open(&mut s, "/a");
        open(&mut s, "/b");
        open(&mut s, "/old");
        reduce(&mut s, Action::CloseTab(2));
        reduce(&mut s, Action::SelectTab(0));
        reduce(&mut s, Action::SetSideBySide(true));
        reduce(&mut s, Action::SetDarkTheme(true));
        let session = s.session();
        assert_eq!(session.tabs, [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(session.active, Some(0));
        let recent: Vec<_> = ["/old", "/b", "/a"].map(PathBuf::from).into();
        assert_eq!(session.recent, recent);

        let mut fresh = AppState::new();
        let fx = reduce(&mut fresh, Action::Restore(session.clone()));
        // Каждой вкладке — очередь и refresh.
        assert_eq!(fx.len(), 4);
        assert_eq!(fresh.session(), session);
        assert_eq!(fresh.tabs[1].name, "b");
        assert!(fresh.settings.dark && fresh.settings.side_by_side);
    }

    #[test]
    fn restore_ignores_bad_active_index_and_recent_is_capped() {
        let mut s = AppState::new();
        let session = Session {
            tabs: vec!["/a".into()],
            active: Some(5),
            recent: (0..RECENT_LIMIT + 5).map(|i| format!("/r{i}").into()).collect(),
            settings: Settings::default(),
        };
        reduce(&mut s, Action::Restore(session));
        assert_eq!(s.active, None);
        assert_eq!(s.recent.len(), RECENT_LIMIT);
        open(&mut s, "/new");
        assert_eq!(s.recent.len(), RECENT_LIMIT);
        assert_eq!(s.recent[0].path, PathBuf::from("/new"));
    }

    fn branch(name: &str, kind: RefKind, head: bool) -> GitRef {
        GitRef {
            name: name.into(),
            full_name: String::new(),
            kind,
            target: "abc".into(),
            is_head: head,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            merged: false,
        }
    }

    fn with_branches() -> (AppState, RepoId) {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        apply_result(
            &mut s,
            id,
            JobResult::Refs(Ok(vec![
                branch("main", RefKind::LocalBranch, true),
                branch("dev", RefKind::LocalBranch, false),
                branch("origin/dev", RefKind::RemoteBranch, false),
                branch("origin/feature/x", RefKind::RemoteBranch, false),
            ])),
        );
        (s, id)
    }

    #[test]
    fn delete_merged_asks_with_list() {
        let (mut s, id) = with_branches();
        assert!(reduce(&mut s, Action::DeleteMerged).is_empty());
        assert!(s.active_repo().unwrap().confirm.is_none());
        assert_eq!(s.notifications.len(), 1);
        let mut refs = vec![
            branch("main", RefKind::LocalBranch, true),
            branch("dev", RefKind::LocalBranch, false),
            branch("old", RefKind::LocalBranch, false),
        ];
        refs[1].merged = true;
        refs[2].merged = true;
        apply_result(&mut s, id, JobResult::Refs(Ok(refs)));
        assert!(reduce(&mut s, Action::DeleteMerged).is_empty());
        let confirm = s.active_repo().unwrap().confirm.clone().unwrap();
        assert!(confirm.title.contains("(2)") && confirm.description.contains("уже есть в main"));
        assert_eq!(confirm.job, Job::DeleteBranches(vec!["dev".into(), "old".into()]));
        assert!(confirm.job.undo_title().is_some());
    }

    #[test]
    fn checkout_remote_reuses_existing_local_branch() {
        let (mut s, id) = with_branches();
        let job = |s: &mut AppState, a| match &reduce(s, a)[..] {
            [Effect::Spawn { repo, job }] if *repo == id => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            job(&mut s, Action::SwitchBranch("dev".into())),
            Job::Switch("dev".into())
        );
        assert_eq!(
            job(&mut s, Action::CheckoutRemote("origin/dev".into())),
            Job::Switch("dev".into())
        );
        assert_eq!(
            job(&mut s, Action::CheckoutRemote("origin/feature/x".into())),
            Job::SwitchTrack("origin/feature/x".into())
        );
    }

    #[test]
    fn create_branch_validates_name() {
        let (mut s, _) = with_branches();
        let create = |name: &str| Action::CreateBranch {
            name: name.into(),
            start: "abc".into(),
            checkout: true,
        };
        assert!(reduce(&mut s, create("bad name")).is_empty());
        assert!(reduce(&mut s, create("dev")).is_empty());
        assert_eq!(s.notifications.len(), 2);
        assert!(s.notifications[1].details.contains("уже есть"));
        let fx = reduce(&mut s, create(" feature/y "));
        assert!(matches!(
            &fx[..],
            [Effect::Spawn { job: Job::CreateBranch { name, .. }, .. }] if name == "feature/y"
        ));
    }

    #[test]
    fn branch_names() {
        for ok in ["main", "feature/x-1", "v1.2", "юникод"] {
            assert_eq!(branch_name_error(ok), None, "{ok}");
        }
        for bad in [
            "", "-x", "a b", "a..b", "a/", "/a", "a.lock", "x/.y", "a~1", "HEAD", "a@{1}",
        ] {
            assert!(branch_name_error(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn delete_branch_asks_first_and_never_deletes_current() {
        let (mut s, id) = with_branches();
        assert!(reduce(&mut s, Action::DeleteBranch("main".into())).is_empty());
        assert!(s.active_repo().unwrap().confirm.is_none());
        assert!(reduce(&mut s, Action::DeleteBranch("dev".into())).is_empty());
        assert_eq!(
            reduce(&mut s, Action::Confirm(true)),
            [Effect::Spawn {
                repo: id,
                job: Job::DeleteBranch("dev".into())
            }]
        );
    }

    #[test]
    fn network_jobs_track_progress_and_can_be_cancelled() {
        let (mut s, id) = with_branches();
        let fx = reduce(&mut s, Action::Fetch);
        assert_eq!(
            fx,
            [Effect::Spawn {
                repo: id,
                job: Job::Fetch
            }]
        );
        job_started(&mut s, id);
        // Пока идёт сетевая задача, вторая не ставится.
        assert!(reduce(&mut s, Action::Pull).is_empty());
        let step = Progress {
            phase: "Receiving objects".into(),
            percent: Some(30),
        };
        apply_result(&mut s, id, JobResult::Progress(step.clone()));
        assert_eq!(s.active_repo().unwrap().progress, Some(step));
        assert_eq!(reduce(&mut s, Action::CancelJob), [Effect::CancelJob(id)]);
        let fx = apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "fetch",
                result: Ok(()),
            },
        );
        assert!(is_refresh(&fx));
        assert_eq!(s.active_repo().unwrap().progress, None);
        assert!(reduce(&mut s, Action::CancelJob).is_empty());
    }

    #[test]
    fn push_sets_upstream_only_when_missing() {
        let (mut s, id) = with_branches();
        let push = |s: &mut AppState| reduce(s, Action::Push);
        assert_eq!(
            push(&mut s),
            [Effect::Spawn {
                repo: id,
                job: Job::Push {
                    set_upstream: Some(("origin".into(), "main".into()))
                }
            }]
        );
        s.active_repo_mut().unwrap().progress = None;
        let r = s.active_repo_mut().unwrap();
        r.refs[0].upstream = Some("refs/remotes/origin/main".into());
        assert_eq!(
            push(&mut s),
            [Effect::Spawn {
                repo: id,
                job: Job::Push { set_upstream: None }
            }]
        );
        // Без remote-веток push объясняет, что делать, а не запускает git.
        let r = s.active_repo_mut().unwrap();
        r.progress = None;
        r.refs.retain(|r| r.kind != RefKind::RemoteBranch);
        r.refs[0].upstream = None;
        assert!(push(&mut s).is_empty());
        assert_eq!(s.notifications.len(), 1);
    }

    #[test]
    fn prompts_queue_and_answers() {
        let mut s = AppState::new();
        for (id, text) in [
            (1, "Username for 'https://h': "),
            (2, "Password for 'https://u@h': "),
        ] {
            let fx = apply_result(
                &mut s,
                RepoId::APP,
                JobResult::Prompt {
                    id,
                    text: text.into(),
                },
            );
            assert!(fx.is_empty());
        }
        assert!(!s.prompts[0].secret);
        assert!(s.prompts[1].secret);
        assert_eq!(
            reduce(&mut s, Action::AnswerPrompt(Some("u".into()))),
            [Effect::AnswerPrompt {
                id: 1,
                answer: Some("u".into())
            }]
        );
        assert_eq!(
            reduce(&mut s, Action::AnswerPrompt(None)),
            [Effect::AnswerPrompt { id: 2, answer: None }]
        );
        assert!(reduce(&mut s, Action::AnswerPrompt(None)).is_empty());
    }

    fn stash(index: usize, id: &str) -> Stash {
        Stash {
            index,
            id: id.into(),
            base: Some("base".into()),
            time: 0,
            message: format!("On main: {id}"),
        }
    }

    #[test]
    fn stash_actions() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let spawned = |fx: Vec<Effect>| match &fx[..] {
            [Effect::Spawn { job, .. }] => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        // Без изменений прятать нечего.
        set_status(&mut s, id, vec![]);
        let untracked = true;
        let push = || Action::Stash {
            message: "  wip ".into(),
            untracked,
        };
        assert!(reduce(&mut s, push()).is_empty());
        assert_eq!(s.notifications.len(), 1);
        set_status(&mut s, id, vec![change("a", None, Some(ChangeKind::Modified))]);
        assert_eq!(
            spawned(reduce(&mut s, push())),
            Job::StashPush {
                message: "wip".into(),
                untracked
            }
        );

        apply_result(
            &mut s,
            id,
            JobResult::Stashes(Ok(vec![stash(0, "s0"), stash(1, "s1")])),
        );
        assert_eq!(
            spawned(reduce(&mut s, Action::PopStash(1))),
            Job::StashApply {
                index: 1,
                id: "s1".into(),
                pop: true
            }
        );
        assert!(reduce(&mut s, Action::ApplyStash(7)).is_empty());

        // Diff записи — относительно базового коммита.
        assert_eq!(
            spawned(reduce(&mut s, Action::SelectStash(1))),
            Job::Diff(DiffTarget::Commit {
                id: "s1".into(),
                parent: Some("base".into())
            })
        );

        // Удаление — только после подтверждения.
        assert!(reduce(&mut s, Action::DropStash(1)).is_empty());
        assert_eq!(
            spawned(reduce(&mut s, Action::Confirm(true))),
            Job::StashDrop {
                index: 1,
                id: "s1".into()
            }
        );
        // Запись пропала из списка — её diff закрывается.
        apply_result(&mut s, id, JobResult::Stashes(Ok(vec![stash(0, "s0")])));
        let repo = s.active_repo().unwrap();
        assert!(repo.diff_target.is_none());
        assert_eq!(repo.stashes.len(), 1);
    }

    #[test]
    fn merge_rebase_and_conflicts() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let spawned = |fx: Vec<Effect>| match &fx[..] {
            [Effect::Spawn { job, .. }] => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        let set = |s: &mut AppState, op: Option<Operation>, changes: Vec<Change>| {
            let status = RepoStatus {
                branch: Some("main".into()),
                changes,
                operation: op,
                ..Default::default()
            };
            apply_result(s, id, JobResult::Status(Ok(status)));
        };
        set(&mut s, None, vec![]);
        let preview = |s: &mut AppState, rebase, result| {
            apply_result(
                s,
                id,
                JobResult::ConflictPreview {
                    target: "dev".into(),
                    rebase,
                    result,
                },
            )
        };
        // Merge — сначала пробное слияние; без конфликтов (или на старом git) идёт сразу.
        assert_eq!(
            spawned(reduce(&mut s, Action::Merge("dev".into()))),
            Job::PreviewConflicts {
                target: "dev".into(),
                rebase: false
            }
        );
        assert_eq!(
            spawned(preview(&mut s, false, Ok(vec![]))),
            Job::Merge("dev".into())
        );
        let old_git = Err(JobError {
            message: "unknown option".into(),
            stderr: String::new(),
        });
        assert_eq!(spawned(preview(&mut s, false, old_git)), Job::Merge("dev".into()));
        // С конфликтами — подтверждение со списком файлов.
        assert!(preview(&mut s, false, Ok(vec!["a.txt".into()])).is_empty());
        let confirm = s.active_repo().unwrap().confirm.clone().unwrap();
        assert!(confirm.description.contains("• a.txt"));
        assert_eq!(
            spawned(reduce(&mut s, Action::Confirm(true))),
            Job::Merge("dev".into())
        );
        // Rebase — после пробного слияния и подтверждения.
        assert_eq!(
            spawned(reduce(&mut s, Action::Rebase("dev".into()))),
            Job::PreviewConflicts {
                target: "dev".into(),
                rebase: true
            }
        );
        let many = (0..12).map(|i| format!("f{i}")).collect();
        assert!(preview(&mut s, true, Ok(many)).is_empty());
        let confirm = s.active_repo().unwrap().confirm.clone().unwrap();
        assert!(confirm.description.contains("• f9\nи ещё 2"));
        assert!(s
            .active_repo()
            .unwrap()
            .confirm
            .as_ref()
            .unwrap()
            .title
            .contains("main на dev"));
        assert_eq!(
            spawned(reduce(&mut s, Action::Confirm(true))),
            Job::Rebase("dev".into())
        );

        // Во время merge новый merge не начинается, а продолжить нельзя, пока есть конфликты.
        let conflict = Change {
            path: "a".into(),
            orig_path: None,
            staged: Some(ChangeKind::Unmerged),
            unstaged: Some(ChangeKind::Unmerged),
        };
        set(&mut s, Some(Operation::Merge), vec![conflict]);
        let notes = s.notifications.len();
        assert!(reduce(&mut s, Action::Merge("x".into())).is_empty());
        assert!(reduce(&mut s, Action::ContinueOperation).is_empty());
        assert_eq!(s.notifications.len(), notes + 2);
        assert_eq!(
            spawned(reduce(
                &mut s,
                Action::ResolveConflict {
                    path: "a".into(),
                    ours: true
                }
            )),
            Job::ResolveConflict {
                path: "a".into(),
                ours: true
            }
        );
        set(&mut s, Some(Operation::Merge), vec![]);
        assert_eq!(
            spawned(reduce(&mut s, Action::ContinueOperation)),
            Job::Continue(Operation::Merge)
        );
        assert!(reduce(&mut s, Action::AbortOperation).is_empty());
        assert_eq!(
            spawned(reduce(&mut s, Action::Confirm(true))),
            Job::Abort(Operation::Merge)
        );
    }

    #[test]
    fn blame_and_file_history_views() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let spawned = |fx: Vec<Effect>| match &fx[..] {
            [Effect::Spawn { job, .. }] => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        let blame = |s: &mut AppState, rev: Option<&str>| {
            reduce(
                s,
                Action::ShowBlame {
                    path: "a.rs".into(),
                    rev: rev.map(Into::into),
                },
            )
        };
        assert_eq!(
            spawned(blame(&mut s, Some("c1"))),
            Job::Blame {
                path: "a.rs".into(),
                rev: Some("c1".into())
            }
        );
        // Ответ на прежний запрос (другой rev) не подменяет текущий.
        blame(&mut s, None);
        let result = |rev: Option<&str>| JobResult::Blame {
            path: "a.rs".into(),
            rev: rev.map(Into::into),
            result: Ok(Blame::default()),
        };
        apply_result(&mut s, id, result(Some("c1")));
        assert!(matches!(
            s.active_repo().unwrap().file_view,
            Some(FileView::Blame { blame: None, .. })
        ));
        apply_result(&mut s, id, result(None));
        assert!(matches!(
            s.active_repo().unwrap().file_view,
            Some(FileView::Blame { blame: Some(_), .. })
        ));
        // Чтение не вызывает обновления.
        let fx = apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "blame",
                result: Ok(()),
            },
        );
        assert!(fx.is_empty());

        assert_eq!(
            spawned(reduce(&mut s, Action::ShowFileHistory("a.rs".into()))),
            Job::FileLog("a.rs".into())
        );
        apply_result(
            &mut s,
            id,
            JobResult::FileLog {
                path: "a.rs".into(),
                result: Err(JobError {
                    message: "нет".into(),
                    stderr: String::new(),
                }),
            },
        );
        assert!(s.active_repo().unwrap().file_view.is_none());
        assert!(!s.notifications.is_empty());

        // Открытие diff закрывает blame.
        blame(&mut s, None);
        reduce(&mut s, Action::SelectCommit(Some("c1".into())));
        assert!(s.active_repo().unwrap().file_view.is_none());
    }

    #[test]
    fn history_search_and_head_only() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let commit = |id: &str, parents: &[&str], subject: &str, author: &str| Commit {
            id: id.into(),
            parents: parents.iter().map(|p| (*p).into()).collect(),
            author: Signature {
                name: author.into(),
                email: format!("{}@x.org", author.to_lowercase()),
                time: 0,
            },
            subject: subject.into(),
            ..Default::default()
        };
        // d — на другой ветке: main = c → b → a, dev = d → a.
        let history = vec![
            commit("dddd1111", &["aaaa1111"], "Фича поиска", "Bob"),
            commit("cccc1111", &["bbbb1111"], "Исправить поиск", "Анна"),
            commit("bbbb1111", &["aaaa1111"], "Docs", "Bob"),
            commit("aaaa1111", &[], "Init", "Анна"),
        ];
        apply_result(&mut s, id, JobResult::History(Ok(History::new(history))));
        let status = RepoStatus {
            head: Some("cccc1111".into()),
            ..Default::default()
        };
        apply_result(&mut s, id, JobResult::Status(Ok(status)));
        let filter = |s: &mut AppState, a| {
            reduce(s, a);
            s.active_repo().unwrap().history_filter.clone()
        };
        assert_eq!(
            filter(&mut s, Action::SetHistoryQuery("ПОИСК".into())),
            Some(vec![0, 1])
        );
        assert_eq!(
            filter(&mut s, Action::SetHistoryQuery("поиск анна".into())),
            Some(vec![1])
        );
        assert_eq!(
            filter(&mut s, Action::SetHistoryQuery("bbbb".into())),
            Some(vec![2])
        );
        assert_eq!(
            filter(&mut s, Action::SetHistoryQuery("bob@x".into())),
            Some(vec![0, 2])
        );
        assert_eq!(filter(&mut s, Action::SetHistoryQuery(" ".into())), None);
        assert_eq!(filter(&mut s, Action::SetHeadOnly(true)), Some(vec![1, 2, 3]));
        assert_eq!(
            filter(&mut s, Action::SetHistoryQuery("поиск".into())),
            Some(vec![1])
        );
        // Новая история — фильтр пересчитывается.
        apply_result(&mut s, id, JobResult::History(Ok(History::default())));
        assert_eq!(s.active_repo().unwrap().history_filter, Some(vec![]));
    }

    #[test]
    fn tags_cherry_pick_and_revert_actions() {
        let (mut s, _) = with_branches();
        let spawned = |fx: Vec<Effect>| match &fx[..] {
            [Effect::Spawn { job, .. }] => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        let create = |name: &str| Action::CreateTag {
            name: name.into(),
            target: "abc".into(),
            message: String::new(),
        };
        assert_eq!(
            spawned(reduce(&mut s, create(" v1 "))),
            Job::CreateTag {
                name: "v1".into(),
                target: "abc".into(),
                message: String::new()
            }
        );
        assert!(reduce(&mut s, create("bad name")).is_empty());
        assert!(reduce(&mut s, create("")).is_empty());
        assert_eq!(s.notifications.len(), 2);

        // Push тега — на origin, с прогрессом.
        assert_eq!(
            spawned(reduce(&mut s, Action::PushTag("v1".into()))),
            Job::PushTag {
                remote: "origin".into(),
                name: "v1".into()
            }
        );
        assert!(s.active_repo().unwrap().progress.is_some());

        let merge = Commit {
            id: "m".into(),
            parents: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        s.tabs[0].history = vec![merge];
        assert_eq!(
            spawned(reduce(&mut s, Action::CherryPick("m".into()))),
            Job::CherryPick {
                commit: "m".into(),
                mainline: true
            }
        );
        assert_eq!(
            spawned(reduce(&mut s, Action::Revert("x".into()))),
            Job::Revert {
                commit: "x".into(),
                mainline: false
            }
        );
    }

    #[test]
    fn remotes_add_edit_remove_and_default() {
        let (mut s, id) = with_branches();
        let remote = |name: &str, url: &str| Remote {
            name: name.into(),
            url: url.into(),
            push_url: url.into(),
        };
        apply_result(
            &mut s,
            id,
            JobResult::Remotes(Ok(vec![remote("fork", "f"), remote("origin", "o")])),
        );
        let spawned = |fx: Vec<Effect>| match &fx[..] {
            [Effect::Spawn { job, .. }] => job.clone(),
            other => panic!("unexpected {other:?}"),
        };
        let add = |name: &str, url: &str| Action::AddRemote {
            name: name.into(),
            url: url.into(),
        };
        assert_eq!(
            spawned(reduce(&mut s, add(" up ", " /srv/a.git "))),
            Job::AddRemote {
                name: "up".into(),
                url: "/srv/a.git".into()
            }
        );
        assert!(reduce(&mut s, add("origin", "x")).is_empty());
        assert!(reduce(&mut s, add("-v", "x")).is_empty());
        assert!(reduce(&mut s, add("up", " ")).is_empty());
        assert_eq!(s.notifications.len(), 3);

        let edit = |name: &str, new_name: &str, url: &str| Action::EditRemote {
            name: name.into(),
            new_name: new_name.into(),
            url: url.into(),
        };
        // Меняется только то, что изменили.
        assert!(reduce(&mut s, edit("fork", "fork", "f")).is_empty());
        assert_eq!(
            spawned(reduce(&mut s, edit("fork", "mine", "f"))),
            Job::EditRemote {
                name: "fork".into(),
                new_name: Some("mine".into()),
                url: None
            }
        );
        assert_eq!(
            spawned(reduce(&mut s, edit("fork", "fork", "g"))),
            Job::EditRemote {
                name: "fork".into(),
                new_name: None,
                url: Some("g".into())
            }
        );
        assert!(reduce(&mut s, edit("fork", "origin", "f")).is_empty());
        assert!(reduce(&mut s, edit("nope", "x", "f")).is_empty());

        assert!(reduce(&mut s, Action::RemoveRemote("fork".into())).is_empty());
        assert_eq!(
            spawned(reduce(&mut s, Action::Confirm(true))),
            Job::RemoveRemote("fork".into())
        );

        assert_eq!(
            spawned(reduce(&mut s, Action::FetchRemote("fork".into()))),
            Job::FetchRemote("fork".into())
        );
        assert!(reduce(&mut s, Action::FetchRemote("fork".into())).is_empty());
        apply_result(
            &mut s,
            id,
            JobResult::Finished {
                job: "fetch",
                result: Ok(()),
            },
        );
        assert_eq!(s.active_repo().unwrap().progress, None);

        // Remote без веток (ещё не было fetch) — всё равно цель push.
        let r = s.active_repo_mut().unwrap();
        r.refs.retain(|r| r.kind != RefKind::RemoteBranch);
        r.remotes = vec![remote("up", "u")];
        assert_eq!(
            spawned(reduce(&mut s, Action::Push)),
            Job::Push {
                set_upstream: Some(("up".into(), "main".into()))
            }
        );
    }

    #[test]
    fn interactive_rebase_plan() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let c = |id: &str, parents: &[&str]| Commit {
            id: id.into(),
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            subject: format!("s{id}"),
            ..Default::default()
        };
        // d → c → b → a (корень); m — слияние в другой ветке.
        let history = vec![
            c("d", &["c"]),
            c("m", &["b", "x"]),
            c("c", &["b"]),
            c("b", &["a"]),
            c("a", &[]),
        ];
        apply_result(&mut s, id, JobResult::History(Ok(History::new(history))));
        apply_result(
            &mut s,
            id,
            JobResult::Status(Ok(RepoStatus {
                branch: Some("main".into()),
                head: Some("d".into()),
                ..Default::default()
            })),
        );
        let plan = |s: &AppState| s.active_repo().unwrap().rebase_plan.clone();
        assert!(reduce(&mut s, Action::PlanRebase("c".into())).is_empty());
        let p = plan(&s).unwrap();
        assert_eq!(p.base.as_deref(), Some("b"));
        let ids: Vec<_> = p.steps.iter().map(|s| s.commit.as_str()).collect();
        assert_eq!(ids, ["c", "d"]);

        // Корень — rebase с `--root`; коммит вне ветки — отказ.
        reduce(&mut s, Action::PlanRebase("a".into()));
        assert_eq!(plan(&s).unwrap().base, None);
        assert_eq!(plan(&s).unwrap().steps.len(), 4);
        reduce(&mut s, Action::CancelRebasePlan);
        reduce(&mut s, Action::PlanRebase("m".into()));
        assert!(plan(&s).is_none());
        assert_eq!(s.notifications.len(), 1);

        reduce(&mut s, Action::PlanRebase("b".into()));
        // b, c, d: squash первым быть не может.
        let set = |s: &mut AppState, index, action| reduce(s, Action::SetRebaseAction { index, action });
        set(&mut s, 0, RebaseAction::Fixup);
        assert!(plan(&s).unwrap().error().is_some());
        assert!(reduce(&mut s, Action::RunRebasePlan).is_empty());
        assert!(plan(&s).is_some());
        reduce(&mut s, Action::MoveRebaseStep { index: 0, up: false });
        assert_eq!(plan(&s).unwrap().steps[0].commit, "c");
        assert_eq!(plan(&s).unwrap().error(), None);
        reduce(&mut s, Action::MoveRebaseStep { index: 0, up: true });
        reduce(&mut s, Action::MoveRebaseStep { index: 1, up: true });
        assert_eq!(plan(&s).unwrap().steps[0].commit, "b");

        // Reword подгружает полное сообщение, пока его нет — запускать нельзя.
        set(&mut s, 0, RebaseAction::Pick);
        let fx = set(&mut s, 2, RebaseAction::Reword);
        assert_eq!(
            fx,
            [Effect::Spawn {
                repo: id,
                job: Job::CommitMessage("d".into())
            }]
        );
        assert!(plan(&s).unwrap().error().is_some());
        apply_result(
            &mut s,
            id,
            JobResult::CommitMessage {
                commit: "d".into(),
                result: Ok("sd\n\nтело".into()),
            },
        );
        assert_eq!(plan(&s).unwrap().steps[2].message.as_deref(), Some("sd\n\nтело"));
        reduce(
            &mut s,
            Action::SetRebaseMessage {
                index: 2,
                message: "новое".into(),
            },
        );
        match &reduce(&mut s, Action::RunRebasePlan)[..] {
            [Effect::Spawn {
                job: Job::InteractiveRebase { base, steps },
                ..
            }] => {
                assert_eq!(base.as_deref(), Some("a"));
                assert_eq!(steps[2].message.as_deref(), Some("новое"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(plan(&s).is_none());

        // Незакоммиченные изменения не мешают: rebase спрячет их сам (`--autostash`).
        let r = s.active_repo_mut().unwrap();
        r.status.as_mut().unwrap().changes.push(Change {
            path: "f".into(),
            orig_path: None,
            staged: None,
            unstaged: Some(ChangeKind::Modified),
        });
        reduce(&mut s, Action::PlanRebase("c".into()));
        assert!(plan(&s).is_some());
    }

    #[test]
    fn compare_with_current_branch() {
        let (mut s, id) = with_branches();
        apply_result(
            &mut s,
            id,
            JobResult::Status(Ok(RepoStatus {
                branch: Some("main".into()),
                ..Default::default()
            })),
        );
        assert_eq!(
            reduce(&mut s, Action::CompareWith("dev".into())),
            [Effect::Spawn {
                repo: id,
                job: Job::Compare {
                    base: "main".into(),
                    other: "dev".into()
                }
            }]
        );
        let result = |other: &str| JobResult::Compare {
            base: "main".into(),
            other: other.into(),
            result: Ok((vec![Commit::default()], vec![])),
        };
        // Результат для другого сравнения не применяется.
        apply_result(&mut s, id, result("origin/dev"));
        let view = |s: &AppState| s.active_repo().unwrap().file_view.clone();
        assert!(matches!(
            view(&s),
            Some(FileView::Compare { only_other: None, .. })
        ));
        apply_result(&mut s, id, result("dev"));
        assert!(matches!(
            view(&s),
            Some(FileView::Compare { only_other: Some(o), only_base: Some(b), .. }) if o.len() == 1 && b.is_empty()
        ));
        let fx = reduce(&mut s, Action::ShowCompareDiff);
        let range = DiffTarget::Range {
            from: "main".into(),
            to: "dev".into(),
        };
        assert_eq!(
            fx,
            [Effect::Spawn {
                repo: id,
                job: Job::Diff(range.clone())
            }]
        );
        let r = s.active_repo().unwrap();
        assert_eq!(r.diff_target, Some(range));
        assert!(r.file_view.is_none());
    }

    #[test]
    fn undo_timeline() {
        let commit = Job::Commit {
            message: "Тема\n\nтело".into(),
            amend: false,
        };
        assert_eq!(commit.undo_title().as_deref(), Some("Коммит «Тема»"));
        assert_eq!(Job::Stage(vec![]).undo_title(), None);
        assert_eq!(Job::Fetch.undo_title(), None);
        assert!(Job::DeleteBranch("x".into()).undo_title().is_some());

        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        assert_eq!(
            reduce(&mut s, Action::ShowOplog),
            [Effect::Spawn {
                repo: id,
                job: Job::Snapshots
            }]
        );
        let snap = Snapshot {
            id: "s1".into(),
            time: 0,
            title: "Коммит «x»".into(),
        };
        apply_result(&mut s, id, JobResult::Snapshots(Ok(vec![snap])));
        assert!(reduce(&mut s, Action::RestoreSnapshot("nope".into())).is_empty());
        assert!(s.active_repo().unwrap().confirm.is_none());
        reduce(&mut s, Action::RestoreSnapshot("s1".into()));
        assert_eq!(
            reduce(&mut s, Action::Confirm(true)),
            [Effect::Spawn {
                repo: id,
                job: Job::Restore {
                    id: "s1".into(),
                    title: "Коммит «x»".into()
                }
            }]
        );
        // После изменяющей задачи открытая лента перечитывается, после чтения — нет.
        let finished =
            |s: &mut AppState, job| apply_result(s, id, JobResult::Finished { job, result: Ok(()) });
        let reloads = |fx: &[Effect]| {
            fx.iter().any(|e| {
                matches!(
                    e,
                    Effect::Spawn {
                        job: Job::Snapshots,
                        ..
                    }
                )
            })
        };
        assert!(reloads(&finished(&mut s, "restore")));
        assert!(!reloads(&finished(&mut s, "oplog")));
        reduce(&mut s, Action::CloseOplog);
        assert!(!reloads(&finished(&mut s, "commit")));
    }

    #[test]
    fn absorb_needs_staged_changes_and_reports_result() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let status = |staged: Option<ChangeKind>| RepoStatus {
            branch: Some("main".into()),
            head: Some("c".into()),
            changes: vec![Change {
                path: "f".into(),
                orig_path: None,
                staged,
                unstaged: Some(ChangeKind::Modified),
            }],
            ..Default::default()
        };
        apply_result(&mut s, id, JobResult::Status(Ok(status(None))));
        assert!(reduce(&mut s, Action::Absorb).is_empty());
        assert_eq!(s.notifications.len(), 1);
        apply_result(
            &mut s,
            id,
            JobResult::Status(Ok(status(Some(ChangeKind::Modified)))),
        );
        assert!(matches!(
            &reduce(&mut s, Action::Absorb)[..],
            [Effect::Spawn { job: Job::Absorb, .. }]
        ));
        apply_result(
            &mut s,
            id,
            JobResult::Absorbed(AbsorbReport {
                hunks: 3,
                commits: 2,
                left: 1,
            }),
        );
        let n = s.notifications.last().unwrap();
        assert!(n.details.contains("Фрагментов: 3, коммитов: 2") && n.details.contains(": 1"));
    }

    #[test]
    fn commit_editing_actions() {
        let mut s = AppState::new();
        let id = open(&mut s, "/a");
        let c = |id: &str, parents: &[&str]| Commit {
            id: id.into(),
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            subject: format!("s{id}"),
            ..Default::default()
        };
        let history = vec![c("c", &["b"]), c("b", &["a"]), c("a", &[])];
        apply_result(&mut s, id, JobResult::History(Ok(History::new(history))));
        apply_result(
            &mut s,
            id,
            JobResult::Status(Ok(RepoStatus {
                branch: Some("main".into()),
                head: Some("c".into()),
                ..Default::default()
            })),
        );
        let job = |s: &mut AppState, a| match &reduce(s, a)[..] {
            [Effect::Spawn { job, .. }] => Some(job.clone()),
            [] => None,
            other => panic!("unexpected {other:?}"),
        };
        // Последний коммит — простой reset, старый — rebase без него.
        assert_eq!(
            job(&mut s, Action::Uncommit("c".into())),
            Some(Job::Uncommit {
                commit: "c".into(),
                rebase: None
            })
        );
        match job(&mut s, Action::Uncommit("b".into())) {
            Some(Job::Uncommit {
                rebase: Some((base, steps)),
                ..
            }) => {
                assert_eq!(base.as_deref(), Some("a"));
                assert_eq!(steps[0].action, RebaseAction::Drop);
                assert_eq!(steps[1].action, RebaseAction::Pick);
            }
            other => panic!("unexpected {other:?}"),
        }

        // Без проиндексированного добавлять нечего.
        assert_eq!(job(&mut s, Action::AmendInto("b".into())), None);
        let r = s.active_repo_mut().unwrap();
        r.status.as_mut().unwrap().changes.push(Change {
            path: "f".into(),
            orig_path: None,
            staged: Some(ChangeKind::Modified),
            unstaged: None,
        });
        assert!(matches!(
            job(&mut s, Action::AmendInto("b".into())),
            Some(Job::AmendInto { base: Some(b), steps, .. }) if b == "a" && steps.len() == 2
        ));

        // Сообщение: сначала загружается, затем reword через rebase.
        assert_eq!(
            job(&mut s, Action::EditMessage("b".into())),
            Some(Job::CommitMessage("b".into()))
        );
        assert_eq!(job(&mut s, Action::SaveMessage), None);
        apply_result(
            &mut s,
            id,
            JobResult::CommitMessage {
                commit: "b".into(),
                result: Ok("sb".into()),
            },
        );
        reduce(&mut s, Action::SetEditedMessage("Новое".into()));
        match job(&mut s, Action::SaveMessage) {
            Some(Job::InteractiveRebase { steps, .. }) => {
                assert_eq!(steps[0].action, RebaseAction::Reword);
                assert_eq!(steps[0].message.as_deref(), Some("Новое"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.active_repo().unwrap().message_editor.is_none());
    }
}
