use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use git::repository::{Branch, GitFailure, GitFailureKind, Remote, RemoteCommandOutput};
use git_ui_core::notifications::open_output;
use gpui::{App, AsyncWindowContext, Entity, SharedString, WeakEntity, Window};
use project::git_store::Repository;
use util::ResultExt as _;
use workspace::Workspace;

use super::delete_dialogs::{DeleteRemoteBranchDialog, DeleteRemoteConfirm, UnmergedCommitsDialog};
use super::push_dialog::PushDialog;
use super::rename_dialog::RenameBranchDialog;
use super::{BranchContext, BranchNotice, error_notice};
use crate::branch_refs::{RefKind, RefTarget};
use crate::remote_output::{RemoteAction, SuccessMessage, SuccessStyle, format_output};

const UNMERGED_COMMIT_LIMIT: usize = 200;
const REMOTE_TAG_REF_PREFIX: &str = "refs/tags/";

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
    has_remotes: bool,
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

pub fn plan_delete_actions(details: &DeleteDetails, forced: bool) -> DeleteActionPlan {
    DeleteActionPlan {
        restore: details.tip.is_some(),
        view_commits: forced && details.tip.is_some(),
        delete_tracked: details.orphaned_upstream.is_some(),
    }
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

pub fn deleted_tag_on_remotes_message(name: &str, remotes: &[String]) -> String {
    format!("Deleted tag {name} on {}", remotes.join(", "))
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

pub fn push_result_notice(
    workspace: WeakEntity<Workspace>,
    branch: SharedString,
    remote: Remote,
    output: RemoteCommandOutput,
) -> BranchNotice {
    let action = RemoteAction::Push(branch, remote);
    let SuccessMessage { message, style } = format_output(&action, output);
    let notice = BranchNotice::info(message);
    match style {
        SuccessStyle::PushPrLink { label, url } => {
            notice.action(label, move |_window, cx| cx.open_url(&url))
        }
        SuccessStyle::Toast => with_create_pull_request(notice),
        SuccessStyle::ToastWithLog { output } => {
            let log = format!("stdout:\n{}\nstderr:\n{}", output.stdout, output.stderr);
            with_create_pull_request(notice).action("View Log", move |window, cx| {
                let log = log.clone();
                workspace
                    .update(cx, move |workspace, cx| {
                        open_output("push", workspace, &log, window, cx)
                    })
                    .log_err();
            })
        }
    }
}

fn with_create_pull_request(notice: BranchNotice) -> BranchNotice {
    notice.action("Create Pull Request", |window, cx| {
        window.dispatch_action(Box::new(zed_actions::git::CreatePullRequest), cx);
    })
}

pub fn delete_ref(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    match target.kind {
        RefKind::Local => delete_local_branch(context, target, window, cx),
        RefKind::Remote => confirm_delete_remote_branch(context, target, window, cx),
        RefKind::Tag => delete_tag(context, target, window, cx),
    }
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
    window
        .spawn(cx, async move |cx| {
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
                if let Err(error) = settle(unset).await {
                    cx.update(|_window, cx| {
                        notify_failure(
                            &context,
                            format!("Could not unset upstream of branch {new_name}"),
                            error,
                            cx,
                        )
                    })
                    .log_err();
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

fn delete_local_branch(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(branch) = context.find_branch(&target, cx) else {
        context.notify(
            BranchNotice::error(format!("Branch {} was not found", target.name)),
            cx,
        );
        return;
    };
    if branch.is_head {
        context.notify(
            BranchNotice::warning("The checked-out branch cannot be deleted.")
                .title("Cannot Delete Branch"),
            cx,
        );
        return;
    }

    let snapshot = context.snapshot(cx);
    let current_branch = snapshot.branch.as_ref().map(|branch| branch.name());
    let details = capture_delete_details(&branch, &snapshot.branch_list, current_branch);

    window
        .spawn(cx, async move |cx| {
            let outcome =
                delete_local_branch_with_fallback(&context.repository, &details.name, cx).await;
            cx.update(|_window, cx| match outcome {
                Ok(forced) => {
                    let notice = deleted_branch_notice(&context, &details, forced);
                    context.notify(notice, cx);
                }
                Err(error) => notify_failure(
                    &context,
                    format!("Branch {} wasn't deleted", details.name),
                    error,
                    cx,
                ),
            })
            .log_err();
        })
        .detach();
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
) -> BranchNotice {
    let plan = plan_delete_actions(details, forced);
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
        notice = notice.action("View Commits", move |window, cx| {
            view_unmerged_commits(context.clone(), details.clone(), window, cx);
        });
    }
    if plan.delete_tracked
        && let Some(upstream) = details.orphaned_upstream.clone()
    {
        let context = context.clone();
        notice = notice.action("Delete Tracked Branch", move |window, cx| {
            confirm_delete_remote_branch(
                context.clone(),
                RefTarget::remote(upstream.clone()),
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
                Ok(RestoreOutcome::Restored) => context.notify(
                    BranchNotice::info(format!("Restored branch {}", details.name)),
                    cx,
                ),
                Ok(RestoreOutcome::WithoutUpstream(error)) => context.notify(
                    BranchNotice::warning(format!(
                        "Restored branch {} without its upstream: {:#}",
                        details.name, error
                    )),
                    cx,
                ),
                Err(error) => notify_failure(
                    &context,
                    format!("Could not restore branch {}", details.name),
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

fn confirm_delete_remote_branch(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let tracking_branches =
        tracking_local_branches(&context.snapshot(cx).branch_list, &target.name);
    let remote_branch = target.name.clone();
    let confirm_context = context.clone();
    let confirm_tracking_branches = tracking_branches.clone();
    let on_confirm: DeleteRemoteConfirm = Box::new(move |delete_tracking, window, cx| {
        let local_branches = if delete_tracking {
            confirm_tracking_branches
        } else {
            Vec::new()
        };
        delete_remote_branch(confirm_context, target, local_branches, window, cx);
    });
    context.open_modal(window, cx, move |_window, cx| {
        DeleteRemoteBranchDialog::new(remote_branch, tracking_branches, on_confirm, cx)
    });
}

fn delete_remote_branch(
    context: BranchContext,
    target: RefTarget,
    local_branches: Vec<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(remote) = target.remote_name().map(str::to_string) else {
        context.notify(
            BranchNotice::error(format!("{} is not a remote branch", target.name)),
            cx,
        );
        return;
    };
    let branch = target.name_without_remote().to_string();

    window
        .spawn(cx, async move |cx| {
            match delete_remote_ref_task(&context, &remote, &branch, cx).await {
                Ok(()) => {
                    let (deleted, failed) =
                        delete_tracking_branches(&context.repository, &local_branches, cx).await;
                    cx.update(|_window, cx| {
                        context.notify(deleted_remote_branch_notice(&target.name, &deleted), cx);
                        for (name, error) in failed {
                            notify_failure(
                                &context,
                                format!("Branch {name} wasn't deleted"),
                                error,
                                cx,
                            );
                        }
                    })
                    .log_err();
                }
                Err(error) => {
                    cx.update(|_window, cx| {
                        notify_failure(
                            &context,
                            format!("Failed to delete remote branch {}", target.name),
                            error,
                            cx,
                        )
                    })
                    .log_err();
                }
            }
        })
        .detach();
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

fn delete_tag(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let outcome = delete_tag_task(&context.repository, &target.name, cx).await;
            cx.update(|_window, cx| match outcome {
                Ok(deleted) => {
                    let notice = deleted_tag_notice(&context, &deleted);
                    context.notify(notice, cx);
                }
                Err(error) => notify_failure(
                    &context,
                    format!("Tag {} wasn't deleted", target.name),
                    error,
                    cx,
                ),
            })
            .log_err();
        })
        .detach();
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
        has_remotes: !remotes.is_empty(),
    })
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
    if deleted.has_remotes {
        let remote_context = context.clone();
        let remote_name = deleted.name.clone();
        notice = notice.action("Delete on Remote(s)", move |window, cx| {
            delete_tag_on_remotes(remote_context.clone(), remote_name.clone(), window, cx);
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
            cx.update(|_window, cx| match result {
                Ok(()) => context.notify(BranchNotice::info(format!("Restored tag {name}")), cx),
                Err(error) => {
                    notify_failure(&context, format!("Could not restore tag {name}"), error, cx)
                }
            })
            .log_err();
        })
        .detach();
}

fn delete_tag_on_remotes(
    context: BranchContext,
    name: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
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
                            format!("Could not delete tag {name} on remotes"),
                            error,
                            cx,
                        )
                    })
                    .log_err();
                    return;
                }
            };

            let reference = format!("{REMOTE_TAG_REF_PREFIX}{name}");
            let mut deleted_on = Vec::new();
            for remote in remotes {
                match delete_remote_ref_task(&context, &remote.name, &reference, cx).await {
                    Ok(()) => deleted_on.push(remote.name.to_string()),
                    Err(error) => {
                        cx.update(|_window, cx| {
                            notify_failure(
                                &context,
                                format!("Could not delete tag {name} on {}", remote.name),
                                error,
                                cx,
                            )
                        })
                        .log_err();
                    }
                }
            }

            if !deleted_on.is_empty() {
                cx.update(|_window, cx| {
                    context.notify(
                        BranchNotice::info(deleted_tag_on_remotes_message(&name, &deleted_on)),
                        cx,
                    );
                })
                .log_err();
            }
        })
        .detach();
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

    fn remote(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/remotes/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn labels(notice: &BranchNotice) -> Vec<String> {
        notice
            .actions
            .iter()
            .map(|action| action.label.to_string())
            .collect()
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
        let details = capture_delete_details(&branch, &[branch.clone()], None);
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
            plan_delete_actions(&details, false),
            DeleteActionPlan {
                restore: true,
                view_commits: false,
                delete_tracked: false
            }
        );
        assert_eq!(
            plan_delete_actions(&details, true),
            DeleteActionPlan {
                restore: true,
                view_commits: true,
                delete_tracked: false
            }
        );
        details.tip = None;
        assert_eq!(
            plan_delete_actions(&details, true),
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
        assert!(plan_delete_actions(&details, false).delete_tracked);
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
    fn tag_and_rename_messages_follow_the_intellij_wording() {
        assert_eq!(deleted_tag_message("v1"), "Deleted Tag: v1");
        assert_eq!(
            deleted_tag_on_remotes_message("v1", &["origin".to_string(), "fork".to_string()]),
            "Deleted tag v1 on origin, fork"
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

    #[test]
    fn push_result_notice_offers_the_pull_request_link_from_the_remote() {
        let output = RemoteCommandOutput {
            stdout: String::new(),
            stderr: "remote: Create a pull request for 'main' on GitHub by visiting:\nremote:      https://github.com/o/r/pull/new/main\n".to_string(),
        };
        let notice = push_result_notice(
            WeakEntity::new_invalid(),
            SharedString::from("main"),
            Remote {
                name: SharedString::from("origin"),
            },
            output,
        );
        assert_eq!(&*notice.message, "Pushed main to origin");
        assert_eq!(labels(&notice), vec!["Create Pull Request".to_string()]);
    }

    #[test]
    fn push_result_notice_offers_log_and_pull_request_for_plain_pushes() {
        let output = RemoteCommandOutput {
            stdout: String::new(),
            stderr: "To origin\n   abc..def  main -> main\n".to_string(),
        };
        let notice = push_result_notice(
            WeakEntity::new_invalid(),
            SharedString::from("main"),
            Remote {
                name: SharedString::from("origin"),
            },
            output,
        );
        assert_eq!(
            labels(&notice),
            vec!["Create Pull Request".to_string(), "View Log".to_string()]
        );
    }

    #[test]
    fn push_result_notice_reports_up_to_date_pushes() {
        let output = RemoteCommandOutput {
            stdout: String::new(),
            stderr: "Everything up-to-date\n".to_string(),
        };
        let notice = push_result_notice(
            WeakEntity::new_invalid(),
            SharedString::from("main"),
            Remote {
                name: SharedString::from("origin"),
            },
            output,
        );
        assert_eq!(&*notice.message, "Push: Everything is up-to-date");
    }
}
