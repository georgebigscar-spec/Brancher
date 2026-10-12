use std::path::Path;

use brancher_core::{Diff, History, Job, JobError, JobResult};
use brancher_git::{CancelToken, Git};

/// Сколько последних снимков показывать в ленте отмены.
const OPLOG_LIMIT: usize = 200;

/// Выполняет задачу и отдаёт результаты по мере готовности через `emit`.
/// Последним всегда идёт [`JobResult::Finished`].
pub fn execute(git: &Git, repo: &Path, job: &Job, cancel: &CancelToken, emit: &mut dyn FnMut(JobResult)) {
    let result: Result<(), JobError> = (|| {
        // Снимок для ленты отмены; без него изменяющая операция не запускается.
        if let Some(title) = job.undo_title() {
            git.snapshot(repo, &title, cancel)?;
        }
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
                emit(JobResult::Remotes(git.remotes(repo, cancel).map_err(Into::into)));
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
            Job::FetchRemote(name) => {
                git.fetch_remote(repo, name, cancel, &mut |p| emit(JobResult::Progress(p)))?
            }
            Job::AddRemote { name, url } => git.add_remote(repo, name, url, cancel)?,
            Job::RemoveRemote(name) => git.remove_remote(repo, name, cancel)?,
            Job::EditRemote { name, new_name, url } => {
                git.edit_remote(repo, name, new_name.as_deref(), url.as_deref(), cancel)?
            }
            Job::Pull => git.pull(repo, cancel, &mut |p| emit(JobResult::Progress(p)))?,
            Job::Push { set_upstream } => {
                let upstream = set_upstream.as_ref().map(|(r, b)| (r.as_str(), b.as_str()));
                git.push(repo, upstream, cancel, &mut |p| emit(JobResult::Progress(p)))?
            }
            Job::StashPush { message, untracked } => git.stash_push(repo, message, *untracked, cancel)?,
            Job::StashApply { index, id, pop } => git.stash_apply(repo, *index, id, *pop, cancel)?,
            Job::StashDrop { index, id } => git.stash_drop(repo, *index, id, cancel)?,
            Job::Blame { path, rev } => emit(JobResult::Blame {
                path: path.clone(),
                rev: rev.clone(),
                result: git.blame(repo, path, rev.as_deref(), cancel).map_err(Into::into),
            }),
            Job::FileLog(path) => emit(JobResult::FileLog {
                path: path.clone(),
                result: git
                    .file_log(repo, path, brancher_core::FILE_HISTORY_LIMIT, cancel)
                    .map_err(Into::into),
            }),
            Job::Compare { base, other } => {
                let limit = brancher_core::FILE_HISTORY_LIMIT;
                let result = git
                    .log_range(repo, base, other, limit, cancel)
                    .and_then(|theirs| Ok((theirs, git.log_range(repo, other, base, limit, cancel)?)));
                emit(JobResult::Compare {
                    base: base.clone(),
                    other: other.clone(),
                    result: result.map_err(Into::into),
                })
            }
            Job::CreateTag {
                name,
                target,
                message,
            } => git.create_tag(repo, name, target, message, cancel)?,
            Job::DeleteTag(name) => git.delete_tag(repo, name, cancel)?,
            Job::PushTag { remote, name } => {
                git.push_tag(repo, remote, name, cancel, &mut |p| emit(JobResult::Progress(p)))?
            }
            Job::CherryPick { commit, mainline } => git.cherry_pick(repo, commit, *mainline, cancel)?,
            Job::Revert { commit, mainline } => git.revert(repo, commit, *mainline, cancel)?,
            Job::Merge(target) => git.merge(repo, target, cancel)?,
            Job::Rebase(onto) => git.rebase(repo, onto, cancel)?,
            Job::InteractiveRebase { base, steps } => {
                git.interactive_rebase(repo, base.as_deref(), steps, cancel)?
            }
            Job::CommitMessage(commit) => emit(JobResult::CommitMessage {
                commit: commit.clone(),
                result: git.commit_message(repo, commit, cancel).map_err(Into::into),
            }),
            Job::Uncommit { commit, rebase } => {
                let rebase = rebase.as_ref().map(|(b, steps)| (b.as_deref(), steps.as_slice()));
                git.uncommit(repo, commit, rebase, cancel)?
            }
            Job::AmendInto { commit, base, steps } => {
                git.amend_into(repo, commit, base.as_deref(), steps, cancel)?
            }
            Job::UncommitFile(path) => git.uncommit_file(repo, path, cancel)?,
            Job::Snapshots => emit(JobResult::Snapshots(
                git.snapshots(repo, OPLOG_LIMIT, cancel).map_err(Into::into),
            )),
            Job::Restore { id, .. } => git.restore(repo, id, cancel)?,
            Job::Continue(op) => git.continue_operation(repo, *op, cancel)?,
            Job::Abort(op) => git.abort_operation(repo, *op, cancel)?,
            Job::ResolveConflict { path, ours } => git.resolve_with(repo, path, *ours, cancel)?,
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
