pub(crate) mod chrome;
mod confirmation;
mod finish;
pub(crate) mod layout;
#[cfg(test)]
mod tests;

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window, px, size,
};
use merge_diff::{Side, ThreeSide};
use ui::prelude::*;
use util::ResultExt as _;

use self::chrome::dialog_background;
pub(crate) use self::chrome::{
    ThemedImage, dialog_button, dialog_button_with_content, themed_image,
};
pub(crate) use self::finish::MergeResult;
use self::{
    confirmation::render_confirmation,
    finish::{
        ACCEPT_LEFT_LABEL, ACCEPT_RIGHT_LABEL, APPLY_CHANGES_LABEL, DiscardAction, PendingFinish,
        apply_button_enabled, apply_needs_confirmation, cancel_button_label,
        cancel_needs_confirmation, confirmation_text, needs_discard_confirmation,
        restores_original_on_finish, window_title, writes_side_text_on_finish,
    },
};
use super::{
    conflict_resolution::rich_text::RichText,
    dialog_window::{
        CloseDecision, CloseRequest, DialogOwner, DialogWindow, DialogWindowContent,
        DialogWindowSpec, close_dialog_window, open_dialog_window,
    },
    merge_model::MergeConflictModel,
    merge_viewer::{
        MergeViewer, MergeViewerEvent, RediffState,
        actions::{AcceptLeft, AcceptRight, ApplyChanges, SaveAndClose},
    },
    palette::editor_background,
};

const MERGE_DIALOG_BOUNDS_KEY: &str = "MergeDialog";
const INITIAL_WIDTH: f32 = 1000.0;
const INITIAL_HEIGHT: f32 = 700.0;
const MINIMUM_WIDTH: f32 = 700.0;
const MINIMUM_HEIGHT: f32 = 450.0;
const BUTTON_ROW_HORIZONTAL_INSET: f32 = 12.0;
const BUTTON_ROW_VERTICAL_INSET: f32 = 8.0;
const BUTTON_GAP: f32 = 12.0;

#[derive(Clone)]
pub(crate) struct MergePaneTitle {
    pub label: RichText,
    pub detail: Option<SharedString>,
    pub detail_tooltip: Option<SharedString>,
    pub show_details: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MergePaneTitle {
    pub(crate) fn new(label: RichText) -> Self {
        Self {
            label,
            detail: None,
            detail_tooltip: None,
            show_details: None,
        }
    }

    pub(crate) fn with_detail(
        mut self,
        detail: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        self.detail = Some(detail.into());
        self.detail_tooltip = Some(tooltip.into());
        self
    }

    pub(crate) fn with_show_details(
        mut self,
        show_details: Rc<dyn Fn(&mut Window, &mut App)>,
    ) -> Self {
        self.show_details = Some(show_details);
        self
    }
}

type FinishCallback = Rc<RefCell<Option<Box<dyn FnOnce(MergeResult, &mut App)>>>>;

pub(crate) struct MergeRequest {
    pub window_title: SharedString,
    pub pane_titles: [MergePaneTitle; 3],
    pub model: Entity<MergeConflictModel>,
    pub file_name: SharedString,
    pub iterative: bool,
    pub original_content: String,
    pub on_finished: Box<dyn FnOnce(MergeResult, &mut App)>,
}

pub(crate) fn open_merge_window(request: MergeRequest, owner: DialogOwner, cx: &mut App) {
    open_merge_window_with(request, owner, cx, None);
}

type OpenedCallback = Box<dyn FnOnce(DialogWindow<MergeWindow>, &mut App)>;

pub(crate) fn open_merge_window_with(
    request: MergeRequest,
    owner: DialogOwner,
    cx: &mut App,
    on_opened: Option<OpenedCallback>,
) {
    let MergeRequest {
        window_title: requested_title,
        pane_titles,
        model,
        file_name,
        iterative,
        original_content,
        on_finished,
    } = request;
    let finish_callback: FinishCallback = Rc::new(RefCell::new(Some(on_finished)));
    let spec = DialogWindowSpec {
        title: window_title(&requested_title).into(),
        initial_size: size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)),
        min_size: size(px(MINIMUM_WIDTH), px(MINIMUM_HEIGHT)),
        bounds_key: MERGE_DIALOG_BOUNDS_KEY,
    };
    let content_callback = finish_callback.clone();
    open_dialog_window(
        spec,
        owner,
        cx,
        move |window: &mut Window, cx: &mut App| {
            cx.new(|cx| {
                MergeWindow::new(
                    MergeWindowInit {
                        model,
                        pane_titles,
                        file_name,
                        iterative,
                        original_content,
                        on_finished: content_callback,
                    },
                    window,
                    cx,
                )
            })
        },
        move |opened: Result<DialogWindow<MergeWindow>>, cx: &mut App| match opened {
            Ok(opened) => {
                if let Some(on_opened) = on_opened {
                    on_opened(opened, cx);
                }
            }
            Err(error) => {
                log::error!("could not open the merge window: {error:#}");
                let callback = finish_callback.borrow_mut().take();
                if let Some(callback) = callback {
                    callback(MergeResult::Cancel, cx);
                }
            }
        },
    );
}

struct MergeWindowInit {
    model: Entity<MergeConflictModel>,
    pane_titles: [MergePaneTitle; 3],
    file_name: SharedString,
    iterative: bool,
    original_content: String,
    on_finished: FinishCallback,
}

struct ActiveConfirmation {
    text: finish::ConfirmationText,
    pending: PendingFinish,
    focus_handle: FocusHandle,
}

pub(crate) struct MergeWindow {
    viewer: Entity<MergeViewer>,
    model: Entity<MergeConflictModel>,
    file_name: SharedString,
    iterative: bool,
    original_content: String,
    on_finished: FinishCallback,
    finished: bool,
    confirmation: Option<ActiveConfirmation>,
    _subscriptions: Vec<Subscription>,
}

impl MergeWindow {
    fn new(init: MergeWindowInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let MergeWindowInit {
            model,
            pane_titles,
            file_name,
            iterative,
            original_content,
            on_finished,
        } = init;
        let viewer = cx.new(|cx| MergeViewer::new(model.clone(), pane_titles, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &viewer,
                window,
                |merge_window, _viewer, event: &MergeViewerEvent, window, cx| {
                    merge_window.on_viewer_event(*event, window, cx);
                },
            ),
            cx.on_release(|merge_window, cx| {
                merge_window.finish_result(MergeResult::Cancel, cx);
            }),
        ];
        Self {
            viewer,
            model,
            file_name,
            iterative,
            original_content,
            on_finished,
            finished: false,
            confirmation: None,
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn viewer(&self) -> &Entity<MergeViewer> {
        &self.viewer
    }

    #[cfg(test)]
    pub(crate) fn shown_confirmation(&self) -> Option<finish::ConfirmationText> {
        self.confirmation
            .as_ref()
            .map(|confirmation| confirmation.text.clone())
    }

    fn on_viewer_event(
        &mut self,
        event: MergeViewerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MergeViewerEvent::FinishRequested(result) => self.finish(result, window, cx),
            MergeViewerEvent::RestartConfirmationRequested(policy) => {
                self.show_confirmation(PendingFinish::RestartMerge(policy), window, cx);
            }
            MergeViewerEvent::RevertConfirmationRequested => {
                self.show_confirmation(PendingFinish::RevertResolution, window, cx);
            }
            MergeViewerEvent::RediffCanceled => self.finish(MergeResult::Cancel, window, cx),
        }
    }

    fn content_modified(&self, cx: &App) -> bool {
        self.model.read(cx).content_modified()
    }

    fn apply_enabled(&self, cx: &App) -> bool {
        let state = self.viewer.read(cx).rediff_state();
        apply_button_enabled(
            matches!(state, RediffState::Running | RediffState::Idle),
            state == RediffState::Failed,
        )
    }

    fn finish_result(&mut self, result: MergeResult, cx: &mut App) {
        if self.finished {
            return;
        }
        self.finished = true;
        let result_text = if let Some(side) = writes_side_text_on_finish(result) {
            let source = match side {
                Side::Left => ThreeSide::Left,
                Side::Right => ThreeSide::Right,
            };
            Some(self.model.read(cx).side_text(source).to_string())
        } else if restores_original_on_finish(result, self.iterative) {
            Some(self.original_content.clone())
        } else {
            None
        };
        if let Some(text) = result_text {
            let result_buffer = self.model.read(cx).buffers().result.clone();
            result_buffer.update(cx, |buffer, cx| {
                buffer.set_text(text, cx);
            });
        }
        let callback = self.on_finished.borrow_mut().take();
        match callback {
            Some(callback) => callback(result, cx),
            None => log::debug!("merge window for {} finished twice", self.file_name),
        }
    }

    fn finish(&mut self, result: MergeResult, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_result(result, cx);
        close_dialog_window(window);
    }

    fn show_confirmation(
        &mut self,
        pending: PendingFinish,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        self.confirmation = Some(ActiveConfirmation {
            text: confirmation_text(pending, self.iterative),
            pending,
            focus_handle,
        });
        cx.notify();
    }

    fn dismiss_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmation.take().is_none() {
            return;
        }
        let editor_focus = self.viewer.focus_handle(cx);
        window.focus(&editor_focus, cx);
        cx.notify();
    }

    fn accept_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirmation) = self.confirmation.take() else {
            return;
        };
        let editor_focus = self.viewer.focus_handle(cx);
        window.focus(&editor_focus, cx);
        cx.notify();
        match confirmation.pending {
            PendingFinish::Discard(DiscardAction::AcceptSide(side)) => {
                self.complete_accept_side(side, window, cx);
            }
            PendingFinish::Discard(DiscardAction::CancelMerge) => {
                self.finish(MergeResult::Cancel, window, cx);
            }
            PendingFinish::UnprocessedChanges { .. } => self.complete_apply_changes(window, cx),
            PendingFinish::RevertResolution => {
                self.viewer
                    .update(cx, |viewer, cx| viewer.revert_conflict_resolution(cx));
            }
            PendingFinish::RestartMerge(policy) => {
                self.viewer
                    .update(cx, |viewer, cx| viewer.restart_merge(policy, window, cx));
            }
        }
    }

    fn accept_side(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        if self.finished || self.confirmation.is_some() {
            return;
        }
        if needs_discard_confirmation(self.content_modified(cx)) {
            self.show_confirmation(
                PendingFinish::Discard(DiscardAction::AcceptSide(side)),
                window,
                cx,
            );
            return;
        }
        self.complete_accept_side(side, window, cx);
    }

    fn complete_accept_side(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        self.model
            .update(cx, |model, cx| model.run_accept_side(side, cx));
        let result = match side {
            Side::Left => MergeResult::Left,
            Side::Right => MergeResult::Right,
        };
        self.finish(result, window, cx);
    }

    fn cancel_or_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.finished || self.confirmation.is_some() {
            return;
        }
        if cancel_needs_confirmation(self.iterative, self.content_modified(cx)) {
            self.show_confirmation(
                PendingFinish::Discard(DiscardAction::CancelMerge),
                window,
                cx,
            );
            return;
        }
        self.finish(MergeResult::Cancel, window, cx);
    }

    fn apply_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.finished || self.confirmation.is_some() || !self.apply_enabled(cx) {
            return;
        }
        let counters = self.model.read(cx).counters();
        match counters {
            Some(counters) if apply_needs_confirmation(Some(counters)) => {
                self.show_confirmation(PendingFinish::UnprocessedChanges { counters }, window, cx);
            }
            _ => self.finish(MergeResult::Resolved, window, cx),
        }
    }

    fn complete_apply_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model
            .update(cx, |model, cx| model.run_apply_changes(cx));
        self.finish(MergeResult::Resolved, window, cx);
    }

    fn request_close(
        &mut self,
        request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_requested(request, window, cx) == CloseDecision::Close {
            close_dialog_window(window);
        }
    }

    fn render_button_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let apply_enabled = self.apply_enabled(cx);
        h_flex()
            .w_full()
            .flex_none()
            .justify_between()
            .px(px(BUTTON_ROW_HORIZONTAL_INSET))
            .py(px(BUTTON_ROW_VERTICAL_INSET))
            .bg(dialog_background(cx))
            .child(
                h_flex()
                    .gap(px(BUTTON_GAP))
                    .child(
                        dialog_button("merge-accept-left", ACCEPT_LEFT_LABEL, false, false, cx)
                            .on_click(cx.listener(|merge_window, _, window, cx| {
                                merge_window.accept_side(Side::Left, window, cx);
                            })),
                    )
                    .child(
                        dialog_button("merge-accept-right", ACCEPT_RIGHT_LABEL, false, false, cx)
                            .on_click(cx.listener(|merge_window, _, window, cx| {
                                merge_window.accept_side(Side::Right, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap(px(BUTTON_GAP))
                    .child(
                        dialog_button(
                            "merge-cancel",
                            cancel_button_label(self.iterative),
                            false,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(
                            |merge_window, _, window, cx| {
                                merge_window.cancel_or_save(window, cx);
                            },
                        )),
                    )
                    .child(
                        dialog_button(
                            "merge-apply-changes",
                            APPLY_CHANGES_LABEL,
                            true,
                            !apply_enabled,
                            cx,
                        )
                        .on_click(cx.listener(
                            |merge_window, _, window, cx| {
                                merge_window.apply_changes(window, cx);
                            },
                        )),
                    ),
            )
    }
}

impl Focusable for MergeWindow {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.viewer.focus_handle(cx)
    }
}

impl DialogWindowContent for MergeWindow {
    fn close_requested(
        &mut self,
        _request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseDecision {
        if self.finished {
            return CloseDecision::Close;
        }
        if self.confirmation.is_some() {
            return CloseDecision::Keep;
        }
        if cancel_needs_confirmation(self.iterative, self.content_modified(cx)) {
            self.show_confirmation(
                PendingFinish::Discard(DiscardAction::CancelMerge),
                window,
                cx,
            );
            return CloseDecision::Keep;
        }
        self.finish_result(MergeResult::Cancel, cx);
        CloseDecision::Close
    }
}

impl Render for MergeWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button_row = self.render_button_row(cx).into_any_element();
        let confirmation = self.confirmation.as_ref().map(|confirmation| {
            let accept_listener = cx.listener(|merge_window, _: &menu::Confirm, window, cx| {
                merge_window.accept_confirmation(window, cx);
            });
            let dismiss_listener = cx.listener(|merge_window, _: &menu::Cancel, window, cx| {
                merge_window.dismiss_confirmation(window, cx);
            });
            let weak_window = cx.weak_entity();
            let dismiss_window = weak_window.clone();
            render_confirmation(
                &confirmation.text,
                &confirmation.focus_handle,
                move |window, cx| {
                    weak_window
                        .update(cx, |merge_window, cx| {
                            merge_window.accept_confirmation(window, cx);
                        })
                        .log_err();
                },
                move |window, cx| {
                    dismiss_window
                        .update(cx, |merge_window, cx| {
                            merge_window.dismiss_confirmation(window, cx);
                        })
                        .log_err();
                },
                cx,
            )
            .on_action(accept_listener)
            .on_action(dismiss_listener)
        });
        v_flex()
            .size_full()
            .relative()
            .key_context("MergeView")
            .bg(editor_background(cx))
            .on_action(cx.listener(|merge_window, _: &AcceptLeft, window, cx| {
                merge_window.accept_side(Side::Left, window, cx);
            }))
            .on_action(cx.listener(|merge_window, _: &AcceptRight, window, cx| {
                merge_window.accept_side(Side::Right, window, cx);
            }))
            .on_action(cx.listener(|merge_window, _: &SaveAndClose, window, cx| {
                merge_window.cancel_or_save(window, cx);
            }))
            .on_action(cx.listener(|merge_window, _: &ApplyChanges, window, cx| {
                merge_window.apply_changes(window, cx);
            }))
            .on_action(cx.listener(|merge_window, _: &menu::Confirm, window, cx| {
                merge_window.apply_changes(window, cx);
            }))
            .on_action(
                cx.listener(|merge_window, _: &editor::actions::Cancel, window, cx| {
                    merge_window.request_close(CloseRequest::Escape, window, cx);
                }),
            )
            .child(self.viewer.clone())
            .child(button_row)
            .when_some(confirmation, |root, confirmation| root.child(confirmation))
    }
}
