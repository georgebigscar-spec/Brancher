use std::path::Path;

use brancher_core::{Diff, History, Job, JobError, JobResult};
use brancher_git::{CancelToken, Git};

/// Выполняет задачу и отдаёт результаты по мере готовности через `emit`.
/// Последним всегда идёт [`JobResult::Finished`].
pub fn execute(git: &Git, repo: &Path, job: &Job, cancel: &CancelToken, emit: &mut dyn FnMut(JobResult)) {
    let result: Result<(), JobError> = (|| {
        match job {
            Job::Refresh { history_limit } => {
                let status = git.status(repo, cancel).map_err(JobError::from);
                let failed = status.as_ref().err().cloned();
                emit(JobResult::Status(status));
                if let Some(e) = failed {
                    return Err(e);
                }
                emit(JobResult::Refs(git.refs(repo, cancel).map_err(Into::into)));
                // Раскладка графа считается здесь, в worker-потоке, а не в кадре UI.
                let history = git.log(repo, *history_limit, cancel).map(History::new);
                emit(JobResult::History(history.map_err(Into::into)));
            }
            Job::Stage(paths) => git.stage(repo, paths, cancel)?,
            Job::Unstage(paths) => {
                let has_head = git.status(repo, cancel)?.head.is_some();
                git.unstage(repo, paths, has_head, cancel)?
            }
            Job::Commit { message, amend } => git.commit(repo, message, *amend, cancel)?,
            Job::ApplyPatch { patch, reverse } => git.apply_cached(repo, patch, *reverse, cancel)?,
            Job::Diff(target) => {
                let diff = git.diff(repo, target, cancel).map(Diff::new);
                emit(JobResult::Diff {
                    target: target.clone(),
                    result: diff.map_err(Into::into),
                });
            }
        }
        Ok(())
    })();
    emit(JobResult::Finished {
        job: job.label(),
        result,
    });
}
