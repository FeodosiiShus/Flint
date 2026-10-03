use std::{num::NonZeroU32, sync::Arc};

use editor::Editor;
use fs::Fs;
use gpui::{
    App, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task, WeakEntity,
};
use language::{Buffer, language_settings::LanguageSettings};
use picker::{Picker, PickerDelegate};
use settings::{Settings, SettingsContent, SettingsStore, update_settings_file};
use ui::{ListItem, ListItemSpacing, Tooltip, prelude::*};
use workspace::{HideStatusItem, ItemHandle, ModalView, StatusBarSettings, StatusItemView};

use crate::SelectIndentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub tab_size: u32,
    pub hard_tabs: bool,
}

impl Indentation {
    pub fn for_buffer(buffer: &Buffer, cx: &App) -> Self {
        let settings = LanguageSettings::for_buffer(buffer, cx);
        Self {
            tab_size: settings.tab_size.get(),
            hard_tabs: settings.hard_tabs,
        }
    }

    pub fn label(&self) -> SharedString {
        if self.hard_tabs {
            return "Tab".into();
        }
        match self.tab_size {
            1 => "1 space".into(),
            tab_size => format!("{tab_size} spaces").into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentationOption {
    Spaces(u32),
    Tab,
}

pub const INDENTATION_OPTIONS: [IndentationOption; 4] = [
    IndentationOption::Spaces(2),
    IndentationOption::Spaces(4),
    IndentationOption::Spaces(8),
    IndentationOption::Tab,
];

impl IndentationOption {
    pub fn label(&self) -> SharedString {
        match self {
            Self::Spaces(tab_size) => Indentation {
                tab_size: *tab_size,
                hard_tabs: false,
            }
            .label(),
            Self::Tab => "Tab".into(),
        }
    }

    pub fn matches(&self, indentation: Indentation) -> bool {
        match self {
            Self::Spaces(tab_size) => !indentation.hard_tabs && indentation.tab_size == *tab_size,
            Self::Tab => indentation.hard_tabs,
        }
    }

    pub fn apply(&self, settings: &mut SettingsContent, language_name: Option<&str>) {
        let language_settings = match language_name {
            Some(language_name) => settings
                .languages_mut()
                .entry(language_name.to_string())
                .or_default(),
            None => &mut settings.project.all_languages.defaults,
        };
        match self {
            Self::Spaces(tab_size) => {
                language_settings.tab_size = NonZeroU32::new(*tab_size);
                language_settings.hard_tabs = Some(false);
            }
            Self::Tab => {
                language_settings.hard_tabs = Some(true);
            }
        }
    }
}

pub struct IndentationIndicator {
    active_buffer: Option<WeakEntity<Buffer>>,
    active_editor: Option<WeakEntity<Editor>>,
    _observe_active_editor: Option<Subscription>,
    _observe_settings: Subscription,
}

impl IndentationIndicator {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            active_buffer: None,
            active_editor: None,
            _observe_active_editor: None,
            _observe_settings: cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
        }
    }

    pub fn indentation(&self, cx: &App) -> Option<Indentation> {
        let buffer = self.active_buffer.as_ref()?.upgrade()?;
        Some(Indentation::for_buffer(buffer.read(cx), cx))
    }

    fn update(&mut self, editor: Entity<Editor>, _: &mut Window, cx: &mut Context<Self>) {
        self.active_buffer = editor
            .read(cx)
            .active_buffer(cx)
            .map(|buffer| buffer.downgrade());
        self.active_editor = Some(editor.downgrade());
        cx.notify();
    }
}

impl Render for IndentationIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !StatusBarSettings::get_global(cx).indentation_button {
            return div();
        }

        div().when_some(self.indentation(cx), |element, indentation| {
            element.child(
                Button::new("change-indentation", indentation.label())
                    .label_size(LabelSize::Small)
                    .chrome_region(ui::ChromeRegion::StatusBar)
                    .tab_index(0isize)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(editor) = this.active_editor.as_ref() {
                            IndentationSelector::toggle(editor, window, cx);
                        }
                    }))
                    .tooltip(|_window, cx| {
                        Tooltip::for_action("Select Indentation", &SelectIndentation, cx)
                    }),
            )
        })
    }
}

impl StatusItemView for IndentationIndicator {
    fn set_active_pane_item(
        &mut self,
        active_pane_item: Option<&dyn ItemHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = active_pane_item.and_then(|item| item.downcast::<Editor>()) {
            self._observe_active_editor = Some(cx.observe_in(&editor, window, Self::update));
            self.update(editor, window, cx);
        } else {
            self.active_buffer = None;
            self.active_editor = None;
            self._observe_active_editor = None;
        }
        cx.notify();
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        Some(HideStatusItem::new(|settings| {
            settings
                .status_bar
                .get_or_insert_default()
                .indentation_button = Some(false);
        }))
    }
}

pub struct IndentationSelector {
    picker: Entity<Picker<IndentationSelectorDelegate>>,
}

impl IndentationSelector {
    pub(crate) fn register(
        editor: &mut Editor,
        _window: Option<&mut Window>,
        cx: &mut Context<Editor>,
    ) {
        let editor_handle = cx.weak_entity();
        editor
            .register_action(move |_: &SelectIndentation, window, cx| {
                Self::toggle(&editor_handle, window, cx);
            })
            .detach();
    }

    pub fn toggle(editor: &WeakEntity<Editor>, window: &mut Window, cx: &mut App) {
        let Some((workspace, buffer)) = editor
            .update(cx, |editor, cx| {
                Some((editor.workspace()?, editor.active_buffer(cx)?))
            })
            .ok()
            .flatten()
        else {
            return;
        };

        workspace.update(cx, |workspace, cx| {
            let fs = workspace.app_state().fs.clone();
            workspace.toggle_modal(window, cx, move |window, cx| {
                IndentationSelector::new(buffer, fs, window, cx)
            });
        })
    }

    fn new(
        buffer: Entity<Buffer>,
        fs: Arc<dyn Fs>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let buffer = buffer.read(cx);
        let indentation = Indentation::for_buffer(buffer, cx);
        let language_name = buffer
            .language()
            .map(|language| language.name().0.to_string());
        let delegate = IndentationSelectorDelegate::new(
            cx.entity().downgrade(),
            fs,
            language_name,
            indentation,
        );
        let picker = cx.new(|cx| Picker::nonsearchable_uniform_list(delegate, window, cx));
        Self { picker }
    }
}

impl Render for IndentationSelector {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().child(self.picker.clone())
    }
}

impl Focusable for IndentationSelector {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl EventEmitter<DismissEvent> for IndentationSelector {}
impl ModalView for IndentationSelector {}

struct IndentationSelectorDelegate {
    indentation_selector: WeakEntity<IndentationSelector>,
    fs: Arc<dyn Fs>,
    language_name: Option<String>,
    indentation: Indentation,
    selected_index: usize,
}

impl IndentationSelectorDelegate {
    fn new(
        indentation_selector: WeakEntity<IndentationSelector>,
        fs: Arc<dyn Fs>,
        language_name: Option<String>,
        indentation: Indentation,
    ) -> Self {
        let selected_index = INDENTATION_OPTIONS
            .iter()
            .position(|option| option.matches(indentation))
            .unwrap_or_default();
        Self {
            indentation_selector,
            fs,
            language_name,
            indentation,
            selected_index,
        }
    }
}

impl PickerDelegate for IndentationSelectorDelegate {
    type ListItem = ListItem;

    fn name() -> &'static str {
        "indentation selector"
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        "Select indentation…".into()
    }

    fn match_count(&self) -> usize {
        INDENTATION_OPTIONS.len()
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(option) = INDENTATION_OPTIONS.get(self.selected_index).copied() {
            let language_name = self.language_name.clone();
            update_settings_file(self.fs.clone(), cx, move |settings, _| {
                option.apply(settings, language_name.as_deref());
            });
        }
        self.dismissed(window, cx);
    }

    fn dismissed(&mut self, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.indentation_selector
            .update(cx, |_, cx| cx.emit(DismissEvent))
            .ok();
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        index: usize,
        _window: &mut Window,
        _: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = index;
    }

    fn update_matches(
        &mut self,
        _query: String,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        Task::ready(())
    }

    fn render_match(
        &self,
        index: usize,
        selected: bool,
        _: &mut Window,
        _: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let option = INDENTATION_OPTIONS.get(index)?;
        Some(
            ListItem::new(index)
                .inset(true)
                .spacing(ListItemSpacing::Sparse)
                .toggle_state(selected)
                .child(Label::new(option.label()))
                .when(option.matches(self.indentation), |list_item| {
                    list_item.end_slot(Icon::new(IconName::Check).color(Color::Muted))
                }),
        )
    }
}
