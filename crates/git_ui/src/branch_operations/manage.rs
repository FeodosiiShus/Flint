use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use git::repository::{Branch, GitFailure, GitFailureKind, RemoteCommandOutput, RemoteTagTarget};
use gpui::{App, AsyncWindowContext, Entity, SharedString, Window};
use project::git_store::Repository;
use util::ResultExt as _;

use super::delete_dialogs::{DeleteRemoteBranchDialog, DeleteRemoteConfirm, UnmergedCommitsDialog};
use super::integrate::{
    is_protected_branch_name, is_protected_remote_reference, protected_branch_patterns,
};
use super::push_dialog::{PushDialog, force_push_prohibited_message};
use super::rename_dialog::RenameBranchDialog;
use super::{
    BranchContext, BranchNotice, deleting_progress_title, deleting_tag_on_remote_progress_title,
    error_notice, renaming_progress_title,
};
use crate::branch_refs::{RefKind, RefTarget};

const UNMERGED_COMMIT_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteDetails {
    pub name: SharedString,
    pub tip: Option<SharedString>,
    pub upstream: Option<SharedString>,
    pub orphaned_upstream: Option<SharedString>,
    pub base: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteActionPlan {
    pub restore: bool,
    pub view_commits: bool,
    pub delete_tracked: bool,
}

struct DeletedTag {
    name: SharedString,
    sha: SharedString,
    remote_count: usize,
}

pub async fn settle<T>(receiver: oneshot::Receiver<Result<T>>) -> Result<T> {
    receiver.await.map_err(anyhow::Error::from)?
}

pub fn notify_failure(
    context: &BranchContext,
    title: impl Into<SharedString>,
    error: anyhow::Error,
    cx: &mut App,
) {
    if let Some(notice) = error_notice("", &error) {
        context.notify(notice.title(title), cx);
    }
}

pub fn is_not_fully_merged(error: &anyhow::Error) -> bool {
    let classified = error.chain().any(|cause| {
        cause
            .downcast_ref::<GitFailure>()
            .is_some_and(|failure| failure.kind == GitFailureKind::BranchNotFullyMerged)
    });
    classified
        || format!("{error:#}")
            .to_lowercase()
            .contains("not fully merged")
}

pub fn capture_delete_details(
    branch: &Branch,
    branch_list: &[Branch],
    current_branch: Option<&str>,
) -> DeleteDetails {
    let name = branch.name();
    let upstream = branch
        .upstream
        .as_ref()
        .and_then(|upstream| upstream.stripped_ref_name())
        .filter(|upstream| {
            branch_list
                .iter()
                .any(|candidate| candidate.is_remote() && candidate.name() == *upstream)
        });
    let orphaned_upstream = upstream.filter(|upstream| {
        !branch_list.iter().any(|candidate| {
            !candidate.is_remote()
                && candidate.name() != name
                && candidate
                    .upstream
                    .as_ref()
                    .and_then(|other| other.stripped_ref_name())
                    == Some(*upstream)
        })
    });
    let base = upstream.or(current_branch).unwrap_or("HEAD");

    DeleteDetails {
        name: SharedString::from(name.to_string()),
        tip: branch
            .most_recent_commit
            .as_ref()
            .map(|commit| commit.sha.clone()),
        upstream: upstream.map(|upstream| SharedString::from(upstream.to_string())),
        orphaned_upstream: orphaned_upstream
            .map(|upstream| SharedString::from(upstream.to_string())),
        base: SharedString::from(base.to_string()),
    }
}

pub fn plan_delete_actions(
    details: &DeleteDetails,
    forced: bool,
    protected_branches: &[String],
) -> DeleteActionPlan {
    DeleteActionPlan {
        restore: details.tip.is_some(),
        view_commits: forced && details.tip.is_some(),
        delete_tracked: details
            .orphaned_upstream
            .as_ref()
            .is_some_and(|upstream| !is_protected_remote_reference(upstream, protected_branches)),
    }
}

/// The local branches IntelliJ offers to delete together with the given remote
/// branches: the union of the branches tracking any of them, without repeats.
pub fn tracking_local_branches_of_all(
    branch_list: &[Branch],
    remote_branches: &[SharedString],
) -> Vec<SharedString> {
    let mut union: Vec<SharedString> = Vec::new();
    for remote_branch in remote_branches {
        for branch in tracking_local_branches(branch_list, remote_branch) {
            if !union.contains(&branch) {
                union.push(branch);
            }
        }
    }
    union
}

pub fn tracking_local_branches(branch_list: &[Branch], remote_branch: &str) -> Vec<SharedString> {
    branch_list
        .iter()
        .filter(|branch| {
            !branch.is_remote()
                && !branch.is_head
                && branch
                    .upstream
                    .as_ref()
                    .and_then(|upstream| upstream.stripped_ref_name())
                    == Some(remote_branch)
        })
        .map(|branch| SharedString::from(branch.name().to_string()))
        .collect()
}

pub fn deleted_branch_message(name: &str, forced: bool) -> String {
    if forced {
        format!("Deleted Branch: {name}\nUnmerged commits were discarded")
    } else {
        format!("Deleted Branch: {name}")
    }
}

pub fn deleted_remote_branch_notice(
    remote_branch: &str,
    deleted_local_branches: &[String],
) -> BranchNotice {
    let title = format!("Deleted remote branch {remote_branch}");
    match deleted_local_branches {
        [] => BranchNotice::info(title),
        [only] => BranchNotice::info(format!("Also deleted local branch: {only}")).title(title),
        several => BranchNotice::info(format!(
            "Also deleted local branches: {}",
            several.join(", ")
        ))
        .title(title),
    }
}

pub fn deleted_tag_message(name: &str) -> String {
    format!("Deleted Tag: {name}")
}

fn remotes_noun(remote_count: usize) -> &'static str {
    if remote_count == 1 {
        "Remote"
    } else {
        "Remotes"
    }
}

pub fn deleted_tag_on_remotes_message(name: &str, remote_count: usize) -> String {
    format!("Deleted Tag on {}: {name}", remotes_noun(remote_count))
}

pub fn tag_missing_on_remotes_message(name: &str, remote_count: usize) -> String {
    format!(
        "Tag Doesn't Exist on {}: {name}",
        remotes_noun(remote_count)
    )
}

pub fn failed_to_delete_tag_on_remotes_title(name: &str, remote_count: usize) -> String {
    format!(
        "Failed to delete tag {name} on {}",
        remotes_noun(remote_count)
    )
}

pub fn renamed_branch_message(old_name: &str, new_name: &str) -> String {
    format!("Branch {old_name} was renamed to {new_name}")
}

pub fn pushed_tag_notice(tag: &str, remote: &str, output: &RemoteCommandOutput) -> BranchNotice {
    if output.stderr.contains("Everything up-to-date") {
        BranchNotice::info(format!("Tag {tag} is already up to date on {remote}"))
    } else {
        BranchNotice::info(format!("Pushed tag {tag} to {remote}"))
    }
}

pub fn delete_ref(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    delete_refs(context, vec![target], window, cx);
}

/// Deletes the refs selected together like IntelliJ's branches dashboard:
/// tags and local branches first, then every remote branch behind a single
/// "Delete Remote Branches" confirmation.
pub fn delete_refs(
    context: BranchContext,
    targets: Vec<RefTarget>,
    window: &mut Window,
    cx: &mut App,
) {
    let (remote, local_and_tags): (Vec<RefTarget>, Vec<RefTarget>) = targets
        .into_iter()
        .partition(|target| target.kind == RefKind::Remote);
    if local_and_tags.is_empty() {
        confirm_delete_remote_branches(context, remote, window, cx);
        return;
    }
    let names = local_and_tags
        .iter()
        .map(|target| target.name.clone())
        .collect::<Vec<_>>();
    let progress = context.start_progress(deleting_progress_title(&names), cx);
    window
        .spawn(cx, async move |cx| {
            let (tags, locals): (Vec<RefTarget>, Vec<RefTarget>) = local_and_tags
                .into_iter()
                .partition(|target| target.kind == RefKind::Tag);
            for tag in tags {
                delete_tag_flow(&context, &tag, cx).await;
            }
            for local in locals {
                delete_local_branch_flow(&context, &local, cx).await;
            }
            drop(progress);
            if !remote.is_empty() {
                cx.update(|window, cx| {
                    confirm_delete_remote_branches(context.clone(), remote, window, cx)
                })
                .log_err();
            }
        })
        .detach();
}

pub fn rename_branch(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    if target.kind != RefKind::Local {
        return;
    }
    let dialog_context = context.clone();
    let name = target.name;
    context.open_modal(window, cx, move |window, cx| {
        RenameBranchDialog::new(dialog_context, name, window, cx)
    });
}

pub fn push_branch(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    if target.kind != RefKind::Local {
        return;
    }
    open_push_dialog(context, target.name, window, cx);
}

pub fn push_current(context: BranchContext, window: &mut Window, cx: &mut App) {
    let current = context.current_branch_name(cx).filter(|name| {
        context
            .find_branch(&RefTarget::local(name.clone()), cx)
            .is_some()
    });
    match current {
        Some(name) => open_push_dialog(context, name, window, cx),
        None => context.notify(
            BranchNotice::warning("There is no checked-out branch to push.").title("Cannot Push"),
            cx,
        ),
    }
}

/// `git: force push` uses the Push dialog's guard, like IntelliJ's `ForcePushAction`: a protected
/// target is refused, any other target asks first unless it was marked "Don't warn".
pub fn force_push_current(context: BranchContext, window: &mut Window, cx: &mut App) {
    let current = context.current_branch_name(cx).and_then(|name| {
        context
            .find_branch(&RefTarget::local(name.clone()), cx)
            .map(|branch| (name, branch))
    });
    let Some((name, branch)) = current else {
        context.notify(
            BranchNotice::warning("There is no checked-out branch to push.").title("Cannot Push"),
            cx,
        );
        return;
    };
    let remote_branch = branch
        .upstream
        .as_ref()
        .and_then(|upstream| upstream.branch_name())
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string());
    if is_protected_branch_name(&remote_branch, &protected_branch_patterns(cx)) {
        let target = match branch
            .upstream
            .as_ref()
            .and_then(|upstream| upstream.remote_name())
        {
            Some(remote) => format!("{remote}/{remote_branch}"),
            None => remote_branch,
        };
        context.notify(
            BranchNotice::error(force_push_prohibited_message(&target)),
            cx,
        );
        return;
    }
    let dialog_context = context.clone();
    context.open_modal(window, cx, move |window, cx| {
        let mut dialog = PushDialog::new(dialog_context, name, window, cx);
        dialog.request_force_push(window, cx);
        dialog
    });
}

pub fn push_tag(
    context: BranchContext,
    target: RefTarget,
    remote: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    if target.kind != RefKind::Tag {
        return;
    }
    window
        .spawn(cx, async move |cx| {
            let result = push_tag_task(&context, &remote, &target.name, cx).await;
            cx.update(|_window, cx| match result {
                Ok(output) => {
                    context.notify(pushed_tag_notice(&target.name, &remote, &output), cx);
                }
                Err(error) => notify_failure(
                    &context,
                    format!("Could not push tag {} to {}", target.name, remote),
                    error,
                    cx,
                ),
            })
            .log_err();
        })
        .detach();
}

pub fn perform_rename(
    context: BranchContext,
    old_name: SharedString,
    new_name: SharedString,
    unset_upstream: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let progress = context.start_progress(renaming_progress_title(&old_name, &new_name), cx);
    window
        .spawn(cx, async move |cx| {
            let _progress = progress;
            let rename = context.repository.update(cx, |repository, _| {
                repository.rename_branch(old_name.to_string(), new_name.to_string())
            });
            if let Err(error) = settle(rename).await {
                cx.update(|_window, cx| {
                    notify_failure(
                        &context,
                        format!("Could not rename {old_name} to {new_name}"),
                        error,
                        cx,
                    )
                })
                .log_err();
                return;
            }

            if unset_upstream {
                let unset = context.repository.update(cx, |repository, _| {
                    repository.set_upstream(new_name.to_string(), None)
                });
                if let Err(error) = settle(unset).await
                    && !is_no_upstream_error(&error)
                {
                    cx.update(|_window, cx| {
                        notify_failure(
                            &context,
                            format!("Could not unset upstream of branch {new_name}"),
                            error,
                            cx,
                        )
                    })
                    .log_err();
                    return;
                }
            }

            cx.update(|_window, cx| {
                context.notify(
                    BranchNotice::info(renamed_branch_message(&old_name, &new_name)),
                    cx,
                );
            })
            .log_err();
        })
        .detach();
}

// IntelliJ's `GitRenameBranchOperation` ignores only this unset failure.
fn is_no_upstream_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.lines()
        .any(|line| line.contains("fatal:") && line.contains("has no upstream information"))
}

fn open_push_dialog(
    context: BranchContext,
    branch: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let dialog_context = context.clone();
    context.open_modal(window, cx, move |window, cx| {
        PushDialog::new(dialog_context, branch, window, cx)
    });
}

async fn delete_local_branch_flow(
    context: &BranchContext,
    target: &RefTarget,
    cx: &mut AsyncWindowContext,
) {
    let prepared = cx.update(|_window, cx| {
        let Some(branch) = context.find_branch(target, cx) else {
            context.notify(
                BranchNotice::error(format!("Branch {} was not found", target.name)),
                cx,
            );
            return None;
        };
        if branch.is_head {
            context.notify(
                BranchNotice::warning("The checked-out branch cannot be deleted.")
                    .title("Cannot Delete Branch"),
                cx,
            );
            return None;
        }
        let snapshot = context.snapshot(cx);
        let current_branch = snapshot.branch.as_ref().map(|branch| branch.name());
        Some((
            capture_delete_details(&branch, &snapshot.branch_list, current_branch),
            protected_branch_patterns(cx),
        ))
    });
    let Ok(Some((details, protected_branches))) = prepared else {
        return;
    };
    let outcome = delete_local_branch_with_fallback(&context.repository, &details.name, cx).await;
    cx.update(|_window, cx| match outcome {
        Ok(forced) => {
            let notice = deleted_branch_notice(context, &details, forced, &protected_branches);
            context.notify(notice, cx);
        }
        Err(error) => notify_failure(
            context,
            format!("Branch {} wasn't deleted", details.name),
            error,
            cx,
        ),
    })
    .log_err();
}

async fn delete_local_branch_with_fallback(
    repository: &Entity<Repository>,
    name: &str,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    let safe_delete = repository.update(cx, |repository, _| {
        repository.delete_branch(false, name.to_string(), false)
    });
    match settle(safe_delete).await {
        Ok(()) => Ok(false),
        Err(error) if is_not_fully_merged(&error) => {
            let force_delete = repository.update(cx, |repository, _| {
                repository.delete_branch(false, name.to_string(), true)
            });
            settle(force_delete).await?;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

fn deleted_branch_notice(
    context: &BranchContext,
    details: &DeleteDetails,
    forced: bool,
    protected_branches: &[String],
) -> BranchNotice {
    let plan = plan_delete_actions(details, forced, protected_branches);
    let mut notice = BranchNotice::info(deleted_branch_message(&details.name, forced));

    if plan.restore {
        let context = context.clone();
        let details = details.clone();
        notice = notice.action("Restore", move |window, cx| {
            restore_branch(context.clone(), details.clone(), window, cx);
        });
    }
    if plan.view_commits {
        let context = context.clone();
        let details = details.clone();
        notice = notice.action_keeping_notice("View Commits", move |window, cx| {
            view_unmerged_commits(context.clone(), details.clone(), window, cx);
        });
    }
    if plan.delete_tracked
        && let Some(upstream) = details.orphaned_upstream.clone()
    {
        let context = context.clone();
        notice = notice.action("Delete Tracked Branch", move |window, cx| {
            confirm_delete_remote_branches(
                context.clone(),
                vec![RefTarget::remote(upstream.clone())],
                window,
                cx,
            );
        });
    }
    notice
}

fn restore_branch(
    context: BranchContext,
    details: DeleteDetails,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tip) = details.tip.clone() else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let result = restore_branch_task(
                &context.repository,
                &details.name,
                &tip,
                details.upstream.as_deref(),
                cx,
            )
            .await;
            cx.update(|_window, cx| match result {
                Ok(RestoreOutcome::Restored) => {}
                Ok(RestoreOutcome::WithoutUpstream(error)) => context.notify(
                    BranchNotice::warning(format!(
                        "Restored branch {} without its upstream: {:#}",
                        details.name, error
                    )),
                    cx,
                ),
                Err(error) => notify_failure(
                    &context,
                    format!("Could not Restore {}", details.name),
                    error,
                    cx,
                ),
            })
            .log_err();
        })
        .detach();
}

enum RestoreOutcome {
    Restored,
    WithoutUpstream(anyhow::Error),
}

async fn restore_branch_task(
    repository: &Entity<Repository>,
    name: &str,
    tip: &str,
    upstream: Option<&str>,
    cx: &mut AsyncWindowContext,
) -> Result<RestoreOutcome> {
    let create = repository.update(cx, |repository, _| {
        repository.create_branch_at(name.to_string(), tip.to_string(), false, false)
    });
    settle(create).await?;
    if let Some(upstream) = upstream {
        let restore_upstream = repository.update(cx, |repository, _| {
            repository.set_upstream(name.to_string(), Some(upstream.to_string()))
        });
        if let Err(error) = settle(restore_upstream).await {
            return Ok(RestoreOutcome::WithoutUpstream(error));
        }
    }
    Ok(RestoreOutcome::Restored)
}

fn view_unmerged_commits(
    context: BranchContext,
    details: DeleteDetails,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tip) = details.tip.clone() else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let commits = context.repository.update(cx, |repository, _| {
                repository.commits_between(
                    details.base.to_string(),
                    tip.to_string(),
                    UNMERGED_COMMIT_LIMIT,
                )
            });
            match settle(commits).await {
                Ok(commits) => {
                    cx.update(|window, cx| {
                        let restore_context = context.clone();
                        let restore_details = details.clone();
                        let branch = details.name.clone();
                        let base = details.base.clone();
                        context.open_modal(window, cx, move |_window, cx| {
                            UnmergedCommitsDialog::new(
                                branch,
                                base,
                                commits,
                                Box::new(move |window, cx| {
                                    restore_branch(restore_context, restore_details, window, cx)
                                }),
                                cx,
                            )
                        });
                    })
                    .log_err();
                }
                Err(error) => {
                    cx.update(|_window, cx| {
                        notify_failure(&context, "Could not load unmerged commits", error, cx)
                    })
                    .log_err();
                }
            }
        })
        .detach();
}

fn confirm_delete_remote_branches(
    context: BranchContext,
    targets: Vec<RefTarget>,
    window: &mut Window,
    cx: &mut App,
) {
    if targets.is_empty() {
        return;
    }
    let branch_list = context.snapshot(cx).branch_list;
    let remote_branches = targets
        .iter()
        .map(|target| target.name.clone())
        .collect::<Vec<_>>();
    let tracking_branches = tracking_local_branches_of_all(&branch_list, &remote_branches);
    let confirm_context = context.clone();
    let on_confirm: DeleteRemoteConfirm = Box::new(move |delete_tracking, window, cx| {
        delete_remote_branches(confirm_context, targets, delete_tracking, window, cx);
    });
    context.open_modal(window, cx, move |_window, cx| {
        DeleteRemoteBranchDialog::new(remote_branches, tracking_branches, on_confirm, cx)
    });
}

fn delete_remote_branches(
    context: BranchContext,
    targets: Vec<RefTarget>,
    delete_tracking: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let branch_list = context.snapshot(cx).branch_list;
    let names = targets
        .iter()
        .map(|target| target.name.clone())
        .collect::<Vec<_>>();
    let progress = context.start_progress(deleting_progress_title(&names), cx);
    window
        .spawn(cx, async move |cx| {
            for target in targets {
                let local_branches = if delete_tracking {
                    tracking_local_branches(&branch_list, &target.name)
                } else {
                    Vec::new()
                };
                delete_remote_branch_flow(&context, &target, &local_branches, cx).await;
            }
            drop(progress);
        })
        .detach();
}

async fn delete_remote_branch_flow(
    context: &BranchContext,
    target: &RefTarget,
    local_branches: &[SharedString],
    cx: &mut AsyncWindowContext,
) {
    let Some(remote) = target.remote_name().map(str::to_string) else {
        cx.update(|_window, cx| {
            context.notify(
                BranchNotice::error(format!("{} is not a remote branch", target.name)),
                cx,
            )
        })
        .log_err();
        return;
    };
    let branch = target.name_without_remote().to_string();
    match delete_remote_ref_task(context, &remote, &branch, cx).await {
        Ok(()) => {
            let (deleted, failed) =
                delete_tracking_branches(&context.repository, local_branches, cx).await;
            cx.update(|_window, cx| {
                context.notify(deleted_remote_branch_notice(&target.name, &deleted), cx);
                for (name, error) in failed {
                    notify_failure(context, format!("Branch {name} wasn't deleted"), error, cx);
                }
            })
            .log_err();
        }
        Err(error) => {
            cx.update(|_window, cx| {
                notify_failure(
                    context,
                    format!("Failed to delete remote branch {}", target.name),
                    error,
                    cx,
                )
            })
            .log_err();
        }
    }
}

async fn delete_remote_ref_task(
    context: &BranchContext,
    remote: &str,
    reference: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let askpass = cx.update(|window, cx| {
        context.askpass_delegate(
            format!("git push {remote} --delete {reference}"),
            window,
            cx,
        )
    })?;
    let receiver = context.repository.update(cx, |repository, cx| {
        repository.delete_remote_branch(remote.to_string(), reference.to_string(), askpass, cx)
    });
    settle(receiver).await.map(|_| ())
}

async fn delete_tracking_branches(
    repository: &Entity<Repository>,
    names: &[SharedString],
    cx: &mut AsyncWindowContext,
) -> (Vec<String>, Vec<(SharedString, anyhow::Error)>) {
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for name in names {
        match delete_local_branch_with_fallback(repository, name, cx).await {
            Ok(_) => deleted.push(name.to_string()),
            Err(error) => failed.push((name.clone(), error)),
        }
    }
    (deleted, failed)
}

async fn delete_tag_flow(context: &BranchContext, target: &RefTarget, cx: &mut AsyncWindowContext) {
    let outcome = delete_tag_task(&context.repository, &target.name, cx).await;
    cx.update(|_window, cx| match outcome {
        Ok(deleted) => {
            let notice = deleted_tag_notice(context, &deleted);
            context.notify(notice, cx);
        }
        Err(error) => notify_failure(
            context,
            format!("Tag {} wasn't deleted", target.name),
            error,
            cx,
        ),
    })
    .log_err();
}

async fn delete_tag_task(
    repository: &Entity<Repository>,
    name: &str,
    cx: &mut AsyncWindowContext,
) -> Result<DeletedTag> {
    let tags = settle(repository.update(cx, |repository, _| repository.tags())).await?;
    let tag = tags
        .into_iter()
        .find(|tag| &*tag.name == name)
        .ok_or_else(|| anyhow!("Could not find tag {name}"))?;
    let delete = repository.update(cx, |repository, _| repository.delete_tag(name.to_string()));
    settle(delete).await?;
    let remotes =
        settle(repository.update(cx, |repository, _| repository.get_remotes(None, false)))
            .await
            .log_err()
            .unwrap_or_default();
    Ok(DeletedTag {
        name: SharedString::from(name.to_string()),
        sha: tag.commit_sha,
        remote_count: remotes.len(),
    })
}

fn delete_tag_on_remotes_label(remote_count: usize) -> Option<&'static str> {
    match remote_count {
        0 => None,
        1 => Some("Delete on Remote"),
        _ => Some("Delete on Remotes"),
    }
}

fn deleted_tag_notice(context: &BranchContext, deleted: &DeletedTag) -> BranchNotice {
    let restore_context = context.clone();
    let restore_name = deleted.name.clone();
    let restore_sha = deleted.sha.clone();
    let mut notice = BranchNotice::info(deleted_tag_message(&deleted.name)).action(
        "Restore",
        move |window, cx| {
            restore_tag(
                restore_context.clone(),
                restore_name.clone(),
                restore_sha.clone(),
                window,
                cx,
            );
        },
    );
    if let Some(label) = delete_tag_on_remotes_label(deleted.remote_count) {
        let remote_context = context.clone();
        let remote_name = deleted.name.clone();
        let remote_sha = deleted.sha.clone();
        let remote_count = deleted.remote_count;
        notice = notice.action(label, move |window, cx| {
            delete_tag_on_remotes(
                remote_context.clone(),
                remote_name.clone(),
                remote_sha.clone(),
                remote_count,
                window,
                cx,
            );
        });
    }
    notice
}

fn restore_tag(
    context: BranchContext,
    name: SharedString,
    sha: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let create = context.repository.update(cx, |repository, _| {
                repository.create_tag(name.to_string(), sha.to_string())
            });
            let result = settle(create).await;
            if let Err(error) = result {
                cx.update(|_window, cx| {
                    notify_failure(&context, format!("Could not Restore {name}"), error, cx)
                })
                .log_err();
            }
        })
        .detach();
}

fn delete_tag_on_remotes(
    context: BranchContext,
    name: SharedString,
    expected_commit: SharedString,
    known_remote_count: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let progress = context.start_progress(deleting_tag_on_remote_progress_title(&name), cx);
    window
        .spawn(cx, async move |cx| {
            let remotes = settle(
                context
                    .repository
                    .update(cx, |repository, _| repository.get_remotes(None, false)),
            )
            .await;
            let remotes = match remotes {
                Ok(remotes) => remotes,
                Err(error) => {
                    cx.update(|_window, cx| {
                        notify_failure(
                            &context,
                            failed_to_delete_tag_on_remotes_title(&name, known_remote_count),
                            error,
                            cx,
                        )
                    })
                    .log_err();
                    return;
                }
            };

            let mut deleted = 0;
            let mut errors = Vec::new();
            for remote in &remotes {
                match delete_tag_on_remote(&context, &remote.name, &name, &expected_commit, cx)
                    .await
                {
                    Ok(true) => deleted += 1,
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }
            drop(progress);

            let remote_count = remotes.len();
            cx.update(|_window, cx| {
                if deleted > 0 {
                    context.notify(
                        BranchNotice::info(deleted_tag_on_remotes_message(&name, remote_count)),
                        cx,
                    );
                } else if errors.is_empty() {
                    context.notify(
                        BranchNotice::info(tag_missing_on_remotes_message(&name, remote_count)),
                        cx,
                    );
                }
                for error in errors {
                    notify_failure(
                        &context,
                        failed_to_delete_tag_on_remotes_title(&name, remote_count),
                        error,
                        cx,
                    );
                }
            })
            .log_err();
        })
        .detach();
}

/// IntelliJ's `GitDeleteRemoteTagOperation` for one remote: deletes the tag
/// only where `ls-remote` finds it, leased on the commit it pointed to locally.
/// Returns whether anything was deleted.
async fn delete_tag_on_remote(
    context: &BranchContext,
    remote: &str,
    tag: &str,
    expected_commit: &str,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    let askpass = cx.update(|window, cx| {
        context.askpass_delegate(
            format!("git ls-remote --tags {remote} refs/tags/{tag}"),
            window,
            cx,
        )
    })?;
    let listing = context.repository.update(cx, |repository, _| {
        repository.remote_tag_target(remote.to_string(), tag.to_string(), askpass)
    });
    let Some(target) = settle(listing).await? else {
        return Ok(false);
    };
    let expected_object = lease_object(&target, expected_commit);
    let askpass = cx.update(|window, cx| {
        context.askpass_delegate(format!("git push {remote} :refs/tags/{tag}"), window, cx)
    })?;
    let deletion = context.repository.update(cx, |repository, _| {
        repository.delete_remote_tag(
            remote.to_string(),
            tag.to_string(),
            expected_object,
            askpass,
        )
    });
    settle(deletion).await?;
    Ok(true)
}

/// The remote ref of an annotated tag points to the tag object, not to the
/// commit the local tag peeled to, so the lease must name that object.
pub fn lease_object(target: &RemoteTagTarget, expected_commit: &str) -> String {
    if target.commit == expected_commit {
        target.object.clone()
    } else {
        expected_commit.to_string()
    }
}

async fn push_tag_task(
    context: &BranchContext,
    remote: &str,
    tag: &str,
    cx: &mut AsyncWindowContext,
) -> Result<RemoteCommandOutput> {
    let askpass = cx.update(|window, cx| {
        context.askpass_delegate(format!("git push {remote} refs/tags/{tag}"), window, cx)
    })?;
    let receiver = context.repository.update(cx, |repository, cx| {
        repository.push_tag(remote.to_string(), tag.to_string(), askpass, cx)
    });
    settle(receiver).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branch_operations::NoticeSeverity;
    use git::repository::{CommitSummary, Upstream, UpstreamTracking, UpstreamTrackingStatus};

    fn commit(sha: &str) -> CommitSummary {
        CommitSummary {
            sha: SharedString::from(sha.to_string()),
            subject: SharedString::from("subject"),
            commit_timestamp: 0,
            author_name: SharedString::from("author"),
            has_parent: true,
        }
    }

    fn local(name: &str, upstream: Option<&str>, tip: Option<&str>, is_head: bool) -> Branch {
        Branch {
            is_head,
            ref_name: SharedString::from(format!("refs/heads/{name}")),
            upstream: upstream.map(|upstream| Upstream {
                ref_name: SharedString::from(format!("refs/remotes/{upstream}")),
                tracking: UpstreamTracking::Tracked(UpstreamTrackingStatus {
                    ahead: 0,
                    behind: 0,
                }),
            }),
            most_recent_commit: tip.map(commit),
        }
    }

    fn protected() -> Vec<String> {
        vec!["master".to_string(), "main".to_string()]
    }

    fn remote(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/remotes/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    #[test]
    fn not_fully_merged_is_detected_from_the_git_failure_kind() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::BranchNotFullyMerged,
            message: "localized message".to_string(),
        });
        assert!(is_not_fully_merged(&error));
    }

    #[test]
    fn not_fully_merged_is_detected_from_plain_error_text() {
        let error = anyhow!("error: the branch 'x' is not fully merged");
        assert!(is_not_fully_merged(&error));
    }

    #[test]
    fn not_fully_merged_is_detected_through_error_context() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::BranchNotFullyMerged,
            message: "x".to_string(),
        })
        .context("deleting");
        assert!(is_not_fully_merged(&error));
    }

    #[test]
    fn other_failures_are_not_treated_as_not_fully_merged() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::Other,
            message: "fatal: branch not found".to_string(),
        });
        assert!(!is_not_fully_merged(&error));
    }

    #[test]
    fn delete_details_use_the_upstream_as_base_when_it_exists() {
        let branch = local("feature", Some("origin/feature"), Some("abc"), false);
        let list = [branch.clone(), remote("origin/feature")];
        let details = capture_delete_details(&branch, &list, Some("main"));
        assert_eq!(&*details.base, "origin/feature");
        assert_eq!(details.upstream.as_deref(), Some("origin/feature"));
        assert_eq!(details.orphaned_upstream.as_deref(), Some("origin/feature"));
        assert_eq!(details.tip.as_deref(), Some("abc"));
    }

    #[test]
    fn delete_details_fall_back_to_the_current_branch_when_upstream_is_gone() {
        let branch = local("feature", Some("origin/feature"), Some("abc"), false);
        let list = [branch.clone()];
        let details = capture_delete_details(&branch, &list, Some("main"));
        assert_eq!(&*details.base, "main");
        assert_eq!(details.upstream, None);
        assert_eq!(details.orphaned_upstream, None);
    }

    #[test]
    fn delete_details_fall_back_to_head_without_any_reference() {
        let branch = local("feature", None, None, false);
        let details = capture_delete_details(&branch, std::slice::from_ref(&branch), None);
        assert_eq!(&*details.base, "HEAD");
        assert_eq!(details.tip, None);
    }

    #[test]
    fn upstream_tracked_by_another_local_branch_is_not_orphaned() {
        let branch = local("feature", Some("origin/shared"), Some("abc"), false);
        let sibling = local("other", Some("origin/shared"), Some("def"), false);
        let list = [branch.clone(), sibling, remote("origin/shared")];
        let details = capture_delete_details(&branch, &list, Some("main"));
        assert_eq!(details.upstream.as_deref(), Some("origin/shared"));
        assert_eq!(details.orphaned_upstream, None);
    }

    #[test]
    fn restore_needs_a_tip_and_view_commits_needs_a_forced_delete() {
        let mut details = capture_delete_details(
            &local("feature", None, Some("abc"), false),
            &[],
            Some("main"),
        );
        assert_eq!(
            plan_delete_actions(&details, false, &protected()),
            DeleteActionPlan {
                restore: true,
                view_commits: false,
                delete_tracked: false
            }
        );
        assert_eq!(
            plan_delete_actions(&details, true, &protected()),
            DeleteActionPlan {
                restore: true,
                view_commits: true,
                delete_tracked: false
            }
        );
        details.tip = None;
        assert_eq!(
            plan_delete_actions(&details, true, &protected()),
            DeleteActionPlan {
                restore: false,
                view_commits: false,
                delete_tracked: false
            }
        );
    }

    #[test]
    fn delete_tracked_branch_is_offered_for_an_orphaned_upstream() {
        let branch = local("feature", Some("origin/feature"), Some("abc"), false);
        let list = [branch.clone(), remote("origin/feature")];
        let details = capture_delete_details(&branch, &list, Some("main"));
        assert!(plan_delete_actions(&details, false, &protected()).delete_tracked);
    }

    #[test]
    fn delete_tracked_branch_is_not_offered_for_a_protected_upstream() {
        let branch = local("main-copy", Some("origin/main"), Some("abc"), false);
        let list = [branch.clone(), remote("origin/main")];
        let details = capture_delete_details(&branch, &list, Some("feature"));
        assert_eq!(details.orphaned_upstream.as_deref(), Some("origin/main"));
        assert!(!plan_delete_actions(&details, false, &protected()).delete_tracked);
        assert!(plan_delete_actions(&details, false, &[]).delete_tracked);
    }

    #[test]
    fn several_remote_branches_offer_the_union_of_their_tracking_branches() {
        let list = [
            local("a", Some("origin/x"), None, false),
            local("b", Some("origin/y"), None, false),
            local("c", Some("origin/x"), None, false),
            local("d", Some("origin/y"), None, true),
            remote("origin/x"),
            remote("origin/y"),
        ];
        let tracking = tracking_local_branches_of_all(
            &list,
            &[
                SharedString::from("origin/x"),
                SharedString::from("origin/y"),
            ],
        );
        assert_eq!(
            tracking,
            vec![
                SharedString::from("a"),
                SharedString::from("c"),
                SharedString::from("b")
            ]
        );
    }

    #[test]
    fn an_annotated_remote_tag_is_leased_on_its_tag_object() {
        let annotated = RemoteTagTarget {
            object: "tag-object".to_string(),
            commit: "commit".to_string(),
        };
        assert_eq!(lease_object(&annotated, "commit"), "tag-object");
        assert_eq!(
            lease_object(&annotated, "other-commit"),
            "other-commit",
            "a tag that moved on the remote must fail the lease"
        );
    }

    #[test]
    fn only_a_missing_upstream_is_ignored_when_unsetting_it_after_a_rename() {
        assert!(is_no_upstream_error(&anyhow!(
            "fatal: branch 'x' has no upstream information"
        )));
        assert!(!is_no_upstream_error(&anyhow!(
            "fatal: could not lock config file .git/config"
        )));
    }

    #[test]
    fn tracking_branches_exclude_the_checked_out_one_and_unrelated_ones() {
        let list = [
            local("a", Some("origin/x"), None, false),
            local("b", Some("origin/x"), None, true),
            local("c", Some("origin/y"), None, false),
            local("d", None, None, false),
            remote("origin/x"),
        ];
        let tracking = tracking_local_branches(&list, "origin/x");
        assert_eq!(tracking, vec![SharedString::from("a")]);
    }

    #[test]
    fn forced_delete_message_mentions_discarded_commits() {
        assert_eq!(deleted_branch_message("x", false), "Deleted Branch: x");
        assert_eq!(
            deleted_branch_message("x", true),
            "Deleted Branch: x\nUnmerged commits were discarded"
        );
    }

    #[test]
    fn remote_delete_notice_reports_also_deleted_local_branches() {
        let plain = deleted_remote_branch_notice("origin/x", &[]);
        assert_eq!(&*plain.message, "Deleted remote branch origin/x");
        assert_eq!(plain.title, None);
        assert_eq!(plain.severity, NoticeSeverity::Info);

        let single = deleted_remote_branch_notice("origin/x", &["x".to_string()]);
        assert_eq!(
            single.title.as_deref(),
            Some("Deleted remote branch origin/x")
        );
        assert_eq!(&*single.message, "Also deleted local branch: x");

        let several = deleted_remote_branch_notice("origin/x", &["x".to_string(), "y".to_string()]);
        assert_eq!(&*several.message, "Also deleted local branches: x, y");
    }

    #[test]
    fn delete_tag_on_remotes_is_pluralised_by_the_number_of_remotes() {
        assert_eq!(delete_tag_on_remotes_label(0), None);
        assert_eq!(delete_tag_on_remotes_label(1), Some("Delete on Remote"));
        assert_eq!(delete_tag_on_remotes_label(2), Some("Delete on Remotes"));
        assert_eq!(delete_tag_on_remotes_label(5), Some("Delete on Remotes"));
    }

    #[test]
    fn tag_and_rename_messages_follow_the_intellij_wording() {
        assert_eq!(deleted_tag_message("v1"), "Deleted Tag: v1");
        assert_eq!(
            deleted_tag_on_remotes_message("v1", 1),
            "Deleted Tag on Remote: v1"
        );
        assert_eq!(
            deleted_tag_on_remotes_message("v1", 2),
            "Deleted Tag on Remotes: v1"
        );
        assert_eq!(
            tag_missing_on_remotes_message("v1", 1),
            "Tag Doesn't Exist on Remote: v1"
        );
        assert_eq!(
            failed_to_delete_tag_on_remotes_title("v1", 2),
            "Failed to delete tag v1 on Remotes"
        );
        assert_eq!(
            renamed_branch_message("old", "new"),
            "Branch old was renamed to new"
        );
    }

    #[test]
    fn pushed_tag_notice_distinguishes_up_to_date_pushes() {
        let up_to_date = RemoteCommandOutput {
            stdout: String::new(),
            stderr: "Everything up-to-date\n".to_string(),
        };
        let pushed = RemoteCommandOutput {
            stdout: String::new(),
            stderr: " * [new tag]         v1 -> v1\n".to_string(),
        };
        assert_eq!(
            &*pushed_tag_notice("v1", "origin", &up_to_date).message,
            "Tag v1 is already up to date on origin"
        );
        assert_eq!(
            &*pushed_tag_notice("v1", "origin", &pushed).message,
            "Pushed tag v1 to origin"
        );
    }
}
