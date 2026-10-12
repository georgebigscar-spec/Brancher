use std::path::Path;

use brancher_core::{Job, JobError, JobResult};
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
                emit(JobResult::History(
                    git.log(repo, *history_limit, cancel).map_err(Into::into),
                ));
            }
            Job::Stage(paths) => git.stage(repo, paths, cancel)?,
            Job::Unstage(paths) => {
                let has_head = git.status(repo, cancel)?.head.is_some();
                git.unstage(repo, paths, has_head, cancel)?
            }
            Job::Commit { message, amend } => git.commit(repo, message, *amend, cancel)?,
        }
        Ok(())
    })();
    emit(JobResult::Finished {
        job: job.label(),
        result,
    });
}
