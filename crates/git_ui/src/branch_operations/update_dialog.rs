use editor::Editor;
use git::repository::Branch;
use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString};
use menu::{Cancel, Confirm};
use ui::{Checkbox, ChoiceCard, DialogButton, prelude::*};
use workspace::ModalView;

use super::BranchContext;
use super::integrate::{
    UpdateMethod, reset_to_remote_branch, save_update_method, set_show_update_options,
    should_show_update_options, update_current_branch, update_from_chosen_branch,
};

const FIX_TRACKED_BRANCH_TITLE: &str = "Update";
const FIX_TRACKED_BRANCH_OK_LABEL: &str = "Update";
const SET_AS_TRACKED_LABEL: &str = "Set as tracked branch";
const BRANCH_PLACEHOLDER: &str = "Specify branch";

pub struct ResetTarget {
    pub branch: SharedString,
    pub upstream: SharedString,
}

pub struct UpdateOptionsDialog {
    context: BranchContext,
    method: UpdateMethod,
    dont_show_again: bool,
    reset_target: Option<ResetTarget>,
    focus_handle: FocusHandle,
}

impl UpdateOptionsDialog {
    pub fn new(
        context: BranchContext,
        method: UpdateMethod,
        reset_target: Option<ResetTarget>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            context,
            method,
            dont_show_again: !should_show_update_options(cx),
            reset_target,
            focus_handle: cx.focus_handle(),
        }
    }

    fn select_method(&mut self, method: UpdateMethod, cx: &mut Context<Self>) {
        self.method = method;
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.context.clone();
        let method = self.method;
        save_update_method(method, cx);
        set_show_update_options(!self.dont_show_again, cx);
        cx.emit(DismissEvent);
        update_current_branch(context, method, window, cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.context.clone();
        cx.emit(DismissEvent);
        reset_to_remote_branch(context, window, cx);
    }
}

impl EventEmitter<DismissEvent> for UpdateOptionsDialog {}
impl ModalView for UpdateOptionsDialog {}

impl Focusable for UpdateOptionsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UpdateOptionsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let merge_selected = self.method == UpdateMethod::Merge;
        let rebase_selected = self.method == UpdateMethod::Rebase;
        let reset_button = self.reset_target.as_ref().map(|target| {
            DialogButton::new("update-reset", "Reset to the Remote Branch")
                .default_button(false)
                .tooltip(ui::Tooltip::text(format!(
                    "Reset {} to {}",
                    target.branch, target.upstream
                )))
                .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx)))
        });

        v_flex()
            .key_context("UpdateOptionsDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_3(cx)
            .w(rems(34.))
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .w_full()
                    .child(Headline::new("Update Project").size(HeadlineSize::Small)),
            )
            .child(
                v_flex()
                    .id("update-method")
                    .role(Role::RadioGroup)
                    .px_3()
                    .py_1()
                    .gap_1()
                    .child(
                        ChoiceCard::radio(
                            "update-merge",
                            "Merge incoming changes into the current branch",
                            merge_selected,
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| {
                                this.select_method(UpdateMethod::Merge, cx)
                            }),
                        ),
                    )
                    .child(
                        ChoiceCard::radio(
                            "update-rebase",
                            "Rebase the current branch on top of incoming changes",
                            rebase_selected,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_method(UpdateMethod::Rebase, cx)
                        })),
                    ),
            )
            .child(
                div().px_3().py_1().child(
                    Checkbox::new("update-dont-show-again", self.dont_show_again.into())
                        .label("Don't show this dialog again")
                        .on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                            this.dont_show_again = state.selected();
                            cx.notify();
                        })),
                ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pt_1()
                    .pb_2()
                    .gap_2()
                    .justify_between()
                    .child(div().children(reset_button))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                DialogButton::new("update-cancel", "Cancel")
                                    .default_button(false)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel(&Cancel, window, cx)
                                    })),
                            )
                            .child(
                                DialogButton::new("update-confirm", "OK")
                                    .default_button(true)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(&Confirm, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedBranchChoice {
    pub remote: SharedString,
    pub branch: String,
    pub method: UpdateMethod,
    pub set_as_tracked: bool,
}

/// The remote and the remote branch IntelliJ's `FixTrackedBranchDialog` offers
/// first: the remote branch named like the local branch, else the first remote.
pub fn initial_tracked_branch(
    current_branch: &str,
    remote_branches: &[Branch],
) -> (Option<SharedString>, String) {
    let matching = remote_branches.iter().find_map(|branch| {
        let remote = branch.remote_name()?;
        let name = branch.name().strip_prefix(remote)?.strip_prefix('/')?;
        (name == current_branch).then(|| (SharedString::from(remote.to_string()), name.to_string()))
    });
    match matching {
        Some((remote, branch)) => (Some(remote), branch),
        None => (
            remote_names(remote_branches).into_iter().next(),
            String::new(),
        ),
    }
}

fn remote_names(remote_branches: &[Branch]) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = Vec::new();
    for branch in remote_branches {
        if let Some(remote) = branch.remote_name()
            && !names.iter().any(|known| known.as_ref() == remote)
        {
            names.push(SharedString::from(remote.to_string()));
        }
    }
    names
}

pub struct FixTrackedBranchDialog {
    context: BranchContext,
    remotes: Vec<SharedString>,
    remote: Option<SharedString>,
    remote_branches: Vec<String>,
    branch_editor: Entity<Editor>,
    set_as_tracked: bool,
    method: UpdateMethod,
}

impl FixTrackedBranchDialog {
    pub fn new(
        context: BranchContext,
        method: UpdateMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let branch_list = context.snapshot(cx).branch_list;
        let remote_branch_list: Vec<Branch> = branch_list
            .iter()
            .filter(|branch| branch.is_remote())
            .cloned()
            .collect();
        let current = context
            .current_branch_name(cx)
            .map(|name| name.to_string())
            .unwrap_or_default();
        let (remote, initial_branch) = initial_tracked_branch(&current, &remote_branch_list);
        let branch_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text(BRANCH_PLACEHOLDER, window, cx);
            editor.set_text(initial_branch, window, cx);
            editor
        });
        cx.observe(&branch_editor, |_, _, cx| cx.notify()).detach();
        Self {
            context,
            remotes: remote_names(&remote_branch_list),
            remote,
            remote_branches: remote_branch_list
                .iter()
                .map(|branch| branch.name().to_string())
                .collect(),
            branch_editor,
            set_as_tracked: false,
            method,
        }
    }

    fn choice(&self, cx: &App) -> Option<TrackedBranchChoice> {
        let remote = self.remote.clone()?;
        let branch = self.branch_editor.read(cx).text(cx).trim().to_string();
        if branch.is_empty() {
            return None;
        }
        let full_name = format!("{remote}/{branch}");
        if !self.remote_branches.contains(&full_name) {
            return None;
        }
        Some(TrackedBranchChoice {
            remote,
            branch,
            method: self.method,
            set_as_tracked: self.set_as_tracked,
        })
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice(cx) else {
            return;
        };
        let context = self.context.clone();
        cx.emit(DismissEvent);
        update_from_chosen_branch(context, choice, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn set_as_tracked(&mut self, set_as_tracked: bool) {
        self.set_as_tracked = set_as_tracked;
    }

    #[cfg(test)]
    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(&Confirm, window, cx);
    }
}

impl EventEmitter<DismissEvent> for FixTrackedBranchDialog {}
impl ModalView for FixTrackedBranchDialog {}

impl Focusable for FixTrackedBranchDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.branch_editor.focus_handle(cx)
    }
}

impl Render for FixTrackedBranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_update = self.choice(cx).is_some();
        let colors = cx.theme().colors();
        let remote_buttons = self.remotes.iter().enumerate().map(|(index, remote)| {
            let selected = self.remote.as_ref() == Some(remote);
            let remote = remote.clone();
            Button::new(("fix-tracked-remote", index), remote.clone())
                .style(ButtonStyle::Subtle)
                .toggle_state(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.remote = Some(remote.clone());
                    cx.notify();
                }))
        });

        v_flex()
            .key_context("FixTrackedBranchDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_3(cx)
            .w(rems(34.))
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .w_full()
                    .child(Headline::new(FIX_TRACKED_BRANCH_TITLE).size(HeadlineSize::Small)),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_1()
                    .w_full()
                    .children(remote_buttons)
                    .child(Label::new("/").color(Color::Muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.editor_background)
                            .child(self.branch_editor.clone()),
                    ),
            )
            .child(
                div().px_3().py_1().child(
                    Checkbox::new("fix-tracked-set-as-tracked", self.set_as_tracked.into())
                        .label(SET_AS_TRACKED_LABEL)
                        .on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                            this.set_as_tracked = state.selected();
                            cx.notify();
                        })),
                ),
            )
            .child(
                v_flex()
                    .id("fix-tracked-update-method")
                    .role(Role::RadioGroup)
                    .px_3()
                    .py_1()
                    .gap_1()
                    .child(
                        ChoiceCard::radio(
                            "fix-tracked-merge",
                            UpdateMethod::Merge.label(),
                            self.method == UpdateMethod::Merge,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.method = UpdateMethod::Merge;
                            cx.notify();
                        })),
                    )
                    .child(
                        ChoiceCard::radio(
                            "fix-tracked-rebase",
                            UpdateMethod::Rebase.label(),
                            self.method == UpdateMethod::Rebase,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.method = UpdateMethod::Rebase;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pt_1()
                    .pb_2()
                    .gap_2()
                    .justify_end()
                    .child(
                        DialogButton::new("fix-tracked-cancel", "Cancel")
                            .default_button(false)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                            ),
                    )
                    .child(
                        DialogButton::new("fix-tracked-confirm", FIX_TRACKED_BRANCH_OK_LABEL)
                            .default_button(true)
                            .disabled(!can_update)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm(&Confirm, window, cx)
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_branch(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/remotes/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    #[test]
    fn the_remote_branch_named_like_the_local_branch_is_offered_first() {
        let branches = [
            remote_branch("origin/main"),
            remote_branch("fork/feature"),
            remote_branch("origin/feature"),
        ];
        assert_eq!(
            initial_tracked_branch("feature", &branches),
            (Some(SharedString::from("fork")), "feature".to_string())
        );
    }

    #[test]
    fn without_a_matching_branch_the_first_remote_and_an_empty_branch_are_offered() {
        let branches = [remote_branch("origin/main"), remote_branch("fork/main")];
        assert_eq!(
            initial_tracked_branch("feature", &branches),
            (Some(SharedString::from("origin")), String::new())
        );
        assert_eq!(
            initial_tracked_branch("feature", &[]),
            (None, String::new())
        );
    }
}
