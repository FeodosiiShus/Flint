use std::pin::pin;
use std::time::Duration;

use anyhow::Result;
use futures::channel::oneshot;
use futures::future::{self, Either};
use git::stash::StashEntry;
use gpui::{
    App, AsyncApp, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Task, Window,
};
use menu::Cancel;
use project::git_store::{Repository, RepositoryEvent};
use release_channel::ReleaseChannel;
use ui::{TintColor, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use crate::branch_operations::{BranchContext, opaque_elevated_surface};

const FALLBACK_PRODUCT_NAME: &str = "Flint";
pub(crate) const STASH_MESSAGE: &str = "Local changes stashed for a smart operation";
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

pub fn stash_local_changes(context: &BranchContext, cx: &mut App) -> Task<Result<bool>> {
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
        repository.stash_entries(paths, Some(STASH_MESSAGE.to_string()), cx)
    });
    cx.spawn(async move |cx| {
        stash.await?;
        wait_for_our_stashes(&repository, existing + 1, cx).await;
        anyhow::Ok(true)
    })
}

pub fn restore_local_changes(
    context: &BranchContext,
    stashed: bool,
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
                cx.update(|window, cx| context.open_conflicts_if_conflicted(window, cx))
                    .ok();
                Ok(())
            }
            Err(error) => Err(error),
        }
    })
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

struct SmartOperationDialog {
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

    fn choose(&mut self, choice: SmartChoice, cx: &mut Context<Self>) {
        if let Some(sender) = self.sender.take() {
            sender.send(choice).ok();
        }
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        self.choose(SmartChoice::Cancel, cx);
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
            Button::new("smart-operation-force", label)
                .color(Color::Error)
                .on_click(cx.listener(|this, _, _, cx| this.choose(SmartChoice::Force, cx)))
        });

        v_flex()
            .key_context("SmartOperationDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .elevation_3(cx)
            .bg(opaque_elevated_surface(cx))
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
                    .gap_1()
                    .justify_between()
                    .child(div().children(force_button))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("smart-operation-decline", operation.decline_label())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose(SmartChoice::Cancel, cx)
                                    })),
                            )
                            .child(
                                Button::new("smart-operation-smart", operation.smart_label())
                                    .style(ButtonStyle::Tinted(TintColor::Accent))
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
