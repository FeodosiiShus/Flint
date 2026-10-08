use anyhow::Result;
use git::repository::{GitFailureKind, RebaseAction, RebaseOutcome, RepositoryOperation};
use gpui::{App, AsyncWindowContext, Window};

use super::integrate;
use super::smart_operation::stash_local_changes;
use super::{BranchContext, BranchNotice};
use crate::merge_tool::has_unmerged_paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OngoingOperationAction {
    AbortRebase,
    ContinueRebase,
    SkipCommit,
    AbortMerge,
}

impl OngoingOperationAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::AbortRebase => "Abort Rebase",
            Self::ContinueRebase => "Continue Rebase",
            Self::SkipCommit => "Skip Commit",
            Self::AbortMerge => "Abort Merge",
        }
    }
}

pub fn ongoing_actions(operation: Option<RepositoryOperation>) -> Vec<OngoingOperationAction> {
    match operation {
        Some(RepositoryOperation::Rebase) => vec![
            OngoingOperationAction::AbortRebase,
            OngoingOperationAction::ContinueRebase,
            OngoingOperationAction::SkipCommit,
        ],
        Some(RepositoryOperation::Merge) => vec![OngoingOperationAction::AbortMerge],
        Some(RepositoryOperation::CherryPick) | Some(RepositoryOperation::Revert) | None => {
            Vec::new()
        }
    }
}

fn smart_stash_present(context: &BranchContext, cx: &App) -> bool {
    context
        .repository
        .read(cx)
        .cached_stash()
        .entries
        .first()
        .is_some_and(|entry| {
            entry
                .message
                .contains(super::smart_operation::STASH_MESSAGE)
        })
}

pub fn run_ongoing_action(
    context: BranchContext,
    action: OngoingOperationAction,
    window: &mut Window,
    cx: &mut App,
) {
    let stashed = smart_stash_present(&context, cx);
    let report = RebaseReport {
        stashed,
        ..RebaseReport::default()
    };
    match action {
        OngoingOperationAction::AbortRebase => {
            advance_rebase_in_background(context, report, RebaseAction::Abort, window, cx)
        }
        OngoingOperationAction::ContinueRebase => {
            advance_rebase_in_background(context, report, RebaseAction::Continue, window, cx)
        }
        OngoingOperationAction::SkipCommit => {
            advance_rebase_in_background(context, report, RebaseAction::Skip, window, cx)
        }
        OngoingOperationAction::AbortMerge => abort_merge(context, stashed, window, cx),
    }
}

#[derive(Clone, Debug)]
pub(super) struct UpdateProgress {
    pub head_before: Option<String>,
    pub commits: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RebaseReport {
    pub branch: Option<String>,
    pub onto: Option<String>,
    pub stashed: bool,
    pub update: Option<UpdateProgress>,
}

pub(super) fn rebase_success_message(branch: Option<&str>, onto: Option<&str>) -> String {
    match (branch, onto) {
        (Some(branch), Some(onto)) => format!("Rebased {branch} on {onto}"),
        (Some(branch), None) => format!("Rebased {branch}"),
        (None, _) => "Rebase completed".to_string(),
    }
}

pub(super) fn rebase_conflict_message(stashed: bool) -> String {
    let mut message = String::from("Resolve the conflicts, then continue the rebase.");
    if stashed {
        message.push_str(" Local changes were stashed before the rebase.");
    }
    message
}

pub(super) async fn rebase_once(
    context: &BranchContext,
    action: RebaseAction,
    cx: &mut AsyncWindowContext,
) -> Result<RebaseOutcome> {
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.rebase(action));
    receiver.await?
}

pub(super) async fn start_rebase(
    context: &BranchContext,
    upstream: String,
    mut report: RebaseReport,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    report.stashed = cx
        .update(|_, app| stash_local_changes(context, app))?
        .await?;
    let action = RebaseAction::Start {
        upstream,
        branch: None,
    };
    match rebase_once(context, action, cx).await {
        Ok(outcome) => finish_rebase(context, &report, outcome, cx).await,
        Err(error) => {
            integrate::restore_stash(context, report.stashed, cx).await;
            report_rebase_failure(context, &error, cx);
            Ok(())
        }
    }
}

pub(super) async fn finish_rebase(
    context: &BranchContext,
    report: &RebaseReport,
    outcome: RebaseOutcome,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    match outcome {
        RebaseOutcome::Completed { .. } => {
            integrate::restore_stash(context, report.stashed, cx).await;
            let notice = match &report.update {
                Some(progress) => {
                    integrate::update_notice(
                        context,
                        report.branch.as_deref(),
                        report.onto.as_deref(),
                        progress,
                        cx,
                    )
                    .await
                }
                None => BranchNotice::info(rebase_success_message(
                    report.branch.as_deref(),
                    report.onto.as_deref(),
                ))
                .title("Rebase successful"),
            };
            integrate::notify(context, notice, cx);
        }
        RebaseOutcome::Conflicted { .. } => {
            cx.update(|window, app| context.open_conflicts_if_conflicted(window, app))?;
            integrate::notify(context, rebase_conflict_notice(context, report), cx);
        }
        RebaseOutcome::Aborted => {
            integrate::restore_stash(context, report.stashed, cx).await;
            integrate::notify(context, BranchNotice::info("Rebase aborted"), cx);
        }
    }
    Ok(())
}

fn rebase_conflict_notice(context: &BranchContext, report: &RebaseReport) -> BranchNotice {
    let resolve_context = context.clone();
    let continue_context = context.clone();
    let continue_report = report.clone();
    let abort_context = context.clone();
    let abort_report = report.clone();
    BranchNotice::warning(rebase_conflict_message(report.stashed))
        .title("Rebase stopped due to conflicts")
        .action("Resolve…", move |window, cx| {
            resolve_context.open_conflicts_if_conflicted(window, cx)
        })
        .action("Continue", move |window, cx| {
            advance_rebase_in_background(
                continue_context.clone(),
                continue_report.clone(),
                RebaseAction::Continue,
                window,
                cx,
            )
        })
        .action("Abort", move |window, cx| {
            advance_rebase_in_background(
                abort_context.clone(),
                abort_report.clone(),
                RebaseAction::Abort,
                window,
                cx,
            )
        })
}

pub(super) fn report_rebase_failure(
    context: &BranchContext,
    error: &anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    let notice = match integrate::git_failure(error).map(|failure| &failure.kind) {
        Some(GitFailureKind::UntrackedFilesWouldBeOverwritten { files }) => Some(
            BranchNotice::error(integrate::untracked_files_message("rebase", files))
                .title("Untracked Files Prevent Rebase"),
        ),
        Some(GitFailureKind::UnmergedFiles) => Some(integrate::unmerged_files_notice(
            context,
            "Rebase failed",
            "Resolve the conflicts before rebasing.",
        )),
        _ => integrate::failure_notice("Rebase failed", error),
    };
    if let Some(notice) = notice {
        integrate::notify(context, notice, cx);
    }
}

pub(super) fn advance_rebase_in_background(
    context: BranchContext,
    report: RebaseReport,
    action: RebaseAction,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = advance_rebase(&context, &report, action, cx).await {
                integrate::notify_failure(&context, "Rebase failed", &error, cx);
            }
        })
        .detach();
}

async fn advance_rebase(
    context: &BranchContext,
    report: &RebaseReport,
    action: RebaseAction,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    if matches!(action, RebaseAction::Continue)
        && cx.update(|_, app| has_unmerged_paths(context.repository.read(app)))?
    {
        integrate::notify(
            context,
            integrate::unmerged_files_notice(
                context,
                "Unresolved conflicts",
                "Resolve all conflicts before continuing the rebase.",
            ),
            cx,
        );
        return Ok(());
    }
    match rebase_once(context, action, cx).await {
        Ok(outcome) => finish_rebase(context, report, outcome, cx).await,
        Err(error) => {
            report_rebase_failure(context, &error, cx);
            Ok(())
        }
    }
}

pub(super) fn abort_merge(
    context: BranchContext,
    stashed: bool,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = abort_merge_flow(&context, stashed, cx).await {
                integrate::notify_failure(&context, "Could Not Abort Merge", &error, cx);
            }
        })
        .detach();
}

async fn abort_merge_flow(
    context: &BranchContext,
    stashed: bool,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.merge_abort());
    receiver.await??;
    integrate::restore_stash(context, stashed, cx).await;
    integrate::notify(context, BranchNotice::info("Merge aborted"), cx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_in_progress_offers_abort_continue_and_skip_in_that_order() {
        assert_eq!(
            ongoing_actions(Some(RepositoryOperation::Rebase)),
            vec![
                OngoingOperationAction::AbortRebase,
                OngoingOperationAction::ContinueRebase,
                OngoingOperationAction::SkipCommit,
            ]
        );
    }

    #[test]
    fn merge_in_progress_offers_only_abort_merge() {
        assert_eq!(
            ongoing_actions(Some(RepositoryOperation::Merge)),
            vec![OngoingOperationAction::AbortMerge]
        );
    }

    #[test]
    fn no_ongoing_actions_without_a_rebase_or_merge() {
        assert!(ongoing_actions(None).is_empty());
        assert!(ongoing_actions(Some(RepositoryOperation::CherryPick)).is_empty());
        assert!(ongoing_actions(Some(RepositoryOperation::Revert)).is_empty());
    }

    #[test]
    fn ongoing_action_labels_match_the_intellij_menu_entries() {
        assert_eq!(OngoingOperationAction::AbortRebase.label(), "Abort Rebase");
        assert_eq!(
            OngoingOperationAction::ContinueRebase.label(),
            "Continue Rebase"
        );
        assert_eq!(OngoingOperationAction::SkipCommit.label(), "Skip Commit");
        assert_eq!(OngoingOperationAction::AbortMerge.label(), "Abort Merge");
    }

    #[test]
    fn rebase_success_message_names_both_branches_when_known() {
        assert_eq!(
            rebase_success_message(Some("feature"), Some("origin/main")),
            "Rebased feature on origin/main"
        );
        assert_eq!(
            rebase_success_message(Some("feature"), None),
            "Rebased feature"
        );
        assert_eq!(
            rebase_success_message(None, Some("main")),
            "Rebase completed"
        );
    }

    #[test]
    fn rebase_conflict_message_mentions_the_stash_only_when_changes_were_stashed() {
        assert!(!rebase_conflict_message(false).contains("stashed"));
        assert!(rebase_conflict_message(true).contains("Local changes were stashed"));
    }
}
