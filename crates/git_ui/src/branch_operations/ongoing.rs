use anyhow::Result;
use git::repository::{
    CherryPickOutcome, GitFailureKind, RebaseAction, RebaseOutcome, RepositoryOperation,
    UnmergedEntry,
};
use gpui::{Action as _, App, AsyncWindowContext, PromptLevel, Window};
use util::ResultExt as _;

use super::integrate;
use super::smart_operation::{RestoreLabels, rebase_stash_message, stash_local_changes};
use super::{BranchContext, BranchNotice};
use crate::merge_tool::conflict_resolution::{
    ConflictParams, RebaseCustomizerSpec, RebaseUpstream, ResolveMode, ResolverBehavior,
    retain_entries_on_disk,
};

const ABORT_LABEL: &str = "Abort";
const CANCEL_LABEL: &str = "Cancel";
const ABORT_REBASE_TITLE: &str = "Abort Rebase";
const ABORT_MERGE_TITLE: &str = "Abort Merge";
const REBASE_ABORTED_MESSAGE: &str = "Abort rebase succeeded";
const MERGE_ABORTED_MESSAGE: &str = "Merge abort succeeded";
const REBASE_ABORT_FAILED_TITLE: &str = "Abort rebase failed";
const MERGE_ABORT_FAILED_TITLE: &str = "Merge abort failed";
const ALL_CONFLICTS_RESOLVED_TITLE: &str = "Resolve Conflicts";
const ALL_CONFLICTS_RESOLVED_MESSAGE: &str =
    "All conflicts have been resolved. Do you want to continue rebase?";
const CONTINUE_REBASE_LABEL: &str = "Continue Rebase";
const REBASE_RESTORE_OPERATION_TITLE: &str = "rebase";
const MERGE_RESTORE_OPERATION_TITLE: &str = "merge";
const MERGE_RESTORE_DESTINATION_NAME: &str = "HEAD";
const STASHED_BEFORE_REBASE_NOTE: &str = "Local changes were stashed before rebase.";
const VIEW_STASH_LABEL: &str = "View Stash…";
const CHERRY_PICK_CONTINUE_PROGRESS_TITLE: &str = "Continuing cherry-pick\u{2026}";
const CHERRY_PICK_CONTINUE_SUCCESS_TITLE: &str = "Cherry-pick continue successful";
const CHERRY_PICK_CONTINUE_FAILED_TITLE: &str = "Continue cherry-pick failed";
const CHERRY_PICK_PERFORMED_WITH_CONFLICTS_TITLE: &str = "Cherry-pick was performed with conflicts";
const UNRESOLVED_CONFLICTS_TEXT: &str = "There are unresolved conflicts in the working tree.";
const SHORT_SHA_LENGTH: usize = 8;
const APPLIED_COMMIT_LIMIT: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OngoingOperationAction {
    AbortRebase,
    ContinueRebase,
    SkipCommit,
    AbortMerge,
    AbortCherryPick,
    ContinueCherryPick,
    AbortRevert,
}

impl OngoingOperationAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::AbortRebase => "Abort Rebase",
            Self::ContinueRebase => "Continue Rebase",
            Self::SkipCommit => "Skip Commit",
            Self::AbortMerge => "Abort Merge",
            Self::AbortCherryPick => "Abort Cherry-Pick",
            Self::ContinueCherryPick => "Continue Cherry-Pick",
            Self::AbortRevert => "Abort Revert",
        }
    }
}

/// The visible part of IntelliJ's `Git.Ongoing.Rebase.Actions` group, in its
/// order: Abort Rebase, Abort Merge, Abort Cherry-Pick, Continue Cherry-Pick,
/// Abort Revert, Continue Rebase, Skip Commit.
pub fn ongoing_actions(operation: Option<RepositoryOperation>) -> Vec<OngoingOperationAction> {
    match operation {
        Some(RepositoryOperation::Rebase) => vec![
            OngoingOperationAction::AbortRebase,
            OngoingOperationAction::ContinueRebase,
            OngoingOperationAction::SkipCommit,
        ],
        Some(RepositoryOperation::Merge) => vec![OngoingOperationAction::AbortMerge],
        Some(RepositoryOperation::CherryPick) => vec![
            OngoingOperationAction::AbortCherryPick,
            OngoingOperationAction::ContinueCherryPick,
        ],
        Some(RepositoryOperation::Revert) => vec![OngoingOperationAction::AbortRevert],
        None => Vec::new(),
    }
}

/// The names IntelliJ's `GitAbortOperationAction` uses in its texts: the
/// lowercase operation name and its capitalized form.
fn abort_operation_names(operation: RepositoryOperation) -> (&'static str, &'static str) {
    match operation {
        RepositoryOperation::Merge => ("merge", "Merge"),
        RepositoryOperation::Rebase => ("rebase", "Rebase"),
        RepositoryOperation::CherryPick => ("cherry-pick", "Cherry-Pick"),
        RepositoryOperation::Revert => ("revert", "Revert"),
    }
}

fn abort_progress_title(operation: RepositoryOperation) -> String {
    format!("Aborting {} Process", abort_operation_names(operation).1)
}

fn abort_dialog_title(operation: RepositoryOperation) -> String {
    format!("Abort {}", abort_operation_names(operation).1)
}

fn abort_dialog_message(operation: RepositoryOperation, mention: &str) -> String {
    format!("Abort {}{mention}?", abort_operation_names(operation).0)
}

fn abort_succeeded_message(operation: RepositoryOperation) -> String {
    format!("{} abort succeeded", abort_operation_names(operation).1)
}

fn abort_failed_title(operation: RepositoryOperation) -> String {
    format!("{} abort failed", abort_operation_names(operation).1)
}

fn smart_stash_present(context: &BranchContext, cx: &App) -> bool {
    let repository = context.repository.read(cx);
    repository
        .cached_stash()
        .entries
        .first()
        .is_some_and(|entry| super::smart_operation::is_operation_stash(repository, entry, cx))
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
        OngoingOperationAction::AbortRebase => abort_rebase(context, report, window, cx),
        OngoingOperationAction::ContinueRebase => {
            advance_rebase_in_background(context, report, RebaseAction::Continue, window, cx)
        }
        OngoingOperationAction::SkipCommit => {
            advance_rebase_in_background(context, report, RebaseAction::Skip, window, cx)
        }
        OngoingOperationAction::AbortMerge => abort_merge(context, stashed, window, cx),
        OngoingOperationAction::AbortCherryPick => {
            abort_operation(context, RepositoryOperation::CherryPick, window, cx)
        }
        OngoingOperationAction::AbortRevert => {
            abort_operation(context, RepositoryOperation::Revert, window, cx)
        }
        OngoingOperationAction::ContinueCherryPick => continue_cherry_pick(context, window, cx),
    }
}

fn abort_operation(
    context: BranchContext,
    operation: RepositoryOperation,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = abort_operation_flow(&context, operation, cx).await {
                integrate::notify_failure(&context, abort_failed_title(operation), &error, cx);
            }
        })
        .detach();
}

async fn abort_operation_flow(
    context: &BranchContext,
    operation: RepositoryOperation,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let mention = cx.update(|_, app| integrate::repository_mention(context, app))?;
    let message = abort_dialog_message(operation, &mention);
    let title = abort_dialog_title(operation);
    let answer = cx.prompt(
        PromptLevel::Info,
        &title,
        Some(message.as_str()),
        &[ABORT_LABEL, CANCEL_LABEL],
    );
    if !matches!(answer.await, Ok(0)) {
        return Ok(());
    }
    let progress =
        cx.update(|_, app| context.start_progress(abort_progress_title(operation), app))?;
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.abort_operation(operation));
    let result = receiver.await?;
    drop(progress);
    result?;
    integrate::notify(
        context,
        BranchNotice::info(abort_succeeded_message(operation)),
        cx,
    );
    Ok(())
}

fn continue_cherry_pick(context: BranchContext, window: &mut Window, cx: &mut App) {
    let progress = context.start_progress(CHERRY_PICK_CONTINUE_PROGRESS_TITLE, cx);
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = continue_cherry_pick_flow(&context, cx).await {
                integrate::notify_failure(&context, CHERRY_PICK_CONTINUE_FAILED_TITLE, &error, cx);
            }
            drop(progress);
        })
        .detach();
}

pub(super) fn applied_commits_message(commits: &[(String, String)]) -> String {
    commits
        .iter()
        .map(|(sha, subject)| {
            let short_sha = sha.get(..SHORT_SHA_LENGTH).unwrap_or(sha);
            format!("{short_sha} {subject}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn continue_cherry_pick_flow(
    context: &BranchContext,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let head_before = context
        .repository
        .update(cx, |repository, _| repository.head_sha())
        .await??;
    loop {
        if conflicts_remain(context, cx).await {
            let resolution = cx.update(|window, app| {
                context.resolve_conflicts(
                    ConflictParams::for_unmerged_files(),
                    ResolverBehavior::Plain,
                    ResolveMode::Initial,
                    window,
                    app,
                )
            })?;
            if !resolution.await.proceed {
                let notice = BranchNotice::warning(UNRESOLVED_CONFLICTS_TEXT)
                    .title(CHERRY_PICK_PERFORMED_WITH_CONFLICTS_TITLE);
                integrate::notify(context, notice, cx);
                return Ok(());
            }
        }
        let receiver = context
            .repository
            .update(cx, |repository, _| repository.continue_cherry_pick());
        match receiver.await?? {
            CherryPickOutcome::Conflicted { .. } => continue,
            CherryPickOutcome::Completed { .. } | CherryPickOutcome::Empty { .. } => break,
        }
    }
    let applied = match head_before {
        Some(head_before) => {
            let receiver = context.repository.update(cx, |repository, _| {
                repository.commits_between(head_before, "HEAD".to_string(), APPLIED_COMMIT_LIMIT)
            });
            receiver
                .await?
                .log_err()
                .unwrap_or_default()
                .into_iter()
                .map(|commit| (commit.sha.to_string(), commit.subject.to_string()))
                .collect::<Vec<_>>()
        }
        None => Vec::new(),
    };
    integrate::notify(
        context,
        BranchNotice::info(applied_commits_message(&applied))
            .title(CHERRY_PICK_CONTINUE_SUCCESS_TITLE),
        cx,
    );
    Ok(())
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

fn rebase_restore_labels(report: &RebaseReport) -> RestoreLabels {
    if report.update.is_some() {
        return RestoreLabels::update();
    }
    RestoreLabels::new(
        REBASE_RESTORE_OPERATION_TITLE,
        report.onto.clone().unwrap_or_default(),
    )
}

pub(super) fn rebase_conflict_message(stashed: bool) -> String {
    if stashed {
        String::from(STASHED_BEFORE_REBASE_NOTE)
    } else {
        String::new()
    }
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
        .update(|_, app| stash_local_changes(context, rebase_stash_message(), app))?
        .await?;
    let action = RebaseAction::Start {
        upstream,
        branch: None,
    };
    match rebase_once(context, action, cx).await {
        Ok(outcome) => finish_rebase(context, &report, outcome, cx).await,
        Err(error) => {
            integrate::restore_stash(context, report.stashed, rebase_restore_labels(&report), cx)
                .await;
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
        RebaseOutcome::Completed { .. } => complete_rebase(context, report, cx).await,
        RebaseOutcome::Conflicted { .. } => resolve_stopped_rebase(context, report, cx).await?,
        RebaseOutcome::Aborted => {
            integrate::restore_stash(context, report.stashed, rebase_restore_labels(report), cx)
                .await;
            integrate::notify(context, BranchNotice::info(REBASE_ABORTED_MESSAGE), cx);
        }
    }
    Ok(())
}

async fn complete_rebase(
    context: &BranchContext,
    report: &RebaseReport,
    cx: &mut AsyncWindowContext,
) {
    integrate::restore_stash(context, report.stashed, rebase_restore_labels(report), cx).await;
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

fn rebase_conflict_params(report: &RebaseReport) -> ConflictParams {
    if report.update.is_some() {
        return ConflictParams::for_update_by_rebase();
    }
    ConflictParams::for_rebase_process(RebaseCustomizerSpec {
        upstream: report.onto.as_deref().map(RebaseUpstream::from_ref_string),
        branch: None,
        initial_branch: report.branch.clone(),
    })
}

pub(super) async fn conflicts_remain(context: &BranchContext, cx: &mut AsyncWindowContext) -> bool {
    match unmerged_files_on_disk(context, cx).await {
        Ok(entries) => !entries.is_empty(),
        Err(error) => {
            log::warn!("could not check the repository for unmerged paths: {error:#}");
            true
        }
    }
}

async fn unmerged_files_on_disk(
    context: &BranchContext,
    cx: &mut AsyncWindowContext,
) -> Result<Vec<UnmergedEntry>> {
    let listing = context
        .repository
        .update(cx, |repository, cx| repository.unmerged_entries(cx));
    let entries = listing.await?;
    let fs = context.workspace.read_with(cx, |workspace, cx| {
        workspace.project().read(cx).fs().clone()
    })?;
    let absolute_paths: Vec<_> = context.repository.read_with(cx, |repository, _| {
        entries
            .iter()
            .map(|entry| repository.repo_path_to_abs_path(&entry.path))
            .collect()
    });
    retain_entries_on_disk(fs.as_ref(), entries, &absolute_paths).await
}

pub(super) async fn resolve_stopped_rebase(
    context: &BranchContext,
    report: &RebaseReport,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let resolution = cx.update(|window, app| {
        context.resolve_conflicts(
            rebase_conflict_params(report),
            ResolverBehavior::ContinueRebase,
            ResolveMode::Initial,
            window,
            app,
        )
    })?;
    if resolution.await.proceed {
        complete_rebase(context, report, cx).await;
    } else if report.update.is_none() && conflicts_remain(context, cx).await {
        integrate::notify(context, rebase_conflict_notice(context, report), cx);
    }
    Ok(())
}

async fn resolve_rebase_from_notification(
    context: &BranchContext,
    report: &RebaseReport,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    if conflicts_remain(context, cx).await {
        return resolve_stopped_rebase(context, report, cx).await;
    }
    let answer = cx.prompt(
        PromptLevel::Info,
        ALL_CONFLICTS_RESOLVED_TITLE,
        Some(ALL_CONFLICTS_RESOLVED_MESSAGE),
        &[CONTINUE_REBASE_LABEL, CANCEL_LABEL],
    );
    if matches!(answer.await, Ok(0)) {
        advance_rebase(context, report, RebaseAction::Continue, cx).await?;
    }
    Ok(())
}

pub(super) fn resolve_rebase_action(
    context: BranchContext,
    report: RebaseReport,
) -> impl Fn(&mut Window, &mut App) + 'static {
    move |window, cx| {
        let context = context.clone();
        let report = RebaseReport {
            stashed: report.stashed || smart_stash_present(&context, cx),
            ..report.clone()
        };
        window
            .spawn(cx, async move |cx| {
                if let Err(error) = resolve_rebase_from_notification(&context, &report, cx).await {
                    integrate::notify_failure(&context, "Rebase failed", &error, cx);
                }
            })
            .detach();
    }
}

pub(super) fn abort_rebase(
    context: BranchContext,
    report: RebaseReport,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = abort_rebase_flow(&context, &report, cx).await {
                integrate::notify_failure(&context, REBASE_ABORT_FAILED_TITLE, &error, cx);
            }
        })
        .detach();
}

async fn abort_rebase_flow(
    context: &BranchContext,
    report: &RebaseReport,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let repository = integrate::repository_name(context, cx)?;
    let message = format!("Abort rebase in {repository}?");
    let answer = cx.prompt(
        PromptLevel::Info,
        ABORT_REBASE_TITLE,
        Some(message.as_str()),
        &[ABORT_LABEL, CANCEL_LABEL],
    );
    if matches!(answer.await, Ok(0)) {
        advance_rebase(context, report, RebaseAction::Abort, cx).await?;
    }
    Ok(())
}

pub(super) fn rebase_conflict_notice(
    context: &BranchContext,
    report: &RebaseReport,
) -> BranchNotice {
    let continue_context = context.clone();
    let continue_report = report.clone();
    let abort_context = context.clone();
    let abort_report = report.clone();
    let notice = BranchNotice::warning(rebase_conflict_message(report.stashed))
        .title("Rebase stopped due to conflicts")
        .action(
            "Resolve…",
            resolve_rebase_action(context.clone(), report.clone()),
        )
        .action("Continue", move |window, cx| {
            advance_rebase_in_background(
                continue_context.clone(),
                continue_report.clone(),
                RebaseAction::Continue,
                window,
                cx,
            )
        })
        .action(ABORT_LABEL, move |window, cx| {
            abort_rebase(abort_context.clone(), abort_report.clone(), window, cx)
        });
    if report.stashed {
        notice.action(VIEW_STASH_LABEL, |window, cx| {
            window.dispatch_action(zed_actions::git::ViewStash.boxed_clone(), cx)
        })
    } else {
        notice
    }
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
    let aborting = matches!(action, RebaseAction::Abort);
    match rebase_once(context, action, cx).await {
        Ok(outcome) => finish_rebase(context, report, outcome, cx).await,
        Err(error) if aborting => {
            integrate::notify_failure(context, REBASE_ABORT_FAILED_TITLE, &error, cx);
            Ok(())
        }
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
                integrate::notify_failure(&context, MERGE_ABORT_FAILED_TITLE, &error, cx);
            }
        })
        .detach();
}

async fn abort_merge_flow(
    context: &BranchContext,
    stashed: bool,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let mention = cx.update(|_, app| integrate::repository_mention(context, app))?;
    let message = format!("Abort merge{mention}?");
    let answer = cx.prompt(
        PromptLevel::Info,
        ABORT_MERGE_TITLE,
        Some(message.as_str()),
        &[ABORT_LABEL, CANCEL_LABEL],
    );
    if !matches!(answer.await, Ok(0)) {
        return Ok(());
    }
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.merge_abort());
    receiver.await??;
    integrate::restore_stash(
        context,
        stashed,
        RestoreLabels::new(
            MERGE_RESTORE_OPERATION_TITLE,
            MERGE_RESTORE_DESTINATION_NAME,
        ),
        cx,
    )
    .await;
    integrate::notify(context, BranchNotice::info(MERGE_ABORTED_MESSAGE), cx);
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
    fn no_ongoing_actions_without_an_operation_in_progress() {
        assert!(ongoing_actions(None).is_empty());
    }

    #[test]
    fn cherry_pick_and_revert_offer_the_intellij_abort_and_continue_actions() {
        assert_eq!(
            ongoing_actions(Some(RepositoryOperation::CherryPick)),
            vec![
                OngoingOperationAction::AbortCherryPick,
                OngoingOperationAction::ContinueCherryPick,
            ]
        );
        assert_eq!(
            ongoing_actions(Some(RepositoryOperation::Revert)),
            vec![OngoingOperationAction::AbortRevert]
        );
        assert_eq!(
            OngoingOperationAction::AbortCherryPick.label(),
            "Abort Cherry-Pick"
        );
        assert_eq!(
            OngoingOperationAction::ContinueCherryPick.label(),
            "Continue Cherry-Pick"
        );
        assert_eq!(OngoingOperationAction::AbortRevert.label(), "Abort Revert");
    }

    #[test]
    fn abort_texts_follow_the_intellij_abort_operation_action() {
        assert_eq!(
            abort_dialog_title(RepositoryOperation::CherryPick),
            "Abort Cherry-Pick"
        );
        assert_eq!(
            abort_dialog_message(RepositoryOperation::CherryPick, ""),
            "Abort cherry-pick?"
        );
        assert_eq!(
            abort_dialog_message(RepositoryOperation::Revert, " in flint"),
            "Abort revert in flint?"
        );
        assert_eq!(
            abort_succeeded_message(RepositoryOperation::Revert),
            "Revert abort succeeded"
        );
        assert_eq!(
            abort_failed_title(RepositoryOperation::CherryPick),
            "Cherry-Pick abort failed"
        );
        assert_eq!(
            abort_progress_title(RepositoryOperation::CherryPick),
            "Aborting Cherry-Pick Process"
        );
    }

    #[test]
    fn applied_commits_are_listed_with_short_hashes() {
        assert_eq!(
            applied_commits_message(&[
                ("0123456789abcdef".to_string(), "first".to_string()),
                ("abc".to_string(), "second".to_string()),
            ]),
            "01234567 first\nabc second"
        );
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
    fn rebase_conflict_message_is_empty_unless_changes_were_stashed() {
        assert_eq!(rebase_conflict_message(false), "");
        assert_eq!(
            rebase_conflict_message(true),
            "Local changes were stashed before rebase."
        );
    }
}
