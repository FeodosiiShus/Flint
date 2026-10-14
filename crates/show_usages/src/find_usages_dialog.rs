use gpui::{App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, Pixels, Window, px};
use menu::{Cancel, Confirm};
use ui::{Checkbox, ContextMenu, DialogButton, DropdownMenu, DropdownStyle, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use crate::UsagesSession;
use crate::model::UsageScope;

const KEY_CONTEXT: &str = "FindUsagesDialog";
const DIALOG_WIDTH: Pixels = px(420.);
const TITLE: &str = "Find Usages";

pub(crate) struct FindUsagesDialog {
    session: UsagesSession,
    scope: UsageScope,
    focus_handle: FocusHandle,
}

impl FindUsagesDialog {
    pub(crate) fn new(session: UsagesSession, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            scope: session.scope,
            session,
            focus_handle: cx.focus_handle(),
        }
    }

    fn select_scope(&mut self, scope: UsageScope, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let scope = self.scope;
        let mut state = crate::saved_state(cx);
        state.scope = scope;
        crate::save_state(&state, cx);
        let mut session = self.session.clone();
        session.scope = scope;
        cx.emit(DismissEvent);
        crate::restart(session, scope, window, cx);
    }
}

impl EventEmitter<DismissEvent> for FindUsagesDialog {}

impl Focusable for FindUsagesDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for FindUsagesDialog {}

impl Render for FindUsagesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = cx.weak_entity();
        let current_scope = self.scope;
        let scope_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for scope in UsageScope::ALL {
                let dialog = dialog.clone();
                menu = menu.toggleable_entry(
                    scope.display_name(),
                    scope == current_scope,
                    IconPosition::Start,
                    None,
                    move |_, cx| {
                        dialog
                            .update(cx, |this, cx| this.select_scope(scope, cx))
                            .log_err();
                    },
                );
            }
            menu
        });
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_3(cx)
            .w(DIALOG_WIDTH)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .child(Headline::new(TITLE).size(HeadlineSize::XSmall)),
            )
            .child(
                div().w_full().px_3().pb_2().child(
                    Label::new(self.session.title())
                        .color(Color::Muted)
                        .single_line()
                        .truncate(),
                ),
            )
            .child(
                v_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_1()
                    .child(Label::new("Find:").color(Color::Muted))
                    .child(
                        div().pl_2().child(
                            Checkbox::new("find-usages-usages", ToggleState::Selected)
                                .label("Usages")
                                .disabled(true),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Label::new("Scope:"))
                    .child(
                        div().flex_1().min_w_0().child(
                            DropdownMenu::new(
                                "find-usages-scope",
                                current_scope.display_name(),
                                scope_menu,
                            )
                            .style(DropdownStyle::Outlined)
                            .full_width(true),
                        ),
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
                        DialogButton::new("find-usages-cancel", "Cancel")
                            .default_button(false)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                            ),
                    )
                    .child(
                        DialogButton::new("find-usages-find", "Find")
                            .default_button(true)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm(&Confirm, window, cx)
                            })),
                    ),
            )
    }
}
