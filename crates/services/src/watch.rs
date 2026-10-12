//! Наблюдение за файлами репозитория (REPO-06): изменения рабочей копии и `.git`
//! превращаются в [`JobResult::FilesChanged`], а core решает, когда обновиться.
//!
//! * События собираются пачкой, пока не наступит тишина ([`QUIET`]), но не дольше [`MAX_DELAY`].
//! * Служебное содержимое `.git` (objects, logs, `*.lock`) не считается изменением.
//! * Файлы рабочей копии, которые git игнорирует (сборка, `node_modules`), отсеиваются через
//!   `git check-ignore`, чтобы идущая сборка не вызывала обновление за обновлением.
//! * Изменения `.git`, пока worker выполняет свою задачу (stage, commit…), — наши собственные:
//!   после задачи обновление и так будет.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use brancher_core::{JobResult, RepoId};
use brancher_git::{CancelToken, Git};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{Completed, Repaint};

/// Сколько ждать новых событий, прежде чем сообщить об изменении.
const QUIET: Duration = Duration::from_millis(300);
/// Дольше пачку не копим, даже если события идут непрерывно.
const MAX_DELAY: Duration = Duration::from_secs(2);
/// Сколько после своей задачи считать изменения `.git` своими.
const OWN_GRACE: Duration = Duration::from_millis(500);
/// Пачку больше этого не проверяем через `git check-ignore`, а просто обновляемся.
const MAX_CHECKED: usize = 10_000;

/// Выполняет ли worker задачу сейчас и когда закончил последнюю.
#[derive(Debug)]
pub(crate) struct Activity {
    running: AtomicBool,
    last_done: Mutex<Option<Instant>>,
}

impl Activity {
    pub(crate) fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            last_done: Mutex::new(None),
        }
    }

    pub(crate) fn begin(&self) {
        self.running.store(true, Ordering::Release);
    }

    pub(crate) fn end(&self) {
        *self.last_done.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        self.running.store(false, Ordering::Release);
    }

    /// Изменения `.git` сейчас вызваны нашей же задачей.
    fn own(&self) -> bool {
        self.running.load(Ordering::Acquire)
            || self
                .last_done
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|t| t.elapsed() < OWN_GRACE)
    }
}

/// Что означает изменённый путь.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    /// Служебные файлы git: объекты, журналы, блокировки.
    Noise,
    /// Ссылки, HEAD, индекс и прочее состояние репозитория.
    Meta,
    /// Файл рабочей копии (путь относительно корня); `None` — путь вне корня.
    Tree(Option<PathBuf>),
}

fn classify(roots: &[PathBuf], path: &Path) -> Kind {
    let Some(rel) = roots.iter().find_map(|r| path.strip_prefix(r).ok()) else {
        return Kind::Tree(None);
    };
    let mut parts = rel.components();
    if parts.next() != Some(Component::Normal(".git".as_ref())) {
        return Kind::Tree(Some(rel.to_path_buf()));
    }
    let lock = rel.extension().is_some_and(|e| e == "lock");
    match parts.next() {
        Some(Component::Normal(p)) if p == "objects" || p == "logs" => Kind::Noise,
        _ if lock => Kind::Noise,
        _ => Kind::Meta,
    }
}

/// Пока значение живо, репозиторий наблюдается.
pub(crate) struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

pub(crate) fn start(
    git: Git,
    root: PathBuf,
    id: RepoId,
    out: Sender<Completed>,
    repaint: Repaint,
    activity: Arc<Activity>,
) -> notify::Result<RepoWatcher> {
    // macOS сообщает канонические пути (/private/var/…), остальные — от пути наблюдения.
    let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let roots = vec![canonical.clone(), root.clone()];

    let (tx, rx) = mpsc::channel::<Kind>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            // Ошибка наблюдения (например, переполнение очереди) — безопаснее обновиться.
            let _ = tx.send(Kind::Tree(None));
            return;
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event.need_rescan() || event.paths.is_empty() {
            let _ = tx.send(Kind::Tree(None));
        }
        for path in &event.paths {
            match classify(&roots, path) {
                Kind::Noise => {}
                Kind::Meta if activity.own() => {}
                kind => {
                    let _ = tx.send(kind);
                }
            }
        }
    })?;
    watcher.watch(&canonical, RecursiveMode::Recursive)?;

    thread::Builder::new()
        .name(format!("watch-{}", id.0))
        .spawn(move || {
            // Канал закрывается, когда RepoWatcher (а с ним и обработчик) удалён.
            while let Ok(first) = rx.recv() {
                let mut batch = vec![first];
                let started = Instant::now();
                while started.elapsed() < MAX_DELAY {
                    match rx.recv_timeout(QUIET) {
                        Ok(kind) => batch.push(kind),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if relevant(&git, &root, batch) {
                    if out.send((id, JobResult::FilesChanged)).is_err() {
                        return;
                    }
                    repaint();
                }
            }
        })
        .map_err(notify::Error::io)?;

    Ok(RepoWatcher { _watcher: watcher })
}

/// Есть ли в пачке что-то, кроме игнорируемых git файлов.
fn relevant(git: &Git, root: &Path, batch: Vec<Kind>) -> bool {
    if cfg!(test) {
        eprintln!("watch batch: {batch:?}");
    }
    let mut paths = Vec::new();
    for kind in batch {
        match kind {
            Kind::Noise => {}
            Kind::Meta | Kind::Tree(None) => return true,
            // git ждёт пути через `/`; на Windows notify отдаёт их через `\\`.
            Kind::Tree(Some(p)) => match p.to_str() {
                Some(s) if cfg!(windows) => paths.push(s.replace('\\', "/")),
                Some(s) => paths.push(s.to_owned()),
                None => return true,
            },
        }
    }
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() {
        return false;
    }
    if paths.len() > MAX_CHECKED {
        return true;
    }
    let mut input = paths.join("\0");
    input.push('\0');
    // Код 1 — ни один путь не игнорируется.
    let ignored = git.runner.run_accepting(
        Some(root),
        &["check-ignore", "--stdin", "-z"],
        Some(input.as_bytes()),
        &CancelToken::new(),
        &[0, 1],
    );
    if cfg!(test) {
        eprintln!("check-ignore {paths:?}: {ignored:?}");
    }
    match ignored {
        Ok(out) => out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).count() < paths.len(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_paths() {
        let roots = [PathBuf::from("/r")];
        let k = |p: &str| classify(&roots, Path::new(p));
        assert_eq!(k("/r/src/a.rs"), Kind::Tree(Some("src/a.rs".into())));
        assert_eq!(k("/elsewhere/a.rs"), Kind::Tree(None));
        assert_eq!(k("/r/.git/index"), Kind::Meta);
        assert_eq!(k("/r/.git/refs/heads/main"), Kind::Meta);
        assert_eq!(k("/r/.git"), Kind::Meta);
        assert_eq!(k("/r/.git/index.lock"), Kind::Noise);
        assert_eq!(k("/r/.git/refs/heads/main.lock"), Kind::Noise);
        assert_eq!(k("/r/.git/objects/ab/cdef"), Kind::Noise);
        assert_eq!(k("/r/.git/logs/HEAD"), Kind::Noise);
        // Каталог с похожим именем — обычный файл рабочей копии.
        assert_eq!(k("/r/.github/ci.yml"), Kind::Tree(Some(".github/ci.yml".into())));
    }

    #[test]
    fn own_activity_window() {
        let a = Activity::new();
        assert!(!a.own());
        a.begin();
        assert!(a.own());
        a.end();
        assert!(a.own());
        *a.last_done.lock().unwrap() = Some(Instant::now() - OWN_GRACE * 2);
        assert!(!a.own());
    }
}
