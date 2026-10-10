use git::repository::CommitSummary;
use gpui::{
    AnyElement, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, SharedString, Window, rems,
};
use menu::{Cancel, Confirm};
use ui::{Checkbox, Modal, ModalFooter, ModalHeader, Section, prelude::*};
use workspace::ModalView;

const DELETE_REMOTE_DIALOG_WIDTH: f32 = 30.;
const UNMERGED_DIALOG_WIDTH: f32 = 40.;
const COMMIT_LIST_MAX_HEIGHT: f32 = 16.;
const SHORT_SHA_LENGTH: usize = 8;

pub type DeleteRemoteConfirm = Box<dyn FnOnce(bool, &mut Window, &mut App)>;
pub type RestoreBranchConfirm = Box<dyn FnOnce(&mut Window, &mut App)>;

pub fn short_sha(sha: &str) -> &str {
    sha.get(..SHORT_SHA_LENGTH).unwrap_or(sha)
}

pub fn tracking_branches_label(tracking_branches: &[SharedString]) -> Option<String> {
    match tracking_branches {
        [] => None,
        [only] => Some(format!("Delete tracking local branch {only} as well")),
        _ => Some("Delete tracking local branches:".to_string()),
    }
}

pub fn render_commit_list(
    id: &'static str,
    commits: &[CommitSummary],
    empty_message: &'static str,
) -> AnyElement {
    if commits.is_empty() {
        return Label::new(empty_message)
            .size(LabelSize::Small)
            .color(Color::Muted)
            .into_any_element();
    }

    v_flex()
        .id(id)
        .w_full()
        .max_h(rems(COMMIT_LIST_MAX_HEIGHT))
        .overflow_y_scroll()
        .children(commits.iter().enumerate().map(|(index, commit)| {
            h_flex()
                .id((id, index))
                .w_full()
                .gap_2()
                .px_1()
                .child(
                    Label::new(short_sha(&commit.sha).to_string())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Label::new(commit.subject.clone())
                            .size(LabelSize::Small)
                            .truncate(),
                    ),
                )
                .child(
                    Label::new(commit.author_name.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
        }))
        .into_any_element()
}

pub struct DeleteRemoteBranchDialog {
    remote_branch: SharedString,
    tracking_branches: Vec<SharedString>,
    delete_tracking_branches: bool,
    on_confirm: Option<DeleteRemoteConfirm>,
    focus_handle: FocusHandle,
}

impl DeleteRemoteBranchDialog {
    pub fn new(
        remote_branch: SharedString,
        tracking_branches: Vec<SharedString>,
        on_confirm: DeleteRemoteConfirm,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            remote_branch,
            tracking_branches,
            delete_tracking_branches: false,
            on_confirm: Some(on_confirm),
            focus_handle: cx.focus_handle(),
        }
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_confirm) = self.on_confirm.take() {
            on_confirm(self.delete_tracking_branches, window, cx);
        }
        cx.emit(DismissEvent);
    }

    fn render_tracking_option(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label = tracking_branches_label(&self.tracking_branches)?;
        let lists_branches = self.tracking_branches.len() > 1;
        let checkbox = Checkbox::new(
            "delete-tracking-local-branches",
            self.delete_tracking_branches.into(),
        )
        .label(label)
        .on_click(cx.listener(|this, state: &ToggleState, _window, cx| {
            this.delete_tracking_branches = state.selected();
            cx.notify();
        }));

        Some(
            v_flex()
                .gap_1()
                .child(checkbox)
                .when(lists_branches, |this| {
                    this.children(self.tracking_branches.iter().map(|branch| {
                        div().ml_6().child(
                            Label::new(branch.clone())
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    }))
                })
                .into_any_element(),
        )
    }
}

impl EventEmitter<DismissEvent> for DeleteRemoteBranchDialog {}

impl ModalView for DeleteRemoteBranchDialog {}

impl Focusable for DeleteRemoteBranchDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DeleteRemoteBranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tracking_option = self.render_tracking_option(cx);

        v_flex()
            .key_context("DeleteRemoteBranchDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .w(rems(DELETE_REMOTE_DIALOG_WIDTH))
            .elevation_3(cx)
            .overflow_hidden()
            .child(
                Modal::new("delete-remote-branch-dialog", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .headline("Delete Remote Branch")
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new()
                            .child(Label::new(format!(
                                "Delete remote branch {}?",
                                self.remote_branch
                            )))
                            .children(tracking_option),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(Button::new("delete-remote-cancel", "Cancel").on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    }),
                                ))
                                .child(
                                    Button::new("delete-remote-confirm", "Delete")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm(&Confirm, window, cx);
                                        })),
                                ),
                        ),
                    ),
            )
    }
}

pub struct UnmergedCommitsDialog {
    branch: SharedString,
    base: SharedString,
    commits: Vec<CommitSummary>,
    on_restore: Option<RestoreBranchConfirm>,
    focus_handle: FocusHandle,
}

impl UnmergedCommitsDialog {
    pub fn new(
        branch: SharedString,
        base: SharedString,
        commits: Vec<CommitSummary>,
        on_restore: RestoreBranchConfirm,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            branch,
            base,
            commits,
            on_restore: Some(on_restore),
            focus_handle: cx.focus_handle(),
        }
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn restore(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_restore) = self.on_restore.take() {
            on_restore(window, cx);
        }
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for UnmergedCommitsDialog {}

impl ModalView for UnmergedCommitsDialog {}

impl Focusable for UnmergedCommitsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UnmergedCommitsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("UnmergedCommitsDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::restore))
            .w(rems(UNMERGED_DIALOG_WIDTH))
            .elevation_3(cx)
            .overflow_hidden()
            .child(
                Modal::new("unmerged-commits-dialog", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .headline("Branch Was Not Fully Merged")
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new()
                            .child(Label::new(format!(
                                "The branch {} was not fully merged to {}.",
                                self.branch, self.base
                            )))
                            .child(Label::new("Below is the list of unmerged commits."))
                            .child(render_commit_list(
                                "unmerged-commits",
                                &self.commits,
                                "No unmerged commits",
                            )),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(Button::new("unmerged-close", "Close").on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    }),
                                ))
                                .child(
                                    Button::new("unmerged-restore", "Restore")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.restore(&Confirm, window, cx);
                                        })),
                                ),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sha_truncates_long_hashes_and_keeps_short_ones() {
        assert_eq!(
            short_sha("0123456789abcdef0123456789abcdef01234567"),
            "01234567"
        );
        assert_eq!(short_sha("abc"), "abc");
        assert_eq!(short_sha(""), "");
    }

    #[test]
    fn short_sha_does_not_split_a_multibyte_character() {
        let sha = "aaaaaaa\u{e9}tail";
        assert_eq!(short_sha(sha), sha);
    }

    #[test]
    fn tracking_label_is_absent_without_tracking_branches() {
        assert_eq!(tracking_branches_label(&[]), None);
    }

    #[test]
    fn tracking_label_names_the_single_tracking_branch() {
        let branches = [SharedString::from("feature/login")];
        assert_eq!(
            tracking_branches_label(&branches).as_deref(),
            Some("Delete tracking local branch feature/login as well")
        );
    }

    #[test]
    fn tracking_label_lists_several_tracking_branches() {
        let branches = [SharedString::from("a"), SharedString::from("b")];
        assert_eq!(
            tracking_branches_label(&branches).as_deref(),
            Some("Delete tracking local branches:")
        );
    }
}
