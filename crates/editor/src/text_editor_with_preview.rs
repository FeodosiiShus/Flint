use std::time::{Duration, Instant};

use db::kvp::KeyValueStore;
use gpui::{
    Action, AnyView, ClickEvent, DragMoveEvent, EntityId, FocusHandle, KeyDownEvent, MouseButton,
    MouseMoveEvent, MouseUpEvent, Subscription, Task, WeakFocusHandle,
};
use settings::Settings as _;
use ui::{IconButtonShape, Tooltip, prelude::*};
use util::ResultExt as _;
use workspace::TabBarSettings;

use crate::{
    Editor, EditorElement, EditorEvent, LayoutEditorAndPreview, LayoutEditorOnly, LayoutPreviewOnly,
};

const LAYOUT_KEY: &str = "text_editor_with_preview_layout";
const SPLIT_PROPORTION_KEY: &str = "text_editor_with_preview_split_proportion";
const DEFAULT_SPLIT_PROPORTION: f32 = 0.5;
const MIN_SPLIT_PROPORTION: f32 = 0.1;
const MAX_SPLIT_PROPORTION: f32 = 0.9;
const DIVIDER_THICKNESS: f32 = 1.;
const DIVIDER_DRAG_AREA: f32 = 8.;
const FLOATING_TOOLBAR_TOP: f32 = 25.;
const FLOATING_TOOLBAR_RIGHT: f32 = 20.;
const FLOATING_TOOLBAR_VERTICAL_PADDING: f32 = 5.;
const FLOATING_TOOLBAR_HORIZONTAL_PADDING: f32 = 7.;
const FLOATING_TOOLBAR_RADIUS: f32 = 3.;
const FLOATING_TOOLBAR_BACKGROUND_ALPHA: f32 = 0.55;
const FLOATING_TOOLBAR_HIDE_DELAY: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewLayout {
    EditorOnly,
    EditorAndPreview,
    PreviewOnly,
}

impl PreviewLayout {
    const ALL: [Self; 3] = [Self::EditorOnly, Self::EditorAndPreview, Self::PreviewOnly];

    fn id(self) -> &'static str {
        match self {
            Self::EditorOnly => "Editor only",
            Self::EditorAndPreview => "Editor and Preview",
            Self::PreviewOnly => "Preview only",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.id() == id)
    }

    fn title(self) -> &'static str {
        match self {
            Self::EditorOnly => "Editor",
            Self::EditorAndPreview => "Editor and Preview",
            Self::PreviewOnly => "Preview",
        }
    }

    fn button_id(self) -> &'static str {
        match self {
            Self::EditorOnly => "preview-layout-editor-only",
            Self::EditorAndPreview => "preview-layout-editor-and-preview",
            Self::PreviewOnly => "preview-layout-preview-only",
        }
    }

    fn action(self) -> &'static dyn Action {
        match self {
            Self::EditorOnly => &LayoutEditorOnly,
            Self::EditorAndPreview => &LayoutEditorAndPreview,
            Self::PreviewOnly => &LayoutPreviewOnly,
        }
    }

    fn icon(self, vertical_split: bool) -> IconName {
        match self {
            Self::EditorOnly => IconName::LayoutEditorOnly,
            Self::EditorAndPreview if vertical_split => IconName::EditorPreviewVertical,
            Self::EditorAndPreview => IconName::LayoutEditorPreview,
            Self::PreviewOnly => IconName::LayoutPreviewOnly,
        }
    }
}

pub struct EditorPreview {
    pub view: AnyView,
    pub focus_handle: FocusHandle,
}

pub(crate) struct PreviewHost {
    preview: EditorPreview,
    layout: PreviewLayout,
    vertical_split: bool,
    split_proportion: f32,
    dragging_divider: bool,
    floating_toolbar: FloatingToolbar,
    _hidden_preview_focus_subscription: Subscription,
}

#[derive(Default)]
struct FloatingToolbar {
    visible: bool,
    hovered: bool,
    hide_at: Option<Instant>,
    hide_task: Option<Task<()>>,
}

#[derive(Clone, Copy)]
struct DraggedPreviewDivider {
    editor_id: EntityId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayoutButtonPlacement {
    TabBar,
    FloatingToolbar,
}

fn persisted_layout(cx: &App) -> PreviewLayout {
    KeyValueStore::global(cx)
        .read_kvp(LAYOUT_KEY)
        .log_err()
        .flatten()
        .as_deref()
        .and_then(PreviewLayout::from_id)
        .unwrap_or(PreviewLayout::EditorAndPreview)
}

fn persisted_split_proportion(cx: &App) -> f32 {
    KeyValueStore::global(cx)
        .read_kvp(SPLIT_PROPORTION_KEY)
        .log_err()
        .flatten()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|proportion| proportion.is_finite())
        .map(|proportion| proportion.clamp(MIN_SPLIT_PROPORTION, MAX_SPLIT_PROPORTION))
        .unwrap_or(DEFAULT_SPLIT_PROPORTION)
}

fn persist(key: &'static str, value: String, cx: &App) {
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store.write_kvp(key.to_string(), value).await
    });
}

impl Editor {
    pub fn set_preview(
        &mut self,
        preview: Option<EditorPreview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match preview {
            Some(preview) => self.attach_preview(preview, window, cx),
            None => self.detach_preview(window, cx),
        }
    }

    pub fn has_preview(&self) -> bool {
        self.preview_host.is_some()
    }

    pub fn preview_view(&self) -> Option<&AnyView> {
        self.preview_host.as_ref().map(|host| &host.preview.view)
    }

    pub fn preview_layout(&self) -> Option<PreviewLayout> {
        self.preview_host.as_ref().map(|host| host.layout)
    }

    pub fn set_preview_layout(
        &mut self,
        layout: PreviewLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        let previous_layout = host.layout;
        if previous_layout == layout {
            return;
        }
        host.layout = layout;
        let preview_focus = host.preview.focus_handle.clone();
        persist(LAYOUT_KEY, layout.id().to_string(), cx);
        self.move_focus_for_layout(previous_layout, layout, &preview_focus, window, cx);
        cx.emit(EditorEvent::PreviewLayoutChanged);
        cx.notify();
    }

    pub fn is_preview_split_vertical(&self) -> bool {
        self.preview_host
            .as_ref()
            .is_some_and(|host| host.vertical_split)
    }

    pub(crate) fn forward_focus_to_preview(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some(host) = self
            .preview_host
            .as_ref()
            .filter(|host| host.layout == PreviewLayout::PreviewOnly)
        else {
            return false;
        };
        window.focus(&host.preview.focus_handle, cx);
        true
    }

    pub(crate) fn render_with_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(host) = self.preview_host.as_ref() else {
            return EditorElement::new(&cx.entity(), self.create_style(cx)).into_any_element();
        };
        let layout = host.layout;
        let vertical_split = host.vertical_split;
        let split_proportion = host.split_proportion;
        let preview_view = host.preview.view.clone();
        let floating_toolbar_visible = host.floating_toolbar.visible;
        let floating_toolbar_enabled = !TabBarSettings::get_global(cx).show;
        let hidden_preview_focus =
            (layout == PreviewLayout::EditorOnly).then(|| host.preview.focus_handle.clone());

        let content = match layout {
            PreviewLayout::EditorOnly => {
                EditorElement::new(&cx.entity(), self.create_style(cx)).into_any_element()
            }
            PreviewLayout::PreviewOnly => div()
                .size_full()
                .track_focus(&self.focus_handle)
                .child(preview_view)
                .into_any_element(),
            PreviewLayout::EditorAndPreview => {
                self.render_preview_split(preview_view, vertical_split, split_proportion, cx)
            }
        };

        div()
            .id("text-editor-with-preview")
            .relative()
            .size_full()
            .on_action(cx.listener(|editor, _: &LayoutEditorOnly, window, cx| {
                editor.select_preview_layout(PreviewLayout::EditorOnly, window, cx)
            }))
            .on_action(
                cx.listener(|editor, _: &LayoutEditorAndPreview, window, cx| {
                    editor.select_preview_layout(PreviewLayout::EditorAndPreview, window, cx)
                }),
            )
            .on_action(cx.listener(|editor, _: &LayoutPreviewOnly, window, cx| {
                editor.select_preview_layout(PreviewLayout::PreviewOnly, window, cx)
            }))
            .when(floating_toolbar_enabled, |wrapper| {
                wrapper
                    .on_mouse_move(cx.listener(|editor, _: &MouseMoveEvent, _, cx| {
                        editor.schedule_floating_toolbar_hide(true, cx)
                    }))
                    .on_hover(cx.listener(|editor, hovered: &bool, window, cx| {
                        if !*hovered {
                            editor.floating_toolbar_mouse_left(window, cx)
                        }
                    }))
                    .capture_key_down(cx.listener(|editor, _: &KeyDownEvent, window, cx| {
                        if editor.focus_handle.is_focused(window) {
                            editor.hide_floating_toolbar(cx)
                        }
                    }))
            })
            .child(content)
            .when_some(hidden_preview_focus, |wrapper, preview_focus| {
                wrapper.child(div().absolute().size_0().track_focus(&preview_focus))
            })
            .when(
                floating_toolbar_enabled && floating_toolbar_visible,
                |wrapper| wrapper.children(self.render_floating_toolbar(cx)),
            )
            .into_any_element()
    }

    pub(crate) fn render_preview_tab_bar_actions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let buttons = self.preview_layout_buttons(LayoutButtonPlacement::TabBar, cx)?;
        Some(h_flex().children(buttons).into_any_element())
    }

    fn attach_preview(
        &mut self,
        preview: EditorPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hidden_preview_focus_subscription =
            cx.on_focus(&preview.focus_handle, window, |editor, window, cx| {
                if editor.preview_layout() == Some(PreviewLayout::EditorOnly) {
                    window.focus(&editor.focus_handle, cx);
                }
            });
        let focus_preview = if let Some(host) = self.preview_host.as_mut() {
            let previous_focus = std::mem::replace(&mut host.preview, preview).focus_handle;
            host._hidden_preview_focus_subscription = hidden_preview_focus_subscription;
            let previous_preview_focused = previous_focus.contains_focused(window, cx);
            self.forget_preview_focus(&previous_focus, window);
            previous_preview_focused
        } else {
            let layout = persisted_layout(cx);
            let focus_preview = layout == PreviewLayout::PreviewOnly
                && self.focus_handle.contains_focused(window, cx);
            self.preview_host = Some(PreviewHost {
                preview,
                layout,
                vertical_split: false,
                split_proportion: persisted_split_proportion(cx),
                dragging_divider: false,
                floating_toolbar: FloatingToolbar::default(),
                _hidden_preview_focus_subscription: hidden_preview_focus_subscription,
            });
            focus_preview
        };
        if focus_preview && let Some(host) = self.preview_host.as_ref() {
            window.focus(&host.preview.focus_handle, cx);
        }
        cx.emit(EditorEvent::PreviewLayoutChanged);
        cx.notify();
    }

    fn detach_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.preview_host.take() else {
            return;
        };
        let preview_focus = host.preview.focus_handle;
        if preview_focus.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
        self.forget_preview_focus(&preview_focus, window);
        cx.emit(EditorEvent::PreviewLayoutChanged);
        cx.notify();
    }

    fn forget_preview_focus(&mut self, preview_focus: &FocusHandle, window: &Window) {
        let remembers_preview = self
            .last_focused_descendant
            .as_ref()
            .and_then(WeakFocusHandle::upgrade)
            .is_some_and(|descendant| {
                descendant == *preview_focus || preview_focus.contains(&descendant, window)
            });
        if remembers_preview {
            self.last_focused_descendant = None;
        }
    }

    fn move_focus_for_layout(
        &mut self,
        previous_layout: PreviewLayout,
        layout: PreviewLayout,
        preview_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if layout == PreviewLayout::PreviewOnly {
            if self.focus_handle.contains_focused(window, cx)
                || preview_focus.contains_focused(window, cx)
            {
                window.focus(preview_focus, cx);
            }
            return;
        }
        if previous_layout != PreviewLayout::PreviewOnly && layout != PreviewLayout::EditorOnly {
            return;
        }
        let preview_focused = preview_focus.contains_focused(window, cx);
        self.forget_preview_focus(preview_focus, window);
        if preview_focused {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn select_preview_layout(
        &mut self,
        layout: PreviewLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.preview_layout() {
            None => {}
            Some(current) if current != layout => self.set_preview_layout(layout, window, cx),
            Some(PreviewLayout::EditorAndPreview) => self.flip_preview_split_orientation(cx),
            Some(_) => {}
        }
    }

    fn flip_preview_split_orientation(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        host.vertical_split = !host.vertical_split;
        cx.emit(EditorEvent::PreviewLayoutChanged);
        cx.notify();
    }

    fn drag_preview_divider(
        &mut self,
        event: &DragMoveEvent<DraggedPreviewDivider>,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        let bounds = event.bounds;
        let position = event.event.position;
        let (offset, extent) = if host.vertical_split {
            (position.y - bounds.top(), bounds.size.height)
        } else {
            (position.x - bounds.left(), bounds.size.width)
        };
        if extent <= px(0.) {
            return;
        }
        host.split_proportion = (offset / extent).clamp(MIN_SPLIT_PROPORTION, MAX_SPLIT_PROPORTION);
        host.dragging_divider = true;
        cx.notify();
    }

    fn finish_preview_divider_drag(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self
            .preview_host
            .as_mut()
            .filter(|host| host.dragging_divider)
        else {
            return;
        };
        host.dragging_divider = false;
        persist(SPLIT_PROPORTION_KEY, host.split_proportion.to_string(), cx);
    }

    fn reset_preview_split_proportion(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        host.split_proportion = DEFAULT_SPLIT_PROPORTION;
        host.dragging_divider = false;
        persist(
            SPLIT_PROPORTION_KEY,
            DEFAULT_SPLIT_PROPORTION.to_string(),
            cx,
        );
        cx.notify();
    }

    fn schedule_floating_toolbar_hide(&mut self, show: bool, cx: &mut Context<Self>) {
        let hide_at = cx.background_executor().now() + FLOATING_TOOLBAR_HIDE_DELAY;
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        let toolbar = &mut host.floating_toolbar;
        toolbar.hide_at = Some(hide_at);
        if show && !toolbar.visible {
            toolbar.visible = true;
            cx.notify();
        }
        if toolbar.hide_task.is_none() {
            toolbar.hide_task = Some(cx.spawn(async move |editor, cx| {
                while let Some(delay) = editor
                    .update(cx, |editor, cx| editor.floating_toolbar_hide_delay(cx))
                    .ok()
                    .flatten()
                {
                    cx.background_executor().timer(delay).await;
                }
            }));
        }
    }

    fn floating_toolbar_hide_delay(&mut self, cx: &mut Context<Self>) -> Option<Duration> {
        let now = cx.background_executor().now();
        let toolbar = &mut self.preview_host.as_mut()?.floating_toolbar;
        let Some(hide_at) = toolbar.hide_at else {
            toolbar.hide_task = None;
            return None;
        };
        if hide_at > now {
            return Some(hide_at - now);
        }
        toolbar.hide_at = None;
        toolbar.hide_task = None;
        if toolbar.visible {
            toolbar.visible = false;
            cx.notify();
        }
        None
    }

    fn floating_toolbar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        host.floating_toolbar.hovered = hovered;
        if hovered {
            host.floating_toolbar.hide_at = None;
        } else {
            self.schedule_floating_toolbar_hide(false, cx);
        }
    }

    fn floating_toolbar_mouse_left(&mut self, window: &Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |editor, _, cx| {
            let toolbar_hovered = editor
                .preview_host
                .as_ref()
                .is_some_and(|host| host.floating_toolbar.hovered);
            if !toolbar_hovered {
                editor.hide_floating_toolbar(cx);
            }
        });
    }

    fn hide_floating_toolbar(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.preview_host.as_mut() else {
            return;
        };
        let toolbar = &mut host.floating_toolbar;
        toolbar.hovered = false;
        toolbar.hide_at = None;
        toolbar.hide_task = None;
        if toolbar.visible {
            toolbar.visible = false;
            cx.notify();
        }
    }

    fn render_preview_split(
        &self,
        preview_view: AnyView,
        vertical_split: bool,
        split_proportion: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor_id = cx.entity_id();
        let editor_element = EditorElement::new(&cx.entity(), self.create_style(cx));
        let divider_color = cx.theme().colors().editor_indent_guide;
        let split_pane = |id: &'static str, proportion: f32| {
            div()
                .id(id)
                .flex_shrink_1()
                .overflow_hidden()
                .flex_basis(DefiniteLength::Fraction(proportion))
                .map(|pane| {
                    if vertical_split {
                        pane.min_h_0().w_full()
                    } else {
                        pane.min_w_0().h_full()
                    }
                })
        };
        let drag_area = div()
            .id("text-editor-with-preview-divider")
            .absolute()
            .block_mouse_except_scroll()
            .map(|area| {
                if vertical_split {
                    area.left_0()
                        .top(px(-DIVIDER_DRAG_AREA / 2.))
                        .w_full()
                        .h(px(DIVIDER_DRAG_AREA))
                        .cursor_row_resize()
                } else {
                    area.top_0()
                        .left(px(-DIVIDER_DRAG_AREA / 2.))
                        .h_full()
                        .w(px(DIVIDER_DRAG_AREA))
                        .cursor_col_resize()
                }
            })
            .on_click(cx.listener(|editor, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    editor.reset_preview_split_proportion(cx);
                }
                cx.stop_propagation();
            }))
            .on_drag(DraggedPreviewDivider { editor_id }, |_, _, _, cx| {
                cx.new(|_| gpui::Empty)
            });
        let divider = div()
            .relative()
            .flex_shrink_0()
            .bg(divider_color)
            .map(|divider| {
                if vertical_split {
                    divider.w_full().h(px(DIVIDER_THICKNESS))
                } else {
                    divider.h_full().w(px(DIVIDER_THICKNESS))
                }
            })
            .child(drag_area);

        div()
            .id("text-editor-with-preview-split")
            .flex()
            .size_full()
            .map(|split| {
                if vertical_split {
                    split.flex_col()
                } else {
                    split.flex_row()
                }
            })
            .on_drag_move(cx.listener(
                move |editor, event: &DragMoveEvent<DraggedPreviewDivider>, _, cx| {
                    let dragged_editor = event.drag(cx).editor_id;
                    if dragged_editor == editor_id {
                        editor.drag_preview_divider(event, cx);
                    }
                },
            ))
            .capture_any_mouse_up(
                cx.listener(|editor, _: &MouseUpEvent, _, cx| {
                    editor.finish_preview_divider_drag(cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _: &MouseUpEvent, _, cx| {
                    editor.finish_preview_divider_drag(cx)
                }),
            )
            .child(
                split_pane("text-editor-with-preview-editor", split_proportion)
                    .child(editor_element),
            )
            .child(divider)
            .child(
                split_pane("text-editor-with-preview-preview", 1. - split_proportion)
                    .child(preview_view),
            )
            .into_any_element()
    }

    fn render_floating_toolbar(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let buttons = self.preview_layout_buttons(LayoutButtonPlacement::FloatingToolbar, cx)?;
        Some(
            h_flex()
                .id("text-editor-with-preview-floating-toolbar")
                .absolute()
                .top(px(FLOATING_TOOLBAR_TOP))
                .right(px(FLOATING_TOOLBAR_RIGHT))
                .py(px(FLOATING_TOOLBAR_VERTICAL_PADDING))
                .px(px(FLOATING_TOOLBAR_HORIZONTAL_PADDING))
                .rounded(px(FLOATING_TOOLBAR_RADIUS))
                .bg(cx
                    .theme()
                    .colors()
                    .elevated_surface_background
                    .alpha(FLOATING_TOOLBAR_BACKGROUND_ALPHA))
                .occlude()
                .on_hover(cx.listener(|editor, hovered: &bool, _, cx| {
                    editor.floating_toolbar_hovered(*hovered, cx)
                }))
                .children(buttons),
        )
    }

    fn preview_layout_buttons(
        &self,
        placement: LayoutButtonPlacement,
        cx: &Context<Self>,
    ) -> Option<[IconButton; 3]> {
        let host = self.preview_host.as_ref()?;
        let current_layout = host.layout;
        let vertical_split = host.vertical_split;
        let focus_handle = &self.focus_handle;
        Some(PreviewLayout::ALL.map(|layout| {
            IconButton::new(layout.button_id(), layout.icon(vertical_split))
                .icon_size(IconSize::Medium)
                .toggle_state(layout == current_layout)
                .tooltip(Tooltip::for_action_title_in(
                    layout.title(),
                    layout.action(),
                    focus_handle,
                ))
                .on_click(cx.listener(move |editor, _: &ClickEvent, window, cx| {
                    editor.select_preview_layout(layout, window, cx)
                }))
                .map(|button| match placement {
                    LayoutButtonPlacement::TabBar => button.chrome_region(ui::ChromeRegion::TabBar),
                    LayoutButtonPlacement::FloatingToolbar => button.shape(IconButtonShape::Square),
                })
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{Entity, Focusable, Render, TestAppContext, VisualTestContext};

    use super::*;
    use crate::{EditorMode, MultiBuffer, editor_tests::init_test};

    struct TestPreview {
        focus_handle: FocusHandle,
    }

    impl Render for TestPreview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().track_focus(&self.focus_handle)
        }
    }

    impl Focusable for TestPreview {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    fn build_focused_editor(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
        init_test(cx, |_| {});
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(
                EditorMode::full(),
                MultiBuffer::build_simple("# Title\n\nBody\n", cx),
                None,
                window,
                cx,
            )
        });
        editor.update_in(cx, |editor, window, cx| {
            window.focus(&editor.focus_handle, cx);
        });
        cx.run_until_parked();
        (editor, cx)
    }

    fn attach_test_preview(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> FocusHandle {
        let preview_focus = editor.update_in(cx, |editor, window, cx| {
            let preview = cx.new(|cx| TestPreview {
                focus_handle: cx.focus_handle(),
            });
            let focus_handle = preview.read(cx).focus_handle.clone();
            editor.set_preview(
                Some(EditorPreview {
                    view: preview.into(),
                    focus_handle: focus_handle.clone(),
                }),
                window,
                cx,
            );
            focus_handle
        });
        cx.run_until_parked();
        preview_focus
    }

    fn layout_actions_available(cx: &mut VisualTestContext) -> [bool; 3] {
        cx.update(|window, cx| {
            PreviewLayout::ALL.map(|layout| window.is_action_available(layout.action(), cx))
        })
    }

    #[gpui::test]
    async fn text_editor_with_preview_routes_focus_to_and_from_preview_only(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = build_focused_editor(cx);
        let preview_focus = attach_test_preview(&editor, cx);
        editor.update_in(cx, |editor, window, cx| {
            assert_eq!(
                editor.preview_layout(),
                Some(PreviewLayout::EditorAndPreview),
                "with nothing persisted the first preview opens side by side with the editor"
            );
            assert!(
                editor.focus_handle.is_focused(window),
                "Editor and Preview keeps the text editor as the preferred focus"
            );
            editor.set_preview_layout(PreviewLayout::PreviewOnly, window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, _| {
            assert!(
                preview_focus.is_focused(window),
                "switching to Preview only while the editor is focused moves focus to the preview"
            );
        });

        editor.update_in(cx, |editor, window, cx| {
            window.focus(&editor.focus_handle, cx);
        });
        cx.run_until_parked();
        cx.update(|window, _| {
            assert!(
                preview_focus.is_focused(window),
                "activating the editor tab in Preview only forwards focus to the preview"
            );
        });

        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview_layout(PreviewLayout::EditorAndPreview, window, cx);
        });
        cx.run_until_parked();
        editor.update_in(cx, |editor, window, _| {
            assert!(
                editor.focus_handle.is_focused(window),
                "leaving Preview only while the preview is focused gives focus back to the editor"
            );
            assert!(!preview_focus.is_focused(window));
        });
    }

    #[gpui::test]
    async fn text_editor_with_preview_forwards_focus_of_the_hidden_preview_to_the_editor(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = build_focused_editor(cx);
        let preview_focus = attach_test_preview(&editor, cx);
        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview_layout(PreviewLayout::EditorOnly, window, cx);
        });
        cx.run_until_parked();

        cx.update(|window, cx| window.focus(&preview_focus, cx));
        cx.run_until_parked();
        editor.update_in(cx, |editor, window, _| {
            assert!(
                editor.focus_handle.is_focused(window),
                "a pane restoring focus to the hidden preview in Editor only must land in the editor, not lose focus"
            );
        });
    }

    #[gpui::test]
    async fn text_editor_with_preview_selecting_editor_and_preview_twice_flips_orientation(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = build_focused_editor(cx);
        attach_test_preview(&editor, cx);
        cx.dispatch_action(LayoutEditorOnly);
        editor.update(cx, |editor, _| {
            assert_eq!(editor.preview_layout(), Some(PreviewLayout::EditorOnly));
        });

        cx.dispatch_action(LayoutEditorAndPreview);
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.preview_layout(),
                Some(PreviewLayout::EditorAndPreview)
            );
            assert!(
                !editor.is_preview_split_vertical(),
                "selecting the layout from another layout keeps the side by side orientation"
            );
        });

        cx.dispatch_action(LayoutEditorAndPreview);
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.preview_layout(),
                Some(PreviewLayout::EditorAndPreview)
            );
            assert!(
                editor.is_preview_split_vertical(),
                "selecting the already selected Editor and Preview stacks the panes, like IntelliJ"
            );
        });

        cx.dispatch_action(LayoutEditorAndPreview);
        editor.update(cx, |editor, _| {
            assert!(
                !editor.is_preview_split_vertical(),
                "a third selection flips back to side by side"
            );
        });

        cx.dispatch_action(LayoutEditorOnly);
        cx.dispatch_action(LayoutEditorOnly);
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.preview_layout(),
                Some(PreviewLayout::EditorOnly),
                "selecting a selected Editor layout again changes nothing"
            );
            assert!(!editor.is_preview_split_vertical());
        });
    }

    #[gpui::test]
    async fn text_editor_with_preview_layout_actions_need_an_attached_preview(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = build_focused_editor(cx);
        assert_eq!(
            layout_actions_available(cx),
            [false; 3],
            "a plain editor offers no preview layout actions"
        );
        cx.dispatch_action(LayoutPreviewOnly);
        editor.update(cx, |editor, _| {
            assert!(!editor.has_preview());
            assert_eq!(editor.preview_layout(), None);
        });

        attach_test_preview(&editor, cx);
        assert_eq!(layout_actions_available(cx), [true; 3]);
        cx.dispatch_action(LayoutPreviewOnly);
        editor.update(cx, |editor, _| {
            assert_eq!(editor.preview_layout(), Some(PreviewLayout::PreviewOnly));
        });
        assert_eq!(
            layout_actions_available(cx),
            [true; 3],
            "the actions stay reachable from the focused preview in Preview only"
        );
        cx.dispatch_action(LayoutEditorOnly);
        editor.update(cx, |editor, _| {
            assert_eq!(editor.preview_layout(), Some(PreviewLayout::EditorOnly));
        });
        assert_eq!(
            layout_actions_available(cx),
            [true; 3],
            "the actions stay reachable from the editor in Editor only"
        );
    }

    #[gpui::test]
    async fn text_editor_with_preview_emits_layout_changed_for_every_visible_change(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = build_focused_editor(cx);
        let layout_changes = Rc::new(Cell::new(0));
        let _subscription = cx.update(|_, cx| {
            let layout_changes = layout_changes.clone();
            cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
                if *event == EditorEvent::PreviewLayoutChanged {
                    layout_changes.set(layout_changes.get() + 1);
                }
            })
        });

        attach_test_preview(&editor, cx);
        assert_eq!(layout_changes.get(), 1, "attaching a preview");

        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview_layout(PreviewLayout::EditorOnly, window, cx);
            editor.set_preview_layout(PreviewLayout::EditorOnly, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            layout_changes.get(),
            2,
            "a layout change is reported once, re-selecting the same layout is not"
        );

        cx.dispatch_action(LayoutEditorAndPreview);
        cx.dispatch_action(LayoutEditorAndPreview);
        assert_eq!(
            layout_changes.get(),
            4,
            "a layout change and an orientation flip"
        );

        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview(None, window, cx);
            editor.set_preview(None, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            layout_changes.get(),
            5,
            "detaching is reported once, detaching nothing is not"
        );
    }

    #[gpui::test]
    async fn text_editor_with_preview_detaching_restores_the_plain_editor(cx: &mut TestAppContext) {
        let (editor, cx) = build_focused_editor(cx);
        let preview_focus = attach_test_preview(&editor, cx);
        cx.dispatch_action(LayoutEditorAndPreview);
        cx.dispatch_action(LayoutPreviewOnly);
        cx.update(|window, _| assert!(preview_focus.is_focused(window)));

        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview(None, window, cx)
        });
        cx.run_until_parked();
        editor.update_in(cx, |editor, window, _| {
            assert!(!editor.has_preview());
            assert_eq!(editor.preview_layout(), None);
            assert!(!editor.is_preview_split_vertical());
            assert!(
                editor.focus_handle.is_focused(window),
                "detaching a focused preview gives focus back to the editor"
            );
        });
        assert_eq!(layout_actions_available(cx), [false; 3]);

        cx.update(|window, cx| window.blur(cx));
        cx.run_until_parked();
        editor.update_in(cx, |editor, window, cx| {
            window.focus(&editor.focus_handle, cx);
        });
        cx.run_until_parked();
        editor.update_in(cx, |editor, window, _| {
            assert!(
                editor.focus_handle.is_focused(window),
                "after detaching, focusing the editor no longer forwards focus anywhere"
            );
        });
    }

    #[gpui::test]
    async fn text_editor_with_preview_reopens_with_the_last_chosen_layout(cx: &mut TestAppContext) {
        let (editor, cx) = build_focused_editor(cx);
        attach_test_preview(&editor, cx);
        cx.dispatch_action(LayoutPreviewOnly);
        editor.update_in(cx, |editor, window, cx| {
            editor.set_preview(None, window, cx)
        });
        cx.run_until_parked();

        let preview_focus = attach_test_preview(&editor, cx);
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.preview_layout(),
                Some(PreviewLayout::PreviewOnly),
                "a newly attached preview starts in the layout the user chose last"
            );
            assert!(
                !editor.is_preview_split_vertical(),
                "the orientation is per editor and starts side by side"
            );
        });
        cx.update(|window, _| {
            assert!(
                preview_focus.is_focused(window),
                "opening straight into Preview only focuses the preview"
            );
        });
    }
}
