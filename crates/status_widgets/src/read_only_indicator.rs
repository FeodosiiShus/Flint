use editor::Editor;
use gpui::{Entity, Subscription};
use language::Capability;
use settings::Settings;
use ui::{Tooltip, prelude::*};
use workspace::{
    HideStatusItem, ItemHandle, StatusBarSettings, StatusItemView, ToggleReadOnlyFile,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOnlyState {
    pub read_only: bool,
    pub toggleable: bool,
}

impl ReadOnlyState {
    pub fn for_editor(editor: &Editor, cx: &App) -> Self {
        let toggleable = editor.buffer().read(cx).is_singleton()
            && editor.capability(cx) != Capability::ReadOnly;
        Self {
            read_only: editor.read_only(cx),
            toggleable,
        }
    }

    pub fn tooltip(&self) -> &'static str {
        match (self.read_only, self.toggleable) {
            (true, true) => "File is read-only. Click to make it writable",
            (true, false) => "File is read-only",
            (false, true) => "File is writable. Click to make it read-only",
            (false, false) => "File is writable",
        }
    }
}

#[derive(Default)]
pub struct ReadOnlyIndicator {
    state: Option<ReadOnlyState>,
    active_item: Option<Box<dyn ItemHandle>>,
    _observe_active_editor: Option<Subscription>,
}

impl ReadOnlyIndicator {
    pub fn state(&self) -> Option<ReadOnlyState> {
        self.state
    }

    pub fn is_visible(&self, cx: &App) -> bool {
        StatusBarSettings::get_global(cx).read_only_button && self.state.is_some()
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_some_and(|state| state.toggleable) {
            return;
        }
        if let Some(item) = self.active_item.as_ref().map(|item| item.boxed_clone()) {
            item.toggle_read_only(window, cx);
        }
    }

    fn update(&mut self, editor: Entity<Editor>, _: &mut Window, cx: &mut Context<Self>) {
        self.state = Some(ReadOnlyState::for_editor(editor.read(cx), cx));
        cx.notify();
    }
}

impl Render for ReadOnlyIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_visible(cx) {
            return div();
        }
        let Some(state) = self.state else {
            return div();
        };

        let icon = if state.read_only {
            IconName::Lock
        } else {
            IconName::LockOff
        };
        let tooltip = state.tooltip();
        div().child(
            IconButton::new("toggle-read-only", icon)
                .icon_size(IconSize::Small)
                .icon_color(Color::Muted)
                .chrome_region(ui::ChromeRegion::StatusBar)
                .tab_index(0isize)
                .aria_label(tooltip)
                .disabled(!state.toggleable)
                .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)))
                .tooltip(move |_window, cx| {
                    if state.toggleable {
                        Tooltip::for_action(tooltip, &ToggleReadOnlyFile, cx)
                    } else {
                        Tooltip::simple(tooltip, cx)
                    }
                }),
        )
    }
}

impl StatusItemView for ReadOnlyIndicator {
    fn set_active_pane_item(
        &mut self,
        active_pane_item: Option<&dyn ItemHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_item = active_pane_item.map(|item| item.boxed_clone());
        if let Some(editor) = active_pane_item.and_then(|item| item.downcast::<Editor>()) {
            self._observe_active_editor = Some(cx.observe_in(&editor, window, Self::update));
            self.update(editor, window, cx);
        } else {
            self.state = None;
            self._observe_active_editor = None;
        }
        cx.notify();
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        Some(HideStatusItem::new(|settings| {
            settings.status_bar.get_or_insert_default().read_only_button = Some(false);
        }))
    }
}
