use anyhow::Result;
use git::Oid;
use git::repository::{
    Branch, CherryPickOutcome, CommitData, CommitDetails, GitFailure, GitFailureKind, RevertOutcome,
};
use gpui::{App, AsyncWindowContext, SharedString, Task, Window};
use util::ResultExt as _;

use super::context_menu::MenuCommit;
use crate::branch_operations::integrate::is_protected_remote_reference;
use crate::branch_operations::new_branch_dialog::{self, NewBranchDialogOptions, NewBranchRequest};
use crate::branch_operations::{BranchContext, BranchNotice};
use crate::commit_modal::{CommitModal, ForceMode};
use crate::git_log::ref_labels::{RefLabel, RefLabelKind};
use crate::git_panel::GitPanel;
use crate::merge_tool::conflict_resolution::customizer::CustomizerKind;
use crate::merge_tool::conflict_resolution::driver::UnresolvedNotice;
use crate::merge_tool::conflict_resolution::labels::short_hash;
use crate::merge_tool::conflict_resolution::{ConflictParams, ResolveMode, ResolverBehavior};

const UNRESOLVED_CONFLICTS_TEXT: &str = "There are unresolved conflicts in the working tree.";
const RESOLVE_ACTION_TEXT: &str = "Resolve…";
const CREATE_BRANCH_CONFIRM_TEXT: &str = "Create";
const CONFLICT_MARKERS: [&str; 2] = [
    "after resolving the conflicts",
    "CONFLICT (content): Merge conflict",
];
const CHERRY_PICK_EMPTY_MARKERS: [&str; 3] = [
    "nothing to commit",
    "nothing added to commit but untracked files present",
    "previous cherry-pick is now empty",
];
const REVERT_EMPTY_MARKERS: [&str; 2] = [
    "nothing to commit",
    "nothing added to commit but untracked files present",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyOperation {
    CherryPick,
    Revert,
}

impl ApplyOperation {
    fn name(self) -> &'static str {
        match self {
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }

    fn capitalized_name(self) -> &'static str {
        match self {
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }

    fn applied_word(self) -> &'static str {
        match self {
            Self::CherryPick => "applied",
            Self::Revert => "reverted",
        }
    }

    fn capitalized_applied_word(self) -> &'static str {
        match self {
            Self::CherryPick => "Applied",
            Self::Revert => "Reverted",
        }
    }

    fn empty_commit_markers(self) -> &'static [&'static str] {
        match self {
            Self::CherryPick => &CHERRY_PICK_EMPTY_MARKERS,
            Self::Revert => &REVERT_EMPTY_MARKERS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommitDescription {
    sha: Oid,
    subject: SharedString,
    author: SharedString,
    message: SharedString,
}

impl CommitDescription {
    fn from_data(sha: Oid, data: &CommitData) -> Self {
        Self {
            sha,
            subject: data.subject.clone(),
            author: user_name(&data.author_name, &data.author_email).into(),
            message: data.message.clone(),
        }
    }

    fn from_details(sha: Oid, details: &CommitDetails) -> Self {
        Self {
            sha,
            subject: details
                .message
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
                .into(),
            author: user_name(&details.author_name, &details.author_email).into(),
            message: details.message.clone(),
        }
    }

    fn unknown(sha: Oid) -> Self {
        Self {
            sha,
            subject: SharedString::default(),
            author: SharedString::default(),
            message: SharedString::default(),
        }
    }

    fn short_hash(&self) -> String {
        short_hash(&self.sha.to_string())
    }

    fn details(&self) -> String {
        format!("{} {}", self.short_hash(), self.subject)
    }
}

enum Application {
    Completed,
    Stopped { output: String },
}

fn user_name(name: &str, email: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    match email.find('@') {
        Some(index) if index > 0 => email[..index].to_string(),
        _ => email.to_string(),
    }
}

fn commits_word(count: usize) -> &'static str {
    if count >= 2 { "commits" } else { "commit" }
}

fn details_lines(commits: &[CommitDescription]) -> String {
    commits
        .iter()
        .map(CommitDescription::details)
        .collect::<Vec<_>>()
        .join("\n")
}

fn short_hashes(commits: &[CommitDescription]) -> String {
    commits
        .iter()
        .map(CommitDescription::short_hash)
        .collect::<Vec<_>>()
        .join(", ")
}

fn successful_details(operation: ApplyOperation, successful: &[CommitDescription]) -> String {
    if successful.is_empty() {
        return String::new();
    }
    format!(
        "\nHowever, {} succeeded for the following {}:\n{}",
        operation.name(),
        commits_word(successful.len()),
        details_lines(successful)
    )
}

fn is_conflict(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.to_lowercase();
        CONFLICT_MARKERS
            .iter()
            .any(|marker| line.contains(&marker.to_lowercase()))
    })
}

fn is_empty_commit(operation: ApplyOperation, output: &str) -> bool {
    operation
        .empty_commit_markers()
        .iter()
        .any(|marker| output.contains(marker))
}

fn performed_with_conflicts_title(operation: ApplyOperation) -> String {
    format!(
        "{} was performed with conflicts",
        operation.capitalized_name()
    )
}

fn failed_title(operation: ApplyOperation) -> String {
    format!("{} failed", operation.capitalized_name())
}

fn conflict_description(operation: ApplyOperation, commit: &CommitDescription) -> String {
    format!(
        "Conflicts during {} of commit {} made by {}\n{}",
        operation.name(),
        commit.short_hash(),
        commit.author,
        commit.subject
    )
}

fn conflict_params(operation: ApplyOperation, commit: &CommitDescription) -> ConflictParams {
    ConflictParams {
        reverse: false,
        error_notification_title: performed_with_conflicts_title(operation).into(),
        error_notification_additional_description: SharedString::default(),
        customizer: CustomizerKind::DescriptionOverride(
            conflict_description(operation, commit).into(),
        ),
        unresolved_notice: UnresolvedNotice::Suppressed,
    }
}

fn result_notice(
    operation: ApplyOperation,
    successful: &[CommitDescription],
    skipped: &[CommitDescription],
) -> BranchNotice {
    if skipped.is_empty() {
        return BranchNotice::info(details_lines(successful))
            .title(format!("{} successful", operation.capitalized_name()));
    }
    if successful.is_empty() {
        return BranchNotice::warning(format!(
            "All changes from {} have already been {}",
            short_hashes(skipped),
            operation.applied_word()
        ))
        .title(format!("Nothing to {}", operation.name()));
    }
    let skipped_verb = if skipped.len() >= 2 { "were" } else { "was" };
    BranchNotice::info(format!(
        "{}\n{} {skipped_verb} skipped, because all changes have already been {}.",
        details_lines(successful),
        short_hashes(skipped),
        operation.applied_word()
    ))
    .title(format!(
        "{} {} {} from {}",
        operation.capitalized_applied_word(),
        successful.len(),
        commits_word(successful.len()),
        successful.len() + skipped.len()
    ))
}

fn error_output(error: &anyhow::Error) -> String {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
    {
        Some(failure) => failure.message.trim().to_string(),
        None => format!("{error:#}").trim().to_string(),
    }
}

fn failure_notice(
    operation: ApplyOperation,
    commit: &CommitDescription,
    error: &anyhow::Error,
    successful: &[CommitDescription],
) -> BranchNotice {
    let kind = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
        .map(|failure| failure.kind.clone());
    match kind {
        Some(GitFailureKind::UntrackedFilesWouldBeOverwritten { files }) => BranchNotice::error(
            format!(
                "Move or commit them before {}\n{}{}",
                operation.name(),
                files.join("\n"),
                successful_details(operation, successful)
            ),
        )
        .title(format!(
            "Untracked Files Prevent {}",
            operation.capitalized_name()
        )),
        Some(GitFailureKind::LocalChangesWouldBeOverwritten { .. }) => {
            BranchNotice::error(format!(
                "{}\nYour local changes will be overwritten by {}.\nCommit, stash, or revert them to proceed.{}",
                commit.details(),
                operation.name(),
                successful_details(operation, successful)
            ))
            .title(failed_title(operation))
        }
        _ => stopped_failure_notice(operation, commit, &error_output(error), successful),
    }
}

fn stopped_failure_notice(
    operation: ApplyOperation,
    commit: &CommitDescription,
    output: &str,
    successful: &[CommitDescription],
) -> BranchNotice {
    BranchNotice::error(format!(
        "{}\n{}{}",
        commit.details(),
        output.trim(),
        successful_details(operation, successful)
    ))
    .title(failed_title(operation))
}

fn conflict_notice(
    context: &BranchContext,
    operation: ApplyOperation,
    commit: &CommitDescription,
    successful: &[CommitDescription],
) -> BranchNotice {
    let resolve_context = context.clone();
    let params = conflict_params(operation, commit);
    BranchNotice::warning(format!(
        "{}\n{UNRESOLVED_CONFLICTS_TEXT}{}",
        commit.details(),
        successful_details(operation, successful)
    ))
    .title(performed_with_conflicts_title(operation))
    .action(RESOLVE_ACTION_TEXT, move |window, cx| {
        resolve_context
            .resolve_conflicts(
                params.clone(),
                ResolverBehavior::Plain,
                ResolveMode::FromNotification,
                window,
                cx,
            )
            .detach();
    })
}

fn revert_message(commit: &CommitDescription) -> String {
    format!(
        "Revert \"{}\"\n\nThis reverts commit {}",
        commit.subject,
        commit.short_hash()
    )
}

fn cherry_pick_message(commit: &CommitDescription, published: bool) -> String {
    if published {
        format!(
            "{}\n\n(cherry picked from commit {})",
            commit.message, commit.sha
        )
    } else {
        commit.message.to_string()
    }
}

fn published_to_protected_branch(remote_branches: &[String], containing: &[SharedString]) -> bool {
    remote_branches.iter().any(|branch| {
        is_protected_remote_reference(branch)
            && containing
                .iter()
                .any(|containing| containing.as_ref() == branch.as_str())
    })
}

fn remote_operation_name(branch: &Branch) -> String {
    let name = branch.name();
    name.split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(name)
        .to_string()
}

fn suggested_branch_name(labels: &[RefLabel], branches: &[Branch]) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for label in labels {
        let is_remote = match label.kind {
            RefLabelKind::LocalBranch => false,
            RefLabelKind::RemoteBranch => true,
            RefLabelKind::Head | RefLabelKind::Tag | RefLabelKind::Other => continue,
        };
        let Some(branch) = branches
            .iter()
            .find(|branch| branch.is_remote() == is_remote && branch.name() == &*label.name)
        else {
            continue;
        };
        let name = if is_remote {
            remote_operation_name(branch)
        } else {
            branch.name().to_string()
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    match names.as_slice() {
        [name] => Some(name.clone()),
        _ => None,
    }
}

fn notify(context: &BranchContext, notice: BranchNotice, cx: &mut AsyncWindowContext) {
    cx.update(|_, app| context.notify(notice, app)).log_err();
}

pub(crate) fn cherry_pick_commits(
    context: BranchContext,
    commits: Vec<MenuCommit>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let oldest_first = commits.into_iter().rev().collect();
    apply_changes(
        context,
        ApplyOperation::CherryPick,
        oldest_first,
        window,
        cx,
    )
}

pub(super) fn revert_commits(
    context: BranchContext,
    commits: Vec<MenuCommit>,
    window: &mut Window,
    cx: &mut App,
) {
    if commits.iter().any(|commit| commit.parents.len() > 1) {
        return;
    }
    apply_changes(context, ApplyOperation::Revert, commits, window, cx).detach();
}

fn apply_changes(
    context: BranchContext,
    operation: ApplyOperation,
    commits: Vec<MenuCommit>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    if commits.is_empty() {
        return Task::ready(());
    }
    window.spawn(cx, async move |cx| {
        apply_changes_flow(&context, operation, commits, cx).await
    })
}

async fn describe_commits(
    context: &BranchContext,
    commits: Vec<MenuCommit>,
    cx: &mut AsyncWindowContext,
) -> Vec<CommitDescription> {
    let mut descriptions = Vec::with_capacity(commits.len());
    for commit in commits {
        if let Some(data) = commit.data.as_deref() {
            descriptions.push(CommitDescription::from_data(commit.sha, data));
            continue;
        }
        let details = cx.update(|_, app| {
            context
                .repository
                .read(app)
                .show_commit(commit.sha.to_string(), app)
        });
        let description = match details {
            Ok(task) => match task.await {
                Ok(details) => CommitDescription::from_details(commit.sha, &details),
                Err(error) => {
                    log::warn!("could not load commit {}: {error:#}", commit.sha);
                    CommitDescription::unknown(commit.sha)
                }
            },
            Err(_) => CommitDescription::unknown(commit.sha),
        };
        descriptions.push(description);
    }
    descriptions
}

async fn apply_commit(
    context: &BranchContext,
    operation: ApplyOperation,
    sha: Oid,
    cx: &mut AsyncWindowContext,
) -> Result<Application> {
    match operation {
        ApplyOperation::CherryPick => {
            let receiver = context
                .repository
                .update(cx, |repository, _| repository.cherry_pick(vec![sha]));
            Ok(match receiver.await?? {
                CherryPickOutcome::Completed { .. } => Application::Completed,
                CherryPickOutcome::Conflicted { output } => Application::Stopped { output },
            })
        }
        ApplyOperation::Revert => {
            let receiver = context
                .repository
                .update(cx, |repository, _| repository.revert_commits(vec![sha]));
            Ok(match receiver.await?? {
                RevertOutcome::Completed { .. } => Application::Completed,
                RevertOutcome::Conflicted { output } => Application::Stopped { output },
            })
        }
    }
}

async fn apply_changes_flow(
    context: &BranchContext,
    operation: ApplyOperation,
    commits: Vec<MenuCommit>,
    cx: &mut AsyncWindowContext,
) {
    let commits = describe_commits(context, commits, cx).await;
    let mut successful: Vec<CommitDescription> = Vec::new();
    let mut skipped: Vec<CommitDescription> = Vec::new();
    for commit in &commits {
        match apply_commit(context, operation, commit.sha, cx).await {
            Ok(Application::Completed) => successful.push(commit.clone()),
            Ok(Application::Stopped { output }) => {
                if is_conflict(&output) {
                    resolve_conflicts_flow(context, operation, commit, &successful, cx).await;
                    return;
                }
                if is_empty_commit(operation, &output) {
                    skipped.push(commit.clone());
                    break;
                }
                let notice = stopped_failure_notice(operation, commit, &output, &successful);
                notify(context, notice, cx);
                return;
            }
            Err(error) => {
                notify(
                    context,
                    failure_notice(operation, commit, &error, &successful),
                    cx,
                );
                return;
            }
        }
    }
    notify(context, result_notice(operation, &successful, &skipped), cx);
}

async fn resolve_conflicts_flow(
    context: &BranchContext,
    operation: ApplyOperation,
    commit: &CommitDescription,
    successful: &[CommitDescription],
    cx: &mut AsyncWindowContext,
) {
    let params = conflict_params(operation, commit);
    let Some(resolution) = cx
        .update(|window, app| {
            context.resolve_conflicts(
                params,
                ResolverBehavior::Plain,
                ResolveMode::Initial,
                window,
                app,
            )
        })
        .log_err()
    else {
        return;
    };
    if !resolution.await.proceed {
        notify(
            context,
            conflict_notice(context, operation, commit, successful),
            cx,
        );
        return;
    }
    let message = match operation {
        ApplyOperation::CherryPick => {
            let published = is_published(context, commit.sha, cx).await;
            cherry_pick_message(commit, published)
        }
        ApplyOperation::Revert => revert_message(commit),
    };
    show_commit_dialog(context, message, cx);
}

async fn is_published(context: &BranchContext, sha: Oid, cx: &mut AsyncWindowContext) -> bool {
    let Some((containing, remote_branches)) = cx
        .update(|_, app| {
            let repository = context.repository.read(app);
            let remote_branches: Vec<String> = repository
                .branch_list
                .iter()
                .filter(|branch| branch.is_remote())
                .map(|branch| branch.name().to_string())
                .collect();
            (
                repository.branches_containing_commit(sha, app),
                remote_branches,
            )
        })
        .log_err()
    else {
        return false;
    };
    let containing = containing.await.log_err().unwrap_or_default();
    published_to_protected_branch(&remote_branches, &containing)
}

fn show_commit_dialog(context: &BranchContext, message: String, cx: &mut AsyncWindowContext) {
    let workspace = context.workspace.clone();
    cx.update(|window, app| {
        workspace
            .update(app, |workspace, cx| {
                if let Some(git_panel) = workspace.panel::<GitPanel>(cx) {
                    let buffer = git_panel.read(cx).commit_message_buffer(cx);
                    buffer.update(cx, |buffer, cx| buffer.set_text(message, cx));
                }
                CommitModal::toggle(workspace, Some(ForceMode::Commit), window, cx);
            })
            .log_err();
    })
    .log_err();
}

pub(super) fn checkout_revision(
    context: BranchContext,
    commit: Oid,
    window: &mut Window,
    cx: &mut App,
) {
    let revision = commit.to_string();
    let label = short_hash(&revision);
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.checkout_detached(revision));
    window
        .spawn(cx, async move |cx| {
            let notice = match receiver.await {
                Ok(Ok(())) => BranchNotice::info(format!("Checked out {label}")),
                Ok(Err(error)) => BranchNotice::error(error_output(&error))
                    .title(format!("Could not checkout {label}")),
                Err(_) => return,
            };
            notify(&context, notice, cx);
        })
        .detach();
}

pub(crate) fn new_branch_from_commit(
    context: BranchContext,
    commit: MenuCommit,
    window: &mut Window,
    cx: &mut App,
) {
    let start_point = commit.sha.to_string();
    let initial_name =
        suggested_branch_name(&commit.labels, &context.repository.read(cx).branch_list)
            .unwrap_or_default();
    let options = NewBranchDialogOptions {
        title: format!("Create Branch from {}", short_hash(&start_point)).into(),
        confirm_label: CREATE_BRANCH_CONFIRM_TEXT.into(),
        initial_name,
        show_checkout_option: true,
    };
    let submit_context = context.clone();
    new_branch_dialog::open(
        &context,
        options,
        move |request, window, cx| create_branch(submit_context, request, start_point, window, cx),
        window,
        cx,
    );
}

fn create_branch(
    context: BranchContext,
    request: NewBranchRequest,
    start_point: String,
    window: &mut Window,
    cx: &mut App,
) {
    let short = short_hash(&start_point);
    let NewBranchRequest {
        name,
        checkout,
        overwrite,
    } = request;
    let receiver = context.repository.update(cx, |repository, _| {
        repository.create_branch_at(name.clone(), start_point, checkout, overwrite)
    });
    window
        .spawn(cx, async move |cx| {
            let notice = match receiver.await {
                Ok(Ok(())) if checkout => {
                    BranchNotice::info(format!("Checked out new branch {name} from {short}"))
                }
                Ok(Ok(())) => BranchNotice::info(format!("Branch {name} was created")),
                Ok(Err(error)) => BranchNotice::error(error_output(&error))
                    .title(format!("Could not create new branch {name}")),
                Err(_) => return,
            };
            notify(&context, notice, cx);
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branch_operations::NoticeSeverity;

    fn description(sha_byte: u8, subject: &str) -> CommitDescription {
        let sha = Oid::from_bytes(&[sha_byte; 20]).expect("valid sha");
        CommitDescription {
            sha,
            subject: subject.to_string().into(),
            author: "Jane".into(),
            message: format!("{subject}\n\nBody").into(),
        }
    }

    fn branch(ref_name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: ref_name.to_string().into(),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn label(name: &str, kind: RefLabelKind) -> RefLabel {
        RefLabel {
            name: name.to_string().into(),
            kind,
        }
    }

    #[test]
    fn user_name_falls_back_to_the_email_local_part_then_the_email() {
        assert_eq!(user_name("Jane Doe", "jane@example.com"), "Jane Doe");
        assert_eq!(user_name("", "jane@example.com"), "jane");
        assert_eq!(user_name("", "@example.com"), "@example.com");
        assert_eq!(user_name("", "jane"), "jane");
    }

    #[test]
    fn conflicts_are_detected_case_insensitively_on_any_output_line() {
        assert!(is_conflict(
            "error: could not apply 1234567... subject\nhint: After resolving the conflicts, mark them"
        ));
        assert!(is_conflict("CONFLICT (content): Merge conflict in a.txt"));
        assert!(!is_conflict(
            "The previous cherry-pick is now empty, possibly due to conflict resolution."
        ));
    }

    #[test]
    fn empty_commit_markers_depend_on_the_operation() {
        let empty_pick =
            "The previous cherry-pick is now empty, possibly due to conflict resolution.";
        assert!(is_empty_commit(ApplyOperation::CherryPick, empty_pick));
        assert!(!is_empty_commit(ApplyOperation::Revert, empty_pick));
        assert!(is_empty_commit(
            ApplyOperation::Revert,
            "On branch main\nnothing to commit, working tree clean"
        ));
    }

    #[test]
    fn result_notice_reports_success_partial_and_nothing_to_do() {
        let first = description(1, "First");
        let second = description(2, "Second");

        let success = result_notice(
            ApplyOperation::CherryPick,
            &[first.clone(), second.clone()],
            &[],
        );
        assert_eq!(success.title.as_deref(), Some("Cherry-pick successful"));
        assert_eq!(success.message.as_ref(), "01010101 First\n02020202 Second");
        assert_eq!(success.severity, NoticeSeverity::Info);

        let partial = result_notice(
            ApplyOperation::CherryPick,
            &[first.clone()],
            &[second.clone()],
        );
        assert_eq!(partial.title.as_deref(), Some("Applied 1 commit from 2"));
        assert_eq!(
            partial.message.as_ref(),
            "01010101 First\n02020202 was skipped, because all changes have already been applied."
        );

        let nothing = result_notice(ApplyOperation::Revert, &[], &[first, second]);
        assert_eq!(nothing.title.as_deref(), Some("Nothing to revert"));
        assert_eq!(
            nothing.message.as_ref(),
            "All changes from 01010101, 02020202 have already been reverted"
        );
        assert_eq!(nothing.severity, NoticeSeverity::Warning);
    }

    #[test]
    fn conflict_texts_follow_the_apply_changes_bundle() {
        let commit = description(3, "Fix parser");
        assert_eq!(
            conflict_description(ApplyOperation::CherryPick, &commit),
            "Conflicts during cherry-pick of commit 03030303 made by Jane\nFix parser"
        );
        assert_eq!(
            performed_with_conflicts_title(ApplyOperation::Revert),
            "Revert was performed with conflicts"
        );
        assert_eq!(
            successful_details(ApplyOperation::CherryPick, &[description(4, "Earlier")]),
            "\nHowever, cherry-pick succeeded for the following commit:\n04040404 Earlier"
        );
        assert_eq!(successful_details(ApplyOperation::CherryPick, &[]), "");
    }

    #[test]
    fn default_commit_messages_match_the_processes() {
        let commit = description(5, "Add feature");
        assert_eq!(
            revert_message(&commit),
            "Revert \"Add feature\"\n\nThis reverts commit 05050505"
        );
        assert_eq!(cherry_pick_message(&commit, false), "Add feature\n\nBody");
        assert_eq!(
            cherry_pick_message(&commit, true),
            format!(
                "Add feature\n\nBody\n\n(cherry picked from commit {})",
                commit.sha
            )
        );
    }

    #[test]
    fn commits_are_published_only_when_a_protected_remote_branch_contains_them() {
        let remote_branches = ["origin/main".to_string(), "origin/feature".to_string()];
        assert!(published_to_protected_branch(
            &remote_branches,
            &["origin/main".into(), "main".into()]
        ));
        assert!(!published_to_protected_branch(
            &remote_branches,
            &["origin/feature".into(), "main".into()]
        ));
    }

    #[test]
    fn branch_name_is_suggested_only_when_the_commit_has_one_distinct_branch_name() {
        let branches = [
            branch("refs/heads/feature"),
            branch("refs/remotes/origin/feature"),
            branch("refs/heads/other"),
        ];
        assert_eq!(
            suggested_branch_name(
                &[
                    label("feature", RefLabelKind::LocalBranch),
                    label("origin/feature", RefLabelKind::RemoteBranch),
                    label("v1", RefLabelKind::Tag),
                ],
                &branches
            ),
            Some("feature".to_string())
        );
        assert_eq!(
            suggested_branch_name(
                &[
                    label("feature", RefLabelKind::LocalBranch),
                    label("other", RefLabelKind::LocalBranch),
                ],
                &branches
            ),
            None
        );
        assert_eq!(
            suggested_branch_name(&[label("missing", RefLabelKind::LocalBranch)], &branches),
            None
        );
    }
}
