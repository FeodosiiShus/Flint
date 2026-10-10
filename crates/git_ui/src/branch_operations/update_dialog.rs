use gpui::{DismissEvent, EventEmitter, FocusHandle, Focusable, Role, SharedString};
use menu::{Cancel, Confirm};
use ui::{Checkbox, ChoiceCard, TintColor, prelude::*};
use workspace::ModalView;

use super::BranchContext;
use super::integrate::{
    UpdateMethod, reset_to_remote_branch, save_update_method, set_show_update_options,
    should_show_update_options, update_current_branch,
};

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
            Button::new("update-reset", "Reset to the Remote Branch")
                .style(ButtonStyle::Subtle)
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
                    .child(Headline::new("Update Project").size(HeadlineSize::XSmall)),
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
                            .child(Button::new("update-cancel", "Cancel").on_click(
                                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                            ))
                            .child(
                                Button::new("update-confirm", "OK")
                                    .style(ButtonStyle::Tinted(TintColor::Accent))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(&Confirm, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}
