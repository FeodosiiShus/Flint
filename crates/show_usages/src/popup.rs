use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use editor::actions::ShowUsages;
use file_icons::FileIcons;
use gpui::{
    Action, AnyElement, App, Bounds, ClickEvent, Context, CursorStyle, DismissEvent, Div,
    DragMoveEvent, ElementId, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollStrategy, SharedString, Size, StyledText, Task, TextRun, TextStyle,
    UniformListScrollHandle, Window, prelude::*, px, uniform_list,
};
use menu::{Cancel, Confirm, SelectFirst, SelectLast, SelectNext, SelectPrevious};
use picker::{MatchLocation, PreviewBackend, PreviewLayout, PreviewUpdate};
use project::Location;
use settings::Settings as _;
use text::ToOffset as _;
use theme::SyntaxTheme;
use theme_settings::ThemeSettings;
use ui::{
    ButtonLike, CommonAnimationExt, ContextMenu, PopoverMenu, PopupRowMetrics, PopupRowStyle,
    Tooltip, prelude::*,
};
use util::ResultExt as _;
use workspace::ModalView;
use workspace::item::ItemSettings;

use crate::model::{
    DisplayRow, ShowUsagesState, SpeedSearchMatch, UsageAccess, UsageFilters, UsageRow, UsageScope,
    UsagesModel, filtered_out_message, footer_path, out_of_scope_message,
};
use crate::symbol::SymbolDescription;
use crate::{
    OpenInFindWindow, SelectPageDown, SelectPageUp, ShowSettings, ToggleComments, ToggleImports,
    TogglePreview, ToggleReadAccess, ToggleWriteAccess, UsagesSession, UsagesTitle,
};

const KEY_CONTEXT: &str = "ShowUsagesPopup";

const HEADER_HEIGHT: Pixels = px(41.);
const HEADER_PADDING_LEFT: Pixels = px(20.);
const HEADER_PADDING_RIGHT: Pixels = px(15.);
const HEADER_TITLE_GAP: Pixels = px(12.);
const HEADER_STATUS_GAP: Pixels = px(8.);
const SPINNER_GAP: Pixels = px(6.);

const TOOLBAR_HEIGHT: Pixels = px(39.);
const TOOLBAR_ROW_HEIGHT: Pixels = px(35.);
const TOOLBAR_LINE_INSET: Pixels = px(12.);
const TOOLBAR_PADDING: Pixels = px(3.);
const TOOLBAR_BOTTOM_GAP: Pixels = px(4.);
const TOOLBAR_GAP: Pixels = px(2.);
const TOOLBAR_SPACER: Pixels = px(8.);
const TOOLBAR_BUTTON_SIZE: Pixels = px(22.);
const TOOLBAR_ICON_BUTTONS: usize = 6;
const TOOLBAR_GAPS: usize = 9;
const BUTTON_EDGE: Pixels = px(1.);
const OPTIONS_BUTTON_PADDING: Pixels = px(3.);
const OPTIONS_BUTTON_GAP: Pixels = px(1.);
const SCOPE_BUTTON_PADDING: Pixels = px(6.);
const SCOPE_BUTTON_GAP: Pixels = px(4.);
const SEPARATOR_WIDTH: Pixels = px(1.);
const SEPARATOR_HEIGHT: Pixels = px(16.);
const SEPARATOR_MARGIN: Pixels = px(4.);
const USAGES_LABEL: &str = "Usages";
const SETTINGS_BUTTON_GAP: Pixels = px(8.);

const ROW_METRICS: PopupRowMetrics = PopupRowMetrics::for_style(PopupRowStyle::List);
const ROW_HEIGHT: Pixels = ROW_METRICS.pill_height;
const ROW_INSET: Pixels = ROW_METRICS.pill_inset_x;
const ROW_PADDING: Pixels = ROW_METRICS.content_padding_x;
const SELECTION_RADIUS: Pixels = ROW_METRICS.pill_radius;
const FILE_ICON_GAP: Pixels = px(4.);
const LINE_NUMBER_PADDING: Pixels = px(8.);
const USAGE_ICON_SLOT: Pixels = px(16.);
const USAGE_ICON_GAP: Pixels = px(5.);
const ORIGIN_GAP: Pixels = px(8.);
const ORIGIN_PADDING: Pixels = px(6.);
const MAX_COLUMN_WIDTH: Pixels = px(500.);
const LIST_BOTTOM_SPACE: Pixels = px(4.);
const SPEED_SEARCH_OFFSET: Pixels = px(2.);

const DIVIDER_HEIGHT: Pixels = px(1.);
const DIVIDER_GRIP_HEIGHT: Pixels = px(7.);
const DIVIDER_GRIP_OFFSET: Pixels = px(-3.);
const RESIZE_HANDLE_SIZE: Pixels = px(8.);
const WINDOW_MARGIN: Pixels = px(8.);
const BORDER_WIDTH: Pixels = px(1.);
const MIN_POPUP_WIDTH: Pixels = px(240.);
const FOOTER_PADDING_X: Pixels = px(20.);
const FOOTER_PADDING_Y: Pixels = px(6.);
const FOOTER_FONT_SIZE_OFFSET: Pixels = px(2.);
const MIN_FOOTER_FONT_SIZE: Pixels = px(1.);

const MAX_VISIBLE_ROWS: usize = 30;
const MAX_VISIBLE_ROWS_WITH_PREVIEW: usize = 20;
const MIN_PREVIEW_LINES: f32 = 5.;
const DEFAULT_PREVIEW_RATIO: f32 = 0.6;
const MIN_PREVIEW_RATIO: f32 = 0.1;
const MAX_PREVIEW_RATIO: f32 = 0.9;
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(50);

const READ_ACCENT: u32 = 0x57965C;
const WRITE_ACCENT: u32 = 0xDB5C5C;

const SEARCHING_TEXT: &str = "Searching…";
const MORE_USAGES_TEXT: &str = "more usages";
const SPECIAL_ROW_PREFIX: &str = "...<";
const SPECIAL_ROW_SUFFIX: &str = ">...";
const STRUCTURE_PREFIX: &str = "  ";
const STRUCTURE_SEPARATOR: &str = " › ";
const CURRENT_USAGE_LABEL: &str = "Current";
const FILTERED_OUT_TOOLTIP: &str = "Some filters are configured to not show these usages. Click or press Enter to restart with filters toggled";

#[derive(Clone, Copy, Default)]
struct ColumnWidths {
    file: Pixels,
    line: Pixels,
    code: Pixels,
}

#[derive(Clone, Copy, Default)]
struct Geometry {
    width: Pixels,
    height: Pixels,
    list_height: Pixels,
    preview_height: Pixels,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragKind {
    Move,
    Resize,
    Divider,
}

#[derive(Clone, Copy)]
struct PopupDrag {
    kind: DragKind,
    start_mouse: Point<Pixels>,
    start_bounds: Option<Bounds<Pixels>>,
    start_geometry: Geometry,
}

struct DragHandle;

#[derive(Clone, Copy)]
enum FilterToggle {
    Imports,
    ReadAccess,
    WriteAccess,
    Comments,
}

impl FilterToggle {
    const ALL: [FilterToggle; 4] = [
        FilterToggle::ReadAccess,
        FilterToggle::WriteAccess,
        FilterToggle::Imports,
        FilterToggle::Comments,
    ];

    fn id(self) -> &'static str {
        match self {
            FilterToggle::Imports => "show-usages-toggle-imports",
            FilterToggle::ReadAccess => "show-usages-toggle-read-access",
            FilterToggle::WriteAccess => "show-usages-toggle-write-access",
            FilterToggle::Comments => "show-usages-toggle-comments",
        }
    }

    fn title(self) -> &'static str {
        match self {
            FilterToggle::Imports => "Show Import Statements",
            FilterToggle::ReadAccess => "Show Read Access",
            FilterToggle::WriteAccess => "Show Write Access",
            FilterToggle::Comments => "Show Usages in Comments",
        }
    }

    fn action(self) -> &'static dyn Action {
        match self {
            FilterToggle::Imports => &ToggleImports,
            FilterToggle::ReadAccess => &ToggleReadAccess,
            FilterToggle::WriteAccess => &ToggleWriteAccess,
            FilterToggle::Comments => &ToggleComments,
        }
    }

    fn icon(self) -> ToolbarIcon {
        match self {
            FilterToggle::Imports => ToolbarIcon::Single(IconName::UsagesImportStatements),
            FilterToggle::ReadAccess => ToolbarIcon::Layered {
                base: IconName::UsagesReadAccess,
                accent: IconName::UsagesReadAccessAccent,
                accent_color: READ_ACCENT,
            },
            FilterToggle::WriteAccess => ToolbarIcon::Layered {
                base: IconName::UsagesWriteAccess,
                accent: IconName::UsagesWriteAccessAccent,
                accent_color: WRITE_ACCENT,
            },
            FilterToggle::Comments => ToolbarIcon::Single(IconName::UsagesComments),
        }
    }

    fn is_shown(self, filters: UsageFilters) -> bool {
        match self {
            FilterToggle::Imports => filters.show_imports,
            FilterToggle::ReadAccess => filters.show_read_access,
            FilterToggle::WriteAccess => filters.show_write_access,
            FilterToggle::Comments => filters.show_comments,
        }
    }

    fn flip(self, filters: &mut UsageFilters) {
        match self {
            FilterToggle::Imports => filters.show_imports = !filters.show_imports,
            FilterToggle::ReadAccess => filters.show_read_access = !filters.show_read_access,
            FilterToggle::WriteAccess => filters.show_write_access = !filters.show_write_access,
            FilterToggle::Comments => filters.show_comments = !filters.show_comments,
        }
    }
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    Single(IconName),
    Layered {
        base: IconName,
        accent: IconName,
        accent_color: u32,
    },
}

struct StyledLine {
    text: SharedString,
    runs: Vec<TextRun>,
}

impl StyledLine {
    fn into_styled_text(self) -> StyledText {
        StyledText::new(self.text).with_runs(self.runs)
    }
}

struct RowStyle {
    base: TextStyle,
    font_size: Pixels,
    label_font_size: Pixels,
    small_label_font_size: Pixels,
    icon_width: Pixels,
    muted: Hsla,
    search_background: Hsla,
    selection_background: Hsla,
    pill_background: Hsla,
    file_icons: bool,
}

impl RowStyle {
    fn new(window: &Window, cx: &App) -> Self {
        let settings = ThemeSettings::get_global(cx);
        let colors = cx.theme().colors();
        let rem_size = window.rem_size();
        let font_size = settings.ui_font_size(cx);
        Self {
            base: TextStyle {
                color: colors.text,
                font_family: settings.ui_font.family.clone(),
                font_features: settings.ui_font.features.clone(),
                font_fallbacks: settings.ui_font.fallbacks.clone(),
                font_size: font_size.into(),
                font_weight: settings.ui_font.weight,
                ..TextStyle::default()
            },
            font_size,
            label_font_size: TextSize::Default.pixels(cx),
            small_label_font_size: TextSize::Small.pixels(cx),
            icon_width: IconSize::Small.rems().to_pixels(rem_size),
            muted: colors.text_muted,
            search_background: colors.search_match_background,
            selection_background: colors.ghost_element_selected,
            pill_background: colors.element_background,
            file_icons: ItemSettings::get_global(cx).file_icons,
        }
    }
}

pub(crate) struct ShowUsagesPopup {
    session: UsagesSession,
    state: ShowUsagesState,
    model: Option<UsagesModel<Location>>,
    focus_handle: FocusHandle,
    position: Point<Pixels>,
    selected: Option<usize>,
    scroll_handle: UniformListScrollHandle,
    query: String,
    speed_search_matches: Vec<SpeedSearchMatch>,
    columns: ColumnWidths,
    natural_width: Pixels,
    layout_dirty: bool,
    manual_size: Option<Size<Pixels>>,
    geometry: Geometry,
    drag: Option<PopupDrag>,
    preview: Option<Arc<dyn PreviewBackend>>,
    _preview_task: Task<()>,
}

impl ShowUsagesPopup {
    pub(crate) fn new(session: UsagesSession, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = crate::saved_state(cx);
        let preview = state
            .preview
            .then(|| picker_preview::editor_preview(session.project.clone(), window, cx));
        let manual_size = state.popup_size.map(|(width, height)| Size {
            width: px(width),
            height: px(height),
        });
        Self {
            position: session.popup_position,
            session,
            state,
            model: None,
            focus_handle: cx.focus_handle(),
            selected: None,
            scroll_handle: UniformListScrollHandle::new(),
            query: String::new(),
            speed_search_matches: Vec::new(),
            columns: ColumnWidths::default(),
            natural_width: Pixels::ZERO,
            layout_dirty: true,
            manual_size,
            geometry: Geometry::default(),
            drag: None,
            preview,
            _preview_task: Task::ready(()),
        }
    }

    pub(crate) fn set_model(
        &mut self,
        mut model: UsagesModel<Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if model.filters() != self.state.filters {
            model.set_filters(self.state.filters);
        }
        self.selected = model.initial_selection();
        self.model = Some(model);
        self.recompute_speed_search();
        if let Some(index) = self.match_at_or_after_selection() {
            self.selected = Some(index);
        }
        self.after_model_change(window, cx);
    }

    pub(crate) fn set_accesses(
        &mut self,
        accesses: Vec<(usize, UsageAccess)>,
        cx: &mut Context<Self>,
    ) {
        let identity = self.selected_identity();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        model.set_accesses(accesses);
        self.restore_selection(identity);
        self.recompute_speed_search();
        self.layout_dirty = true;
        cx.notify();
    }

    pub(crate) fn set_symbol(&mut self, symbol: SymbolDescription, cx: &mut Context<Self>) {
        self.session.symbol = symbol;
        self.layout_dirty = true;
        cx.notify();
    }

    fn footer_reserved(&self) -> bool {
        self.model
            .as_ref()
            .is_some_and(|model| model.shown_count() > 0)
    }

    fn footer_text(&self, cx: &App) -> Option<SharedString> {
        let model = self.model.as_ref()?;
        let location = model.usage_row_at(self.selected?)?.locations.first()?;
        let file = location.buffer.read(cx).file()?;
        let path = crate::collect::absolute_path(file.as_ref(), cx);
        Some(SharedString::from(footer_path(
            &path,
            util::paths::home_dir(),
        )))
    }

    #[cfg(test)]
    pub(crate) fn session(&self) -> &UsagesSession {
        &self.session
    }

    pub(crate) fn is_searching(&self) -> bool {
        self.model.is_none()
    }

    fn display_count(&self) -> usize {
        self.model
            .as_ref()
            .map_or(1, |model| model.display_rows().len())
    }

    fn preview_shown(&self) -> bool {
        self.state.preview && self.preview.is_some()
    }

    fn after_model_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_speed_search();
        self.layout_dirty = true;
        if let Some(index) = self.selected {
            self.scroll_handle
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
        self.schedule_preview_update(window, cx);
        cx.notify();
    }

    fn selected_identity(&self) -> Option<(SharedString, u32)> {
        let model = self.model.as_ref()?;
        let row = model.usage_row_at(self.selected?)?;
        Some((row.file_path.clone(), row.row))
    }

    fn restore_selection(&mut self, identity: Option<(SharedString, u32)>) {
        let Some(model) = self.model.as_ref() else {
            self.selected = None;
            return;
        };
        let count = model.display_rows().len();
        let had_identity = identity.is_some();
        let found = identity.and_then(|(file_path, line)| {
            (0..count).find(|index| {
                model
                    .usage_row_at(*index)
                    .is_some_and(|row| row.file_path == file_path && row.row == line)
            })
        });
        let kept = self
            .selected
            .filter(|index| !had_identity && *index < count);
        self.selected = found.or(kept).or_else(|| model.initial_selection());
    }

    fn recompute_speed_search(&mut self) {
        self.speed_search_matches = match &self.model {
            Some(model) if !self.query.is_empty() => {
                model.speed_search(&self.query, self.state.short_file_path)
            }
            _ => Vec::new(),
        };
    }

    fn speed_search_match(&self, display_index: usize) -> Option<&SpeedSearchMatch> {
        let position = self
            .speed_search_matches
            .binary_search_by_key(&display_index, |speed_search_match| {
                speed_search_match.display_index
            })
            .ok()?;
        self.speed_search_matches.get(position)
    }

    fn match_at_or_after_selection(&self) -> Option<usize> {
        let start = self.selected.unwrap_or(0);
        self.speed_search_matches
            .iter()
            .map(|speed_search_match| speed_search_match.display_index)
            .find(|index| *index >= start)
            .or_else(|| {
                self.speed_search_matches
                    .first()
                    .map(|speed_search_match| speed_search_match.display_index)
            })
    }

    fn adjacent_match(&self, forward: bool) -> Option<usize> {
        let current = self.selected;
        let mut indices = self
            .speed_search_matches
            .iter()
            .map(|speed_search_match| speed_search_match.display_index);
        if forward {
            indices
                .clone()
                .find(|index| current.is_none_or(|selected| *index > selected))
                .or_else(|| indices.next())
        } else {
            indices
                .clone()
                .rev()
                .find(|index| current.is_none_or(|selected| *index < selected))
                .or_else(|| indices.next_back())
        }
    }

    fn wrapped_neighbor(&self, forward: bool) -> Option<usize> {
        let count = self.display_count();
        let last = count.checked_sub(1)?;
        let neighbor = match self.selected.filter(|index| *index <= last) {
            None if forward => 0,
            None => last,
            Some(index) if forward => {
                if index == last {
                    0
                } else {
                    index + 1
                }
            }
            Some(index) => {
                if index == 0 {
                    last
                } else {
                    index - 1
                }
            }
        };
        Some(neighbor)
    }

    fn select_and_reveal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(index);
        self.scroll_handle
            .scroll_to_item(index, ScrollStrategy::Nearest);
        self.schedule_preview_update(window, cx);
        cx.notify();
    }

    fn move_selection(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_none() {
            return;
        }
        let target = if self.query.is_empty() {
            self.wrapped_neighbor(forward)
        } else {
            self.adjacent_match(forward)
        };
        if let Some(index) = target {
            self.select_and_reveal(index, window, cx);
        }
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(true, window, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(false, window, cx);
    }

    fn select_first(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_none() {
            return;
        }
        let target = if self.query.is_empty() {
            (self.display_count() > 0).then_some(0)
        } else {
            self.speed_search_matches
                .first()
                .map(|speed_search_match| speed_search_match.display_index)
        };
        if let Some(index) = target {
            self.select_and_reveal(index, window, cx);
        }
    }

    fn select_last(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_none() {
            return;
        }
        let target = if self.query.is_empty() {
            self.display_count().checked_sub(1)
        } else {
            self.speed_search_matches
                .last()
                .map(|speed_search_match| speed_search_match.display_index)
        };
        if let Some(index) = target {
            self.select_and_reveal(index, window, cx);
        }
    }

    fn page_size(&self) -> usize {
        ((self.geometry.list_height / ROW_HEIGHT).floor() as usize).max(1)
    }

    fn select_page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_none() || self.display_count() == 0 {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let target = current.saturating_sub(self.page_size());
        self.select_and_reveal(target, window, cx);
    }

    fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.is_none() {
            return;
        }
        let Some(last) = self.display_count().checked_sub(1) else {
            return;
        };
        let current = self.selected.unwrap_or(0);
        let target = current.saturating_add(self.page_size()).min(last);
        self.select_and_reveal(target, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected {
            self.activate(index, window, cx);
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let Some(display_row) = model.display_rows().get(index).copied() else {
            return;
        };
        match display_row {
            DisplayRow::Usage(row_index) => {
                let Some(location) = model
                    .row(row_index)
                    .and_then(|row| row.locations.first().cloned())
                else {
                    return;
                };
                cx.emit(DismissEvent);
                crate::navigate_to_usage(&self.session, location, window, cx);
            }
            DisplayRow::MoreUsages => {
                model.show_more();
                self.after_model_change(window, cx);
            }
            DisplayRow::OutOfScope(_) => {
                crate::restart(self.session.clone(), UsageScope::AllPlaces, window, cx);
            }
            DisplayRow::FilteredOut(_) => {
                self.state.filters = UsageFilters::default();
                crate::save_state(&self.state, cx);
                crate::restart(self.session.clone(), self.session.scope, window, cx);
            }
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            cx.emit(DismissEvent);
            return;
        }
        self.query.clear();
        self.speed_search_matches.clear();
        cx.notify();
    }

    fn show_usages_again(&mut self, _: &ShowUsages, window: &mut Window, cx: &mut Context<Self>) {
        crate::restart(self.session.clone(), UsageScope::maximal(), window, cx);
    }

    fn toggle_filter(&mut self, toggle: FilterToggle, window: &mut Window, cx: &mut Context<Self>) {
        let identity = self.selected_identity();
        toggle.flip(&mut self.state.filters);
        let filters = self.state.filters;
        if let Some(model) = self.model.as_mut() {
            model.set_filters(filters);
        }
        crate::save_state(&self.state, cx);
        self.restore_selection(identity);
        self.after_model_change(window, cx);
    }

    fn toggle_read_access(
        &mut self,
        _: &ToggleReadAccess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_filter(FilterToggle::ReadAccess, window, cx);
    }

    fn toggle_write_access(
        &mut self,
        _: &ToggleWriteAccess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_filter(FilterToggle::WriteAccess, window, cx);
    }

    fn toggle_imports(&mut self, _: &ToggleImports, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_filter(FilterToggle::Imports, window, cx);
    }

    fn toggle_comments(&mut self, _: &ToggleComments, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_filter(FilterToggle::Comments, window, cx);
    }

    fn toggle_preview(&mut self, _: &TogglePreview, window: &mut Window, cx: &mut Context<Self>) {
        self.state.preview = !self.state.preview;
        self.state.popup_size = None;
        self.manual_size = None;
        if self.state.preview {
            self.preview = Some(picker_preview::editor_preview(
                self.session.project.clone(),
                window,
                cx,
            ));
        } else {
            self.preview = None;
            self._preview_task = Task::ready(());
        }
        crate::save_state(&self.state, cx);
        self.layout_dirty = true;
        self.schedule_preview_update(window, cx);
        cx.notify();
    }

    fn toggle_short_file_path(&mut self, cx: &mut Context<Self>) {
        self.state.short_file_path = !self.state.short_file_path;
        crate::save_state(&self.state, cx);
        self.recompute_speed_search();
        self.layout_dirty = true;
        cx.notify();
    }

    fn toggle_file_structure(&mut self, cx: &mut Context<Self>) {
        self.state.file_structure = !self.state.file_structure;
        crate::save_state(&self.state, cx);
        self.layout_dirty = true;
        cx.notify();
    }

    fn choose_scope(&mut self, scope: UsageScope, window: &mut Window, cx: &mut Context<Self>) {
        self.state.scope = scope;
        crate::save_state(&self.state, cx);
        crate::restart(self.session.clone(), scope, window, cx);
    }

    fn open_in_find_window(
        &mut self,
        _: &OpenInFindWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let locations = model.visible_locations();
        cx.emit(DismissEvent);
        crate::open_in_find_window(&self.session, locations, window, cx);
    }

    fn show_settings(&mut self, _: &ShowSettings, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
        crate::show_find_usages_dialog(self.session.clone(), window, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        if keystroke.key == "backspace" {
            if self.query.pop().is_some() {
                cx.stop_propagation();
                self.update_speed_search(window, cx);
            }
            return;
        }
        let Some(text) = keystroke.key_char.as_ref() else {
            return;
        };
        if text.is_empty() || text.chars().any(char::is_control) {
            return;
        }
        cx.stop_propagation();
        self.query.push_str(text);
        self.update_speed_search(window, cx);
    }

    fn update_speed_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_speed_search();
        match self.match_at_or_after_selection() {
            Some(index) => self.select_and_reveal(index, window, cx),
            None => cx.notify(),
        }
    }

    fn hover_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.model.is_none() || self.state.preview || self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        cx.notify();
    }

    fn click_row(
        &mut self,
        index: usize,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.is_none() {
            return;
        }
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.schedule_preview_update(window, cx);
            cx.notify();
        }
        if !self.state.preview || click_count >= 2 {
            self.activate(index, window, cx);
        }
    }

    fn selected_location(&self) -> Option<Location> {
        let model = self.model.as_ref()?;
        model
            .usage_row_at(self.selected?)?
            .locations
            .first()
            .cloned()
    }

    fn schedule_preview_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preview_shown() {
            return;
        }
        self._preview_task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
            this.update_in(cx, |this, window, cx| this.update_preview(window, cx))
                .log_err();
        });
    }

    fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.clone() else {
            return;
        };
        match self.selected_location() {
            Some(location) => {
                let snapshot = location.buffer.read(cx).text_snapshot();
                let range = location.range.start.to_offset(&snapshot)
                    ..location.range.end.to_offset(&snapshot);
                preview.update(
                    PreviewUpdate::from_buffer(
                        location.buffer,
                        MatchLocation {
                            anchor_range: location.range,
                            range,
                        },
                    ),
                    window,
                    cx,
                );
            }
            None => preview.clear(cx),
        }
        cx.notify();
    }

    fn begin_drag(&mut self, kind: DragKind, mouse: Point<Pixels>) {
        self.drag = Some(PopupDrag {
            kind,
            start_mouse: mouse,
            start_bounds: None,
            start_geometry: self.geometry,
        });
    }

    fn drag_move(
        &mut self,
        event: &DragMoveEvent<DragHandle>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let start_bounds = *drag.start_bounds.get_or_insert(event.bounds);
        let drag = *drag;
        let delta = event.event.position - drag.start_mouse;
        match drag.kind {
            DragKind::Move => {
                self.position = start_bounds.origin + delta;
            }
            DragKind::Resize => {
                let minimum_height = self.minimum_height(cx);
                self.manual_size = Some(Size {
                    width: (start_bounds.size.width + delta.x).max(MIN_POPUP_WIDTH),
                    height: (start_bounds.size.height + delta.y).max(minimum_height),
                });
            }
            DragKind::Divider => {
                let split = drag.start_geometry.list_height + drag.start_geometry.preview_height;
                if split <= Pixels::ZERO {
                    return;
                }
                let maximum_list_height = (split - min_preview_height(cx)).max(ROW_HEIGHT);
                let list_height = (drag.start_geometry.list_height + delta.y)
                    .max(ROW_HEIGHT)
                    .min(maximum_list_height);
                self.state.preview_ratio = preview_ratio(list_height / split);
                if self.manual_size.is_none() {
                    self.manual_size = Some(start_bounds.size);
                }
            }
        }
        cx.notify();
    }

    fn end_drag(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.start_bounds.is_none() {
            return;
        }
        match drag.kind {
            DragKind::Move => {}
            DragKind::Resize => {
                let geometry = self.compute_geometry(window, cx);
                self.manual_size = Some(Size {
                    width: geometry.width,
                    height: geometry.height,
                });
                self.state.popup_size = Some((geometry.width.as_f32(), geometry.height.as_f32()));
                crate::save_state(&self.state, cx);
                self.schedule_preview_update(window, cx);
            }
            DragKind::Divider => {
                crate::save_state(&self.state, cx);
                self.schedule_preview_update(window, cx);
            }
        }
        cx.notify();
    }

    fn minimum_height(&self, cx: &App) -> Pixels {
        let preview = if self.preview_shown() {
            DIVIDER_HEIGHT + min_preview_height(cx)
        } else {
            Pixels::ZERO
        };
        self.chrome_height(cx) + ROW_HEIGHT + preview
    }

    fn chrome_height(&self, cx: &App) -> Pixels {
        let footer = if self.footer_reserved() {
            footer_height(cx)
        } else {
            Pixels::ZERO
        };
        HEADER_HEIGHT + TOOLBAR_HEIGHT + BORDER_WIDTH * 2. + footer
    }

    fn compute_geometry(&self, window: &Window, cx: &App) -> Geometry {
        let viewport = window.viewport_size();
        let maximum_width = (viewport.width - WINDOW_MARGIN * 2.).max(MIN_POPUP_WIDTH);
        let maximum_height = (viewport.height - WINDOW_MARGIN * 2.).max(ROW_HEIGHT);
        let chrome = self.chrome_height(cx);
        let preview_shown = self.preview_shown();
        let divider = if preview_shown {
            DIVIDER_HEIGHT
        } else {
            Pixels::ZERO
        };
        let minimum_preview = if preview_shown {
            min_preview_height(cx)
        } else {
            Pixels::ZERO
        };
        let ratio = preview_ratio(self.state.preview_ratio);

        if let Some(size) = self.manual_size {
            let width = size.width.min(maximum_width).max(MIN_POPUP_WIDTH);
            let height = size.height.min(maximum_height).max(self.minimum_height(cx));
            let area = height - chrome - divider;
            let (list_height, preview_height) = if preview_shown {
                let list_height = (area * ratio).min(area - minimum_preview).max(ROW_HEIGHT);
                (list_height, (area - list_height).max(Pixels::ZERO))
            } else {
                (area, Pixels::ZERO)
            };
            return Geometry {
                width,
                height,
                list_height,
                preview_height,
            };
        }

        let width = self.natural_width.min(maximum_width).max(MIN_POPUP_WIDTH);
        let row_count = self.display_count().max(1);
        let row_limit = if preview_shown {
            MAX_VISIBLE_ROWS_WITH_PREVIEW
        } else {
            MAX_VISIBLE_ROWS
        };
        let visible_rows = row_count.min(row_limit);
        let bottom_space = if visible_rows == row_count {
            LIST_BOTTOM_SPACE
        } else {
            Pixels::ZERO
        };
        let mut list_height = ROW_HEIGHT * visible_rows + bottom_space;
        let mut preview_height = if preview_shown {
            (list_height * ((1. - ratio) / ratio)).max(minimum_preview)
        } else {
            Pixels::ZERO
        };
        let available = maximum_height - chrome - divider;
        if list_height + preview_height > available {
            let list_budget = if preview_shown {
                (available * ratio).min(available - minimum_preview)
            } else {
                available
            };
            let fitting_rows = ((list_budget / ROW_HEIGHT).floor() as usize).clamp(1, visible_rows);
            list_height = ROW_HEIGHT * fitting_rows;
            if preview_shown {
                preview_height = (available - list_height).max(Pixels::ZERO);
            }
        }
        Geometry {
            width,
            height: chrome + list_height + divider + preview_height,
            list_height,
            preview_height,
        }
    }

    fn measure(&mut self, window: &Window, cx: &App) {
        self.layout_dirty = false;
        let style = RowStyle::new(window, cx);
        let columns = self.measure_columns(&style, cx.theme().syntax(), window);
        let table_width =
            ROW_INSET * 2. + ROW_PADDING * 2. + columns.file + columns.line + columns.code;
        let content_width = table_width
            .max(self.measure_header(&style, window))
            .max(self.measure_toolbar(&style, window));
        self.columns = columns;
        self.natural_width = self.natural_width.max(content_width + BORDER_WIDTH * 2.);
    }

    fn measure_columns(
        &self,
        style: &RowStyle,
        syntax: &SyntaxTheme,
        window: &Window,
    ) -> ColumnWidths {
        let Some(model) = self.model.as_ref() else {
            return ColumnWidths {
                code: line_width(window, &searching_line(style), style.font_size)
                    .min(MAX_COLUMN_WIDTH),
                ..ColumnWidths::default()
            };
        };
        let file_icon_width = if style.file_icons {
            style.icon_width + FILE_ICON_GAP
        } else {
            Pixels::ZERO
        };
        let origin_width = ORIGIN_GAP
            + ORIGIN_PADDING * 2.
            + label_width(
                window,
                CURRENT_USAGE_LABEL,
                style.small_label_font_size,
                &style.base,
            );
        let mut columns = ColumnWidths::default();
        for display_row in model.display_rows() {
            match *display_row {
                DisplayRow::Usage(row_index) => {
                    let Some(row) = model.row(row_index) else {
                        continue;
                    };
                    let file_label = file_label_line(model, row, &self.state, style, &[]);
                    let line_number = line_number_line(row, style);
                    let code = code_line(row, style, syntax, &[]);
                    let origin = if row.is_origin {
                        origin_width
                    } else {
                        Pixels::ZERO
                    };
                    columns.file = columns
                        .file
                        .max(file_icon_width + line_width(window, &file_label, style.font_size));
                    columns.line = columns.line.max(
                        LINE_NUMBER_PADDING * 2.
                            + line_width(window, &line_number, style.font_size),
                    );
                    columns.code = columns.code.max(
                        USAGE_ICON_SLOT
                            + USAGE_ICON_GAP
                            + line_width(window, &code, style.font_size)
                            + origin,
                    );
                }
                special => {
                    if let Some(line) = special_line(special, model.scope(), style) {
                        columns.code = columns.code.max(line_width(window, &line, style.font_size));
                    }
                }
            }
        }
        ColumnWidths {
            file: columns.file.min(MAX_COLUMN_WIDTH),
            line: columns.line.min(MAX_COLUMN_WIDTH),
            code: columns.code.min(MAX_COLUMN_WIDTH),
        }
    }

    fn measure_header(&self, style: &RowStyle, window: &Window) -> Pixels {
        let title = line_width(window, &self.title_line(style), style.label_font_size);
        let status = self.model.as_ref().map_or(Pixels::ZERO, |model| {
            label_width(
                window,
                &model.status_text(self.is_searching()),
                style.label_font_size,
                &style.base,
            )
        });
        let spinner = if self.is_searching() {
            SPINNER_GAP + style.icon_width
        } else {
            Pixels::ZERO
        };
        HEADER_PADDING_LEFT
            + HEADER_PADDING_RIGHT
            + title
            + spinner
            + HEADER_TITLE_GAP
            + status
            + HEADER_STATUS_GAP
            + TOOLBAR_BUTTON_SIZE
    }

    fn measure_toolbar(&self, style: &RowStyle, window: &Window) -> Pixels {
        let rem_size = window.rem_size();
        let chevron = IconSize::XSmall.rems().to_pixels(rem_size);
        let indicator = IconSize::Indicator.rems().to_pixels(rem_size);
        let options = BUTTON_EDGE * 2.
            + OPTIONS_BUTTON_PADDING * 2.
            + style.icon_width
            + OPTIONS_BUTTON_GAP
            + indicator;
        let scope_label = label_width(
            window,
            self.session.scope.display_name(),
            style.label_font_size,
            &style.base,
        );
        let scope =
            BUTTON_EDGE * 2. + SCOPE_BUTTON_PADDING * 2. + scope_label + SCOPE_BUTTON_GAP + chevron;
        let separator = SEPARATOR_WIDTH + SEPARATOR_MARGIN * 2.;
        let usages_label = label_width(window, USAGES_LABEL, style.label_font_size, &style.base);
        (TOOLBAR_LINE_INSET + TOOLBAR_PADDING) * 2.
            + TOOLBAR_BUTTON_SIZE * TOOLBAR_ICON_BUTTONS
            + options
            + separator
            + scope
            + TOOLBAR_SPACER
            + usages_label
            + SETTINGS_BUTTON_GAP
            + TOOLBAR_GAP * TOOLBAR_GAPS
    }

    fn title_line(&self, style: &RowStyle) -> StyledLine {
        match self.session.title() {
            UsagesTitle::Symbol(title) => {
                let mut text = title.emphasized.to_string();
                let mut highlights = vec![(0..text.len(), bold())];
                if let Some(location) = title.location {
                    text.push(' ');
                    let location_start = text.len();
                    text.push_str(&location);
                    highlights.push((location_start..text.len(), foreground(style.muted)));
                }
                styled_line(text.into(), &style.base, highlights)
            }
            UsagesTitle::Implementations(text) => {
                let length = text.len();
                styled_line(text, &style.base, vec![(0..length, medium())])
            }
        }
    }

    fn render_header(
        &self,
        style: &RowStyle,
        border_color: Hsla,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let status = self
            .model
            .as_ref()
            .map(|model| model.status_text(self.is_searching()));
        h_flex()
            .id("show-usages-header")
            .flex_none()
            .w_full()
            .h(HEADER_HEIGHT)
            .pl(HEADER_PADDING_LEFT)
            .pr(HEADER_PADDING_RIGHT)
            .border_b_1()
            .border_color(border_color)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.begin_drag(DragKind::Move, event.position)
                }),
            )
            .on_drag(DragHandle, |_, _, _, cx| cx.new(|_| gpui::Empty))
            .debug_selector(|| "show-usages-header".into())
            .child(
                div()
                    .debug_selector(|| "show-usages-title".into())
                    .min_w_0()
                    .truncate()
                    .text_size(style.label_font_size)
                    .child(self.title_line(style).into_styled_text()),
            )
            .child(div().flex_1().min_w(HEADER_TITLE_GAP))
            .when(self.is_searching(), |this| {
                this.child(
                    div()
                        .debug_selector(|| "show-usages-spinner".into())
                        .flex_none()
                        .mr(SPINNER_GAP)
                        .child(
                            Icon::new(IconName::LoadCircle)
                                .size(IconSize::Small)
                                .color(Color::Muted)
                                .with_rotate_animation(2),
                        ),
                )
            })
            .children(status.map(|status| Label::new(status).color(Color::Muted).single_line()))
            .child(
                div()
                    .debug_selector(|| "show-usages-status-gap".into())
                    .flex_none()
                    .w(HEADER_STATUS_GAP),
            )
            .child(
                toolbar_button(
                    "show-usages-open-in-find-window",
                    ToolbarIcon::Single(IconName::UsagesOpenInFindWindow),
                )
                .tooltip(Tooltip::for_action_title_in(
                    "Open in Find Tool Window",
                    &OpenInFindWindow,
                    &self.focus_handle,
                ))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_in_find_window(&OpenInFindWindow, window, cx)
                })),
            )
    }

    fn render_toolbar(&self, border_color: Hsla, cx: &Context<Self>) -> impl IntoElement {
        let filters = self.state.filters;
        let row = h_flex()
            .debug_selector(|| "show-usages-toolbar-row".into())
            .flex_none()
            .w_full()
            .h(TOOLBAR_ROW_HEIGHT)
            .px(TOOLBAR_PADDING)
            .gap(TOOLBAR_GAP)
            .border_b_1()
            .border_color(border_color)
            .children(
                FilterToggle::ALL
                    .into_iter()
                    .map(|toggle| self.render_filter_toggle(toggle, filters, cx)),
            )
            .child(self.render_show_options(cx))
            .child(
                div()
                    .flex_none()
                    .w(SEPARATOR_WIDTH)
                    .h(SEPARATOR_HEIGHT)
                    .mx(SEPARATOR_MARGIN)
                    .bg(border_color),
            )
            .child(
                toolbar_button(
                    "show-usages-preview",
                    ToolbarIcon::Single(IconName::UsagesPreview),
                )
                .toggle_state(self.state.preview)
                .tooltip(Tooltip::for_action_title_in(
                    "Preview Source",
                    &TogglePreview,
                    &self.focus_handle,
                ))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_preview(&TogglePreview, window, cx)
                })),
            )
            .child(self.render_scope_chooser(cx))
            .child(div().flex_1().min_w(TOOLBAR_SPACER))
            .child(
                h_flex()
                    .flex_none()
                    .gap(SETTINGS_BUTTON_GAP)
                    .child(Label::new(USAGES_LABEL).color(Color::Muted).single_line())
                    .child(
                        toolbar_button(
                            "show-usages-settings",
                            ToolbarIcon::Single(IconName::GeneralSettings),
                        )
                        .tooltip(Tooltip::for_action_title_in(
                            "Settings…",
                            &ShowSettings,
                            &self.focus_handle,
                        ))
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.show_settings(&ShowSettings, window, cx)
                            },
                        )),
                    ),
            );
        // IntelliJ insets the toolbar's separator line from the popup edges and
        // leaves a gap below it (ShowUsagesAction.createComplexPopupToolbarBorder).
        v_flex()
            .debug_selector(|| "show-usages-toolbar".into())
            .flex_none()
            .w_full()
            .h(TOOLBAR_HEIGHT)
            .px(TOOLBAR_LINE_INSET)
            .pb(TOOLBAR_BOTTOM_GAP)
            .child(row)
    }

    fn render_filter_toggle(
        &self,
        toggle: FilterToggle,
        filters: UsageFilters,
        cx: &Context<Self>,
    ) -> ButtonLike {
        toolbar_button(toggle.id(), toggle.icon())
            .toggle_state(toggle.is_shown(filters))
            .tooltip(Tooltip::for_action_title_in(
                toggle.title(),
                toggle.action(),
                &self.focus_handle,
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_filter(toggle, window, cx)
            }))
    }

    fn render_show_options(&self, cx: &Context<Self>) -> impl IntoElement {
        let popup = cx.weak_entity();
        let trigger = ButtonLike::new("show-usages-show-options-button")
            .style(ButtonStyle::Subtle)
            .selected_style(ButtonStyle::Filled)
            .size(ButtonSize::None)
            .height(TOOLBAR_BUTTON_SIZE.into())
            .child(
                h_flex()
                    .px(OPTIONS_BUTTON_PADDING)
                    .gap(OPTIONS_BUTTON_GAP)
                    .child(
                        Icon::new(IconName::UsagesShowOptions)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::Indicator)
                            .color(Color::Muted),
                    ),
            );
        PopoverMenu::new("show-usages-show-options")
            .trigger_with_tooltip(trigger, Tooltip::text("Show Options"))
            .menu(move |window, cx| {
                let (short_file_path, file_structure) = popup.upgrade().map(|popup| {
                    let state = &popup.read(cx).state;
                    (state.short_file_path, state.file_structure)
                })?;
                let short_file_path_popup = popup.clone();
                let file_structure_popup = popup.clone();
                Some(ContextMenu::build(window, cx, move |menu, _, _| {
                    menu.header("Show")
                        .toggleable_entry(
                            "Short File Path",
                            short_file_path,
                            IconPosition::Start,
                            None,
                            move |_, cx| {
                                short_file_path_popup
                                    .update(cx, |this, cx| this.toggle_short_file_path(cx))
                                    .log_err();
                            },
                        )
                        .toggleable_entry(
                            "File Structure",
                            file_structure,
                            IconPosition::Start,
                            None,
                            move |_, cx| {
                                file_structure_popup
                                    .update(cx, |this, cx| this.toggle_file_structure(cx))
                                    .log_err();
                            },
                        )
                }))
            })
    }

    fn render_scope_chooser(&self, cx: &Context<Self>) -> impl IntoElement {
        let popup = cx.weak_entity();
        let current_scope = self.session.scope;
        let trigger = ButtonLike::new("show-usages-scope-button")
            .style(ButtonStyle::Subtle)
            .selected_style(ButtonStyle::Filled)
            .size(ButtonSize::None)
            .height(TOOLBAR_BUTTON_SIZE.into())
            .child(
                h_flex()
                    .px(SCOPE_BUTTON_PADDING)
                    .gap(SCOPE_BUTTON_GAP)
                    .child(Label::new(current_scope.display_name()).single_line())
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        PopoverMenu::new("show-usages-scope")
            .trigger_with_tooltip(trigger, Tooltip::text("Scope"))
            .menu(move |window, cx| {
                let popup = popup.clone();
                Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                    for scope in UsageScope::ALL {
                        let popup = popup.clone();
                        menu = menu.toggleable_entry(
                            scope.display_name(),
                            scope == current_scope,
                            IconPosition::Start,
                            None,
                            move |window, cx| {
                                popup
                                    .update(cx, |this, cx| this.choose_scope(scope, window, cx))
                                    .log_err();
                            },
                        );
                    }
                    menu
                }))
            })
    }

    fn render_list(&self, geometry: Geometry, cx: &Context<Self>) -> impl IntoElement {
        let item_count = self.display_count();
        div()
            .relative()
            .flex_none()
            .w_full()
            .h(geometry.list_height)
            .child(
                uniform_list(
                    "show-usages-list",
                    item_count,
                    cx.processor(|this, range: Range<usize>, window, cx| {
                        this.render_rows(range, window, cx)
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .size_full(),
            )
            .when(!self.query.is_empty(), |this| {
                this.child(self.render_speed_search(cx))
            })
    }

    fn render_speed_search(&self, cx: &App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let found = !self.speed_search_matches.is_empty();
        let border_color = if found {
            colors.border
        } else {
            cx.theme().status().error
        };
        h_flex()
            .absolute()
            .top(SPEED_SEARCH_OFFSET)
            .left(ROW_INSET)
            .gap_1()
            .px_1p5()
            .rounded(SELECTION_RADIUS)
            .border_1()
            .border_color(border_color)
            .bg(colors.elevated_surface_background)
            .child(
                Icon::new(IconName::MagnifyingGlass)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new(self.query.clone())
                    .size(LabelSize::Small)
                    .color(if found { Color::Default } else { Color::Error }),
            )
    }

    fn render_rows(
        &self,
        range: Range<usize>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let style = RowStyle::new(window, cx);
        let syntax = cx.theme().syntax().clone();
        range
            .map(|index| self.render_row(index, &style, &syntax, cx))
            .collect()
    }

    fn render_row(
        &self,
        index: usize,
        style: &RowStyle,
        syntax: &SyntaxTheme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let display_row = self
            .model
            .as_ref()
            .and_then(|model| model.display_rows().get(index).copied());
        let content = match (&self.model, display_row) {
            (None, _) => special_cell(searching_line(style)),
            (Some(model), Some(DisplayRow::Usage(row_index))) => match model.row(row_index) {
                Some(row) => self.render_usage_cells(model, row, index, style, syntax, cx),
                None => div().into_any_element(),
            },
            (Some(model), Some(special)) => match special_line(special, model.scope(), style) {
                Some(line) => special_cell(line),
                None => div().into_any_element(),
            },
            (Some(_), None) => div().into_any_element(),
        };
        let selected = self.selected == Some(index);
        let filtered_out = matches!(display_row, Some(DisplayRow::FilteredOut(_)));
        div()
            .id(("show-usages-row", index))
            .w_full()
            .h(ROW_HEIGHT)
            .px(ROW_INSET)
            .on_mouse_move(
                cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.hover_row(index, cx)),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_row(index, event.click_count(), window, cx)
            }))
            .when(filtered_out, |this| {
                this.tooltip(Tooltip::text(FILTERED_OUT_TOOLTIP))
            })
            .child(
                h_flex()
                    .size_full()
                    .min_w_0()
                    .px(ROW_PADDING)
                    .rounded(SELECTION_RADIUS)
                    .overflow_hidden()
                    .text_size(style.font_size)
                    .when(selected, |this| this.bg(style.selection_background))
                    .child(content),
            )
            .into_any_element()
    }

    fn render_usage_cells(
        &self,
        model: &UsagesModel<Location>,
        row: &UsageRow<Location>,
        display_index: usize,
        style: &RowStyle,
        syntax: &SyntaxTheme,
        cx: &App,
    ) -> AnyElement {
        let speed_search = self.speed_search_match(display_index);
        let file_label_ranges = speed_search.map_or(&[][..], |speed_search_match| {
            speed_search_match.file_label_ranges.as_slice()
        });
        let text_ranges = speed_search.map_or(&[][..], |speed_search_match| {
            speed_search_match.text_ranges.as_slice()
        });
        let file_label = file_label_line(model, row, &self.state, style, file_label_ranges);
        let line_number = line_number_line(row, style);
        let code = code_line(row, style, syntax, text_ranges);
        h_flex()
            .size_full()
            .min_w_0()
            .child(
                h_flex()
                    .flex_none()
                    .w(self.columns.file)
                    .min_w_0()
                    .overflow_hidden()
                    .gap(FILE_ICON_GAP)
                    .children(file_icon_slot(&row.file_path, style, cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(file_label.into_styled_text()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(self.columns.line)
                    .pl(LINE_NUMBER_PADDING)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(line_number.into_styled_text()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .child(usage_icon(row.access))
                    .child(div().flex_none().w(USAGE_ICON_GAP))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(code.into_styled_text()),
                    )
                    .when(row.is_origin, |this| this.child(origin_pill(style))),
            )
            .into_any_element()
    }

    fn render_footer(&self, border_color: Hsla, cx: &App) -> Option<impl IntoElement> {
        if !self.footer_reserved() {
            return None;
        }
        let font_size = footer_font_size(cx);
        let muted = cx.theme().colors().text_muted;
        Some(div().flex_none().w_full().h(footer_height(cx)).when_some(
            self.footer_text(cx),
            |this, path| {
                this.border_t_1()
                    .border_color(border_color)
                    .px(FOOTER_PADDING_X)
                    .py(FOOTER_PADDING_Y)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(font_size)
                            .line_height(font_size)
                            .text_color(muted)
                            .child(path),
                    )
            },
        ))
    }

    fn render_divider(&self, border_color: Hsla, cx: &Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex_none()
            .w_full()
            .h(DIVIDER_HEIGHT)
            .bg(border_color)
            .child(
                div()
                    .id("show-usages-preview-divider")
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(DIVIDER_GRIP_OFFSET)
                    .h(DIVIDER_GRIP_HEIGHT)
                    .cursor(CursorStyle::ResizeRow)
                    .block_mouse_except_scroll()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.begin_drag(DragKind::Divider, event.position)
                        }),
                    )
                    .on_drag(DragHandle, |_, _, _, cx| cx.new(|_| gpui::Empty)),
            )
    }

    fn render_resize_handle(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("show-usages-resize-handle")
            .absolute()
            .right_0()
            .bottom_0()
            .w(RESIZE_HANDLE_SIZE)
            .h(RESIZE_HANDLE_SIZE)
            .cursor(CursorStyle::ResizeUpLeftDownRight)
            .block_mouse_except_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.begin_drag(DragKind::Resize, event.position)
                }),
            )
            .on_drag(DragHandle, |_, _, _, cx| cx.new(|_| gpui::Empty))
    }
}

impl EventEmitter<DismissEvent> for ShowUsagesPopup {}

impl Focusable for ShowUsagesPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for ShowUsagesPopup {
    fn anchor_position(&self) -> Option<Point<Pixels>> {
        Some(self.position)
    }
}

impl Render for ShowUsagesPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.layout_dirty {
            self.measure(window, cx);
        }
        let geometry = self.compute_geometry(window, cx);
        self.geometry = geometry;
        let border_color = cx.theme().colors().border_variant;
        let style = RowStyle::new(window, cx);
        let preview_element = self
            .preview
            .clone()
            .filter(|_| self.state.preview)
            .map(|preview| preview.render(PreviewLayout::Below, cx));
        v_flex()
            .id("show-usages-popup")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::show_usages_again))
            .on_action(cx.listener(Self::toggle_read_access))
            .on_action(cx.listener(Self::toggle_write_access))
            .on_action(cx.listener(Self::toggle_imports))
            .on_action(cx.listener(Self::toggle_comments))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::open_in_find_window))
            .on_action(cx.listener(Self::show_settings))
            .on_key_down(cx.listener(Self::key_down))
            .on_drag_move::<DragHandle>(cx.listener(Self::drag_move))
            .capture_any_mouse_up(cx.listener(Self::end_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_drag))
            .relative()
            .w(geometry.width)
            .h(geometry.height)
            .elevation_2(cx)
            .overflow_hidden()
            .child(self.render_header(&style, border_color, cx))
            .child(self.render_toolbar(border_color, cx))
            .child(self.render_list(geometry, cx))
            .when_some(preview_element, |this, preview_element| {
                this.child(self.render_divider(border_color, cx)).child(
                    div()
                        .flex_none()
                        .w_full()
                        .h(geometry.preview_height)
                        .overflow_hidden()
                        .child(preview_element),
                )
            })
            .children(self.render_footer(border_color, cx))
            .child(self.render_resize_handle(cx))
    }
}

fn footer_font_size(cx: &App) -> Pixels {
    (ThemeSettings::get_global(cx).ui_font_size(cx) - FOOTER_FONT_SIZE_OFFSET)
        .max(MIN_FOOTER_FONT_SIZE)
}

fn footer_height(cx: &App) -> Pixels {
    BORDER_WIDTH + FOOTER_PADDING_Y * 2. + footer_font_size(cx)
}

fn min_preview_height(cx: &App) -> Pixels {
    let settings = ThemeSettings::get_global(cx);
    settings.buffer_font_size(cx) * settings.line_height() * MIN_PREVIEW_LINES
}

fn preview_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_PREVIEW_RATIO, MAX_PREVIEW_RATIO)
    } else {
        DEFAULT_PREVIEW_RATIO
    }
}

fn toolbar_button(id: impl Into<ElementId>, icon: ToolbarIcon) -> ButtonLike {
    ButtonLike::new(id)
        .style(ButtonStyle::Subtle)
        .selected_style(ButtonStyle::Filled)
        .size(ButtonSize::None)
        .width(TOOLBAR_BUTTON_SIZE)
        .height(TOOLBAR_BUTTON_SIZE.into())
        .child(toolbar_icon(icon))
}

fn toolbar_icon(icon: ToolbarIcon) -> AnyElement {
    match icon {
        ToolbarIcon::Single(name) => Icon::new(name)
            .size(IconSize::Small)
            .color(Color::Muted)
            .into_any_element(),
        ToolbarIcon::Layered {
            base,
            accent,
            accent_color,
        } => layered_icon(base, accent, accent_color).into_any_element(),
    }
}

fn layered_icon(base: IconName, accent: IconName, accent_color: u32) -> Div {
    let size = IconSize::Small;
    div()
        .relative()
        .flex_none()
        .w(size.rems())
        .h(size.rems())
        .child(Icon::new(base).size(size).color(Color::Muted))
        .child(
            div().absolute().top_0().left_0().child(
                Icon::new(accent)
                    .size(size)
                    .color(Color::Custom(gpui::rgb(accent_color).into())),
            ),
        )
}

fn usage_icon(access: UsageAccess) -> Div {
    let layers = match access {
        UsageAccess::Read => Some((
            IconName::UsageReadAccess,
            IconName::UsageReadAccessAccent,
            READ_ACCENT,
        )),
        UsageAccess::Write | UsageAccess::ReadWrite => Some((
            IconName::UsageWriteAccess,
            IconName::UsageWriteAccessAccent,
            WRITE_ACCENT,
        )),
        UsageAccess::Unknown => None,
    };
    h_flex()
        .flex_none()
        .w(USAGE_ICON_SLOT)
        .justify_center()
        .children(
            layers.map(|(base, accent, accent_color)| layered_icon(base, accent, accent_color)),
        )
}

fn file_icon_slot(file_path: &str, style: &RowStyle, cx: &App) -> Option<Div> {
    if !style.file_icons {
        return None;
    }
    let icon = FileIcons::get_icon(Path::new(file_path), cx).map(|icon| {
        Icon::from_path(icon)
            .size(IconSize::Small)
            .color(Color::Muted)
    });
    Some(h_flex().flex_none().w(style.icon_width).children(icon))
}

fn origin_pill(style: &RowStyle) -> Div {
    div()
        .flex_none()
        .ml(ORIGIN_GAP)
        .px(ORIGIN_PADDING)
        .rounded(SELECTION_RADIUS)
        .bg(style.pill_background)
        .child(
            Label::new(CURRENT_USAGE_LABEL)
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
}

fn special_cell(line: StyledLine) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .child(line.into_styled_text())
        .into_any_element()
}

fn bold() -> HighlightStyle {
    HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        ..HighlightStyle::default()
    }
}

fn medium() -> HighlightStyle {
    HighlightStyle {
        font_weight: Some(FontWeight::MEDIUM),
        ..HighlightStyle::default()
    }
}

fn background(color: Hsla) -> HighlightStyle {
    HighlightStyle {
        background_color: Some(color),
        ..HighlightStyle::default()
    }
}

fn foreground(color: Hsla) -> HighlightStyle {
    HighlightStyle {
        color: Some(color),
        ..HighlightStyle::default()
    }
}

fn text_runs(
    text: &str,
    base: &TextStyle,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<TextRun> {
    let valid_highlights = highlights.into_iter().filter(|(range, _)| {
        range.start < range.end
            && range.end <= text.len()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
    });
    let mut runs = Vec::new();
    let mut offset = 0;
    for (range, highlight) in gpui::combine_highlights(
        valid_highlights,
        std::iter::empty::<(Range<usize>, HighlightStyle)>(),
    ) {
        if offset < range.start {
            runs.push(base.to_run(range.start - offset));
        }
        runs.push(base.clone().highlight(highlight).to_run(range.len()));
        offset = range.end;
    }
    if offset < text.len() {
        runs.push(base.to_run(text.len() - offset));
    }
    runs
}

fn styled_line(
    text: SharedString,
    base: &TextStyle,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> StyledLine {
    let runs = text_runs(&text, base, highlights);
    StyledLine { text, runs }
}

fn file_label_line(
    model: &UsagesModel<Location>,
    row: &UsageRow<Location>,
    state: &ShowUsagesState,
    style: &RowStyle,
    search_ranges: &[Range<usize>],
) -> StyledLine {
    let label = model.file_label(row, state.short_file_path);
    let mut text = label.to_string();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = search_ranges
        .iter()
        .filter(|range| range.end <= label.len())
        .map(|range| (range.clone(), background(style.search_background)))
        .collect();
    if state.file_structure && !row.structure.is_empty() {
        let start = text.len();
        text.push_str(STRUCTURE_PREFIX);
        let names: Vec<&str> = row.structure.iter().map(SharedString::as_str).collect();
        text.push_str(&names.join(STRUCTURE_SEPARATOR));
        highlights.push((start..text.len(), foreground(style.muted)));
    }
    styled_line(text.into(), &style.base, highlights)
}

fn line_number_line(row: &UsageRow<Location>, style: &RowStyle) -> StyledLine {
    let base = TextStyle {
        color: style.muted,
        ..style.base.clone()
    };
    styled_line(row.line_number_label(), &base, Vec::new())
}

fn code_line(
    row: &UsageRow<Location>,
    style: &RowStyle,
    syntax: &SyntaxTheme,
    search_ranges: &[Range<usize>],
) -> StyledLine {
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = row
        .syntax_highlights
        .iter()
        .filter_map(|(range, highlight_id)| Some((range.clone(), *syntax.get(*highlight_id)?)))
        .collect();
    highlights.extend(row.match_ranges.iter().map(|range| (range.clone(), bold())));
    highlights.extend(
        search_ranges
            .iter()
            .map(|range| (range.clone(), background(style.search_background))),
    );
    styled_line(row.display_text.clone(), &style.base, highlights)
}

fn searching_line(style: &RowStyle) -> StyledLine {
    bold_line(SEARCHING_TEXT.into(), style)
}

fn bold_line(text: SharedString, style: &RowStyle) -> StyledLine {
    let length = text.len();
    styled_line(text, &style.base, vec![(0..length, bold())])
}

fn bracketed_line(message: &str, style: &RowStyle) -> StyledLine {
    let start = SPECIAL_ROW_PREFIX.len();
    let end = start + message.len();
    let text = format!("{SPECIAL_ROW_PREFIX}{message}{SPECIAL_ROW_SUFFIX}");
    styled_line(text.into(), &style.base, vec![(start..end, bold())])
}

fn special_line(
    display_row: DisplayRow,
    scope: UsageScope,
    style: &RowStyle,
) -> Option<StyledLine> {
    match display_row {
        DisplayRow::Usage(_) => None,
        DisplayRow::MoreUsages => Some(bracketed_line(MORE_USAGES_TEXT, style)),
        DisplayRow::OutOfScope(count) => {
            Some(bracketed_line(&out_of_scope_message(count, scope), style))
        }
        DisplayRow::FilteredOut(count) => {
            Some(bold_line(filtered_out_message(count).into(), style))
        }
    }
}

fn line_width(window: &Window, line: &StyledLine, font_size: Pixels) -> Pixels {
    if line.text.is_empty() {
        return Pixels::ZERO;
    }
    window
        .text_system()
        .layout_line(&line.text, font_size, &line.runs, None)
        .width
        .ceil()
}

fn label_width(window: &Window, text: &str, font_size: Pixels, style: &TextStyle) -> Pixels {
    if text.is_empty() {
        return Pixels::ZERO;
    }
    window
        .text_system()
        .layout_line(text, font_size, &[style.to_run(text.len())], None)
        .width
        .ceil()
}

#[cfg(test)]
mod test_support;
