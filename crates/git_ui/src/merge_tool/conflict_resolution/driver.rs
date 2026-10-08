use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use futures::channel::oneshot;
use futures::future::join_all;
use git::repository::{
    GitFailure, RebaseAction, RebaseOutcome, RepositoryOperation, UnmergedEntry,
};
use gpui::{AnyWindowHandle, App, AsyncApp, Entity, SharedString, Task, WeakEntity, Window};
use project::Fs;
use project::git_store::Repository;
use util::ResultExt as _;
use workspace::Workspace;

use super::customizer::{
    CustomizerKind, RebaseCustomizerSpec, SmartRestoreSpec, UnstashSpec, build_dialog_texts,
    load_description, load_dialog_texts,
};
use super::labels::RepositoryFacts;
use crate::branch_operations::{BranchContext, BranchNotice, NoticeAction};
use crate::merge_tool::conflicts_dialog::open_conflicts_dialog;
use crate::merge_tool::conflicts_dialog::request::{ConflictsDialogRequest, ConflictsDialogResult};

const RESOLVE_LABEL: &str = "Resolve…";
const RESOLVE_CONFLICTS_LABEL: &str = "Resolve conflicts…";
const UNRESOLVED_REMAIN_BODY: &str = "There are pending unresolved conflicts.";
const PENDING_UNRESOLVED_TITLE: &str = "Pending Unresolved conflicts";
const UNMERGED_CHECK_ERROR_PREFIX: &str =
    "Cannot check the working tree for unmerged files because of an error. ";
const REBASE_ERROR_TITLE: &str = "Rebase Error";
const REBASE_STOPPED_FOR_EDITING_TITLE: &str = "Rebase stopped for editing";
const UPDATE_ERROR_TITLE: &str = "Cannot update";
const UNFINISHED_MERGE_DESCRIPTION: &str =
    "You have unfinished merge. These conflicts must be resolved before update.";
const UNMERGED_FILES_DESCRIPTION: &str =
    "Unmerged files detected. These conflicts must be resolved before update.";
const CONTINUE_OR_ABORT_REBASE_HINT: &str = "Then you may <b>continue rebase</b>. <br/> You also may <b>abort rebase</b> to restore the original branch and stop rebasing.";
const MERGE_UPDATE_ERROR_TITLE: &str = "Cannot complete update";
const MERGE_UPDATE_DESCRIPTION: &str =
    "Merge conflicts detected. Resolve them before continuing update.";
const REBASE_UPDATE_ERROR_TITLE: &str = "Cannot continue rebase";
const REBASE_UPDATE_DESCRIPTION: &str =
    "Merge conflicts detected. Resolve them before continuing rebase.";
const UNFINISHED_REBASE_DESCRIPTION: &str =
    "You have unfinished rebase process. These conflicts must be resolved before update.";
const UNRESOLVED_FILES_REMAIN_TITLE: &str = "Unresolved files remain.";
const UNSTASHED_WITH_CONFLICTS_TITLE: &str = "Unstashed with conflicts";
const UNSTASH_UNRESOLVED_TITLE: &str = "Conflicts were not resolved during unstash";
const UNSTASH_UNRESOLVED_MESSAGE: &str =
    "Unstash is not complete, you have unresolved merges in your working tree.";
const LOCAL_CHANGES_NOT_RESTORED_TITLE: &str = "Local changes were not restored";
const RESTORED_WITH_CONFLICTS_TITLE: &str = "Local changes were restored with conflicts";
const RESTORED_WITH_CONFLICTS_MESSAGE: &str = "Your uncommitted changes were saved to stash.<br/>Unstash is not complete, you have unresolved merges in your working tree<br/>Resolve conflicts and drop the stash.";
const NO_CHANGES_INDICATOR: &str = "No changes - did you forget to use 'git add'?";
const STOPPED_FOR_EDITING_INDICATOR: &str = "You can amend the commit now";
const REBASE_CONFLICT_INDICATORS: [&str; 7] = [
    "Merge conflict in",
    "after resolving the conflicts, mark the corrected paths",
    "After resolving the conflicts, mark them with",
    "Resolve all conflicts manually, mark them as resolved with",
    "You must edit all merge conflicts",
    "Failed to merge in the changes",
    "could not apply",
];

#[derive(Clone)]
pub(crate) struct ConflictContext {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) window: AnyWindowHandle,
    pub(crate) repository: Entity<Repository>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolveMode {
    Initial,
    FromNotification,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolveOutcome {
    pub(crate) proceed: bool,
    pub(crate) nothing_to_merge: bool,
    pub(crate) all_resolved: bool,
    pub(crate) should_finish_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolverBehavior {
    Plain,
    CommitMergeWhenFinishRequested,
    CommitMergeAlways,
    ContinueRebase,
}

#[derive(Clone)]
pub(crate) struct CustomNotice {
    pub(crate) title: SharedString,
    pub(crate) message: SharedString,
    pub(crate) leading_actions: Vec<NoticeAction>,
    pub(crate) resolve_label: SharedString,
}

#[derive(Clone)]
pub(crate) enum UnresolvedNotice {
    Warning,
    Suppressed,
    Custom(CustomNotice),
}

#[derive(Clone)]
pub(crate) struct ConflictParams {
    pub(crate) reverse: bool,
    pub(crate) error_notification_title: SharedString,
    pub(crate) error_notification_additional_description: SharedString,
    pub(crate) customizer: CustomizerKind,
    pub(crate) unresolved_notice: UnresolvedNotice,
}

impl ConflictParams {
    fn forward(customizer: CustomizerKind) -> Self {
        Self {
            reverse: false,
            error_notification_title: SharedString::default(),
            error_notification_additional_description: SharedString::default(),
            customizer,
            unresolved_notice: UnresolvedNotice::Warning,
        }
    }

    fn reversed(customizer: CustomizerKind) -> Self {
        Self {
            reverse: true,
            ..Self::forward(customizer)
        }
    }

    pub(crate) fn for_resolve_action(reverse: bool) -> Self {
        let customizer = CustomizerKind::GitDefault;
        let params = if reverse {
            Self::reversed(customizer)
        } else {
            Self::forward(customizer)
        };
        Self {
            unresolved_notice: UnresolvedNotice::Suppressed,
            ..params
        }
    }

    pub(crate) fn for_user_merge(branch: &str) -> Self {
        Self {
            unresolved_notice: UnresolvedNotice::Custom(CustomNotice {
                title: SharedString::from(format!("{branch} Merged with Conflicts")),
                message: SharedString::default(),
                leading_actions: Vec::new(),
                resolve_label: SharedString::new_static(RESOLVE_LABEL),
            }),
            ..Self::forward(CustomizerKind::GitDefault)
        }
    }

    pub(crate) fn for_update_by_merge() -> Self {
        Self {
            error_notification_title: SharedString::new_static(MERGE_UPDATE_ERROR_TITLE),
            ..Self::forward(CustomizerKind::DescriptionOverride(
                SharedString::new_static(MERGE_UPDATE_DESCRIPTION),
            ))
        }
    }

    pub(crate) fn for_update_by_rebase() -> Self {
        Self {
            error_notification_title: SharedString::new_static(REBASE_UPDATE_ERROR_TITLE),
            error_notification_additional_description: SharedString::new_static(
                CONTINUE_OR_ABORT_REBASE_HINT,
            ),
            ..Self::reversed(CustomizerKind::DescriptionOverride(
                SharedString::new_static(REBASE_UPDATE_DESCRIPTION),
            ))
        }
    }

    pub(crate) fn for_unfinished_merge() -> Self {
        Self {
            error_notification_title: SharedString::new_static(UPDATE_ERROR_TITLE),
            ..Self::forward(CustomizerKind::DescriptionOverride(
                SharedString::new_static(UNFINISHED_MERGE_DESCRIPTION),
            ))
        }
    }

    pub(crate) fn for_unmerged_files() -> Self {
        Self {
            error_notification_title: SharedString::new_static(UPDATE_ERROR_TITLE),
            ..Self::forward(CustomizerKind::DescriptionOverride(
                SharedString::new_static(UNMERGED_FILES_DESCRIPTION),
            ))
        }
    }

    pub(crate) fn for_unfinished_rebase() -> Self {
        Self {
            error_notification_title: SharedString::new_static(UPDATE_ERROR_TITLE),
            error_notification_additional_description: SharedString::new_static(
                CONTINUE_OR_ABORT_REBASE_HINT,
            ),
            ..Self::reversed(CustomizerKind::DescriptionOverride(
                SharedString::new_static(UNFINISHED_REBASE_DESCRIPTION),
            ))
        }
    }

    pub(crate) fn for_unmerged_files_before(operation: &str) -> Self {
        Self {
            error_notification_title: SharedString::new_static(UNRESOLVED_FILES_REMAIN_TITLE),
            ..Self::forward(CustomizerKind::DescriptionOverride(SharedString::from(
                format!(
                    "The following files have unresolved conflicts. You need to resolve them before {operation}."
                ),
            )))
        }
    }

    pub(crate) fn for_rebase_process(spec: RebaseCustomizerSpec) -> Self {
        Self {
            unresolved_notice: UnresolvedNotice::Suppressed,
            ..Self::reversed(CustomizerKind::Rebase(spec))
        }
    }

    pub(crate) fn for_unstash(stash: &str, message: &str) -> Self {
        Self {
            error_notification_title: SharedString::new_static(UNSTASHED_WITH_CONFLICTS_TITLE),
            unresolved_notice: UnresolvedNotice::Custom(CustomNotice {
                title: SharedString::new_static(UNSTASH_UNRESOLVED_TITLE),
                message: SharedString::new_static(UNSTASH_UNRESOLVED_MESSAGE),
                leading_actions: Vec::new(),
                resolve_label: SharedString::new_static(RESOLVE_CONFLICTS_LABEL),
            }),
            ..Self::forward(CustomizerKind::Unstash(UnstashSpec {
                stash: SharedString::from(stash),
                message: SharedString::from(message),
            }))
        }
    }

    pub(crate) fn for_smart_restore(
        operation_title: &str,
        destination_name: &str,
        stash: bool,
        leading_notice_actions: Vec<NoticeAction>,
    ) -> Self {
        let unresolved_notice = if stash {
            stash_restore_notice(leading_notice_actions)
        } else {
            UnresolvedNotice::Warning
        };
        Self {
            error_notification_title: SharedString::new_static(LOCAL_CHANGES_NOT_RESTORED_TITLE),
            unresolved_notice,
            ..Self::reversed(CustomizerKind::SmartRestore(SmartRestoreSpec {
                operation_title: SharedString::from(operation_title),
                destination_name: SharedString::from(destination_name),
                stash,
            }))
        }
    }
}

fn stash_restore_notice(leading_actions: Vec<NoticeAction>) -> UnresolvedNotice {
    UnresolvedNotice::Custom(CustomNotice {
        title: SharedString::new_static(RESTORED_WITH_CONFLICTS_TITLE),
        message: SharedString::new_static(RESTORED_WITH_CONFLICTS_MESSAGE),
        leading_actions,
        resolve_label: SharedString::new_static(RESOLVE_CONFLICTS_LABEL),
    })
}

pub(super) type OpenDialog = Rc<dyn Fn(ConflictsDialogRequest, &mut App)>;
pub(super) type NotifyHandler = Rc<dyn Fn(BranchNotice, &mut App)>;
type ResolveAgain = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub(super) struct DriverEffects {
    pub(super) open_dialog: OpenDialog,
    pub(super) notify: NotifyHandler,
}

impl DriverEffects {
    fn production(context: &ConflictContext) -> Self {
        let branch_context =
            BranchContext::new(context.workspace.clone(), context.repository.clone());
        Self {
            open_dialog: Rc::new(open_conflicts_dialog),
            notify: Rc::new(move |notice: BranchNotice, cx: &mut App| {
                branch_context.notify(notice, cx)
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogCycle {
    NothingToMerge,
    AllMerged { should_finish_merge: bool },
    Remain { should_finish_merge: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NextStep {
    ProceedIfNothingToMerge,
    Finished,
    ProceedAfterAllMerged { should_finish_merge: bool },
    NotifyUnresolvedRemain,
    NotifyUnresolvedRemainAfterNotification,
}

pub(super) fn next_step(cycle: DialogCycle, mode: ResolveMode) -> NextStep {
    match (cycle, mode) {
        (DialogCycle::NothingToMerge, ResolveMode::Initial) => NextStep::ProceedIfNothingToMerge,
        (DialogCycle::NothingToMerge, ResolveMode::FromNotification) => NextStep::Finished,
        (
            DialogCycle::AllMerged {
                should_finish_merge: false,
            },
            ResolveMode::FromNotification,
        ) => NextStep::Finished,
        (
            DialogCycle::AllMerged {
                should_finish_merge,
            },
            _,
        ) => NextStep::ProceedAfterAllMerged {
            should_finish_merge,
        },
        (DialogCycle::Remain { .. }, ResolveMode::Initial) => NextStep::NotifyUnresolvedRemain,
        (DialogCycle::Remain { .. }, ResolveMode::FromNotification) => {
            NextStep::NotifyUnresolvedRemainAfterNotification
        }
    }
}

pub(super) fn outcome_for(cycle: DialogCycle, proceed: bool) -> ResolveOutcome {
    match cycle {
        DialogCycle::NothingToMerge => ResolveOutcome {
            proceed,
            nothing_to_merge: true,
            all_resolved: false,
            should_finish_merge: false,
        },
        DialogCycle::AllMerged {
            should_finish_merge,
        } => ResolveOutcome {
            proceed,
            nothing_to_merge: false,
            all_resolved: true,
            should_finish_merge,
        },
        DialogCycle::Remain {
            should_finish_merge,
        } => ResolveOutcome {
            proceed,
            nothing_to_merge: false,
            all_resolved: false,
            should_finish_merge,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hook {
    NothingToMerge,
    AllMerged { should_finish_merge: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProceedPlan {
    Nothing,
    CommitMerge,
    ContinueRebase,
}

pub(super) fn proceed_plan(behavior: ResolverBehavior, hook: Hook) -> ProceedPlan {
    match (behavior, hook) {
        (ResolverBehavior::Plain, _) => ProceedPlan::Nothing,
        (ResolverBehavior::CommitMergeWhenFinishRequested, Hook::NothingToMerge) => {
            ProceedPlan::Nothing
        }
        (
            ResolverBehavior::CommitMergeWhenFinishRequested,
            Hook::AllMerged {
                should_finish_merge,
            },
        ) => {
            if should_finish_merge {
                ProceedPlan::CommitMerge
            } else {
                ProceedPlan::Nothing
            }
        }
        (ResolverBehavior::CommitMergeAlways, _) => ProceedPlan::CommitMerge,
        (ResolverBehavior::ContinueRebase, _) => ProceedPlan::ContinueRebase,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RebaseStop {
    NothingToCommit,
    StoppedForEditing,
    Conflict,
    Other,
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub(super) fn classify_rebase_stop(output: &str) -> RebaseStop {
    if REBASE_CONFLICT_INDICATORS
        .iter()
        .any(|indicator| contains_ignore_case(output, indicator))
    {
        RebaseStop::Conflict
    } else if contains_ignore_case(output, NO_CHANGES_INDICATOR) {
        RebaseStop::NothingToCommit
    } else if contains_ignore_case(output, STOPPED_FOR_EDITING_INDICATOR) {
        RebaseStop::StoppedForEditing
    } else {
        RebaseStop::Other
    }
}

pub(super) fn plain_notification_text(markup: &str) -> String {
    let mut text = String::with_capacity(markup.len());
    let mut remaining = markup;
    while let Some(tag_start) = remaining.find('<') {
        text.push_str(&remaining[..tag_start]);
        let from_tag = &remaining[tag_start..];
        let Some(tag_end) = from_tag.find('>') else {
            remaining = from_tag;
            break;
        };
        let tag_name = from_tag[1..tag_end]
            .trim_end_matches('/')
            .trim()
            .to_ascii_lowercase();
        if tag_name == "br" {
            text.push('\n');
        }
        remaining = &from_tag[tag_end + 1..];
    }
    text.push_str(remaining);
    text.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

fn error_message(error: &anyhow::Error) -> String {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
    {
        Some(failure) => failure.message.trim().to_string(),
        None => format!("{error:#}").trim().to_string(),
    }
}

fn titled(notice: BranchNotice, title: &str) -> BranchNotice {
    if title.is_empty() {
        notice
    } else {
        notice.title(title.to_string())
    }
}

fn with_resolve_action(
    mut notice: BranchNotice,
    leading_actions: &[NoticeAction],
    label: SharedString,
    resolve: ResolveAgain,
) -> BranchNotice {
    notice.actions.extend(leading_actions.iter().cloned());
    notice.actions.push(NoticeAction {
        label,
        handler: resolve,
    });
    notice
}

pub(super) fn unresolved_conflicts_notice(
    params: &ConflictParams,
    resolve: ResolveAgain,
) -> Option<BranchNotice> {
    match &params.unresolved_notice {
        UnresolvedNotice::Suppressed => None,
        UnresolvedNotice::Warning => {
            let message = plain_notification_text(&format!(
                "{UNRESOLVED_REMAIN_BODY}{}",
                params.error_notification_additional_description
            ));
            Some(with_resolve_action(
                titled(
                    BranchNotice::warning(message),
                    &params.error_notification_title,
                ),
                &[],
                SharedString::new_static(RESOLVE_LABEL),
                resolve,
            ))
        }
        UnresolvedNotice::Custom(custom) => Some(with_resolve_action(
            titled(
                BranchNotice::warning(plain_notification_text(&custom.message)),
                &custom.title,
            ),
            &custom.leading_actions,
            custom.resolve_label.clone(),
            resolve,
        )),
    }
}

pub(super) fn unresolved_after_notification_notice(
    params: &ConflictParams,
    resolve: ResolveAgain,
) -> BranchNotice {
    with_resolve_action(
        titled(
            BranchNotice::warning(plain_notification_text(
                &params.error_notification_additional_description,
            )),
            PENDING_UNRESOLVED_TITLE,
        ),
        &[],
        SharedString::new_static(RESOLVE_LABEL),
        resolve,
    )
}

pub(super) fn exception_notice(params: &ConflictParams, error_message: &str) -> BranchNotice {
    let description = plain_notification_text(&format!(
        "{UNMERGED_CHECK_ERROR_PREFIX}{}",
        params.error_notification_additional_description
    ));
    titled(
        BranchNotice::error(format!("{description}\n{error_message}")),
        &params.error_notification_title,
    )
}

pub(crate) async fn retain_entries_on_disk(
    fs: &dyn Fs,
    entries: Vec<UnmergedEntry>,
    absolute_paths: &[PathBuf],
) -> Result<Vec<UnmergedEntry>> {
    let metadata = join_all(absolute_paths.iter().map(|path| fs.metadata(path))).await;
    let mut existing = Vec::with_capacity(entries.len());
    for (entry, metadata) in entries.into_iter().zip(metadata) {
        if metadata?.is_some() {
            existing.push(entry);
        }
    }
    Ok(existing)
}

pub(crate) async fn unmerged_files_on_disk(
    context: &ConflictContext,
    cx: &mut AsyncApp,
) -> Result<Vec<UnmergedEntry>> {
    let listing = context
        .repository
        .update(cx, |repository, cx| repository.unmerged_entries(cx));
    let entries = listing.await?;
    let fs = context.workspace.read_with(cx, |workspace, cx| {
        workspace.project().read(cx).fs().clone()
    })?;
    let absolute_paths: Vec<PathBuf> = context.repository.read_with(cx, |repository, _| {
        entries
            .iter()
            .map(|entry| repository.repo_path_to_abs_path(&entry.path))
            .collect()
    });
    let mut existing = retain_entries_on_disk(fs.as_ref(), entries, &absolute_paths).await?;
    existing.sort_by(|left, right| left.path.as_unix_str().cmp(right.path.as_unix_str()));
    Ok(existing)
}

struct Resolver {
    effects: DriverEffects,
    context: ConflictContext,
    params: ConflictParams,
    behavior: ResolverBehavior,
}

impl Resolver {
    async fn merge(&self, mode: ResolveMode, cx: &mut AsyncApp) -> ResolveOutcome {
        let cycle = match self.dialog_cycle(cx).await {
            Ok(cycle) => cycle,
            Err(error) => {
                self.notify_exception(&error, cx);
                return ResolveOutcome::default();
            }
        };
        let proceed = match next_step(cycle, mode) {
            NextStep::ProceedIfNothingToMerge => self.proceed(Hook::NothingToMerge, cx).await,
            NextStep::Finished => true,
            NextStep::ProceedAfterAllMerged {
                should_finish_merge,
            } => {
                self.proceed(
                    Hook::AllMerged {
                        should_finish_merge,
                    },
                    cx,
                )
                .await
            }
            NextStep::NotifyUnresolvedRemain => {
                self.notify_unresolved_remain(cx);
                false
            }
            NextStep::NotifyUnresolvedRemainAfterNotification => {
                self.notify_unresolved_remain_after_notification(cx);
                false
            }
        };
        outcome_for(cycle, proceed)
    }

    async fn proceed(&self, hook: Hook, cx: &mut AsyncApp) -> bool {
        let result = match proceed_plan(self.behavior, hook) {
            ProceedPlan::Nothing => Ok(true),
            ProceedPlan::CommitMerge => self.commit_merge_if_merging(cx).await.map(|()| true),
            ProceedPlan::ContinueRebase => Ok(self.continue_rebase(cx).await),
        };
        match result {
            Ok(proceed) => proceed,
            Err(error) => {
                self.notify_exception(&error, cx);
                false
            }
        }
    }

    async fn dialog_cycle(&self, cx: &mut AsyncApp) -> Result<DialogCycle> {
        let entries = self.unmerged_files(cx).await?;
        if entries.is_empty() {
            return Ok(DialogCycle::NothingToMerge);
        }
        let result = self.show_dialog(entries, cx).await?;
        log::debug!(
            "conflicts dialog processed {} file(s)",
            result.processed_files.len()
        );
        let remaining = self.unmerged_files(cx).await?;
        let should_finish_merge = result.should_finish_merge;
        Ok(if remaining.is_empty() {
            DialogCycle::AllMerged {
                should_finish_merge,
            }
        } else {
            DialogCycle::Remain {
                should_finish_merge,
            }
        })
    }

    fn project_fs(&self, cx: &AsyncApp) -> Result<Arc<dyn Fs>> {
        self.context.workspace.read_with(cx, |workspace, cx| {
            workspace.project().read(cx).fs().clone()
        })
    }

    async fn unmerged_files(&self, cx: &mut AsyncApp) -> Result<Vec<UnmergedEntry>> {
        unmerged_files_on_disk(&self.context, cx).await
    }

    async fn show_dialog(
        &self,
        entries: Vec<UnmergedEntry>,
        cx: &mut AsyncApp,
    ) -> Result<ConflictsDialogResult> {
        let fs = self.project_fs(cx)?;
        let files_count = entries.len();
        let reverse = self.params.reverse;
        let texts = cx.update(|app| {
            load_dialog_texts(
                &self.context.repository,
                &fs,
                &self.params.customizer,
                reverse,
                app,
            )
        });
        let description = cx.update(|app| {
            load_description(
                &self.context.repository,
                &fs,
                &self.params.customizer,
                files_count,
                app,
            )
        });
        let texts = texts.await.warn_on_err().unwrap_or_else(|| {
            build_dialog_texts(
                &self.params.customizer,
                &RepositoryFacts::default(),
                None,
                reverse,
            )
        });
        let (sender, receiver) = oneshot::channel();
        let request = ConflictsDialogRequest {
            workspace: self.context.workspace.clone(),
            owner_window: self.context.window,
            repository: self.context.repository.clone(),
            entries,
            texts,
            description,
            reversed: reverse,
            on_closed: Box::new(move |result: ConflictsDialogResult, _cx: &mut App| {
                if sender.send(result).is_err() {
                    log::warn!("the conflicts dialog closed after its resolver stopped waiting");
                }
            }),
        };
        cx.update(|app| (self.effects.open_dialog)(request, app));
        match receiver.await {
            Ok(result) => Ok(result),
            Err(oneshot::Canceled) => {
                log::warn!("the conflicts dialog was dropped without reporting a result");
                Ok(ConflictsDialogResult {
                    processed_files: Vec::new(),
                    should_finish_merge: false,
                })
            }
        }
    }

    async fn commit_merge_if_merging(&self, cx: &mut AsyncApp) -> Result<()> {
        let operation = self
            .context
            .repository
            .update(cx, |repository, _| repository.operation_in_progress());
        if operation.await?? != Some(RepositoryOperation::Merge) {
            return Ok(());
        }
        let commit = self
            .context
            .repository
            .update(cx, |repository, cx| repository.commit_merge(cx));
        commit.await
    }

    async fn run_rebase(&self, action: RebaseAction, cx: &mut AsyncApp) -> Result<RebaseOutcome> {
        let receiver = self
            .context
            .repository
            .update(cx, |repository, _| repository.rebase(action));
        receiver.await?
    }

    async fn continue_rebase(&self, cx: &mut AsyncApp) -> bool {
        let mut action = RebaseAction::Continue;
        loop {
            match self.run_rebase(action, cx).await {
                Ok(RebaseOutcome::Completed { .. }) => return true,
                Ok(RebaseOutcome::Aborted) => return false,
                Ok(RebaseOutcome::Conflicted { output }) => match self.dialog_cycle(cx).await {
                    Err(error) => {
                        self.notify_exception(&error, cx);
                        return false;
                    }
                    Ok(DialogCycle::AllMerged { .. }) => action = RebaseAction::Continue,
                    Ok(DialogCycle::Remain { .. }) => {
                        self.notify_unresolved_remain(cx);
                        return false;
                    }
                    Ok(DialogCycle::NothingToMerge) => match classify_rebase_stop(&output) {
                        RebaseStop::NothingToCommit => action = RebaseAction::Skip,
                        RebaseStop::StoppedForEditing => {
                            self.notify(
                                BranchNotice::info("").title(REBASE_STOPPED_FOR_EDITING_TITLE),
                                cx,
                            );
                            return false;
                        }
                        RebaseStop::Conflict => {
                            self.notify_unresolved_remain(cx);
                            return false;
                        }
                        RebaseStop::Other => {
                            self.notify_rebase_error(&output, cx);
                            return false;
                        }
                    },
                },
                Err(error) => {
                    self.notify_rebase_error(&error_message(&error), cx);
                    return false;
                }
            }
        }
    }

    fn resolve_again(&self) -> ResolveAgain {
        let effects = self.effects.clone();
        let context = self.context.clone();
        let params = self.params.clone();
        let behavior = self.behavior;
        Rc::new(move |_window: &mut Window, cx: &mut App| {
            resolve_conflicts_with(
                effects.clone(),
                context.clone(),
                params.clone(),
                behavior,
                ResolveMode::FromNotification,
                cx,
            )
            .detach();
        })
    }

    fn notify(&self, notice: BranchNotice, cx: &mut AsyncApp) {
        cx.update(|app| (self.effects.notify)(notice, app));
    }

    fn notify_unresolved_remain(&self, cx: &mut AsyncApp) {
        if let Some(notice) = unresolved_conflicts_notice(&self.params, self.resolve_again()) {
            self.notify(notice, cx);
        }
    }

    fn notify_unresolved_remain_after_notification(&self, cx: &mut AsyncApp) {
        self.notify(
            unresolved_after_notification_notice(&self.params, self.resolve_again()),
            cx,
        );
    }

    fn notify_exception(&self, error: &anyhow::Error, cx: &mut AsyncApp) {
        self.notify(exception_notice(&self.params, &error_message(error)), cx);
    }

    fn notify_rebase_error(&self, message: &str, cx: &mut AsyncApp) {
        self.notify(
            BranchNotice::error(message.to_string()).title(REBASE_ERROR_TITLE),
            cx,
        );
    }
}

pub(super) fn resolve_conflicts_with(
    effects: DriverEffects,
    context: ConflictContext,
    params: ConflictParams,
    behavior: ResolverBehavior,
    mode: ResolveMode,
    cx: &mut App,
) -> Task<ResolveOutcome> {
    let resolver = Resolver {
        effects,
        context,
        params,
        behavior,
    };
    cx.spawn(async move |cx| resolver.merge(mode, cx).await)
}

pub(crate) fn resolve_conflicts(
    context: ConflictContext,
    params: ConflictParams,
    behavior: ResolverBehavior,
    mode: ResolveMode,
    cx: &mut App,
) -> Task<ResolveOutcome> {
    let effects = DriverEffects::production(&context);
    resolve_conflicts_with(effects, context, params, behavior, mode, cx)
}
