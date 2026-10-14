use editor::{Editor, EditorEvent};
use gpui::{
    App, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    Keystroke, MouseDownEvent, Pixels, Point, SharedString, Subscription, Window, anchored, px,
};
use ui::{POPUP_SURFACE_METRICS, prelude::*};
use workspace::ModalView;

use crate::UsagesSession;

const WINDOW_MARGIN: Pixels = px(8.);
const HORIZONTAL_PADDING: Pixels = px(12.);
const VERTICAL_PADDING: Pixels = px(8.);
const LINE_GAP: Pixels = px(4.);
const FIND_OPTIONS_LABEL: &str = "Find Options…";

pub(crate) struct HintContent {
    pub(crate) message: SharedString,
    pub(crate) warning: bool,
    pub(crate) second_invocation: Option<SharedString>,
    pub(crate) find_options_shortcut: Option<SharedString>,
    pub(crate) session: UsagesSession,
}

pub(crate) struct UsagesHint {
    content: HintContent,
    position: Point<Pixels>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl UsagesHint {
    pub(crate) fn new(
        content: HintContent,
        position: Point<Pixels>,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = window
            .focused(cx)
            .unwrap_or_else(|| editor.read(cx).focus_handle(cx));
        let editor_subscription = cx.subscribe(&editor, |_, _, event: &EditorEvent, cx| {
            if hides_hint(event) {
                cx.emit(DismissEvent);
            }
        });
        cx.defer_in(window, |this, _, cx| {
            let keystroke_subscription = cx.observe_keystrokes(|_, event, _, cx| {
                if !is_standalone_modifier(&event.keystroke) {
                    cx.emit(DismissEvent);
                }
            });
            this._subscriptions.push(keystroke_subscription);
        });
        Self {
            content,
            position,
            focus_handle,
            _subscriptions: vec![editor_subscription],
        }
    }

    pub(crate) fn session(&self) -> &UsagesSession {
        &self.content.session
    }

    fn open_find_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.content.session.clone();
        cx.emit(DismissEvent);
        crate::show_find_usages_dialog(session, window, cx);
    }
}

fn hides_hint(event: &EditorEvent) -> bool {
    match event {
        EditorEvent::Edited { .. } | EditorEvent::BufferEdited => true,
        EditorEvent::ScrollPositionChanged { local, autoscroll } => *local && !*autoscroll,
        EditorEvent::SelectionsChanged { local } => *local,
        _ => false,
    }
}

fn is_standalone_modifier(keystroke: &Keystroke) -> bool {
    keystroke.key_char.is_none()
        && !keystroke.modifiers.modified()
        && matches!(
            keystroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        )
}

impl EventEmitter<DismissEvent> for UsagesHint {}

impl Focusable for UsagesHint {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for UsagesHint {
    fn render_bare(&self) -> bool {
        true
    }
}

impl Render for UsagesHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = if self.content.warning {
            cx.theme().status().warning_background
        } else {
            cx.theme().status().info_background
        };
        anchored()
            .position(self.position)
            .snap_to_window_with_margin(WINDOW_MARGIN)
            .child(
                div()
                    .id("usages-hint")
                    .occlude()
                    .elevation_2(cx)
                    .overflow_hidden()
                    .on_mouse_down_out(
                        cx.listener(|_, _: &MouseDownEvent, _, cx| cx.emit(DismissEvent)),
                    )
                    .child(
                        v_flex()
                            .px(HORIZONTAL_PADDING)
                            .py(VERTICAL_PADDING)
                            .rounded(POPUP_SURFACE_METRICS.corner_radius)
                            .bg(background)
                            .child(Label::new(self.content.message.clone()))
                            .when_some(self.content.second_invocation.clone(), |this, text| {
                                this.child(div().mt(LINE_GAP).child(
                                    Label::new(text).size(LabelSize::Small).color(Color::Muted),
                                ))
                            })
                            .child(
                                h_flex()
                                    .mt(LINE_GAP)
                                    .gap_1()
                                    .child(
                                        div()
                                            .id("usages-hint-find-options")
                                            .cursor_pointer()
                                            .child(
                                                Label::new(FIND_OPTIONS_LABEL).color(Color::Accent),
                                            )
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, window, cx| {
                                                    this.open_find_options(window, cx)
                                                },
                                            )),
                                    )
                                    .when_some(
                                        self.content.find_options_shortcut.clone(),
                                        |this, shortcut| {
                                            this.child(
                                                Label::new(format!("({shortcut})"))
                                                    .color(Color::Muted),
                                            )
                                        },
                                    ),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod test_support;
