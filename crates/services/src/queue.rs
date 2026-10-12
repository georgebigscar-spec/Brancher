use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use brancher_core::{Job, JobResult, RepoId};
use brancher_git::{CancelToken, Git};

use crate::execute;

/// Будит UI, когда готов результат (в приложении — `ctx.request_repaint()`).
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// Результат задачи, адресованный репозиторию.
pub type Completed = (RepoId, JobResult);

struct Worker {
    tx: Sender<Job>,
    cancel: CancelToken,
    handle: Option<JoinHandle<()>>,
}

/// Очереди задач всех открытых репозиториев.
pub struct Jobs {
    git: Git,
    workers: HashMap<RepoId, Worker>,
    results_tx: Sender<Completed>,
    results_rx: Receiver<Completed>,
    repaint: Repaint,
}

impl Jobs {
    pub fn new(git: Git, repaint: Repaint) -> Self {
        let (results_tx, results_rx) = mpsc::channel();
        Self {
            git,
            workers: HashMap::new(),
            results_tx,
            results_rx,
            repaint,
        }
    }

    pub fn start_repo(&mut self, id: RepoId, path: PathBuf) {
        if self.workers.contains_key(&id) {
            return;
        }
        let (tx, rx) = mpsc::channel::<Job>();
        let cancel = CancelToken::new();
        let (git, out, repaint, c) = (
            self.git.clone(),
            self.results_tx.clone(),
            self.repaint.clone(),
            cancel.clone(),
        );
        let handle = thread::Builder::new()
            .name(format!("repo-{}", id.0))
            .spawn(move || {
                for job in rx {
                    if c.is_cancelled() {
                        break;
                    }
                    // TODO: схлопывать подряд идущие Refresh и вынести чтения в общий пул (раздел 3).
                    execute(&git, &path, &job, &c, &mut |r| {
                        let _ = out.send((id, r));
                        repaint();
                    });
                }
            })
            .expect("не удалось создать поток");
        self.workers.insert(
            id,
            Worker {
                tx,
                cancel,
                handle: Some(handle),
            },
        );
    }

    /// Ставит задачу в очередь. `false` — репозиторий не открыт.
    pub fn spawn(&self, id: RepoId, job: Job) -> bool {
        self.workers.get(&id).is_some_and(|w| w.tx.send(job).is_ok())
    }

    /// Отменяет текущую задачу и останавливает worker (вкладка закрыта).
    pub fn stop_repo(&mut self, id: RepoId) {
        if let Some(w) = self.workers.remove(&id) {
            w.cancel.cancel();
            drop(w.tx);
            // Не ждём join: git будет убит, поток завершится сам.
            drop(w.handle);
        }
    }

    /// Забирает готовые результаты без блокировки (вызывается в начале кадра).
    pub fn drain(&self) -> Vec<Completed> {
        self.results_rx.try_iter().collect()
    }

    /// Ждёт результат — только для тестов и headless-режима.
    pub fn recv_timeout(&self, t: std::time::Duration) -> Option<Completed> {
        self.results_rx.recv_timeout(t).ok()
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        let ids: Vec<_> = self.workers.keys().copied().collect();
        for id in ids {
            self.stop_repo(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn refresh_on_real_repo_emits_results_and_repaints() {
        let dir = std::env::temp_dir().join(format!("brancher-services-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        std::fs::write(dir.join("a.txt"), "a").unwrap();

        let repaints = Arc::new(AtomicUsize::new(0));
        let r = repaints.clone();
        let mut jobs = Jobs::new(
            Git::detect().unwrap().0,
            Arc::new(move || {
                r.fetch_add(1, Ordering::Relaxed);
            }),
        );
        let id = RepoId(1);
        jobs.start_repo(id, dir.clone());
        assert!(jobs.spawn(id, Job::Refresh { history_limit: 100 }));

        let mut got = vec![];
        while let Some((rid, res)) = jobs.recv_timeout(Duration::from_secs(10)) {
            assert_eq!(rid, id);
            let done = matches!(res, JobResult::Finished { .. });
            got.push(res);
            if done {
                break;
            }
        }
        assert!(matches!(&got[0], JobResult::Status(Ok(s)) if s.changes.len() == 1));
        assert!(matches!(
            got.last(),
            Some(JobResult::Finished {
                job: "refresh",
                result: Ok(())
            })
        ));
        assert_eq!(repaints.load(Ordering::Relaxed), got.len());

        jobs.stop_repo(id);
        assert!(!jobs.spawn(id, Job::Refresh { history_limit: 1 }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
