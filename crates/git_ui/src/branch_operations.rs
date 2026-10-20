pub mod branch_notification;
pub mod branch_workspace;
pub mod checkout;
pub mod checkout_revision_dialog;
pub mod compare;
pub mod compare_view;
pub mod delete_dialogs;
pub mod integrate;
pub mod manage;
pub mod new_branch_dialog;
pub mod ongoing;
pub mod push_dialog;
pub mod push_result;
pub mod ref_diff;
pub mod ref_suggestions;
pub mod rename_dialog;
pub mod smart_operation;
pub mod update_dialog;

#[cfg(test)]
mod operation_tests;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use askpass::AskPassDelegate;
use collections::HashSet;
use futures::StreamExt as _;
use futures::channel::mpsc;
use git::repository::{
    Branch, GitFailure, REMOTE_CANCELLED_BY_USER, RemoteProgress, RemoteProgressSender,
};
use git_ui_core::askpass_modal::AskPassModal;
use gpui::{App, AppContext as _, Context, Entity, SharedString, Task, WeakEntity, Window};
use language::{Buffer, DiskState};
use project::git_store::{Repository, RepositorySnapshot};
use util::ResultExt as _;
use workspace::{BackgroundTaskProgress, ModalView, Workspace, notifications::NotificationId};

use self::branch_notification::BranchNotification;
use crate::branch_refs::{RefKind, RefTarget};
use crate::merge_tool;
use crate::merge_tool::conflict_resolution::{
    ConflictContext, ConflictParams, ResolveMode, ResolveOutcome, ResolverBehavior,
    resolve_conflicts,
};

const MAX_QUOTED_REF_NAME_CHARS: usize = 40;
// IntelliJ's `GitBranchPresentation.truncateBranchName` leaves names up to four characters over
// the limit untouched and keeps the last five characters of longer ones.
const QUOTED_REF_NAME_DELTA: usize = 4;
const QUOTED_REF_NAME_SUFFIX: usize = 5;
const ELLIPSIS: char = '\u{2026}';
pub(crate) const PUSH_PROGRESS_TITLE: &str = "Pushing\u{2026}";
pub(crate) const UPDATE_PROGRESS_TITLE: &str = "Updating\u{2026}";
pub(crate) const UPDATE_BRANCHES_PROGRESS_TITLE: &str = "Updating branches\u{2026}";

pub(crate) fn checking_out_progress_title(reference: &str) -> String {
    format!("Checking out {reference}{ELLIPSIS}")
}

pub(crate) fn checking_out_new_branch_progress_title(name: &str) -> String {
    format!("Checking out new branch {name}{ELLIPSIS}")
}

pub(crate) fn checking_out_from_progress_title(name: &str, start: &str) -> String {
    format!("Checking out {name} from {start}{ELLIPSIS}")
}

pub(crate) fn creating_branch_progress_title(name: &str) -> String {
    format!("Creating branch {name}{ELLIPSIS}")
}

pub(crate) fn merging_progress_title(reference: &str) -> String {
    format!("Merging {reference}{ELLIPSIS}")
}

pub(crate) fn rebasing_progress_title(branch: &str) -> String {
    format!("Rebasing {branch}{ELLIPSIS}")
}

pub(crate) fn rebasing_onto_progress_title(reference: &str) -> String {
    format!("Rebasing onto {reference}{ELLIPSIS}")
}

pub(crate) fn deleting_progress_title(names: &[impl AsRef<str>]) -> String {
    let names = names
        .iter()
        .map(|name| name.as_ref())
        .collect::<Vec<_>>()
        .join(", ");
    format!("Deleting {names}{ELLIPSIS}")
}

pub(crate) fn renaming_progress_title(old_name: &str, new_name: &str) -> String {
    format!("Renaming {old_name} to {new_name}{ELLIPSIS}")
}

pub(crate) fn deleting_tag_on_remote_progress_title(tag: &str) -> String {
    format!("Deleting tag {tag} on remote{ELLIPSIS}")
}

#[derive(Clone)]
pub struct BranchContext {
    pub workspace: WeakEntity<Workspace>,
    pub repository: Entity<Repository>,
}

impl BranchContext {
    pub fn new(workspace: WeakEntity<Workspace>, repository: Entity<Repository>) -> Self {
        Self {
            workspace,
            repository,
        }
    }

    pub fn snapshot(&self, cx: &App) -> RepositorySnapshot {
        self.repository.read(cx).snapshot()
    }

    /// Goes through the worktree switch so the worktree becomes a project tab in this window,
    /// or the window that already holds it comes to the front; it never opens a new window.
    pub fn open_worktree(&self, path: PathBuf, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let action = zed_actions::SwitchWorktree { path, display_name };
        workspace.update(cx, |workspace, cx| {
            git_ui_core::worktree_service::handle_switch_worktree(
                workspace, &action, window, None, cx,
            )
        });
    }

    pub fn current_branch(&self, cx: &App) -> Option<Branch> {
        self.repository.read(cx).branch.clone()
    }

    pub fn current_branch_name(&self, cx: &App) -> Option<SharedString> {
        self.current_branch(cx)
            .map(|branch| SharedString::from(branch.name().to_string()))
    }

    pub fn find_branch(&self, target: &RefTarget, cx: &App) -> Option<Branch> {
        let is_remote = match target.kind {
            RefKind::Local => false,
            RefKind::Remote => true,
            RefKind::Tag => return None,
        };
        self.repository
            .read(cx)
            .branch_list
            .iter()
            .find(|branch| branch.is_remote() == is_remote && branch.name() == &*target.name)
            .cloned()
    }

    pub fn askpass_delegate(
        &self,
        operation: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> AskPassDelegate {
        let workspace = self.workspace.clone();
        let operation = operation.into();
        let window_handle = window.window_handle();
        AskPassDelegate::new_with_cancellation(
            &mut cx.to_async(),
            move |prompt, tx, cancellation, cx| {
                window_handle
                    .update(cx, |_, window, cx| {
                        workspace.update(cx, |workspace, cx| {
                            workspace.toggle_modal(window, cx, |window, cx| {
                                AskPassModal::new(
                                    operation.clone(),
                                    prompt.into(),
                                    tx,
                                    cancellation,
                                    window,
                                    cx,
                                )
                            });
                        })
                    })
                    .ok();
            },
        )
    }

    /// Saves every modified file-backed buffer without asking, like IntelliJ's
    /// `saveAllDocuments` before a branch operation, so the operation sees and
    /// carries the edits instead of changing files under unsaved buffers.
    pub fn save_documents(&self, cx: &mut App) -> Task<()> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Task::ready(());
        };
        let project = workspace.read(cx).project().clone();
        let buffers: HashSet<Entity<Buffer>> = project
            .read(cx)
            .opened_buffers(cx)
            .into_iter()
            .filter(|buffer| saves_before_operation(buffer.read(cx)))
            .collect();
        if buffers.is_empty() {
            return Task::ready(());
        }
        let saving = project.update(cx, |project, cx| project.save_buffers(buffers, cx));
        cx.spawn(async move |_| {
            saving.await.log_err();
        })
    }

    pub fn start_progress(
        &self,
        title: impl Into<SharedString>,
        cx: &mut App,
    ) -> OperationProgress {
        OperationProgress::start(&self.workspace, title, cx)
    }

    pub fn notify(&self, notice: BranchNotice, cx: &mut App) {
        let workspace = self.workspace.clone();
        cx.defer(move |cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_notification(next_notification_id(), cx, |cx| {
                        cx.new(|cx| BranchNotification::new(notice, cx))
                    });
                })
                .log_err();
        });
    }

    pub fn notify_error(
        &self,
        operation: impl Into<SharedString>,
        error: anyhow::Error,
        cx: &mut App,
    ) {
        let operation = operation.into();
        if let Some(notice) = error_notice(&operation, &error) {
            self.notify(notice, cx);
        }
    }

    pub(crate) fn resolve_conflicts(
        &self,
        params: ConflictParams,
        behavior: ResolverBehavior,
        mode: ResolveMode,
        window: &Window,
        cx: &mut App,
    ) -> Task<ResolveOutcome> {
        let merge_tool_available = self.workspace.upgrade().is_some_and(|workspace| {
            merge_tool::is_available(workspace.read(cx).project().read(cx), cx)
        });
        if !merge_tool_available {
            return Task::ready(ResolveOutcome::default());
        }
        let context = ConflictContext {
            workspace: self.workspace.clone(),
            window: window.window_handle(),
            repository: self.repository.clone(),
        };
        resolve_conflicts(context, params, behavior, mode, cx)
    }

    pub fn open_modal<V: ModalView>(
        &self,
        window: &mut Window,
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    ) {
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.toggle_modal(window, cx, build);
                })
                .log_err();
        });
    }
}

fn saves_before_operation(buffer: &Buffer) -> bool {
    buffer.is_dirty()
        && !buffer.has_conflict()
        && buffer.file().is_some_and(|file| {
            matches!(
                file.disk_state(),
                DiskState::Present { .. } | DiskState::New
            )
        })
}

fn next_notification_id() -> NotificationId {
    static NEXT_NOTIFICATION_INDEX: AtomicUsize = AtomicUsize::new(0);
    NotificationId::composite::<BranchNotification>(
        NEXT_NOTIFICATION_INDEX.fetch_add(1, Ordering::Relaxed),
    )
}

/// A status-bar progress entry that lives as long as the operation holds it,
/// like the title of IntelliJ's `Task.Backgroundable` for branch operations.
pub struct OperationProgress {
    indicator: Entity<BackgroundTaskProgress>,
    _forwarding: Option<Task<()>>,
}

impl OperationProgress {
    pub fn start(
        workspace: &WeakEntity<Workspace>,
        title: impl Into<SharedString>,
        cx: &mut App,
    ) -> Self {
        let indicator = cx.new(|_| BackgroundTaskProgress::new(title));
        let weak_indicator = indicator.downgrade();
        let workspace = workspace.clone();
        // Operations start from handlers that may run inside a workspace update.
        cx.defer(move |cx| {
            let Some(indicator) = weak_indicator.upgrade() else {
                return;
            };
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_background_task(&indicator, cx)
                })
                .log_err();
        });
        Self {
            indicator,
            _forwarding: None,
        }
    }

    pub fn start_reporting(
        workspace: &WeakEntity<Workspace>,
        title: impl Into<SharedString>,
        cx: &mut App,
    ) -> (Self, RemoteProgressSender) {
        let mut progress = Self::start(workspace, title, cx);
        let (sender, mut receiver) = mpsc::unbounded::<RemoteProgress>();
        let weak_indicator = progress.indicator.downgrade();
        progress._forwarding = Some(cx.spawn(async move |cx| {
            while let Some(update) = receiver.next().await {
                let forwarded = weak_indicator.update(cx, |indicator, cx| {
                    indicator.set_progress(update.fraction, update.details, cx)
                });
                if forwarded.is_err() {
                    break;
                }
            }
        }));
        (progress, sender)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone)]
pub struct NoticeAction {
    pub label: SharedString,
    pub handler: Rc<dyn Fn(&mut Window, &mut App)>,
    /// Whether running the action closes the notice, which IntelliJ's
    /// `NotificationAction` does unless the action keeps it on purpose.
    pub expires: bool,
}

#[derive(Clone)]
pub struct BranchNotice {
    pub title: Option<SharedString>,
    pub message: SharedString,
    pub severity: NoticeSeverity,
    pub actions: Vec<NoticeAction>,
}

impl BranchNotice {
    pub fn info(message: impl Into<SharedString>) -> Self {
        Self::with_severity(message, NoticeSeverity::Info)
    }

    pub fn warning(message: impl Into<SharedString>) -> Self {
        Self::with_severity(message, NoticeSeverity::Warning)
    }

    pub fn error(message: impl Into<SharedString>) -> Self {
        Self::with_severity(message, NoticeSeverity::Error)
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push(NoticeAction {
            label: label.into(),
            handler: Rc::new(handler),
            expires: true,
        });
        self
    }

    pub fn action_keeping_notice(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push(NoticeAction {
            label: label.into(),
            handler: Rc::new(handler),
            expires: false,
        });
        self
    }

    fn with_severity(message: impl Into<SharedString>, severity: NoticeSeverity) -> Self {
        Self {
            title: None,
            message: message.into(),
            severity,
            actions: Vec::new(),
        }
    }
}

pub fn quoted_ref_label(name: &str) -> String {
    let length = name.chars().count();
    if length <= MAX_QUOTED_REF_NAME_CHARS + QUOTED_REF_NAME_DELTA {
        return format!("'{name}'");
    }
    let shortened_length = MAX_QUOTED_REF_NAME_CHARS - 1;
    let prefix: String = name
        .chars()
        .take(shortened_length - QUOTED_REF_NAME_SUFFIX - 1)
        .collect();
    let suffix: String = name.chars().skip(length - QUOTED_REF_NAME_SUFFIX).collect();
    format!("'{prefix}{ELLIPSIS}{suffix}'")
}

pub(crate) fn failure_text(error: &anyhow::Error) -> Option<String> {
    let description = format!("{error:#}");
    if description.contains(REMOTE_CANCELLED_BY_USER) {
        return None;
    }
    let text = match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
    {
        Some(failure) => failure.message.clone(),
        None => description,
    };
    Some(text)
}

pub fn error_notice(operation: &str, error: &anyhow::Error) -> Option<BranchNotice> {
    let message = failure_text(error)?.trim().to_string();
    Some(BranchNotice::error(message).title(failed_title(operation)))
}

/// IntelliJ names a failed git operation like "Fetch failed".
fn failed_title(operation: &str) -> String {
    let mut characters = operation.chars();
    match characters.next() {
        Some(first) => format!("{}{} failed", first.to_uppercase(), characters.as_str()),
        None => "Failed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use git::repository::GitFailureKind;
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn quoted_ref_label_wraps_short_names_in_single_quotes() {
        assert_eq!(quoted_ref_label("main"), "'main'");
        assert_eq!(quoted_ref_label("feature/login"), "'feature/login'");
        assert_eq!(quoted_ref_label(""), "''");
    }

    #[test]
    fn quoted_ref_label_keeps_a_name_of_exactly_the_limit_untouched() {
        let name = "a".repeat(MAX_QUOTED_REF_NAME_CHARS);
        assert_eq!(quoted_ref_label(&name), format!("'{name}'"));
    }

    #[test]
    fn quoted_ref_label_keeps_names_up_to_four_characters_over_the_limit_untouched() {
        let name = "a".repeat(44);
        assert_eq!(quoted_ref_label(&name), format!("'{name}'"));
    }

    #[test]
    fn quoted_ref_label_shortens_longer_names_in_the_middle_keeping_the_last_five_characters() {
        let name = format!("feature/{}-12345", "b".repeat(40));
        let label = quoted_ref_label(&name);
        let expected = format!("'{}{ELLIPSIS}12345'", &name[..33]);
        assert_eq!(label, expected);
        assert_eq!(label.trim_matches('\'').chars().count(), 39);
    }

    #[test]
    fn quoted_ref_label_truncates_on_character_boundaries() {
        let name = "\u{e9}".repeat(MAX_QUOTED_REF_NAME_CHARS + 5);
        let label = quoted_ref_label(&name);
        assert!(label.starts_with("'\u{e9}"));
        assert!(label.ends_with(&format!("{ELLIPSIS}{}'", "\u{e9}".repeat(5))));
        assert_eq!(label.trim_matches('\'').chars().count(), 39);
    }

    #[test]
    fn notice_constructors_set_severity_and_start_without_title_or_actions() {
        let info = BranchNotice::info("Done");
        assert_eq!(info.severity, NoticeSeverity::Info);
        assert_eq!(info.message.as_ref(), "Done");
        assert!(info.title.is_none());
        assert!(info.actions.is_empty());

        assert_eq!(
            BranchNotice::warning("Careful").severity,
            NoticeSeverity::Warning
        );
        assert_eq!(
            BranchNotice::error("Broken").severity,
            NoticeSeverity::Error
        );
    }

    #[test]
    fn notice_builders_keep_title_and_action_order() {
        let invoked = Rc::new(Cell::new(0));
        let notice = BranchNotice::warning("Merged with conflicts")
            .title("Merge")
            .action("Resolve\u{2026}", {
                let invoked = invoked.clone();
                move |_, _| invoked.set(invoked.get() + 1)
            })
            .action("Abort", |_, _| {});

        assert_eq!(notice.title.as_deref(), Some("Merge"));
        let labels: Vec<&str> = notice
            .actions
            .iter()
            .map(|action| action.label.as_ref())
            .collect();
        assert_eq!(labels, vec!["Resolve\u{2026}", "Abort"]);
        assert_eq!(invoked.get(), 0);
    }

    #[test]
    fn error_notice_is_hidden_when_the_user_cancelled_the_operation() {
        let error = anyhow::anyhow!(REMOTE_CANCELLED_BY_USER);
        assert!(error_notice("push", &error).is_none());

        let wrapped = anyhow::anyhow!(REMOTE_CANCELLED_BY_USER).context("pushing branch");
        assert!(error_notice("push", &wrapped).is_none());
    }

    #[test]
    fn error_notice_trims_the_message_and_titles_it_with_the_operation() {
        let error = anyhow::anyhow!("  fatal: unable to access remote \n");
        let notice = error_notice("fetch", &error).expect("a notice for a real failure");
        assert_eq!(notice.severity, NoticeSeverity::Error);
        assert_eq!(notice.message.as_ref(), "fatal: unable to access remote");
        assert_eq!(notice.title.as_deref(), Some("Fetch failed"));
        assert!(notice.actions.is_empty());
    }

    #[test]
    fn error_notice_prefers_the_git_failure_message_over_context() {
        let failure = GitFailure {
            kind: GitFailureKind::Other,
            message: "error: pathspec 'nope' did not match\n".to_string(),
        };
        let error = anyhow::Error::new(failure).context("checking out nope");
        let notice = error_notice("checkout", &error).expect("a notice for a real failure");
        assert_eq!(
            notice.message.as_ref(),
            "error: pathspec 'nope' did not match"
        );
    }

    #[test]
    fn error_notice_keeps_the_whole_context_chain_for_plain_errors() {
        let error = anyhow::anyhow!("connection reset").context("pushing branch");
        let notice = error_notice("push", &error).expect("a notice for a real failure");
        assert_eq!(notice.message.as_ref(), "pushing branch: connection reset");
    }

    #[test]
    fn failure_text_keeps_the_raw_git_output_and_hides_cancellations() {
        let failure = GitFailure {
            kind: GitFailureKind::Other,
            message: " ! [rejected]        main -> main (fetch first)\n".to_string(),
        };
        let error = anyhow::Error::new(failure).context("pushing main");
        assert_eq!(
            failure_text(&error).as_deref(),
            Some(" ! [rejected]        main -> main (fetch first)\n")
        );
        assert_eq!(
            failure_text(&anyhow::anyhow!("fatal: unable to access")).as_deref(),
            Some("fatal: unable to access")
        );
        assert_eq!(
            failure_text(&anyhow::anyhow!(REMOTE_CANCELLED_BY_USER)),
            None
        );
    }

    #[gpui::test]
    fn push_progress_forwards_git_progress_until_the_push_ends(cx: &mut TestAppContext) {
        let (progress, sender) = cx.update(|cx| {
            OperationProgress::start_reporting(&WeakEntity::new_invalid(), PUSH_PROGRESS_TITLE, cx)
        });
        let indicator = progress.indicator.downgrade();
        indicator
            .read_with(cx, |task, _| {
                assert_eq!(task.title().as_ref(), PUSH_PROGRESS_TITLE);
                assert_eq!(task.fraction(), None);
            })
            .expect("the indicator is alive while the push runs");

        sender
            .unbounded_send(RemoteProgress {
                fraction: 0.5,
                details: "Writing objects:  50% (1/2)".to_string(),
            })
            .expect("the progress receiver is alive while the push runs");
        cx.run_until_parked();
        indicator
            .read_with(cx, |task, _| {
                assert_eq!(task.fraction(), Some(0.5));
                assert_eq!(
                    task.details().map(|details| details.as_ref()),
                    Some("Writing objects:  50% (1/2)")
                );
            })
            .expect("the indicator is alive while the push runs");

        drop(progress);
        cx.run_until_parked();
        assert!(
            indicator.upgrade().is_none(),
            "the progress indicator outlived the push"
        );
    }
}
