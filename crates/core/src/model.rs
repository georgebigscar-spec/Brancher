//! Модели данных репозитория. Заполняются парсерами из крейта `brancher-git`.

use std::path::PathBuf;

/// Полный SHA объекта (40 или 64 hex-символа).
pub type Oid = String;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Unix-время в секундах.
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commit {
    pub id: Oid,
    pub parents: Vec<Oid>,
    pub author: Signature,
    pub committer: Signature,
    pub subject: String,
}

impl Commit {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(10)]
    }
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRef {
    /// Короткое имя: `main`, `origin/main`, `v1.0`.
    pub name: String,
    /// Полное имя: `refs/heads/main`.
    pub full_name: String,
    pub kind: RefKind,
    pub target: Oid,
    pub is_head: bool,
    /// Полное имя upstream-ветки, если задана.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Upstream задан, но ветки на remote больше нет.
    pub upstream_gone: bool,
}

/// Вид изменения файла — одна буква из `git status --porcelain=v2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Untracked,
    Ignored,
}

impl ChangeKind {
    /// Буква XY-кода porcelain-вывода. `.` — без изменений.
    pub fn from_porcelain(c: char) -> Option<Self> {
        Some(match c {
            'M' => Self::Modified,
            'A' => Self::Added,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'T' => Self::TypeChanged,
            'U' => Self::Unmerged,
            '?' => Self::Untracked,
            '!' => Self::Ignored,
            _ => return None,
        })
    }

    pub fn letter(self) -> char {
        match self {
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChanged => 'T',
            Self::Unmerged => 'U',
            Self::Untracked => '?',
            Self::Ignored => '!',
        }
    }
}

/// Remote репозитория из `git remote -v`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Remote {
    pub name: String,
    pub url: String,
    /// Обычно совпадает с `url`; отличается, если задан `pushurl`.
    pub push_url: String,
}

/// Изменённый файл рабочей копии.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Путь относительно корня репозитория, с `/` как разделителем.
    pub path: String,
    /// Исходный путь для переименования или копирования.
    pub orig_path: Option<String>,
    /// Изменение в индексе (staged).
    pub staged: Option<ChangeKind>,
    /// Изменение в рабочем дереве (unstaged).
    pub unstaged: Option<ChangeKind>,
}

impl Change {
    pub fn is_conflicted(&self) -> bool {
        self.staged == Some(ChangeKind::Unmerged)
    }
}

/// Запись `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stash {
    /// Номер в `stash@{n}`: 0 — последний спрятанный.
    pub index: usize,
    /// Коммит stash: рабочее дерево на момент `git stash`.
    pub id: Oid,
    /// Коммит, поверх которого спрятаны изменения (первый родитель).
    pub base: Option<Oid>,
    /// Unix-время в секундах.
    pub time: i64,
    /// `On main: сообщение` или `WIP on main: abc1234 тема коммита`.
    pub message: String,
}

/// Коммит, которому принадлежат строки blame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlameCommit {
    pub id: Oid,
    pub author: String,
    /// Unix-время автора в секундах.
    pub time: i64,
    pub summary: String,
}

impl BlameCommit {
    /// Строки, которых ещё нет ни в одном коммите (blame рабочей копии).
    pub fn is_uncommitted(&self) -> bool {
        self.id.bytes().all(|b| b == b'0')
    }
}

/// Результат `git blame`: для каждой строки файла — номер коммита в `commits` и текст.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<(usize, String)>,
}

/// Незавершённая операция git, которая ждёт разрешения конфликтов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

/// Результат `git status`: текущая ветка и изменения.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoStatus {
    /// `None` — detached HEAD.
    pub branch: Option<String>,
    /// `None` — ещё нет ни одного коммита.
    pub head: Option<Oid>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
    /// Идёт merge, rebase, cherry-pick или revert.
    pub operation: Option<Operation>,
    /// Шаг rebase: `(текущий, всего)`.
    pub rebase_step: Option<(u32, u32)>,
}

impl RepoStatus {
    pub fn staged(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.staged.is_some() && !c.is_conflicted())
    }
    pub fn conflicts(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.is_conflicted())
    }
    pub fn unstaged(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.unstaged.is_some() || c.is_conflicted())
    }
}

/// Ошибка git, показанная пользователю (NFR-06: stderr виден всегда).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    pub message: String,
    pub stderr: String,
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.stderr.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.message, self.stderr.trim())
        }
    }
}

impl std::error::Error for JobError {}

/// Запись стартовой страницы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub name: String,
}
