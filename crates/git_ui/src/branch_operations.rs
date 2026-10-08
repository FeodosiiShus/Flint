pub mod branch_notification;
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
pub mod ref_diff;
pub mod ref_suggestions;
pub mod rename_dialog;
pub mod smart_operation;
pub mod update_dialog;

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use askpass::AskPassDelegate;
use git::repository::{Branch, GitFailure, REMOTE_CANCELLED_BY_USER};
use git_ui_core::askpass_modal::AskPassModal;
use gpui::{App, AppContext as _, Context, Entity, SharedString, WeakEntity, Window};
use project::git_store::{Repository, RepositorySnapshot};
use theme::ActiveTheme as _;
use util::ResultExt as _;
use workspace::{ModalView, Workspace, notifications::NotificationId};

use self::branch_notification::BranchNotification;
use crate::branch_refs::{RefKind, RefTarget};
use crate::merge_tool;

const MAX_QUOTED_REF_NAME_CHARS: usize = 40;
const ELLIPSIS: char = '\u{2026}';

pub fn opaque_elevated_surface(cx: &App) -> gpui::Hsla {
    cx.theme().colors().elevated_surface_background.alpha(1.0)
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

    pub fn open_conflicts_if_conflicted(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let merge_tool_available =
            merge_tool::is_available(workspace.read(cx).project().read(cx), cx);
        if merge_tool_available {
            merge_tool::open_conflicts_dialog_if_conflicted(
                self.workspace.clone(),
                self.repository.clone(),
                window,
                cx,
            );
        }
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

fn next_notification_id() -> NotificationId {
    static NEXT_NOTIFICATION_INDEX: AtomicUsize = AtomicUsize::new(0);
    NotificationId::composite::<BranchNotification>(
        NEXT_NOTIFICATION_INDEX.fetch_add(1, Ordering::Relaxed),
    )
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
    if name.chars().count() > MAX_QUOTED_REF_NAME_CHARS {
        let kept: String = name.chars().take(MAX_QUOTED_REF_NAME_CHARS - 1).collect();
        format!("'{kept}{ELLIPSIS}'")
    } else {
        format!("'{name}'")
    }
}

pub fn error_notice(operation: &str, error: &anyhow::Error) -> Option<BranchNotice> {
    if format!("{error:#}").contains(REMOTE_CANCELLED_BY_USER) {
        return None;
    }
    let message = match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
    {
        Some(failure) => failure.message.trim().to_string(),
        None => format!("{error:#}").trim().to_string(),
    };
    Some(BranchNotice::error(message).title(format!("git {operation} failed")))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use git::repository::GitFailureKind;

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
    fn quoted_ref_label_truncates_longer_names_with_an_ellipsis_to_the_limit() {
        let name = "b".repeat(MAX_QUOTED_REF_NAME_CHARS + 1);
        let label = quoted_ref_label(&name);
        let expected = format!("'{}{ELLIPSIS}'", "b".repeat(MAX_QUOTED_REF_NAME_CHARS - 1));
        assert_eq!(label, expected);
        assert_eq!(
            label.trim_matches('\'').chars().count(),
            MAX_QUOTED_REF_NAME_CHARS
        );
    }

    #[test]
    fn quoted_ref_label_truncates_on_character_boundaries() {
        let name = "\u{e9}".repeat(MAX_QUOTED_REF_NAME_CHARS + 5);
        let label = quoted_ref_label(&name);
        assert!(label.starts_with("'\u{e9}"));
        assert!(label.ends_with(&format!("{ELLIPSIS}'")));
        assert_eq!(
            label.trim_matches('\'').chars().count(),
            MAX_QUOTED_REF_NAME_CHARS
        );
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
        assert_eq!(notice.title.as_deref(), Some("git fetch failed"));
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
}
