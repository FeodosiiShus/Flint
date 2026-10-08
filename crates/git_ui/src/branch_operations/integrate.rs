use std::path::Path;

use anyhow::Result;
use db::kvp::KeyValueStore;
use git::repository::{
    Branch, FetchOptions, GitFailure, GitFailureKind, MergeOutcome, Remote, RepositoryOperation,
};
use git::status::DiffTreeType;
use gpui::{App, AsyncWindowContext, PromptLevel, SharedString, Window};
use util::ResultExt as _;

use super::ongoing::{self, RebaseReport, UpdateProgress};
use super::smart_operation::{
    SmartChoice, SmartOperation, confirm_smart_operation, restore_local_changes,
    stash_local_changes,
};
use super::update_dialog::{ResetTarget, UpdateOptionsDialog};
use super::{BranchContext, BranchNotice, error_notice, quoted_ref_label};
use crate::branch_refs::{RefKind, RefTarget};
use crate::merge_tool::has_unmerged_paths;

const UPDATE_METHOD_KEY: &str = "git_update_method";
const SHOW_UPDATE_OPTIONS_KEY: &str = "git_update_show_options";
const COMMIT_COUNT_LIMIT: usize = 10_000;
const PROTECTED_BRANCH_NAMES: [&str; 2] = ["main", "master"];
const DETACHED_HEAD_LABEL: &str = "HEAD";
const PUBLISHED_COMMIT_DETAIL: &str = "You are trying to rebase some commits already pushed to a protected branch.\n\nRebasing them would duplicate commits, which is not recommended and most likely unwanted.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateMethod {
    #[default]
    Merge,
    Rebase,
    BranchDefault,
}

impl UpdateMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::BranchDefault => "Branch Default",
        }
    }

    fn stored_value(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::BranchDefault => "branch_default",
        }
    }

    fn from_stored(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("rebase") => Self::Rebase,
            Some("branch_default") => Self::BranchDefault,
            _ => Self::Merge,
        }
    }
}

pub fn saved_update_method(cx: &App) -> UpdateMethod {
    let stored = KeyValueStore::global(cx)
        .read_kvp(UPDATE_METHOD_KEY)
        .log_err()
        .flatten();
    UpdateMethod::from_stored(stored.as_deref())
}

pub fn save_update_method(method: UpdateMethod, cx: &App) {
    write_preference(UPDATE_METHOD_KEY, method.stored_value().to_string(), cx);
}

pub fn should_show_update_options(cx: &App) -> bool {
    let stored = KeyValueStore::global(cx)
        .read_kvp(SHOW_UPDATE_OPTIONS_KEY)
        .log_err()
        .flatten();
    stored.as_deref().map(str::trim) != Some("false")
}

pub fn set_show_update_options(show: bool, cx: &App) {
    write_preference(SHOW_UPDATE_OPTIONS_KEY, show.to_string(), cx);
}

fn write_preference(key: &'static str, value: String, cx: &App) {
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store.write_kvp(key.to_string(), value).await
    });
}

pub fn merge_into_current(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let failure_title = format!("Could Not Merge {}", target.name);
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = merge_flow(&context, &target, cx).await {
                notify_failure(&context, failure_title, &error, cx);
            }
        })
        .detach();
}

pub fn rebase_current_onto(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = rebase_flow(&context, &target, cx).await {
                notify_failure(&context, "Rebase failed", &error, cx);
            }
        })
        .detach();
}

pub fn pull_into_current(
    context: BranchContext,
    target: RefTarget,
    rebase: bool,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = pull_flow(&context, &target, rebase, cx).await {
                notify_failure(&context, "Update failed", &error, cx);
            }
        })
        .detach();
}

pub fn update_branch(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = update_branch_flow(&context, &target, cx).await {
                notify_failure(&context, "Update failed", &error, cx);
            }
        })
        .detach();
}

pub fn update_project_label(show_options: bool) -> &'static str {
    if show_options {
        "Update Project…"
    } else {
        "Update Project"
    }
}

pub fn update_project(context: BranchContext, window: &mut Window, cx: &mut App) {
    if window.modifiers().shift || should_show_update_options(cx) {
        update_project_options(context, window, cx);
    } else {
        let method = saved_update_method(cx);
        update_current_branch(context, method, window, cx);
    }
}

pub fn update_project_options(context: BranchContext, window: &mut Window, cx: &mut App) {
    let method = saved_update_method(cx);
    let reset_target = reset_target(&context, cx);
    let dialog_context = context.clone();
    context.open_modal(window, cx, move |window, cx| {
        UpdateOptionsDialog::new(dialog_context, method, reset_target, window, cx)
    });
}

pub fn update_current_branch(
    context: BranchContext,
    method: UpdateMethod,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = update_current_flow(&context, method, cx).await {
                notify_failure(&context, "Update failed", &error, cx);
            }
        })
        .detach();
}

pub fn reset_to_remote_branch(context: BranchContext, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = reset_flow(&context, cx).await {
                notify_failure(&context, "Reset failed", &error, cx);
            }
        })
        .detach();
}

fn reset_target(context: &BranchContext, cx: &App) -> Option<ResetTarget> {
    let branch = context.current_branch(cx)?;
    let tracked = tracked_upstream(&branch)?;
    Some(ResetTarget {
        branch: SharedString::from(branch.name().to_string()),
        upstream: SharedString::from(tracked.reference),
    })
}

struct TrackedUpstream {
    remote: String,
    branch_name: String,
    reference: String,
}

fn tracked_upstream(branch: &Branch) -> Option<TrackedUpstream> {
    let upstream = branch.upstream.as_ref()?;
    Some(TrackedUpstream {
        remote: upstream.remote_name()?.to_string(),
        branch_name: upstream.branch_name()?.to_string(),
        reference: upstream.stripped_ref_name()?.to_string(),
    })
}

struct UpdateRequest {
    branch: String,
    upstream: String,
    remote: String,
    rebase: bool,
}

pub(super) fn git_failure(error: &anyhow::Error) -> Option<&GitFailure> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
}

fn overwritten_files(error: &anyhow::Error) -> Option<Vec<String>> {
    match git_failure(error).map(|failure| &failure.kind) {
        Some(GitFailureKind::LocalChangesWouldBeOverwritten { files }) => Some(files.clone()),
        _ => None,
    }
}

pub(super) fn failure_notice(
    title: impl Into<SharedString>,
    error: &anyhow::Error,
) -> Option<BranchNotice> {
    error_notice("", error).map(|notice| notice.title(title))
}

pub(super) fn notify(context: &BranchContext, notice: BranchNotice, cx: &mut AsyncWindowContext) {
    cx.update(|_, app| context.notify(notice, app)).log_err();
}

pub(super) fn notify_failure(
    context: &BranchContext,
    title: impl Into<SharedString>,
    error: &anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    if let Some(notice) = failure_notice(title, error) {
        notify(context, notice, cx);
    }
}

pub(super) fn untracked_files_message(operation: &str, files: &[String]) -> String {
    format!(
        "These untracked working tree files would be overwritten by {operation}:\n\n{}\n\nMove or remove the files and try again.",
        files.join("\n")
    )
}

pub(super) fn unmerged_files_notice(
    context: &BranchContext,
    title: &'static str,
    message: &'static str,
) -> BranchNotice {
    let context = context.clone();
    BranchNotice::error(message)
        .title(title)
        .action("Resolve…", move |window, cx| {
            context.open_conflicts_if_conflicted(window, cx)
        })
}

fn operation_name(operation: RepositoryOperation) -> &'static str {
    match operation {
        RepositoryOperation::Merge => "merge",
        RepositoryOperation::Rebase => "rebase",
        RepositoryOperation::CherryPick => "cherry-pick",
        RepositoryOperation::Revert => "revert",
    }
}

fn unfinished_operation_message(
    operation: RepositoryOperation,
    repository_name: &str,
    purpose: &str,
) -> String {
    format!(
        "There is an unfinished {} process in {repository_name}. You should complete or abort it before {purpose}.",
        operation_name(operation)
    )
}

fn blocking_notice(
    context: &BranchContext,
    operation: RepositoryOperation,
    title: &'static str,
    purpose: &str,
    repository_name: &str,
) -> BranchNotice {
    let mut notice = BranchNotice::error(unfinished_operation_message(
        operation,
        repository_name,
        purpose,
    ))
    .title(title);
    if matches!(
        operation,
        RepositoryOperation::Merge | RepositoryOperation::Rebase
    ) {
        let resolve_context = context.clone();
        notice = notice.action("Resolve…", move |window, cx| {
            resolve_context.open_conflicts_if_conflicted(window, cx)
        });
    }
    for action in ongoing::ongoing_actions(Some(operation)) {
        let action_context = context.clone();
        notice = notice.action(action.label(), move |window, cx| {
            ongoing::run_ongoing_action(action_context.clone(), action, window, cx)
        });
    }
    notice
}

fn repository_display_name(context: &BranchContext, cx: &App) -> String {
    context
        .repository
        .read(cx)
        .work_directory_abs_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the repository".to_string())
}

fn repository_name(context: &BranchContext, cx: &mut AsyncWindowContext) -> Result<String> {
    cx.update(|_, app| repository_display_name(context, app))
}

pub(crate) fn is_protected_branch_name(name: &str) -> bool {
    PROTECTED_BRANCH_NAMES.contains(&name)
}

pub(crate) fn is_protected_remote_reference(reference: &str) -> bool {
    reference
        .split_once('/')
        .is_some_and(|(_, branch)| is_protected_branch_name(branch))
}

fn should_prompt_for_published_commits(pushed: &[SharedString], rebased_commits: usize) -> bool {
    rebased_commits > 0
        && pushed
            .iter()
            .any(|reference| is_protected_remote_reference(reference))
}

fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

fn merge_success_message(reference: &str, current: &str) -> String {
    format!("Merged {reference} to {current}")
}

fn update_title(files: Option<usize>, commits: usize) -> String {
    let commits = pluralize(commits, "commit", "commits");
    match files {
        Some(files) => format!("{} updated in {commits}", pluralize(files, "file", "files")),
        None => format!("Updated in {commits}"),
    }
}

fn update_message(branch: Option<&str>, onto: Option<&str>) -> String {
    match (branch, onto) {
        (Some(branch), Some(onto)) => format!("Updated {branch} from {onto}"),
        (Some(branch), None) => format!("Updated {branch}"),
        (None, _) => "Updated the current branch".to_string(),
    }
}

fn fetched_targets_message(count: usize) -> String {
    format!("Fetched {}", pluralize(count, "target", "targets"))
}

fn reset_confirmation_message(branch: &str, upstream: &str) -> String {
    format!(
        "Do you want to reset local branch {branch} to {upstream}? Local commits will be dropped"
    )
}

fn parse_rebase_preference(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" | "merges" | "interactive" | "i" | "m" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

async fn read_git_config(directory: &Path, key: &str) -> Option<String> {
    let output = util::command::new_command("git")
        .args(["config", "--get", key])
        .current_dir(directory)
        .output()
        .await
        .log_err()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn configured_rebase_preference(directory: &Path, branch: &str) -> Option<bool> {
    let branch_key = format!("branch.{branch}.rebase");
    for key in [branch_key.as_str(), "pull.rebase"] {
        if let Some(preference) = read_git_config(directory, key)
            .await
            .and_then(|value| parse_rebase_preference(&value))
        {
            return Some(preference);
        }
    }
    None
}

async fn resolve_rebase(
    context: &BranchContext,
    method: UpdateMethod,
    branch: &str,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    match method {
        UpdateMethod::Merge => Ok(false),
        UpdateMethod::Rebase => Ok(true),
        UpdateMethod::BranchDefault => {
            let directory =
                cx.update(|_, app| context.repository.read(app).work_directory_abs_path.clone())?;
            Ok(configured_rebase_preference(&directory, branch)
                .await
                .unwrap_or(false))
        }
    }
}

async fn operation_in_progress(
    context: &BranchContext,
    cx: &mut AsyncWindowContext,
) -> Result<Option<RepositoryOperation>> {
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.operation_in_progress());
    receiver.await?
}

async fn unfinished_operation(
    context: &BranchContext,
    cx: &mut AsyncWindowContext,
) -> Option<RepositoryOperation> {
    operation_in_progress(context, cx).await.log_err().flatten()
}

async fn count_commits(
    context: &BranchContext,
    base: &str,
    head: &str,
    cx: &mut AsyncWindowContext,
) -> Result<usize> {
    let (base, head) = (base.to_string(), head.to_string());
    let receiver = context.repository.update(cx, |repository, _| {
        repository.commits_between(base, head, COMMIT_COUNT_LIMIT)
    });
    Ok(receiver.await??.len())
}

async fn head_sha(context: &BranchContext, cx: &mut AsyncWindowContext) -> Result<Option<String>> {
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.head_sha());
    receiver.await?
}

async fn updated_file_count(
    context: &BranchContext,
    before: Option<&str>,
    cx: &mut AsyncWindowContext,
) -> Result<usize> {
    let after = head_sha(context, cx).await?;
    let (Some(before), Some(after)) = (before, after) else {
        return Ok(0);
    };
    if before == after {
        return Ok(0);
    }
    let diff_type = DiffTreeType::Since {
        base: SharedString::from(before.to_string()),
        head: SharedString::from(after),
    };
    let receiver = context
        .repository
        .update(cx, |repository, cx| repository.diff_tree(diff_type, cx));
    Ok(receiver.await??.entries.len())
}

pub(super) async fn update_notice(
    context: &BranchContext,
    branch: Option<&str>,
    onto: Option<&str>,
    progress: &UpdateProgress,
    cx: &mut AsyncWindowContext,
) -> BranchNotice {
    let files = updated_file_count(context, progress.head_before.as_deref(), cx)
        .await
        .log_err();
    BranchNotice::info(update_message(branch, onto)).title(update_title(files, progress.commits))
}

async fn pushed_remote_branches(
    context: &BranchContext,
    cx: &mut AsyncWindowContext,
) -> Result<Vec<SharedString>> {
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.check_for_pushed_commits());
    receiver.await?
}

async fn confirm_published_rebase(
    context: &BranchContext,
    onto: &str,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    let rebased_commits = count_commits(context, onto, "HEAD", cx).await?;
    if rebased_commits == 0 {
        return Ok(true);
    }
    let pushed = pushed_remote_branches(context, cx).await?;
    if !should_prompt_for_published_commits(&pushed, rebased_commits) {
        return Ok(true);
    }
    let answer = cx.prompt(
        PromptLevel::Warning,
        "Rebasing Published Commit",
        Some(PUBLISHED_COMMIT_DETAIL),
        &["Rebase Anyway", "Cancel"],
    );
    Ok(matches!(answer.await, Ok(0)))
}

async fn fetch_remote(
    context: &BranchContext,
    remote: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let askpass = cx.update(|window, app| {
        context.askpass_delegate(format!("git fetch {remote}"), window, app)
    })?;
    let remote = Remote {
        name: SharedString::from(remote.to_string()),
    };
    let receiver = context.repository.update(cx, |repository, cx| {
        repository.fetch(FetchOptions::Remote(remote), askpass, cx)
    });
    receiver.await??;
    Ok(())
}

async fn merge_once(
    context: &BranchContext,
    reference: &str,
    cx: &mut AsyncWindowContext,
) -> Result<MergeOutcome> {
    let reference = reference.to_string();
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.merge(reference));
    receiver.await?
}

async fn reset_hard(
    context: &BranchContext,
    commit: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let commit = commit.to_string();
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.reset_hard(commit));
    receiver.await?
}

pub(super) async fn restore_stash(
    context: &BranchContext,
    stashed: bool,
    cx: &mut AsyncWindowContext,
) {
    if !stashed {
        return;
    }
    let restoring = cx.update(|window, app| restore_local_changes(context, true, window, app));
    match restoring {
        Ok(task) => {
            if let Err(error) = task.await {
                notify_failure(context, "Could Not Restore Local Changes", &error, cx);
            }
        }
        Err(error) => {
            log::error!("could not restore stashed local changes: {error:#}");
        }
    }
}

enum MergeAttempt {
    Finished {
        outcome: MergeOutcome,
        stashed: bool,
    },
    Cancelled,
    Failed(anyhow::Error),
}

async fn merge_with_smart_stash(
    context: &BranchContext,
    reference: &str,
    cx: &mut AsyncWindowContext,
) -> Result<MergeAttempt> {
    let first_error = match merge_once(context, reference, cx).await {
        Ok(outcome) => {
            return Ok(MergeAttempt::Finished {
                outcome,
                stashed: false,
            });
        }
        Err(error) => error,
    };
    let Some(files) = overwritten_files(&first_error) else {
        return Ok(MergeAttempt::Failed(first_error));
    };
    let choice = cx
        .update(|window, app| {
            confirm_smart_operation(context, SmartOperation::Merge, files, window, app)
        })?
        .await;
    if !matches!(choice, SmartChoice::Smart) {
        return Ok(MergeAttempt::Cancelled);
    }
    let stashed = cx
        .update(|_, app| stash_local_changes(context, app))?
        .await?;
    match merge_once(context, reference, cx).await {
        Ok(outcome) => Ok(MergeAttempt::Finished { outcome, stashed }),
        Err(error) => {
            restore_stash(context, stashed, cx).await;
            Ok(MergeAttempt::Failed(error))
        }
    }
}

fn report_merge_failure(
    context: &BranchContext,
    reference: &str,
    error: &anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    let notice = match git_failure(error).map(|failure| &failure.kind) {
        Some(GitFailureKind::UnmergedFiles) => Some(unmerged_files_notice(
            context,
            "Cannot merge because of unmerged files",
            "Resolve the conflicts before merging.",
        )),
        Some(GitFailureKind::UntrackedFilesWouldBeOverwritten { files }) => Some(
            BranchNotice::error(untracked_files_message("merge", files))
                .title("Untracked Files Prevent Merge"),
        ),
        _ => failure_notice(format!("Could Not Merge {reference}"), error),
    };
    if let Some(notice) = notice {
        notify(context, notice, cx);
    }
}

fn merge_conflict_notice(context: &BranchContext, reference: &str, stashed: bool) -> BranchNotice {
    let mut message = String::from("Resolve the conflicts to complete the merge, or abort it.");
    if stashed {
        message.push_str(" Local changes were stashed before the merge.");
    }
    let resolve_context = context.clone();
    let abort_context = context.clone();
    BranchNotice::warning(message)
        .title(format!("{reference} Merged with Conflicts"))
        .action("Resolve…", move |window, cx| {
            resolve_context.open_conflicts_if_conflicted(window, cx)
        })
        .action("Abort", move |window, cx| {
            ongoing::abort_merge(abort_context.clone(), stashed, window, cx)
        })
}

fn merged_notice(context: &BranchContext, target: &RefTarget, current: &str) -> BranchNotice {
    let mut notice = BranchNotice::info(merge_success_message(&target.name, current));
    if target.kind == RefKind::Local && !is_protected_branch_name(&target.name) {
        let delete_context = context.clone();
        let delete_target = target.clone();
        notice = notice.action(
            format!("Delete {}", quoted_ref_label(&target.name)),
            move |window, cx| {
                super::manage::delete_ref(delete_context.clone(), delete_target.clone(), window, cx)
            },
        );
    }
    notice
}

async fn merge_flow(
    context: &BranchContext,
    target: &RefTarget,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let current = cx
        .update(|_, app| context.current_branch_name(app))?
        .map(|name| name.to_string())
        .unwrap_or_else(|| DETACHED_HEAD_LABEL.to_string());
    let reference = target.name.to_string();

    if let Some(operation) = unfinished_operation(context, cx).await {
        let name = repository_name(context, cx)?;
        let notice = blocking_notice(context, operation, "Cannot Merge", "merging", &name);
        notify(context, notice, cx);
        return Ok(());
    }

    match merge_with_smart_stash(context, &reference, cx).await? {
        MergeAttempt::Cancelled => {}
        MergeAttempt::Failed(error) => report_merge_failure(context, &reference, &error, cx),
        MergeAttempt::Finished { outcome, stashed } => match outcome {
            MergeOutcome::AlreadyUpToDate => {
                restore_stash(context, stashed, cx).await;
                notify(context, BranchNotice::info("Already up to date"), cx);
            }
            MergeOutcome::Merged { .. } => {
                restore_stash(context, stashed, cx).await;
                notify(context, merged_notice(context, target, &current), cx);
            }
            MergeOutcome::Conflicted { .. } => {
                cx.update(|window, app| context.open_conflicts_if_conflicted(window, app))?;
                notify(
                    context,
                    merge_conflict_notice(context, &reference, stashed),
                    cx,
                );
            }
        },
    }
    Ok(())
}

async fn rebase_flow(
    context: &BranchContext,
    target: &RefTarget,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let Some(branch) = cx.update(|_, app| context.current_branch_name(app))? else {
        let notice = BranchNotice::warning("Rebase is not possible in the detached HEAD state")
            .title("Rebase not started");
        notify(context, notice, cx);
        return Ok(());
    };

    if let Some(operation) = unfinished_operation(context, cx).await {
        let name = repository_name(context, cx)?;
        let notice = blocking_notice(
            context,
            operation,
            "Rebase not allowed",
            "starting a rebase",
            &name,
        );
        notify(context, notice, cx);
        return Ok(());
    }

    let upstream = target.full_ref_name();
    if !confirm_published_rebase(context, &upstream, cx).await? {
        return Ok(());
    }
    let report = RebaseReport {
        branch: Some(branch.to_string()),
        onto: Some(target.name.to_string()),
        stashed: false,
        update: None,
    };
    ongoing::start_rebase(context, upstream, report, cx).await
}

async fn pull_flow(
    context: &BranchContext,
    target: &RefTarget,
    rebase: bool,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let Some(branch) = cx.update(|_, app| context.current_branch_name(app))? else {
        notify(context, BranchNotice::info("Nothing to update"), cx);
        return Ok(());
    };
    let Some(remote) = target.remote_name() else {
        notify(
            context,
            BranchNotice::error(format!("{} is not a remote branch", target.name))
                .title("Update failed"),
            cx,
        );
        return Ok(());
    };
    let request = UpdateRequest {
        branch: branch.to_string(),
        upstream: target.name.to_string(),
        remote: remote.to_string(),
        rebase,
    };
    update_from(context, request, cx).await
}

async fn update_current_flow(
    context: &BranchContext,
    method: UpdateMethod,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let Some(branch) = cx.update(|_, app| context.current_branch(app))? else {
        let notice = BranchNotice::error(
            "You are in the detached HEAD state. Check out a branch to update it.",
        )
        .title("Cannot Update: No Current Branch");
        notify(context, notice, cx);
        return Ok(());
    };
    let branch_name = branch.name().to_string();
    let Some(tracked) = tracked_upstream(&branch) else {
        let notice = BranchNotice::error(format!(
            "Tracked branch is not configured for {}",
            quoted_ref_label(&branch_name)
        ))
        .title("Cannot Update");
        notify(context, notice, cx);
        return Ok(());
    };
    let rebase = resolve_rebase(context, method, &branch_name, cx).await?;
    let request = UpdateRequest {
        branch: branch_name,
        upstream: tracked.reference,
        remote: tracked.remote,
        rebase,
    };
    update_from(context, request, cx).await
}

async fn update_from(
    context: &BranchContext,
    request: UpdateRequest,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    if let Some(operation) = unfinished_operation(context, cx).await {
        let name = repository_name(context, cx)?;
        let notice = blocking_notice(context, operation, "Cannot update", "updating", &name);
        notify(context, notice, cx);
        return Ok(());
    }
    if cx.update(|_, app| has_unmerged_paths(context.repository.read(app)))? {
        let notice = unmerged_files_notice(
            context,
            "Cannot update",
            "Unmerged files detected. Resolve the conflicts before updating.",
        );
        notify(context, notice, cx);
        return Ok(());
    }

    fetch_remote(context, &request.remote, cx).await?;

    let upstream_ref = format!("refs/remotes/{}", request.upstream);
    let incoming = count_commits(context, "HEAD", &upstream_ref, cx).await?;
    if incoming == 0 {
        notify(context, BranchNotice::info("Already up to date"), cx);
        return Ok(());
    }
    let progress = UpdateProgress {
        head_before: head_sha(context, cx).await?,
        commits: incoming,
    };

    if request.rebase {
        rebase_update(context, &request, upstream_ref, progress, cx).await
    } else {
        merge_update(context, &request, progress, cx).await
    }
}

async fn rebase_update(
    context: &BranchContext,
    request: &UpdateRequest,
    upstream_ref: String,
    progress: UpdateProgress,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    if !confirm_published_rebase(context, &upstream_ref, cx).await? {
        return Ok(());
    }
    let report = RebaseReport {
        branch: Some(request.branch.clone()),
        onto: Some(request.upstream.clone()),
        stashed: false,
        update: Some(progress),
    };
    ongoing::start_rebase(context, upstream_ref, report, cx).await
}

async fn merge_update(
    context: &BranchContext,
    request: &UpdateRequest,
    progress: UpdateProgress,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    match merge_with_smart_stash(context, &request.upstream, cx).await? {
        MergeAttempt::Cancelled => {}
        MergeAttempt::Failed(error) => report_merge_failure(context, &request.upstream, &error, cx),
        MergeAttempt::Finished { outcome, stashed } => match outcome {
            MergeOutcome::AlreadyUpToDate => {
                restore_stash(context, stashed, cx).await;
                notify(context, BranchNotice::info("Already up to date"), cx);
            }
            MergeOutcome::Merged { .. } => {
                restore_stash(context, stashed, cx).await;
                let notice = update_notice(
                    context,
                    Some(&request.branch),
                    Some(&request.upstream),
                    &progress,
                    cx,
                )
                .await;
                notify(context, notice, cx);
            }
            MergeOutcome::Conflicted { .. } => {
                cx.update(|window, app| context.open_conflicts_if_conflicted(window, app))?;
                let notice = merge_conflict_notice(context, &request.upstream, stashed);
                notify(context, notice, cx);
            }
        },
    }
    Ok(())
}

async fn update_branch_flow(
    context: &BranchContext,
    target: &RefTarget,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let Some(branch) = cx.update(|_, app| context.find_branch(target, app))? else {
        let notice = BranchNotice::error(format!(
            "Branch {} was not found",
            quoted_ref_label(&target.name)
        ))
        .title("Update failed");
        notify(context, notice, cx);
        return Ok(());
    };
    if branch.is_head {
        let method = cx.update(|_, app| saved_update_method(app))?;
        return update_current_flow(context, method, cx).await;
    }
    let Some(tracked) = tracked_upstream(&branch) else {
        let notice = BranchNotice::error(format!(
            "Tracked branch is not configured for {}",
            quoted_ref_label(&target.name)
        ))
        .title("Update failed");
        notify(context, notice, cx);
        return Ok(());
    };

    let local_branch = branch.name().to_string();
    let askpass = cx.update(|window, app| {
        context.askpass_delegate(format!("git fetch {}", tracked.remote), window, app)
    })?;
    let receiver = context.repository.update(cx, |repository, cx| {
        repository.fast_forward_branch(
            tracked.remote.clone(),
            tracked.branch_name.clone(),
            local_branch,
            askpass,
            cx,
        )
    });
    receiver.await??;
    notify(context, BranchNotice::info(fetched_targets_message(1)), cx);
    Ok(())
}

async fn reset_flow(context: &BranchContext, cx: &mut AsyncWindowContext) -> Result<()> {
    let Some(branch) = cx.update(|_, app| context.current_branch(app))? else {
        let notice = BranchNotice::error(
            "You are in the detached HEAD state. Check out a branch to reset it.",
        )
        .title("Cannot Reset: No Current Branch");
        notify(context, notice, cx);
        return Ok(());
    };
    let branch_name = branch.name().to_string();
    let Some(tracked) = tracked_upstream(&branch) else {
        let notice = BranchNotice::error(format!(
            "Tracked branch is not configured for {}",
            quoted_ref_label(&branch_name)
        ))
        .title("Cannot Reset");
        notify(context, notice, cx);
        return Ok(());
    };
    if let Some(operation) = unfinished_operation(context, cx).await {
        let name = repository_name(context, cx)?;
        let notice = blocking_notice(context, operation, "Cannot Reset", "resetting", &name);
        notify(context, notice, cx);
        return Ok(());
    }

    let detail = reset_confirmation_message(&branch_name, &tracked.reference);
    let answer = cx.prompt(
        PromptLevel::Warning,
        "Reset to the Remote Branch",
        Some(detail.as_str()),
        &["Reset", "Cancel"],
    );
    if !matches!(answer.await, Ok(0)) {
        return Ok(());
    }

    fetch_remote(context, &tracked.remote, cx).await?;
    let stashed = cx
        .update(|_, app| stash_local_changes(context, app))?
        .await?;
    let upstream_ref = format!("refs/remotes/{}", tracked.reference);
    let result = reset_hard(context, &upstream_ref, cx).await;
    restore_stash(context, stashed, cx).await;
    result?;
    let notice = BranchNotice::info(format!("Reset {} to {}", branch_name, tracked.reference))
        .title("Reset to the Remote Branch");
    notify(context, notice, cx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use git::repository::{REMOTE_CANCELLED_BY_USER, Upstream, UpstreamTracking};

    use super::*;

    fn branch_tracking(upstream_ref: Option<&str>) -> Branch {
        Branch {
            is_head: true,
            ref_name: "refs/heads/feature".into(),
            upstream: upstream_ref.map(|reference| Upstream {
                ref_name: reference.to_string().into(),
                tracking: UpstreamTracking::Gone,
            }),
            most_recent_commit: None,
        }
    }

    #[test]
    fn update_method_round_trips_through_its_stored_value() {
        for method in [
            UpdateMethod::Merge,
            UpdateMethod::Rebase,
            UpdateMethod::BranchDefault,
        ] {
            assert_eq!(
                UpdateMethod::from_stored(Some(method.stored_value())),
                method
            );
        }
    }

    #[test]
    fn update_method_defaults_to_merge_for_missing_or_unknown_values() {
        assert_eq!(UpdateMethod::from_stored(None), UpdateMethod::Merge);
        assert_eq!(UpdateMethod::from_stored(Some("")), UpdateMethod::Merge);
        assert_eq!(
            UpdateMethod::from_stored(Some("squash")),
            UpdateMethod::Merge
        );
        assert_eq!(UpdateMethod::default(), UpdateMethod::Merge);
    }

    #[test]
    fn update_method_labels_match_the_settings_page_entries() {
        assert_eq!(UpdateMethod::Merge.label(), "Merge");
        assert_eq!(UpdateMethod::Rebase.label(), "Rebase");
        assert_eq!(UpdateMethod::BranchDefault.label(), "Branch Default");
    }

    #[test]
    fn git_rebase_preference_accepts_every_boolean_spelling_and_rebase_modes() {
        for truthy in [
            "true",
            "TRUE",
            "yes",
            "on",
            "1",
            "merges",
            "interactive",
            "i",
        ] {
            assert_eq!(parse_rebase_preference(truthy), Some(true), "{truthy}");
        }
        for falsy in ["false", "No", "off", "0"] {
            assert_eq!(parse_rebase_preference(falsy), Some(false), "{falsy}");
        }
        assert_eq!(parse_rebase_preference(""), None);
        assert_eq!(parse_rebase_preference("sometimes"), None);
        assert_eq!(parse_rebase_preference("  true\n"), Some(true));
    }

    #[test]
    fn only_main_and_master_are_protected_branch_names() {
        assert!(is_protected_branch_name("main"));
        assert!(is_protected_branch_name("master"));
        assert!(!is_protected_branch_name("feature/main"));
        assert!(!is_protected_branch_name("main2"));
    }

    #[test]
    fn remote_reference_is_protected_when_its_branch_part_is_protected() {
        assert!(is_protected_remote_reference("origin/main"));
        assert!(is_protected_remote_reference("upstream/master"));
        assert!(!is_protected_remote_reference("origin/feature/main"));
        assert!(!is_protected_remote_reference("main"));
    }

    #[test]
    fn published_commit_prompt_needs_commits_to_rebase_and_a_protected_remote() {
        let protected = vec![SharedString::from("origin/main")];
        let unprotected = vec![SharedString::from("origin/feature")];
        assert!(should_prompt_for_published_commits(&protected, 3));
        assert!(
            !should_prompt_for_published_commits(&protected, 0),
            "nothing is rebased when the branch is already contained in the base"
        );
        assert!(!should_prompt_for_published_commits(&unprotected, 3));
        assert!(!should_prompt_for_published_commits(&[], 3));
    }

    #[test]
    fn update_title_pluralizes_files_and_commits() {
        assert_eq!(update_title(Some(1), 1), "1 file updated in 1 commit");
        assert_eq!(update_title(Some(0), 2), "0 files updated in 2 commits");
        assert_eq!(update_title(Some(12), 5), "12 files updated in 5 commits");
    }

    #[test]
    fn update_title_omits_the_file_count_when_it_could_not_be_computed() {
        assert_eq!(update_title(None, 1), "Updated in 1 commit");
        assert_eq!(update_title(None, 4), "Updated in 4 commits");
    }

    #[test]
    fn update_message_names_the_branch_and_its_source_when_known() {
        assert_eq!(
            update_message(Some("main"), Some("origin/main")),
            "Updated main from origin/main"
        );
        assert_eq!(update_message(Some("main"), None), "Updated main");
        assert_eq!(update_message(None, None), "Updated the current branch");
    }

    #[test]
    fn merge_success_message_uses_plain_reference_names() {
        assert_eq!(
            merge_success_message("feature/login", "main"),
            "Merged feature/login to main"
        );
    }

    #[test]
    fn fetched_targets_message_pluralizes() {
        assert_eq!(fetched_targets_message(1), "Fetched 1 target");
        assert_eq!(fetched_targets_message(3), "Fetched 3 targets");
    }

    #[test]
    fn reset_confirmation_message_matches_the_intellij_wording() {
        assert_eq!(
            reset_confirmation_message("main", "origin/main"),
            "Do you want to reset local branch main to origin/main? Local commits will be dropped"
        );
    }

    #[test]
    fn unfinished_operation_message_names_the_operation_and_purpose() {
        assert_eq!(
            unfinished_operation_message(RepositoryOperation::Merge, "flint", "starting a rebase"),
            "There is an unfinished merge process in flint. You should complete or abort it before starting a rebase."
        );
        assert!(
            unfinished_operation_message(RepositoryOperation::CherryPick, "flint", "merging")
                .contains("cherry-pick")
        );
    }

    #[test]
    fn untracked_files_message_lists_every_file_on_its_own_line() {
        let message =
            untracked_files_message("merge", &["a.txt".to_string(), "b/c.txt".to_string()]);
        assert!(message.contains("overwritten by merge:\n\na.txt\nb/c.txt\n\n"));
    }

    #[test]
    fn tracked_upstream_splits_remote_branch_and_reference() {
        let tracked = tracked_upstream(&branch_tracking(Some("refs/remotes/origin/feature/x")))
            .expect("remote upstream is tracked");
        assert_eq!(tracked.remote, "origin");
        assert_eq!(tracked.branch_name, "feature/x");
        assert_eq!(tracked.reference, "origin/feature/x");
    }

    #[test]
    fn branch_without_a_remote_upstream_is_not_tracked() {
        assert!(tracked_upstream(&branch_tracking(None)).is_none());
        assert!(tracked_upstream(&branch_tracking(Some("refs/heads/main"))).is_none());
    }

    #[test]
    fn git_failure_is_found_through_added_context() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::LocalChangesWouldBeOverwritten {
                files: vec!["src/lib.rs".to_string()],
            },
            message: "local changes".to_string(),
        })
        .context("while merging");
        assert_eq!(
            overwritten_files(&error),
            Some(vec!["src/lib.rs".to_string()])
        );
        assert!(git_failure(&anyhow!("plain error")).is_none());
    }

    #[test]
    fn overwritten_files_ignores_other_failure_kinds() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::UnmergedFiles,
            message: "unmerged".to_string(),
        });
        assert_eq!(overwritten_files(&error), None);
    }

    #[test]
    fn failure_notice_hides_user_cancellations() {
        assert!(failure_notice("Update failed", &anyhow!(REMOTE_CANCELLED_BY_USER)).is_none());
    }

    #[test]
    fn failure_notice_carries_the_title_and_trimmed_git_message() {
        let notice = failure_notice("Update failed", &anyhow!("  fatal: no route to host\n"))
            .expect("ordinary errors are shown");
        assert_eq!(notice.message.as_ref(), "fatal: no route to host");
        assert_eq!(
            notice.title.as_ref().map(|title| title.as_ref()),
            Some("Update failed")
        );
    }

    #[test]
    fn failure_notice_prefers_the_git_failure_message_over_context() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::Other,
            message: "error: pathspec 'nope' did not match\n".to_string(),
        })
        .context("updating branch");
        let notice = failure_notice("Update failed", &error).expect("failures are shown");
        assert_eq!(
            notice.message.as_ref(),
            "error: pathspec 'nope' did not match"
        );
    }

    #[test]
    fn update_project_label_has_an_ellipsis_only_while_the_options_dialog_is_shown() {
        assert_eq!(update_project_label(true), "Update Project…");
        assert_eq!(update_project_label(false), "Update Project");
    }
}
