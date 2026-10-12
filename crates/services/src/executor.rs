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
                emit(JobResult::Stashes(git.stashes(repo, cancel).map_err(Into::into)));
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
            Job::ApplyPatch {
                patch,
                cached,
                reverse,
            } => git.apply(repo, patch, *cached, *reverse, cancel)?,
            Job::Switch(branch) => git.switch(repo, branch, cancel)?,
            Job::SwitchTrack(remote) => git.switch_track(repo, remote, cancel)?,
            Job::CreateBranch {
                name,
                start,
                checkout,
            } => git.create_branch(repo, name, start, *checkout, cancel)?,
            Job::DeleteBranch(name) => git.delete_branch(repo, name, true, cancel)?,
            Job::Fetch => git.fetch(repo, cancel, &mut |p| emit(JobResult::Progress(p)))?,
            Job::Pull => git.pull(repo, cancel, &mut |p| emit(JobResult::Progress(p)))?,
            Job::Push { set_upstream } => {
                let upstream = set_upstream.as_ref().map(|(r, b)| (r.as_str(), b.as_str()));
                git.push(repo, upstream, cancel, &mut |p| emit(JobResult::Progress(p)))?
            }
            Job::StashPush { message, untracked } => git.stash_push(repo, message, *untracked, cancel)?,
            Job::StashApply { index, id, pop } => git.stash_apply(repo, *index, id, *pop, cancel)?,
            Job::StashDrop { index, id } => git.stash_drop(repo, *index, id, cancel)?,
            Job::Diff(target) => {
                let diff = git.diff(repo, target, cancel).map(|mut files| {
                    crate::highlight::highlight(&mut files);
                    Diff::new(files)
                });
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
