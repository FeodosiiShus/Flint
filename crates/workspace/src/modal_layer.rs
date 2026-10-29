use std::time::Instant;

use gpui::{
    AnyView, App, DismissEvent, Entity, EventEmitter, FocusHandle, Global, ManagedView,
    MouseButton, Point, Subscription, Task, WeakFocusHandle, anchored,
};
use ui::{SurfaceMotion, prelude::*, surfaces_move};

#[derive(Debug)]
pub enum DismissDecision {
    Dismiss(bool),
    Pending,
}

// A modal that hosts a picker forwards its own focus handle to the inner picker,
// so the modal layer recognizes a reopenable picker by focus identity rather than
// requiring each modal wrapper to implement its own opt-in.
#[derive(Default)]
struct ReopenablePickerRegistry {
    handles: Vec<WeakFocusHandle>,
}

impl Global for ReopenablePickerRegistry {}

pub fn register_reopenable_picker(focus_handle: &FocusHandle, cx: &mut App) {
    let registry = cx.default_global::<ReopenablePickerRegistry>();
    registry.handles.retain(|handle| handle.upgrade().is_some());
    if !registry.handles.iter().any(|handle| handle == focus_handle) {
        registry.handles.push(focus_handle.downgrade());
    }
}

pub fn deregister_reopenable_picker(focus_handle: &FocusHandle, cx: &mut App) {
    let registry = cx.default_global::<ReopenablePickerRegistry>();
    registry
        .handles
        .retain(|handle| handle.upgrade().is_some() && handle != focus_handle);
}

fn focus_handle_is_reopenable(focus_handle: &FocusHandle, cx: &App) -> bool {
    cx.try_global::<ReopenablePickerRegistry>()
        .is_some_and(|registry| {
            registry
                .handles
                .iter()
                .filter_map(|handle| handle.upgrade())
                .any(|handle| &handle == focus_handle)
        })
}

pub trait ModalView: ManagedView {
    fn on_before_dismiss(
        &mut self,
        _window: &mut Window,
        _: &mut Context<Self>,
    ) -> DismissDecision {
        DismissDecision::Dismiss(true)
    }

    fn fade_out_background(&self) -> bool {
        false
    }

    fn render_bare(&self) -> bool {
        false
    }

    fn anchor_position(&self) -> Option<Point<Pixels>> {
        None
    }

    /// Whether the content the modal loads after it opens is in place (see [`ui::ReadyGate`]):
    /// the modal appears from the frame it is, so it never comes in half-loaded.
    fn surface_ready(&self) -> bool {
        true
    }
}

trait ModalViewHandle {
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> DismissDecision;
    fn view(&self) -> AnyView;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn subscribe_dismiss(&self, window: &mut Window, cx: &mut Context<ModalLayer>) -> Subscription;
    fn fade_out_background(&self, cx: &mut App) -> bool;
    fn render_bare(&self, cx: &mut App) -> bool;
    fn anchor_position(&self, cx: &App) -> Option<Point<Pixels>>;
    fn surface_ready(&self, cx: &App) -> bool;
}

impl<V: ModalView> ModalViewHandle for Entity<V> {
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> DismissDecision {
        self.update(cx, |this, cx| this.on_before_dismiss(window, cx))
    }

    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn subscribe_dismiss(&self, window: &mut Window, cx: &mut Context<ModalLayer>) -> Subscription {
        cx.subscribe_in(self, window, |this, _, _: &DismissEvent, window, cx| {
            this.hide_modal(window, cx);
        })
    }

    fn fade_out_background(&self, cx: &mut App) -> bool {
        self.read(cx).fade_out_background()
    }

    fn render_bare(&self, cx: &mut App) -> bool {
        self.read(cx).render_bare()
    }

    fn anchor_position(&self, cx: &App) -> Option<Point<Pixels>> {
        self.read(cx).anchor_position()
    }

    fn surface_ready(&self, cx: &App) -> bool {
        self.read(cx).surface_ready()
    }
}

pub struct ActiveModal {
    modal: Box<dyn ModalViewHandle>,
    _subscriptions: [Subscription; 2],
    previous_focus_handle: Option<FocusHandle>,
    focus_handle: FocusHandle,
    /// Tells this showing of the modal from earlier ones, so it comes in with its own motion
    /// even when the same view is shown again.
    presentation: usize,
}

/// How a dismissed modal was placed, kept while it fades out.
struct ClosingModal {
    view: AnyView,
    presentation: usize,
    render_bare: bool,
    fade_out_background: bool,
    anchor_position: Option<Point<Pixels>>,
    since: Instant,
    _removal: Task<()>,
}

pub struct ModalLayer {
    active_modal: Option<ActiveModal>,
    closing_modal: Option<ClosingModal>,
    presentations: usize,
    // Kept alive (hidden) rather than dropped on dismissal so `ReopenLastPicker`
    // can reveal it again with its exact prior state. Left intact across
    // non-reopenable modals (e.g. the command palette), which may trigger the reopen.
    stashed_modal: Option<Box<dyn ModalViewHandle>>,
    // Set when a reveal was requested while another modal was still active (e.g. the
    // which-key popup that is dismissed only after the reopen action fires). The reveal
    // then happens once that modal closes, so it doesn't matter whether the triggering
    // modal is dismissed before or after the action dispatches.
    reveal_stash_when_free: bool,
    dismiss_on_focus_lost: bool,
}

pub(crate) struct ModalOpenedEvent;

impl EventEmitter<ModalOpenedEvent> for ModalLayer {}

impl Default for ModalLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalLayer {
    pub fn new() -> Self {
        Self {
            active_modal: None,
            closing_modal: None,
            presentations: 0,
            stashed_modal: None,
            reveal_stash_when_free: false,
            dismiss_on_focus_lost: false,
        }
    }

    /// Toggles a modal of type `V`. If a modal of the same type is currently active,
    /// it will be hidden. If a different modal is active, it will be replaced with the new one.
    /// If no modal is active, the new modal will be shown.
    ///
    /// If closing the current modal fails (e.g., due to `on_before_dismiss` returning
    /// `DismissDecision::Dismiss(false)` or `DismissDecision::Pending`), the new modal
    /// will not be shown.
    pub fn toggle_modal<V, B>(&mut self, window: &mut Window, cx: &mut Context<Self>, build_view: B)
    where
        V: ModalView,
        B: FnOnce(&mut Window, &mut Context<V>) -> V,
    {
        // Opening a modal explicitly supersedes any reveal that was waiting for the
        // layer to become free.
        self.reveal_stash_when_free = false;
        let mut previous_focus_handle = window.focused(cx);
        if let Some(active_modal) = &self.active_modal {
            let should_close = active_modal.modal.view().downcast::<V>().is_ok();
            previous_focus_handle = active_modal.previous_focus_handle.clone();
            let did_close = self.hide_modal(window, cx);
            if should_close || !did_close {
                return;
            }
        }
        let new_modal = cx.new(|cx| build_view(window, cx));
        self.show_modal(Box::new(new_modal), previous_focus_handle, window, cx);
        cx.emit(ModalOpenedEvent);
    }

    /// Shows a modal and sets up subscriptions for dismiss events and focus tracking.
    /// The modal is automatically focused after being shown.
    fn show_modal(
        &mut self,
        modal: Box<dyn ModalViewHandle>,
        previous_focus_handle: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_handle = cx.focus_handle();
        let modal_focus_handle = modal.focus_handle(cx);
        let dismiss_subscription = modal.subscribe_dismiss(window, cx);
        // A view is drawn once per frame, so a modal shown again while it still fades out stops
        // fading and comes back in.
        if self
            .closing_modal
            .as_ref()
            .is_some_and(|closing| closing.view.entity_id() == modal.view().entity_id())
        {
            self.closing_modal = None;
        }
        self.presentations += 1;
        self.active_modal = Some(ActiveModal {
            modal,
            _subscriptions: [
                dismiss_subscription,
                cx.on_focus_out(&focus_handle, window, |this, _event, window, cx| {
                    if this.dismiss_on_focus_lost {
                        this.hide_modal(window, cx);
                    }
                }),
            ],
            previous_focus_handle,
            focus_handle,
            presentation: self.presentations,
        });
        cx.defer_in(window, move |_, window, cx| {
            window.focus(&modal_focus_handle, cx);
        });
        cx.notify();
    }

    /// Reveals the most recently stashed reopenable modal, if any. Returns whether
    /// a modal was revealed.
    pub fn reveal_stashed_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.stashed_modal.is_none() {
            return false;
        }
        if self.active_modal.is_some() {
            // Another modal is still closing (e.g. the which-key popup that dismisses
            // only after this action fires). Reveal once it is gone; see `hide_modal`.
            self.reveal_stash_when_free = true;
            return false;
        }
        let Some(modal) = self.stashed_modal.take() else {
            return false;
        };
        let previous_focus_handle = window.focused(cx);
        self.show_modal(modal, previous_focus_handle, window, cx);
        cx.emit(ModalOpenedEvent);
        true
    }

    /// Attempts to hide the currently active modal.
    ///
    /// The modal's `on_before_dismiss` method is called to determine if dismissal should proceed.
    /// If dismissal is allowed, the modal is removed and focus is restored to the previously
    /// focused element.
    ///
    /// Returns `true` if the modal was successfully hidden, `false` otherwise.
    pub fn hide_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(active_modal) = self.active_modal.as_mut() else {
            self.dismiss_on_focus_lost = false;
            return false;
        };

        match active_modal.modal.on_before_dismiss(window, cx) {
            DismissDecision::Dismiss(should_dismiss) => {
                if !should_dismiss {
                    self.dismiss_on_focus_lost = !should_dismiss;
                    return false;
                }
            }
            DismissDecision::Pending => {
                self.dismiss_on_focus_lost = false;
                return false;
            }
        }

        if let Some(active_modal) = self.active_modal.take() {
            let reopenable = focus_handle_is_reopenable(&active_modal.modal.focus_handle(cx), cx);
            self.fade_out(&active_modal, cx);
            if let Some(previous_focus) = active_modal.previous_focus_handle
                && active_modal.focus_handle.contains_focused(window, cx)
            {
                previous_focus.focus(window, cx);
            }
            if reopenable {
                self.stashed_modal = Some(active_modal.modal);
            }
            cx.notify();
        }
        self.dismiss_on_focus_lost = false;
        // A reveal was requested while this modal was still active; now that the layer
        // is free, honor it.
        if self.reveal_stash_when_free && self.active_modal.is_none() {
            self.reveal_stash_when_free = false;
            self.reveal_stashed_modal(window, cx);
        }
        true
    }

    /// Keeps a dismissed modal on screen, inert, while it fades out.
    fn fade_out(&mut self, modal: &ActiveModal, cx: &mut Context<Self>) {
        let exit = SurfaceMotion::DIALOG.exit;
        if !surfaces_move(cx) || exit.is_zero() {
            return;
        }
        let removal = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(exit).await;
            this.update(cx, |this, cx| {
                this.closing_modal = None;
                cx.notify();
            })
            .ok();
        });
        self.closing_modal = Some(ClosingModal {
            view: modal.modal.view(),
            presentation: modal.presentation,
            render_bare: modal.modal.render_bare(cx),
            fade_out_background: modal.modal.fade_out_background(cx),
            anchor_position: modal.modal.anchor_position(cx),
            since: cx.background_executor().now(),
            _removal: removal,
        });
    }

    /// Returns the currently active modal if it is of type `V`.
    pub fn active_modal<V>(&self) -> Option<Entity<V>>
    where
        V: 'static,
    {
        let active_modal = self.active_modal.as_ref()?;
        active_modal.modal.view().downcast::<V>().ok()
    }

    pub fn has_active_modal(&self) -> bool {
        self.active_modal.is_some()
    }
}

impl Render for ModalLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let closing = self.closing_modal.as_ref().map(|closing| {
            let placement = ModalPlacement {
                view: closing.view.clone(),
                presentation: closing.presentation,
                render_bare: closing.render_bare,
                fade_out_background: closing.fade_out_background,
                anchor_position: closing.anchor_position,
                ready: true,
                leaving_since: Some(closing.since),
            };
            // A modal that is going away blocks nothing: the window beneath takes the clicks.
            placement.render(div().absolute().size_full().inset_0(), None, cx)
        });

        let Some(active_modal) = &self.active_modal else {
            return closing.unwrap_or_else(|| div().into_any_element());
        };

        let placement = ModalPlacement {
            view: active_modal.modal.view(),
            presentation: active_modal.presentation,
            render_bare: active_modal.modal.render_bare(cx),
            fade_out_background: active_modal.modal.fade_out_background(cx),
            anchor_position: active_modal.modal.anchor_position(cx),
            ready: active_modal.modal.surface_ready(cx),
            leaving_since: None,
        };
        let active = if placement.render_bare {
            placement.render(div().absolute().size_full().inset_0(), None, cx)
        } else {
            placement.render(
                div()
                    .absolute()
                    .size_full()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.hide_modal(window, cx);
                        }),
                    ),
                Some(&active_modal.focus_handle),
                cx,
            )
        };
        match closing {
            Some(closing) => div()
                .absolute()
                .size_full()
                .inset_0()
                .child(closing)
                .child(active)
                .into_any_element(),
            None => active,
        }
    }
}

/// Where and how one showing of a modal is drawn, whether it is coming in or fading out.
struct ModalPlacement {
    view: AnyView,
    presentation: usize,
    render_bare: bool,
    fade_out_background: bool,
    anchor_position: Option<Point<Pixels>>,
    ready: bool,
    leaving_since: Option<Instant>,
}

impl ModalPlacement {
    fn render(self, layer: Div, focus_handle: Option<&FocusHandle>, cx: &App) -> AnyElement {
        let motion = SurfaceMotion::DIALOG;
        let presentation = self.presentation;
        if self.render_bare {
            return layer
                .child(motion.present_when_ready(
                    ("modal", presentation),
                    self.view,
                    self.ready,
                    self.leaving_since,
                    cx,
                ))
                .into_any_element();
        }

        let modal_content = motion.present_when_ready(
            ("modal", presentation),
            h_flex()
                .occlude()
                .child(self.view)
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                }),
            self.ready,
            self.leaving_since,
            cx,
        );
        // The wash fades with the modal but stays where it is: it covers the whole window.
        let wash = self.fade_out_background.then(|| {
            let mut background = cx.theme().colors().elevated_surface_background;
            background.fade_out(0.2);
            SurfaceMotion {
                offset: Point::default(),
                ..motion
            }
            .present_when_ready(
                ("modal-wash", presentation),
                div().absolute().size_full().inset_0().bg(background),
                self.ready,
                self.leaving_since,
                cx,
            )
        });
        let placed = match self.anchor_position {
            Some(position) => anchored()
                .position(position)
                .anchor(gpui::Anchor::TopLeft)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .when_some(focus_handle, |this, focus_handle| {
                            this.track_focus(focus_handle)
                        })
                        .child(modal_content),
                )
                .into_any_element(),
            None => v_flex()
                .h(px(0.0))
                .top_20()
                .items_center()
                .when_some(focus_handle, |this, focus_handle| {
                    this.track_focus(focus_handle)
                })
                .child(modal_content)
                .into_any_element(),
        };
        layer.children(wash).child(placed).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Focusable, Modifiers, TestAppContext, point};

    const MODAL_SELECTOR: &str = "anchored_test_modal";

    struct AnchoredTestModal {
        focus_handle: FocusHandle,
        anchor_position: Option<Point<Pixels>>,
    }

    impl EventEmitter<DismissEvent> for AnchoredTestModal {}

    impl Focusable for AnchoredTestModal {
        fn focus_handle(&self, _cx: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl ModalView for AnchoredTestModal {
        fn anchor_position(&self) -> Option<Point<Pixels>> {
            self.anchor_position
        }
    }

    impl Render for AnchoredTestModal {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| MODAL_SELECTOR.to_string())
                .track_focus(&self.focus_handle)
                .w(px(120.))
                .h(px(60.))
        }
    }

    struct ModalHost {
        modal_layer: Entity<ModalLayer>,
    }

    impl Render for ModalHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().relative().size_full().child(self.modal_layer.clone())
        }
    }

    #[gpui::test]
    async fn test_modal_anchor_position_places_modal_and_outside_click_dismisses(
        cx: &mut TestAppContext,
    ) {
        let (host, cx) = cx.add_window_view(|_, cx| ModalHost {
            modal_layer: cx.new(|_| ModalLayer::new()),
        });
        let modal_layer = host.read_with(cx, |host, _| host.modal_layer.clone());
        let anchor = point(px(150.), px(90.));

        modal_layer.update_in(cx, |modal_layer, window, cx| {
            modal_layer.toggle_modal(window, cx, |_, cx| AnchoredTestModal {
                focus_handle: cx.focus_handle(),
                anchor_position: Some(anchor),
            });
        });
        cx.run_until_parked();

        let modal_bounds = cx
            .debug_bounds(MODAL_SELECTOR)
            .expect("the anchored modal should be rendered");
        assert_eq!(
            modal_bounds.origin, anchor,
            "an anchored modal should put its top-left corner at its anchor position"
        );

        cx.simulate_click(modal_bounds.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            modal_layer.read_with(cx, |modal_layer, _| modal_layer.has_active_modal()),
            "a click inside an anchored modal should keep it open"
        );

        cx.simulate_click(point(px(10.), px(10.)), Modifiers::none());
        cx.run_until_parked();
        assert!(
            !modal_layer.read_with(cx, |modal_layer, _| modal_layer.has_active_modal()),
            "a click outside an anchored modal should dismiss it"
        );
    }
}
