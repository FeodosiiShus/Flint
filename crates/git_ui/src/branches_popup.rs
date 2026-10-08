mod delegate;
mod ref_menu;
mod rows;
mod state;
#[cfg(test)]
mod tests;
mod top_actions;
pub mod tree;

use git_ui_core::GitPickerPopover;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, MouseButton, Render,
    Subscription, WeakEntity, Window, actions,
};
use picker::{Picker, PickerDelegate};
use project::git_store::Repository;
use ui::prelude::*;
use workspace::{ModalView, Workspace};

use self::delegate::BranchesDelegate;

const POPUP_WIDTH_REMS: f32 = 26.;
const MAX_LIST_HEIGHT_PX: f32 = 480.;
const KEY_CONTEXT: &str = "BranchesPopup";

actions!(branches_popup, [ToggleFavorite, ClearSearchOrClose]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchesPopupStyle {
    Popover,
    Modal,
}

pub struct BranchesPopup {
    picker: Entity<Picker<BranchesDelegate>>,
    style: BranchesPopupStyle,
    _picker_subscription: Subscription,
}

impl BranchesPopup {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        repository: Option<Entity<Repository>>,
        style: BranchesPopupStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = BranchesDelegate::new(workspace, repository, cx.focus_handle(), cx);
        let picker = cx.new(|cx| {
            Picker::list(delegate, window, cx)
                .initial_width(rems(POPUP_WIDTH_REMS))
                .max_height(rems_from_px(MAX_LIST_HEIGHT_PX))
                .embedded()
                .reopenable(false, cx)
        });
        let picker_focus_handle = picker.focus_handle(cx);
        picker.update(cx, |picker, cx| {
            picker.delegate.focus_handle = picker_focus_handle;
            picker.delegate.attach_repository(window, cx);
            picker.refresh(window, cx);
        });
        let picker_subscription =
            cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent));

        Self {
            picker,
            style,
            _picker_subscription: picker_subscription,
        }
    }

    fn has_open_menu(&self, window: &Window, cx: &App) -> bool {
        self.picker
            .read(cx)
            .delegate
            .has_another_open_menu(window, cx)
    }

    fn toggle_favorite(&mut self, _: &ToggleFavorite, window: &mut Window, cx: &mut Context<Self>) {
        self.picker.update(cx, |picker, cx| {
            if picker.delegate.toggle_favorite_of_selected(cx) {
                picker.refresh(window, cx);
            }
        });
    }

    fn clear_search_or_close(
        &mut self,
        _: &ClearSearchOrClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.update(cx, |picker, cx| {
            if picker.query(cx).is_empty() {
                picker.cancel(&menu::Cancel, window, cx);
            } else {
                picker.set_query("", window, cx);
            }
        });
    }

    fn select_child(&mut self, _: &menu::SelectChild, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picker.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
        self.picker.update(cx, |picker, cx| {
            picker.delegate.select_child(window, cx);
        });
        cx.stop_propagation();
    }

    fn select_parent(
        &mut self,
        _: &menu::SelectParent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closed = self.picker.update(cx, |picker, cx| {
            picker.delegate.close_open_ref_menu(window, cx)
        });
        if closed {
            cx.stop_propagation();
            return;
        }
        if !self.picker.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
        self.picker.update(cx, |picker, cx| {
            picker.delegate.select_parent(window, cx);
        });
        cx.stop_propagation();
    }
}

impl EventEmitter<DismissEvent> for BranchesPopup {}

impl ModalView for BranchesPopup {}

impl Focusable for BranchesPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl Render for BranchesPopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let opaque_background = cx.theme().colors().elevated_surface_background.alpha(1.0);
        v_flex()
            .occlude()
            .w(rems(POPUP_WIDTH_REMS))
            .elevation_3(cx)
            .bg(opaque_background)
            .overflow_hidden()
            .key_context(KEY_CONTEXT)
            .when(self.style == BranchesPopupStyle::Popover, |this| {
                this.on_mouse_down_out(cx.listener(|this, _, window, cx| {
                    if this.has_open_menu(window, cx) {
                        return;
                    }
                    cx.emit(DismissEvent);
                }))
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(Self::toggle_favorite))
            .on_action(cx.listener(Self::clear_search_or_close))
            .capture_action(cx.listener(Self::select_child))
            .capture_action(cx.listener(Self::select_parent))
            .child(self.picker.clone())
    }
}

pub fn popover(
    workspace: WeakEntity<Workspace>,
    repository: Option<Entity<Repository>>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<GitPickerPopover> {
    let popup = cx.new(|cx| {
        let popup = BranchesPopup::new(
            workspace,
            repository,
            BranchesPopupStyle::Popover,
            window,
            cx,
        );
        popup.focus_handle(cx).focus(window, cx);
        popup
    });
    cx.new(|cx| GitPickerPopover::new(popup, cx))
}

pub fn open_modal(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let workspace_handle = workspace.weak_handle();
    let repository = workspace.project().read(cx).active_repository(cx);
    workspace.toggle_modal(window, cx, move |window, cx| {
        BranchesPopup::new(
            workspace_handle,
            repository,
            BranchesPopupStyle::Modal,
            window,
            cx,
        )
    });
}
