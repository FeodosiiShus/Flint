mod delegate;
pub(crate) mod ref_menu;
mod rows;
pub(crate) mod state;
#[cfg(test)]
mod tests;
mod top_actions;
pub mod tree;

use git_ui_core::GitPickerPopover;
use gpui::{
    App, Context, CursorStyle, DismissEvent, DragMoveEvent, Entity, EventEmitter, FocusHandle,
    Focusable, MouseButton, MouseDownEvent, Pixels, Point, Render, Size, Subscription, WeakEntity,
    Window, actions, size,
};
use picker::{Picker, PickerDelegate};
use project::git_store::Repository;
use ui::prelude::*;
use workspace::{ModalView, Workspace};

use self::delegate::{
    BranchesDelegate, HEADER_TOP_INSET_PX, LeafDirection, ROW_HEIGHT_PX, RunningFetches,
    SEARCH_BOX_HEIGHT_PX,
};
use self::rows::is_filtering;
use self::state::PopupSize;
use self::top_actions::TopActionKind;

const KEY_CONTEXT: &str = "BranchesPopup";
pub(crate) const DEBUG_SELECTOR: &str = "branches-popup";
// `GitBranchesPopupBase`: the New UI popup is at least 375 px wide and 300 px plus the 12 px drag
// area tall, and opens with at most 20 visible tree rows.
pub(crate) const MIN_WIDTH_PX: f32 = 375.;
pub(crate) const MIN_HEIGHT_PX: f32 = 312.;
pub(crate) const MAX_VISIBLE_ROWS: usize = 20;
const BORDER_PX: f32 = 1.;
const RESIZE_HANDLE_PX: f32 = 4.;

actions!(
    branches_popup,
    [
        ToggleFavorite,
        ClearSearchOrClose,
        /// Deletes the branch or tag whose menu is open in the branches popup.
        DeleteRef
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeEdge {
    Right,
    Bottom,
    BottomRight,
}

impl ResizeEdge {
    fn cursor(self) -> CursorStyle {
        match self {
            ResizeEdge::Right => CursorStyle::ResizeLeftRight,
            ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
            ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        }
    }

    fn resizes_width(self) -> bool {
        matches!(self, ResizeEdge::Right | ResizeEdge::BottomRight)
    }

    fn resizes_height(self) -> bool {
        matches!(self, ResizeEdge::Bottom | ResizeEdge::BottomRight)
    }
}

struct ResizeDrag(ResizeEdge);

struct MoveDrag;

struct NoDragPreview;

impl Render for NoDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The popup size IntelliJ's `pack` arrives at: as wide as the widest row and tall enough for
/// `visibleRowCount` rows, but never below the minimum size.
pub(crate) fn packed_size(
    widest_row: Pixels,
    visible_rows: usize,
    list_padding: Pixels,
) -> Size<Pixels> {
    let rows = visible_rows.min(MAX_VISIBLE_ROWS) as f32;
    let chrome =
        px(HEADER_TOP_INSET_PX + SEARCH_BOX_HEIGHT_PX + 2. * BORDER_PX) + list_padding * 2.;
    size(
        (widest_row + px(2. * BORDER_PX)).max(px(MIN_WIDTH_PX)),
        (chrome + px(ROW_HEIGHT_PX) * rows).max(px(MIN_HEIGHT_PX)),
    )
}

fn clamp_to_minimum(popup_size: Size<Pixels>) -> Size<Pixels> {
    size(
        popup_size.width.max(px(MIN_WIDTH_PX)),
        popup_size.height.max(px(MIN_HEIGHT_PX)),
    )
}

fn to_pixels(popup_size: PopupSize) -> Size<Pixels> {
    size(px(popup_size.width as f32), px(popup_size.height as f32))
}

fn to_popup_size(popup_size: Size<Pixels>) -> PopupSize {
    PopupSize {
        width: f32::from(popup_size.width).round().max(0.) as u32,
        height: f32::from(popup_size.height).round().max(0.) as u32,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchesPopupStyle {
    Popover,
    Modal,
}

pub struct BranchesPopup {
    picker: Entity<Picker<BranchesDelegate>>,
    style: BranchesPopupStyle,
    resizing: Option<Size<Pixels>>,
    resize_start: Option<(Point<Pixels>, Size<Pixels>)>,
    move_offset: Point<Pixels>,
    move_start: Option<(Point<Pixels>, Point<Pixels>)>,
    _picker_subscription: Subscription,
    _picker_observation: Subscription,
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
        // The popup sizes the picker itself, so the list scrolls inside a height fixed at open.
        let picker = cx.new(|cx| {
            Picker::list(delegate, window, cx)
                .fill_container()
                .reopenable(false, cx)
        });
        let picker_focus_handle = picker.focus_handle(cx);
        picker.update(cx, |picker, cx| {
            picker.delegate.focus_handle = picker_focus_handle;
            picker.delegate._fetch_observation =
                Some(cx.observe_global::<RunningFetches>(|_, cx| cx.notify()));
            picker.delegate.attach_repository(window, cx);
            picker.refresh(window, cx);
        });
        let picker_subscription =
            cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent));
        // The frame follows the delegate's packed and user sizes.
        let picker_observation = cx.observe(&picker, |_, _, cx| cx.notify());

        Self {
            picker,
            style,
            resizing: None,
            resize_start: None,
            move_offset: Point::default(),
            move_start: None,
            _picker_subscription: picker_subscription,
            _picker_observation: picker_observation,
        }
    }

    /// The size the popup renders at: while resizing the live size, then the user's size, then
    /// the packed size.
    pub fn popup_size(&self, cx: &App) -> Size<Pixels> {
        if let Some(resizing) = self.resizing {
            return resizing;
        }
        let delegate = &self.picker.read(cx).delegate;
        delegate
            .user_size()
            .map(|user_size| clamp_to_minimum(to_pixels(user_size)))
            .or(delegate.packed_size)
            .unwrap_or_else(|| size(px(MIN_WIDTH_PX), px(MIN_HEIGHT_PX)))
    }

    pub fn move_offset(&self) -> Point<Pixels> {
        self.move_offset
    }

    /// Remembers the size a resize ended at, when the mouse button is released.
    fn finish_resize(&mut self, cx: &mut Context<Self>) {
        let Some(resized) = self.resizing.take() else {
            return;
        };
        self.picker.update(cx, |picker, cx| {
            picker.delegate.set_user_size(to_popup_size(resized), cx);
            cx.notify();
        });
    }

    fn start_resize(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.resize_start = Some((position, self.popup_size(cx)));
    }

    fn resize(&mut self, edge: ResizeEdge, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((start_position, start_size)) = self.resize_start else {
            return;
        };
        let delta = position - start_position;
        let mut resized = start_size;
        if edge.resizes_width() {
            resized.width = start_size.width + delta.x;
        }
        if edge.resizes_height() {
            resized.height = start_size.height + delta.y;
        }
        self.resizing = Some(clamp_to_minimum(resized));
        cx.notify();
    }

    fn start_move(&mut self, position: Point<Pixels>) {
        self.move_start = Some((position, self.move_offset));
    }

    fn move_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((start_position, start_offset)) = self.move_start else {
            return;
        };
        self.move_offset = start_offset + (position - start_position);
        cx.notify();
    }

    /// IntelliJ closes the popup when a top-level action's shortcut is pressed in it and runs the
    /// action for the popup's repository (`closePopupOnTopLevelActionsShortcuts`).
    fn run_top_action_shortcut(
        &mut self,
        kind: TopActionKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.update(cx, |picker, cx| {
            picker.delegate.run_top_action(kind, window, cx);
        });
    }

    fn delete_ref(&mut self, _: &DeleteRef, window: &mut Window, cx: &mut Context<Self>) {
        self.picker.update(cx, |picker, cx| {
            picker.delegate.delete_selected_ref(window, cx);
        });
    }

    fn render_resize_handle(&self, edge: ResizeEdge, cx: &mut Context<Self>) -> impl IntoElement {
        let id = match edge {
            ResizeEdge::Right => "branches-popup-resize-right",
            ResizeEdge::Bottom => "branches-popup-resize-bottom",
            ResizeEdge::BottomRight => "branches-popup-resize-corner",
        };
        let handle = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .absolute()
            .cursor(edge.cursor())
            .map(|handle| match edge {
                ResizeEdge::Right => handle.top_0().bottom_0().right_0().w(px(RESIZE_HANDLE_PX)),
                ResizeEdge::Bottom => handle.left_0().right_0().bottom_0().h(px(RESIZE_HANDLE_PX)),
                ResizeEdge::BottomRight => {
                    handle.right_0().bottom_0().size(px(RESIZE_HANDLE_PX * 2.))
                }
            });
        handle
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.start_resize(event.position, cx);
                }),
            )
            .on_drag(ResizeDrag(edge), |_, _, _, cx| cx.new(|_| NoDragPreview))
            .on_drag_move::<ResizeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<ResizeDrag>, _, cx| {
                    let edge = event.drag(cx).0;
                    this.resize(edge, event.event.position, cx);
                },
            ))
    }

    /// IntelliJ's popup moves with its drag area above the search field.
    fn render_drag_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("branches-popup-drag-area")
            .debug_selector(|| "branches-popup-drag-area".to_string())
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(rems_from_px(HEADER_TOP_INSET_PX))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.start_move(event.position);
                }),
            )
            .on_drag(MoveDrag, |_, _, _, cx| cx.new(|_| NoDragPreview))
            .on_drag_move::<MoveDrag>(cx.listener(
                |this, event: &DragMoveEvent<MoveDrag>, _, cx| {
                    this.move_to(event.event.position, cx);
                },
            ))
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

    /// While searching, IntelliJ's arrow keys move over the matching leaves only, opening
    /// sections and folders on the way and wrapping at the ends.
    fn select_leaf(
        &mut self,
        direction: LeafDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let filtering = is_filtering(&self.picker.read(cx).delegate.query);
        if !filtering || !self.picker.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
        self.picker.update(cx, |picker, cx| {
            if picker.delegate.select_leaf(direction) {
                picker.refresh(window, cx);
            }
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
        let popup_size = self.popup_size(cx);
        v_flex()
            .debug_selector(|| DEBUG_SELECTOR.to_string())
            .occlude()
            .relative()
            .left(self.move_offset.x)
            .top(self.move_offset.y)
            .w(popup_size.width)
            .h(popup_size.height)
            .elevation_3(cx)
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
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_resize(cx)),
            )
            .on_action(cx.listener(Self::toggle_favorite))
            .on_action(cx.listener(Self::clear_search_or_close))
            .on_action(cx.listener(Self::delete_ref))
            .on_action(cx.listener(|this, _: &git::UpdateProject, window, cx| {
                this.run_top_action_shortcut(TopActionKind::UpdateProject, window, cx)
            }))
            .on_action(cx.listener(|this, _: &git::Commit, window, cx| {
                this.run_top_action_shortcut(TopActionKind::Commit, window, cx)
            }))
            .on_action(cx.listener(|this, _: &git::PushDialog, window, cx| {
                this.run_top_action_shortcut(TopActionKind::Push, window, cx)
            }))
            .on_action(cx.listener(|this, _: &git::NewBranch, window, cx| {
                this.run_top_action_shortcut(TopActionKind::NewBranch, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &git::CheckoutTagOrRevision, window, cx| {
                    this.run_top_action_shortcut(TopActionKind::CheckoutTagOrRevision, window, cx)
                }),
            )
            .capture_action(cx.listener(Self::select_child))
            .capture_action(cx.listener(Self::select_parent))
            .capture_action(cx.listener(|this, _: &menu::SelectNext, window, cx| {
                this.select_leaf(LeafDirection::Next, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &menu::SelectPrevious, window, cx| {
                this.select_leaf(LeafDirection::Previous, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &menu::SelectFirst, window, cx| {
                this.select_leaf(LeafDirection::First, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &menu::SelectLast, window, cx| {
                this.select_leaf(LeafDirection::Last, window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &zed_actions::editor::MoveDown, window, cx| {
                    this.select_leaf(LeafDirection::Next, window, cx)
                }),
            )
            .capture_action(
                cx.listener(|this, _: &zed_actions::editor::MoveUp, window, cx| {
                    this.select_leaf(LeafDirection::Previous, window, cx)
                }),
            )
            .child(div().flex_1().min_h_0().child(self.picker.clone()))
            .child(self.render_drag_area(cx))
            .child(self.render_resize_handle(ResizeEdge::Right, cx))
            .child(self.render_resize_handle(ResizeEdge::Bottom, cx))
            .child(self.render_resize_handle(ResizeEdge::BottomRight, cx))
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
