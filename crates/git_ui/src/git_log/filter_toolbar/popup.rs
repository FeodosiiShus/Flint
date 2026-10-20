use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, KeyDownEvent, MouseDownEvent, Pixels, Point, SharedString, Subscription, Window,
    anchored, canvas, deferred, point, px,
};
use ui::{HighlightedLabel, ListItem, POPUP_MENU_METRICS, PopupRowStyle, Tooltip, prelude::*};

use crate::branches_popup::tree::match_query;

pub(super) type PopupHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub(super) type FavoriteHandler = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

const KEY_CONTEXT: &str = "GitLogFilterPopup";
const ADVERTISER_HORIZONTAL_INSET: Pixels = px(12.);
const ADVERTISER_VERTICAL_INSET: Pixels = px(6.);
const MAXIMUM_HEIGHT_FRACTION: f32 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PopupIcon {
    None,
    Favorite(bool),
    Checked(bool),
}

#[derive(Clone)]
pub(super) struct PopupAction {
    text: SharedString,
    tooltip: Option<SharedString>,
    icon: PopupIcon,
    speed_search_only: bool,
    hidden_while_filtering: bool,
    on_select: PopupHandler,
    on_icon_click: Option<FavoriteHandler>,
}

impl PopupAction {
    pub(super) fn new(text: impl Into<SharedString>, on_select: PopupHandler) -> Self {
        Self {
            text: text.into(),
            tooltip: None,
            icon: PopupIcon::None,
            speed_search_only: false,
            hidden_while_filtering: false,
            on_select,
            on_icon_click: None,
        }
    }

    pub(super) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        let tooltip = tooltip.into();
        self.tooltip = (!tooltip.is_empty()).then_some(tooltip);
        self
    }

    pub(super) fn icon(mut self, icon: PopupIcon) -> Self {
        self.icon = icon;
        self
    }

    pub(super) fn speed_search_only(mut self) -> Self {
        self.speed_search_only = true;
        self
    }

    pub(super) fn hidden_while_filtering(mut self, hidden: bool) -> Self {
        self.hidden_while_filtering = hidden;
        self
    }

    pub(super) fn on_icon_click(mut self, handler: FavoriteHandler) -> Self {
        self.on_icon_click = Some(handler);
        self
    }
}

#[derive(Clone)]
pub(super) enum PopupItem {
    Action(PopupAction),
    Separator(Option<SharedString>),
    Submenu {
        text: SharedString,
        items: Rc<Vec<PopupItem>>,
    },
}

impl PopupItem {
    pub(super) fn into_speed_search_only(self) -> Self {
        match self {
            PopupItem::Action(action) => PopupItem::Action(action.speed_search_only()),
            other => other,
        }
    }
}

pub(super) enum FilterPopupEvent {
    Performed,
}

#[derive(Clone)]
enum Row {
    Separator(Option<SharedString>),
    Action { item: usize, highlights: Vec<usize> },
    Submenu { item: usize },
}

impl Row {
    fn is_selectable(&self) -> bool {
        !matches!(self, Row::Separator(_))
    }
}

struct OpenSubmenu {
    row: usize,
    popup: Entity<FilterPopup>,
    position: Point<Pixels>,
    _subscriptions: Vec<Subscription>,
}

pub(super) struct FilterPopup {
    focus_handle: FocusHandle,
    items: Vec<PopupItem>,
    rows: Vec<Row>,
    query: String,
    selected: Option<usize>,
    advertisement: Option<SharedString>,
    minimum_width: Pixels,
    on_reset: Option<PopupHandler>,
    submenu: Option<OpenSubmenu>,
    nested: bool,
    row_bounds: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
    bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
}

impl FilterPopup {
    pub(super) fn new(
        items: Vec<PopupItem>,
        advertisement: Option<SharedString>,
        on_reset: Option<PopupHandler>,
        minimum_width: Pixels,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut popup = Self {
            focus_handle: cx.focus_handle(),
            items,
            rows: Vec::new(),
            query: String::new(),
            selected: None,
            advertisement,
            minimum_width,
            on_reset,
            submenu: None,
            nested: false,
            row_bounds: Rc::new(RefCell::new(Vec::new())),
            bounds: Rc::new(RefCell::new(None)),
        };
        popup.rebuild_rows();
        popup
    }

    fn rebuild_rows(&mut self) {
        let filtering = !self.query.trim().is_empty();
        let mut rows: Vec<Row> = Vec::new();
        let mut pending_separator: Option<Option<SharedString>> = None;
        for (index, item) in self.items.iter().enumerate() {
            let row = match item {
                PopupItem::Separator(text) => {
                    pending_separator = Some(text.clone());
                    continue;
                }
                PopupItem::Action(action) => {
                    let visible = if filtering {
                        !action.hidden_while_filtering
                    } else {
                        !action.speed_search_only
                    };
                    if !visible {
                        continue;
                    }
                    let highlights = if filtering {
                        match match_query(&self.query, &action.text) {
                            Some((_, positions)) => byte_offsets(&action.text, &positions),
                            None => continue,
                        }
                    } else {
                        Vec::new()
                    };
                    Row::Action {
                        item: index,
                        highlights,
                    }
                }
                PopupItem::Submenu { .. } => {
                    if filtering {
                        continue;
                    }
                    Row::Submenu { item: index }
                }
            };
            if let Some(separator) = pending_separator.take()
                && (!rows.is_empty() || separator.is_some())
            {
                rows.push(Row::Separator(separator));
            }
            rows.push(row);
        }
        self.rows = rows;
        self.selected = self.rows.iter().position(Row::is_selectable);
    }

    fn action(&self, item: usize) -> Option<&PopupAction> {
        match self.items.get(item) {
            Some(PopupItem::Action(action)) => Some(action),
            _ => None,
        }
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_selectable())
            .map(|(index, _)| index)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let position = self
            .selected
            .and_then(|selected| selectable.iter().position(|index| *index == selected));
        let next = match (position, forward) {
            (None, true) => 0,
            (None, false) => selectable.len() - 1,
            (Some(position), true) => (position + 1) % selectable.len(),
            (Some(position), false) => (position + selectable.len() - 1) % selectable.len(),
        };
        self.selected = selectable.get(next).copied();
        cx.notify();
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(true, cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(false, cx);
    }

    fn confirm(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.selected {
            self.activate(row, window, cx);
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            cx.emit(DismissEvent);
        } else {
            self.query.clear();
            self.rebuild_rows();
            cx.notify();
        }
    }

    fn select_child(&mut self, _: &menu::SelectChild, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected.and_then(|row| self.rows.get(row)) {
            Some(Row::Submenu { .. }) => {
                if let Some(row) = self.selected {
                    self.open_submenu(row, window, cx);
                }
            }
            _ => cx.propagate(),
        }
    }

    fn select_parent(&mut self, _: &menu::SelectParent, _: &mut Window, cx: &mut Context<Self>) {
        if self.nested {
            cx.emit(DismissEvent);
        } else {
            cx.propagate();
        }
    }

    fn activate(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.get(row).cloned() {
            Some(Row::Action { item, .. }) => {
                let Some(handler) = self.action(item).map(|action| action.on_select.clone()) else {
                    return;
                };
                handler(window, cx);
                cx.emit(FilterPopupEvent::Performed);
                cx.emit(DismissEvent);
            }
            Some(Row::Submenu { .. }) => self.open_submenu(row, window, cx),
            _ => {}
        }
    }

    fn open_submenu(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Row::Submenu { item }) = self.rows.get(row).cloned() else {
            return;
        };
        let Some(PopupItem::Submenu { items, .. }) = self.items.get(item) else {
            return;
        };
        let items = items.as_ref().clone();
        let on_reset = self.on_reset.clone();
        let minimum_width = self.minimum_width;
        let popup = cx.new(|cx| {
            let mut popup = FilterPopup::new(items, None, on_reset, minimum_width, cx);
            popup.nested = true;
            popup
        });
        let row_bounds = self.row_bounds.borrow().get(row).copied().flatten();
        let popup_bounds = *self.bounds.borrow();
        let position = match (row_bounds, popup_bounds) {
            (Some(row_bounds), Some(popup_bounds)) => point(popup_bounds.right(), row_bounds.top()),
            (Some(row_bounds), None) => row_bounds.top_right(),
            (None, Some(popup_bounds)) => popup_bounds.top_right(),
            (None, None) => Point::default(),
        };
        let performed = cx.subscribe_in(&popup, window, |this, _, _: &FilterPopupEvent, _, cx| {
            this.submenu = None;
            cx.emit(FilterPopupEvent::Performed);
            cx.emit(DismissEvent);
        });
        let dismissed = cx.subscribe_in(&popup, window, |this, _, _: &DismissEvent, window, cx| {
            if this.submenu.take().is_some() {
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }
        });
        window.focus(&popup.focus_handle(cx), cx);
        self.submenu = Some(OpenSubmenu {
            row,
            popup,
            position,
            _subscriptions: vec![performed, dismissed],
        });
        cx.notify();
    }

    fn reset_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_reset) = self.on_reset.clone() {
            on_reset(window, cx);
            cx.emit(FilterPopupEvent::Performed);
            cx.emit(DismissEvent);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        match keystroke.key.as_str() {
            "backspace" => {
                cx.stop_propagation();
                if self.query.pop().is_some() {
                    self.rebuild_rows();
                    cx.notify();
                } else {
                    self.reset_filter(window, cx);
                }
            }
            "delete" => {
                cx.stop_propagation();
                self.reset_filter(window, cx);
            }
            _ => {
                let Some(text) = keystroke.key_char.as_ref() else {
                    return;
                };
                if text.is_empty() || text.chars().any(char::is_control) {
                    return;
                }
                cx.stop_propagation();
                self.query.push_str(text);
                self.rebuild_rows();
                cx.notify();
            }
        }
    }

    fn toggle_icon(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(handler) = self
            .action(item)
            .and_then(|action| action.on_icon_click.clone())
        else {
            return;
        };
        let favorite = handler(window, cx);
        if let Some(PopupItem::Action(action)) = self.items.get_mut(item)
            && let PopupIcon::Favorite(_) = action.icon
        {
            action.icon = PopupIcon::Favorite(favorite);
        }
        cx.notify();
    }

    fn mouse_down_outside(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(submenu) = &self.submenu
            && (*submenu.popup.read(cx).bounds.borrow())
                .is_some_and(|bounds| bounds.contains(&event.position))
        {
            return;
        }
        cx.emit(DismissEvent);
    }

    fn render_icon(&self, item: usize, icon: PopupIcon, cx: &mut Context<Self>) -> AnyElement {
        let icon_element = match icon {
            PopupIcon::None => None,
            PopupIcon::Favorite(true) => Some(
                Icon::new(IconName::NodesStar)
                    .size(IconSize::Medium)
                    .color(Color::Warning),
            ),
            PopupIcon::Favorite(false) => None,
            PopupIcon::Checked(true) => Some(Icon::new(IconName::Check).size(IconSize::Medium)),
            PopupIcon::Checked(false) => None,
        };
        let clickable = self
            .action(item)
            .is_some_and(|action| action.on_icon_click.is_some());
        div()
            .id(("git-log-filter-popup-icon", item))
            .flex_none()
            .size(POPUP_MENU_METRICS.icon_column_width)
            .flex()
            .items_center()
            .justify_center()
            .children(icon_element)
            .when(clickable, |this| {
                this.cursor_pointer().on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.toggle_icon(item, window, cx);
                    },
                ))
            })
            .into_any_element()
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let reserve_icon_column = self.items.iter().any(
            |item| matches!(item, PopupItem::Action(action) if action.icon != PopupIcon::None),
        );
        let selected = self.selected == Some(index);
        let element = match row {
            Row::Separator(Some(text)) => h_flex()
                .w_full()
                .px(px(12.))
                .pt_1()
                .child(
                    Label::new(text.clone())
                        .size(LabelSize::Small)
                        .weight(FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .into_any_element(),
            Row::Separator(None) => div()
                .w_full()
                .flex_none()
                .h(POPUP_MENU_METRICS.separator_height)
                .pt(POPUP_MENU_METRICS.separator_line_offset)
                .px(POPUP_MENU_METRICS.separator_inset)
                .child(
                    div()
                        .w_full()
                        .h(POPUP_MENU_METRICS.separator_line_thickness)
                        .bg(colors.border_variant),
                )
                .into_any_element(),
            Row::Action { item, highlights } => {
                let Some(action) = self.action(*item) else {
                    return div().into_any_element();
                };
                let item = *item;
                let label = if highlights.is_empty() {
                    Label::new(action.text.clone()).into_any_element()
                } else {
                    HighlightedLabel::new(action.text.clone(), highlights.clone())
                        .into_any_element()
                };
                let tooltip = action.tooltip.clone();
                ListItem::new(("git-log-filter-popup-row", index))
                    .inset(true)
                    .popup_row_style(PopupRowStyle::Menu)
                    .toggle_state(selected)
                    .when(reserve_icon_column, |this| {
                        this.start_slot(self.render_icon(item, action.icon, cx))
                    })
                    .child(label)
                    .when_some(tooltip, |this, tooltip| {
                        this.tooltip(Tooltip::text(tooltip))
                    })
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.selected != Some(index) {
                            this.selected = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected = Some(index);
                        this.activate(index, window, cx);
                    }))
                    .into_any_element()
            }
            Row::Submenu { item } => {
                let text = match self.items.get(*item) {
                    Some(PopupItem::Submenu { text, .. }) => text.clone(),
                    _ => SharedString::default(),
                };
                ListItem::new(("git-log-filter-popup-row", index))
                    .inset(true)
                    .popup_row_style(PopupRowStyle::Menu)
                    .toggle_state(selected)
                    .when(reserve_icon_column, |this| {
                        this.start_slot(div().size(POPUP_MENU_METRICS.icon_column_width))
                    })
                    .child(Label::new(text))
                    .end_slot(Icon::new(IconName::GeneralChevronRight).size(IconSize::Medium))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.selected != Some(index) {
                            this.selected = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected = Some(index);
                        this.open_submenu(index, window, cx);
                    }))
                    .into_any_element()
            }
        };
        let row_bounds = self.row_bounds.clone();
        div()
            .relative()
            .w_full()
            .child(element)
            .child(
                canvas(
                    move |bounds, _, _| {
                        if let Some(slot) = row_bounds.borrow_mut().get_mut(index) {
                            *slot = Some(bounds);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

fn byte_offsets(text: &str, positions: &[usize]) -> Vec<usize> {
    let offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
    positions
        .iter()
        .filter_map(|position| offsets.get(*position).copied())
        .collect()
}

impl EventEmitter<DismissEvent> for FilterPopup {}
impl EventEmitter<FilterPopupEvent> for FilterPopup {}

impl Focusable for FilterPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FilterPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        *self.row_bounds.borrow_mut() = vec![None; self.rows.len()];
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(index, row, cx))
            .collect();
        let colors = cx.theme().colors();
        let maximum_height = window.viewport_size().height * MAXIMUM_HEIGHT_FRACTION;
        let bounds_cell = self.bounds.clone();
        let submenu = self.submenu.as_ref().map(|submenu| {
            deferred(
                anchored()
                    .position(submenu.position)
                    .snap_to_window()
                    .child(submenu.popup.clone()),
            )
            .with_priority(2)
        });
        let open_submenu_row = self.submenu.as_ref().map(|submenu| submenu.row);
        v_flex()
            .id("git-log-filter-popup")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_child))
            .on_action(cx.listener(Self::select_parent))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(Self::mouse_down_outside))
            .popup_surface(cx)
            .min_w(self.minimum_width)
            .max_h(maximum_height)
            .overflow_y_scroll()
            .py(POPUP_MENU_METRICS.vertical_padding)
            .when(!self.query.is_empty(), |this| {
                this.child(
                    h_flex()
                        .px(ADVERTISER_HORIZONTAL_INSET)
                        .pb_1()
                        .gap_1()
                        .border_b_1()
                        .border_color(colors.border_variant)
                        .child(
                            Icon::new(IconName::MagnifyingGlass)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        )
                        .child(Label::new(self.query.clone())),
                )
            })
            .children(rows)
            .when_some(self.advertisement.clone(), |this, advertisement| {
                this.child(
                    div()
                        .mt_1()
                        .px(ADVERTISER_HORIZONTAL_INSET)
                        .py(ADVERTISER_VERTICAL_INSET)
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(
                            Label::new(advertisement)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
            })
            .child(
                canvas(
                    move |bounds, _, _| {
                        *bounds_cell.borrow_mut() = Some(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .when(open_submenu_row.is_some(), |this| this.children(submenu))
    }
}
