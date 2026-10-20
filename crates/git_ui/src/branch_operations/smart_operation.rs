use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use db::kvp::KeyValueStore;
use futures::channel::oneshot;
use futures::future::{self, Either};
use git::stash::StashEntry;
use gpui::{
    Action as _, App, AsyncApp, AsyncWindowContext, Context, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, SharedString, Task, Window,
};
use menu::{Cancel, Confirm};
use project::git_store::{Repository, RepositoryEvent};
use release_channel::ReleaseChannel;
use time::OffsetDateTime;
use ui::{DialogButton, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use crate::branch_operations::{BranchContext, BranchNotice, NoticeAction};
use crate::merge_tool::conflict_resolution::{ConflictParams, ResolveMode, ResolverBehavior};

const VIEW_SAVED_CHANGES_LABEL: &str = "View saved changes…";
const LOCAL_CHANGES_NOT_RESTORED_TITLE: &str = "Local changes were not restored";
pub(super) const UPDATE_OPERATION_TITLE: &str = "Update";
const UPDATE_DESTINATION_NAME: &str = "Remote";

const FALLBACK_PRODUCT_NAME: &str = "Flint";
/// Every stash a branch operation makes starts with IntelliJ's
/// `stash.changes.message`, which is how the operation finds it again.
pub(crate) const STASH_MESSAGE: &str = "Uncommitted changes before ";
const REBASE_OPERATION_NAME: &str = "rebase";
const OPERATION_STASH_KEY_PREFIX: &str = "branch-operation-stash";
const STASH_KEPT_MARKER: &str = "kept in case you need it again";
const CONFLICT_MARKER: &str = "conflict";
const OVERWRITTEN_MARKER: &str = "would be overwritten";
const STASH_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartOperation {
    Checkout,
    Merge,
    Rebase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartChoice {
    Smart,
    Force,
    Cancel,
}

impl SmartOperation {
    pub fn title_case(self) -> &'static str {
        match self {
            SmartOperation::Checkout => "Checkout",
            SmartOperation::Merge => "Merge",
            SmartOperation::Rebase => "Rebase",
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            SmartOperation::Checkout => "checkout",
            SmartOperation::Merge => "merge",
            SmartOperation::Rebase => "rebase",
        }
    }

    pub fn dialog_title(self) -> String {
        format!("Git {} Problem", self.title_case())
    }

    pub fn smart_label(self) -> String {
        format!("Smart {}", self.title_case())
    }

    pub fn decline_label(self) -> String {
        format!("Don't {}", self.title_case())
    }

    pub fn force_label(self) -> Option<&'static str> {
        match self {
            SmartOperation::Checkout => Some("Force Checkout"),
            SmartOperation::Merge | SmartOperation::Rebase => None,
        }
    }

    pub fn problem_message(self) -> String {
        format!(
            "Your local changes to the following files would be overwritten by {}.",
            self.title_case()
        )
    }

    pub fn explanation(self, product_name: &str) -> String {
        format!(
            "{product_name} can stash the changes, {} and unstash them after that.",
            self.title_case()
        )
    }
}

pub fn confirm_smart_operation(
    context: &BranchContext,
    operation: SmartOperation,
    files: Vec<String>,
    window: &mut Window,
    cx: &mut App,
) -> Task<SmartChoice> {
    let (sender, receiver) = oneshot::channel();
    let product_name = ReleaseChannel::try_global(cx)
        .map(|channel| channel.display_name())
        .unwrap_or(FALLBACK_PRODUCT_NAME);
    context.open_modal(window, cx, move |_, cx| {
        SmartOperationDialog::new(operation, files, product_name, sender, cx)
    });
    cx.background_spawn(async move { receiver.await.unwrap_or(SmartChoice::Cancel) })
}

/// IntelliJ's `GitPreservingProcess` message: "Uncommitted changes before
/// Checkout at 10/20/26, 3:45 PM".
pub fn stash_message_with_date(operation_title: &str, now: OffsetDateTime) -> String {
    let date = now
        .format(STASH_DATE_FORMAT)
        .unwrap_or_else(|_| now.date().to_string());
    format!("{STASH_MESSAGE}{operation_title} at {date}")
}

/// IntelliJ's rebase saves local changes without a date.
pub fn rebase_stash_message() -> String {
    format!("{STASH_MESSAGE}{REBASE_OPERATION_NAME}")
}

pub fn stash_message_now(operation_title: &str) -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    stash_message_with_date(operation_title, now)
}

const STASH_DATE_FORMAT: &[time::format_description::BorrowedFormatItem<'static>] = time::macros::format_description!(
    "[month padding:none]/[day padding:none]/[year repr:last_two], [hour repr:12 padding:none]:[minute] [period]"
);

impl SmartOperation {
    pub fn stash_message(self) -> String {
        match self {
            SmartOperation::Rebase => rebase_stash_message(),
            SmartOperation::Checkout | SmartOperation::Merge => {
                stash_message_now(self.title_case())
            }
        }
    }
}

pub fn stash_local_changes(
    context: &BranchContext,
    message: String,
    cx: &mut App,
) -> Task<Result<bool>> {
    let repository = context.repository.clone();
    let paths: Vec<_> = repository
        .read(cx)
        .cached_status()
        .filter(|entry| !entry.status.is_untracked())
        .map(|entry| entry.repo_path)
        .collect();
    if paths.is_empty() {
        return Task::ready(Ok(false));
    }
    let existing = our_stash_count(&repository.read(cx).cached_stash().entries);
    let stash = repository.update(cx, |repository, cx| {
        repository.stash_entries(paths, Some(message), cx)
    });
    cx.spawn(async move |cx| {
        stash.await?;
        wait_for_our_stashes(&repository, existing + 1, cx).await;
        cx.update(|cx| remember_operation_stash(&repository, cx));
        anyhow::Ok(true)
    })
}

fn operation_stash_key(repository: &Repository) -> String {
    format!(
        "{OPERATION_STASH_KEY_PREFIX}:{}",
        repository.work_directory_abs_path.display()
    )
}

/// The message is IntelliJ's, so IntelliJ's own leftover stashes look the same; the stash's
/// commit is what tells the one this operation made apart when a later Abort or Continue
/// restores it.
fn remember_operation_stash(repository: &Entity<Repository>, cx: &App) {
    let repository = repository.read(cx);
    let stash = repository.cached_stash();
    let Some(entry) = stash
        .entries
        .iter()
        .find(|entry| entry.index == 0 && entry.message.contains(STASH_MESSAGE))
    else {
        return;
    };
    let key = operation_stash_key(repository);
    let oid = entry.oid.to_string();
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move { store.write_kvp(key, oid).await });
}

pub(crate) fn is_operation_stash(repository: &Repository, entry: &StashEntry, cx: &App) -> bool {
    entry.message.contains(STASH_MESSAGE)
        && KeyValueStore::global(cx)
            .read_kvp(&operation_stash_key(repository))
            .log_err()
            .flatten()
            .is_some_and(|oid| oid == entry.oid.to_string())
}

#[derive(Clone, Debug)]
pub struct RestoreLabels {
    operation_title: SharedString,
    destination_name: SharedString,
}

impl RestoreLabels {
    pub fn new(
        operation_title: impl Into<SharedString>,
        destination_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            operation_title: operation_title.into(),
            destination_name: destination_name.into(),
        }
    }

    pub fn update() -> Self {
        Self::new(UPDATE_OPERATION_TITLE, UPDATE_DESTINATION_NAME)
    }
}

pub fn restore_local_changes(
    context: &BranchContext,
    stashed: bool,
    labels: RestoreLabels,
    window: &mut Window,
    cx: &mut App,
) -> Task<Result<()>> {
    if !stashed {
        return Task::ready(Ok(()));
    }
    let context = context.clone();
    window.spawn(cx, async move |cx| {
        let Some(index) = find_our_stash(&context.repository, cx).await else {
            return Ok(());
        };
        let pop = cx.update(|_, cx| {
            context
                .repository
                .update(cx, |repository, cx| repository.stash_pop(Some(index), cx))
        })?;
        match pop.await {
            Ok(()) => Ok(()),
            Err(error) if stash_pop_left_conflicts(&error) => {
                resolve_restore_conflicts(&context, &labels, cx).await
            }
            Err(error) => Err(error),
        }
    })
}

fn view_saved_changes_action() -> NoticeAction {
    NoticeAction {
        label: SharedString::new_static(VIEW_SAVED_CHANGES_LABEL),
        handler: Rc::new(|window: &mut Window, cx: &mut App| {
            window.dispatch_action(zed_actions::git::ViewStash.boxed_clone(), cx)
        }),
        expires: true,
    }
}

pub(super) fn local_changes_not_restored_notice(operation: &str) -> BranchNotice {
    let mut notice = BranchNotice::warning(format!(
        "Before {operation} your uncommitted changes were saved to stash."
    ))
    .title(LOCAL_CHANGES_NOT_RESTORED_TITLE);
    notice.actions.push(view_saved_changes_action());
    notice
}

async fn resolve_restore_conflicts(
    context: &BranchContext,
    labels: &RestoreLabels,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let resolution = cx.update(|window, app| {
        context.resolve_conflicts(
            ConflictParams::for_smart_restore(
                &labels.operation_title,
                &labels.destination_name,
                true,
                vec![view_saved_changes_action()],
            ),
            ResolverBehavior::Plain,
            ResolveMode::Initial,
            window,
            app,
        )
    })?;
    resolution.await;
    Ok(())
}

pub fn stash_pop_left_conflicts(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_lowercase();
    if message.contains(OVERWRITTEN_MARKER) {
        return false;
    }
    message.contains(CONFLICT_MARKER) || message.contains(STASH_KEPT_MARKER)
}

fn our_stash_count(entries: &[StashEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.message.contains(STASH_MESSAGE))
        .count()
}

fn our_stash_index(entries: &[StashEntry]) -> Option<usize> {
    entries
        .iter()
        .filter(|entry| entry.message.contains(STASH_MESSAGE))
        .map(|entry| entry.index)
        .min()
}

async fn find_our_stash(repository: &Entity<Repository>, cx: &mut AsyncApp) -> Option<usize> {
    wait_for_our_stashes(repository, 1, cx).await;
    cx.update(|cx| our_stash_index(&repository.read(cx).cached_stash().entries))
}

async fn wait_for_our_stashes(
    repository: &Entity<Repository>,
    at_least: usize,
    cx: &mut AsyncApp,
) -> bool {
    let (sender, receiver) = async_channel::unbounded::<()>();
    let subscription = cx.update(|cx| {
        let notify_sender = sender.clone();
        let subscription = cx.subscribe(
            repository,
            move |repository, event: &RepositoryEvent, cx| {
                let stash = repository.read(cx).cached_stash();
                if matches!(event, RepositoryEvent::StashEntriesChanged)
                    && our_stash_count(&stash.entries) >= at_least
                {
                    notify_sender.try_send(()).log_err();
                }
            },
        );
        if our_stash_count(&repository.read(cx).cached_stash().entries) >= at_least {
            sender.try_send(()).log_err();
        }
        subscription
    });
    let received = pin!(receiver.recv());
    let timeout = pin!(cx.background_executor().timer(STASH_SETTLE_TIMEOUT));
    let settled = matches!(
        future::select(received, timeout).await,
        Either::Left((Ok(()), _))
    );
    drop(subscription);
    settled
}

pub(crate) struct SmartOperationDialog {
    operation: SmartOperation,
    files: Vec<SharedString>,
    explanation: SharedString,
    sender: Option<oneshot::Sender<SmartChoice>>,
    focus_handle: FocusHandle,
}

impl SmartOperationDialog {
    fn new(
        operation: SmartOperation,
        files: Vec<String>,
        product_name: &str,
        sender: oneshot::Sender<SmartChoice>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            operation,
            files: files.into_iter().map(SharedString::from).collect(),
            explanation: operation.explanation(product_name).into(),
            sender: Some(sender),
            focus_handle: cx.focus_handle(),
        }
    }

    pub(super) fn choose(&mut self, choice: SmartChoice, cx: &mut Context<Self>) {
        if let Some(sender) = self.sender.take() {
            sender.send(choice).ok();
        }
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        self.choose(SmartChoice::Cancel, cx);
    }

    fn confirm(&mut self, _: &Confirm, _window: &mut Window, cx: &mut Context<Self>) {
        self.choose(SmartChoice::Smart, cx);
    }
}

impl EventEmitter<DismissEvent> for SmartOperationDialog {}

impl ModalView for SmartOperationDialog {}

impl Focusable for SmartOperationDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SmartOperationDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let operation = self.operation;
        let force_button = operation.force_label().map(|label| {
            DialogButton::new("smart-operation-force", label)
                .default_button(false)
                .on_click(cx.listener(|this, _, _, cx| this.choose(SmartChoice::Force, cx)))
        });

        v_flex()
            .key_context("SmartOperationDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_3(cx)
            .w(rems(36.))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(Headline::new(operation.dialog_title()).size(HeadlineSize::Small)),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(Label::new(operation.problem_message()))
                    .child(Label::new(self.explanation.clone()).color(Color::Muted)),
            )
            .child(
                v_flex()
                    .id("smart-operation-files")
                    .mx_3()
                    .mb_2()
                    .p_2()
                    .max_h(rems(14.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .children(
                        self.files
                            .iter()
                            .map(|file| Label::new(file.clone()).size(LabelSize::Small)),
                    ),
            )
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .justify_between()
                    .child(div().children(force_button))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                DialogButton::new(
                                    "smart-operation-decline",
                                    operation.decline_label(),
                                )
                                .default_button(false)
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.choose(SmartChoice::Cancel, cx),
                                )),
                            )
                            .child(
                                DialogButton::new("smart-operation-smart", operation.smart_label())
                                    .default_button(true)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose(SmartChoice::Smart, cx)
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_messages_follow_the_intellij_preserving_process() {
        let now = time::macros::datetime!(2026-10-20 15:04 UTC);
        assert_eq!(
            stash_message_with_date("Checkout", now),
            "Uncommitted changes before Checkout at 10/20/26, 3:04 PM"
        );
        assert_eq!(rebase_stash_message(), "Uncommitted changes before rebase");
        assert!(
            our_stash_count(&[stash_entry(
                0,
                &format!("On main: {}", rebase_stash_message())
            )]) == 1
        );
    }

    #[gpui::test]
    fn enter_in_the_smart_dialog_chooses_the_smart_operation(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let (sender, mut receiver) = oneshot::channel();
        let (dialog, cx) = cx.add_window_view(|_, cx| {
            SmartOperationDialog::new(
                SmartOperation::Checkout,
                vec!["a.rs".to_string()],
                "Flint",
                sender,
                cx,
            )
        });
        dialog.update_in(cx, |dialog, window, cx| {
            dialog.confirm(&Confirm, window, cx)
        });
        assert_eq!(receiver.try_recv().ok().flatten(), Some(SmartChoice::Smart));
    }

    #[test]
    fn local_changes_not_restored_notice_follows_the_bundle_texts() {
        let notice = local_changes_not_restored_notice("merge");
        assert_eq!(
            notice.title.as_deref(),
            Some("Local changes were not restored")
        );
        assert_eq!(
            notice.message.as_ref(),
            "Before merge your uncommitted changes were saved to stash."
        );
        assert_eq!(
            notice.severity,
            crate::branch_operations::NoticeSeverity::Warning
        );
        let labels: Vec<&str> = notice
            .actions
            .iter()
            .map(|action| action.label.as_ref())
            .collect();
        assert_eq!(labels, vec!["View saved changes…"]);
    }

    #[test]
    fn dialog_texts_follow_the_git_problem_wording_for_every_operation() {
        assert_eq!(
            SmartOperation::Checkout.dialog_title(),
            "Git Checkout Problem"
        );
        assert_eq!(SmartOperation::Checkout.smart_label(), "Smart Checkout");
        assert_eq!(SmartOperation::Checkout.decline_label(), "Don't Checkout");
        assert_eq!(SmartOperation::Merge.dialog_title(), "Git Merge Problem");
        assert_eq!(SmartOperation::Merge.smart_label(), "Smart Merge");
        assert_eq!(SmartOperation::Merge.decline_label(), "Don't Merge");
        assert_eq!(SmartOperation::Rebase.dialog_title(), "Git Rebase Problem");
        assert_eq!(SmartOperation::Rebase.smart_label(), "Smart Rebase");
    }

    #[test]
    fn only_checkout_offers_the_force_button() {
        assert_eq!(
            SmartOperation::Checkout.force_label(),
            Some("Force Checkout")
        );
        assert_eq!(SmartOperation::Merge.force_label(), None);
        assert_eq!(SmartOperation::Rebase.force_label(), None);
    }

    #[test]
    fn problem_message_names_the_operation_and_the_explanation_names_the_product() {
        assert_eq!(
            SmartOperation::Checkout.problem_message(),
            "Your local changes to the following files would be overwritten by Checkout."
        );
        assert_eq!(
            SmartOperation::Merge.explanation("Flint"),
            "Flint can stash the changes, Merge and unstash them after that."
        );
    }

    #[test]
    fn stash_pop_failures_that_mention_conflicts_are_treated_as_conflicted_pops() {
        let conflicted =
            anyhow::anyhow!("Failed to stash pop:\nCONFLICT (content): Merge conflict in a.rs");
        let kept = anyhow::anyhow!(
            "Failed to stash pop:\nThe stash entry is kept in case you need it again."
        );
        let unrelated = anyhow::anyhow!("Failed to stash pop:\nNo stash entries found.");
        assert!(stash_pop_left_conflicts(&conflicted));
        assert!(stash_pop_left_conflicts(&kept));
        assert!(!stash_pop_left_conflicts(&unrelated));
    }

    #[test]
    fn stash_pop_refused_because_of_overwritten_local_changes_is_an_error_not_a_conflict() {
        let overwritten = anyhow::anyhow!(
            "Failed to stash pop:\nerror: Your local changes to the following files would be \
             overwritten by merge:\n\ta.rs\nPlease commit your changes or stash them before you \
             merge.\nAborting\nThe stash entry is kept in case you need it again."
        );
        let untracked = anyhow::anyhow!(
            "Failed to stash pop:\na.rs already exists, no checkout\nerror: The following \
             untracked working tree files would be overwritten by merge:\n\ta.rs\nThe stash \
             entry is kept in case you need it again."
        );
        assert!(!stash_pop_left_conflicts(&overwritten));
        assert!(!stash_pop_left_conflicts(&untracked));
    }

    fn stash_entry(index: usize, message: &str) -> StashEntry {
        StashEntry {
            index,
            oid: "a".repeat(40).parse().expect("valid object id"),
            message: message.to_string(),
            branch: None,
            timestamp: 0,
        }
    }

    #[test]
    fn only_stash_entries_created_by_a_smart_operation_are_counted() {
        let entries = vec![
            stash_entry(0, STASH_MESSAGE),
            stash_entry(1, "work in progress"),
            stash_entry(2, STASH_MESSAGE),
        ];
        assert_eq!(our_stash_count(&entries), 2);
        assert_eq!(our_stash_count(&[]), 0);
        assert_eq!(our_stash_count(&entries[1..2]), 0);
    }

    #[test]
    fn the_stash_to_restore_is_the_newest_entry_with_the_smart_operation_message() {
        let entries = vec![
            stash_entry(0, "user stash"),
            stash_entry(1, STASH_MESSAGE),
            stash_entry(2, STASH_MESSAGE),
        ];
        assert_eq!(our_stash_index(&entries), Some(1));
        assert_eq!(our_stash_index(&entries[..1]), None);
        assert_eq!(our_stash_index(&[]), None);
    }
}
