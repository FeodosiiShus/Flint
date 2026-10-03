use crate::WorkspaceSettings;
use crate::dock::{Dock, DockPosition, PanelHandle, panel_context_menu};
use crate::workspace_settings::ToolWindowBarsSettings;
use gpui::{
    Action, Anchor, AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, IntoElement,
    ParentElement, Render, Role, SharedString, Styled, Subscription, Window, div, px, rems,
};
use settings::{Settings, SettingsStore};
use std::sync::Arc;
use theme::ThemeColors;
use ui::{
    BackgroundImageArea, BackgroundImageTarget, ContextMenu, CountBadge, Divider, DividerColor,
    PopoverMenu, Tooltip, background_image_layer, prelude::*, right_click_menu,
};

const BUTTON_PADDING: Pixels = px(6.);
const BUTTON_RADIUS: Pixels = px(6.);
const BAR_VERTICAL_PADDING: Pixels = px(4.);
const BUTTON_GAP: Pixels = px(4.);
const NAMED_BUTTON_VERTICAL_PADDING: Pixels = px(4.);
const LIGHT_BACKGROUND_LUMINANCE: f32 = 0.55;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolWindowBarSide {
    Left,
    Right,
}

impl ToolWindowBarSide {
    fn key(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    fn menu_placement(self) -> (Anchor, Anchor) {
        match self {
            Self::Left => (Anchor::TopLeft, Anchor::TopRight),
            Self::Right => (Anchor::TopRight, Anchor::TopLeft),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolWindowButtonState {
    Closed,
    Open,
    Focused,
}

impl ToolWindowButtonState {
    pub fn new(panel_visible: bool, panel_focused: bool) -> Self {
        match (panel_visible, panel_focused) {
            (false, _) => Self::Closed,
            (true, false) => Self::Open,
            (true, true) => Self::Focused,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolWindowButtonColors {
    pub background: Option<Hsla>,
    pub icon: Hsla,
}

pub fn tool_window_button_colors(
    state: ToolWindowButtonState,
    colors: &ThemeColors,
) -> ToolWindowButtonColors {
    match state {
        ToolWindowButtonState::Closed => ToolWindowButtonColors {
            background: None,
            icon: colors.icon,
        },
        ToolWindowButtonState::Open => ToolWindowButtonColors {
            background: Some(colors.ghost_element_active),
            icon: colors.icon,
        },
        ToolWindowButtonState::Focused => ToolWindowButtonColors {
            background: Some(colors.text_accent),
            icon: contrasting_foreground(colors.text_accent),
        },
    }
}

fn contrasting_foreground(background: Hsla) -> Hsla {
    let rgb = background.to_rgb();
    let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
    if luminance > LIGHT_BACKGROUND_LUMINANCE {
        gpui::black()
    } else {
        gpui::white()
    }
}

struct ToolWindowButton {
    panel: Arc<dyn PanelHandle>,
    dock: Entity<Dock>,
    dock_focus_handle: FocusHandle,
    selector: String,
    icon: IconName,
    name: &'static str,
    state: ToolWindowButtonState,
    click_action: Box<dyn Action>,
    toggle_action: Box<dyn Action>,
    badge: Option<usize>,
}

pub struct ToolWindowBar {
    side: ToolWindowBarSide,
    left_dock: Entity<Dock>,
    bottom_dock: Entity<Dock>,
    right_dock: Entity<Dock>,
    _subscriptions: Vec<Subscription>,
}

impl ToolWindowBar {
    pub fn new(
        side: ToolWindowBarSide,
        left_dock: Entity<Dock>,
        bottom_dock: Entity<Dock>,
        right_dock: Entity<Dock>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&left_dock, |_, _, cx| cx.notify()),
            cx.observe(&bottom_dock, |_, _, cx| cx.notify()),
            cx.observe(&right_dock, |_, _, cx| cx.notify()),
            cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
        ];
        Self {
            side,
            left_dock,
            bottom_dock,
            right_dock,
            _subscriptions: subscriptions,
        }
    }

    fn top_dock(&self) -> &Entity<Dock> {
        match self.side {
            ToolWindowBarSide::Left => &self.left_dock,
            ToolWindowBarSide::Right => &self.right_dock,
        }
    }

    fn bottom_group_dock(&self) -> Option<&Entity<Dock>> {
        match self.side {
            ToolWindowBarSide::Left => Some(&self.bottom_dock),
            ToolWindowBarSide::Right => None,
        }
    }

    fn dock_buttons(dock: &Entity<Dock>, window: &Window, cx: &App) -> Vec<ToolWindowButton> {
        let dock_state = dock.read(cx);
        let position_key = dock_position_key(dock_state.position());
        let active_index = dock_state.active_panel_index();
        let dock_open = dock_state.is_open();
        let dock_focus_handle = dock_state.focus_handle(cx);
        dock_state
            .panel_handles()
            .enumerate()
            .filter_map(|(index, panel)| {
                let icon = panel.icon(window, cx)?;
                let name = panel.icon_tooltip(window, cx)?;
                let panel_visible = dock_open && active_index == Some(index);
                let panel_focused =
                    panel_visible && panel.panel_focus_handle(cx).contains_focused(window, cx);
                let toggle_action = panel.toggle_action(window, cx);
                let click_action = if panel_visible {
                    dock_state.toggle_action()
                } else {
                    toggle_action.boxed_clone()
                };
                let badge = panel
                    .icon_label(window, cx)
                    .filter(|_| !panel_visible)
                    .and_then(|label| label.parse::<usize>().ok());
                Some(ToolWindowButton {
                    panel: panel.clone(),
                    dock: dock.clone(),
                    dock_focus_handle: dock_focus_handle.clone(),
                    selector: format!("tool_window_button_{position_key}_{index}"),
                    icon,
                    name,
                    state: ToolWindowButtonState::new(panel_visible, panel_focused),
                    click_action,
                    toggle_action,
                    badge,
                })
            })
            .collect()
    }

    fn render_button(
        &self,
        button: ToolWindowButton,
        settings: &ToolWindowBarsSettings,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let button_colors = tool_window_button_colors(button.state, colors);
        let hover_background = colors.ghost_element_hover;
        let label_color = if button.state == ToolWindowButtonState::Focused {
            Color::Custom(button_colors.icon)
        } else {
            Color::Muted
        };
        let button_side = settings.icon_size + BUTTON_PADDING * 2.;
        let icon_size = IconSize::Custom(rems(
            f32::from(settings.icon_size) / f32::from(window.rem_size()),
        ));
        let show_names = settings.show_names;
        let name = button.name;
        let selector = button.selector;
        let panel_id = button.panel.panel_id().as_u64();
        let toggle_action = button.toggle_action;
        let click_action = button.click_action;
        let dock_focus_handle = button.dock_focus_handle;
        let (menu_anchor, menu_attach) = self.side.menu_placement();
        let panel = button.panel;
        let dock = button.dock;
        let badge = button.badge;
        let icon = button.icon;

        let trigger = div()
            .id(("tool-window-button", panel_id))
            .debug_selector(move || selector)
            .role(Role::Button)
            .aria_label(name)
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .justify_center()
            .gap_0p5()
            .rounded(BUTTON_RADIUS)
            .cursor_pointer()
            .map(|this| {
                if show_names {
                    this.w(settings.bar_width() - BUTTON_GAP)
                        .py(NAMED_BUTTON_VERTICAL_PADDING)
                } else {
                    this.size(button_side)
                }
            })
            .map(|this| match button_colors.background {
                Some(background) => this.bg(background),
                None => this.hover(move |style| style.bg(hover_background)),
            })
            .child(
                Icon::new(icon)
                    .size(icon_size)
                    .color(Color::Custom(button_colors.icon)),
            )
            .when(show_names, |this| {
                this.child(
                    Label::new(name)
                        .size(LabelSize::XSmall)
                        .color(label_color)
                        .truncate(),
                )
            })
            .on_click(move |_, window, cx| {
                window.focus(&dock_focus_handle, cx);
                window.dispatch_action(click_action.boxed_clone(), cx);
            })
            .tooltip(move |_window, cx| Tooltip::for_action(name, &*toggle_action, cx));

        right_click_menu(("tool-window-menu", panel_id))
            .menu(move |window, cx| panel_context_menu(panel.clone(), dock.clone(), window, cx))
            .anchor(menu_anchor)
            .attach(menu_attach)
            .trigger(move |_is_menu_open, _window, _cx| {
                div()
                    .relative()
                    .child(trigger)
                    .when_some(badge, |this, count| this.child(CountBadge::new(count)))
            })
            .into_any_element()
    }

    fn render_more_button(
        &self,
        settings: &ToolWindowBarsSettings,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let docks = [
            self.left_dock.clone(),
            self.bottom_dock.clone(),
            self.right_dock.clone(),
        ];
        let icon_size = IconSize::Custom(rems(
            f32::from(settings.icon_size) / f32::from(window.rem_size()),
        ));
        let button_side = settings.icon_size + BUTTON_PADDING * 2.;
        let (menu_anchor, menu_attach) = self.side.menu_placement();
        let has_panels = docks.iter().any(|dock| dock.read(cx).panels_len() > 0);

        div()
            .debug_selector(|| "tool_window_more_button".into())
            .child(
                PopoverMenu::new("tool-window-more")
                    .anchor(menu_anchor)
                    .attach(menu_attach)
                    .menu(move |window, cx| {
                        let docks = docks.clone();
                        Some(ContextMenu::build(window, cx, move |menu, window, cx| {
                            more_tool_windows_menu(menu, &docks, window, cx)
                        }))
                    })
                    .trigger_with_tooltip(
                        IconButton::new("tool-window-more-button", IconName::Ellipsis)
                            .size(ButtonSize::Large)
                            .icon_size(icon_size)
                            .icon_color(Color::Muted)
                            .width(button_side)
                            .disabled(!has_panels)
                            .aria_label("More Tool Windows"),
                        Tooltip::text("More Tool Windows"),
                    ),
            )
            .into_any_element()
    }
}

fn more_tool_windows_menu(
    mut menu: ContextMenu,
    docks: &[Entity<Dock>],
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    for dock in docks {
        let dock_state = dock.read(cx);
        let dock_open = dock_state.is_open();
        let active_index = dock_state.active_panel_index();
        let dock_focus_handle = dock_state.focus_handle(cx);
        let close_action = dock_state.toggle_action();
        let entries = dock_state
            .panel_handles()
            .enumerate()
            .map(|(index, panel)| {
                let label: SharedString = panel
                    .icon_tooltip(window, cx)
                    .unwrap_or_else(|| panel.persistent_name())
                    .into();
                let panel_visible = dock_open && active_index == Some(index);
                let action = if panel_visible {
                    close_action.boxed_clone()
                } else {
                    panel.toggle_action(window, cx)
                };
                (label, panel_visible, action)
            })
            .collect::<Vec<_>>();
        for (label, panel_visible, action) in entries {
            let dock_focus_handle = dock_focus_handle.clone();
            menu = menu.toggleable_entry(
                label,
                panel_visible,
                IconPosition::Start,
                None,
                move |window, cx| {
                    window.focus(&dock_focus_handle, cx);
                    window.dispatch_action(action.boxed_clone(), cx);
                },
            );
        }
    }
    menu
}

fn dock_position_key(position: DockPosition) -> &'static str {
    match position {
        DockPosition::Left => "left",
        DockPosition::Bottom => "bottom",
        DockPosition::Right => "right",
    }
}

impl Render for ToolWindowBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = WorkspaceSettings::get_global(cx).tool_window_bars;
        let background = cx.theme().colors().background;
        let side = self.side;

        let top_buttons = Self::dock_buttons(self.top_dock(), window, cx)
            .into_iter()
            .map(|button| self.render_button(button, &settings, window, cx))
            .collect::<Vec<_>>();
        let bottom_buttons = self
            .bottom_group_dock()
            .map(|dock| Self::dock_buttons(dock, window, cx))
            .unwrap_or_default()
            .into_iter()
            .map(|button| self.render_button(button, &settings, window, cx))
            .collect::<Vec<_>>();
        let more_button = (side == ToolWindowBarSide::Left)
            .then(|| self.render_more_button(&settings, window, cx));
        let separator_width = settings.icon_size + BUTTON_PADDING * 2.;

        v_flex()
            .id(("tool-window-bar", side as usize))
            .debug_selector(move || format!("tool_window_bar_{}", side.key()))
            .role(Role::Toolbar)
            .aria_label(match side {
                ToolWindowBarSide::Left => "Left tool window bar",
                ToolWindowBarSide::Right => "Right tool window bar",
            })
            .flex_none()
            .h_full()
            .w(settings.bar_width())
            .py(BAR_VERTICAL_PADDING)
            .gap(BUTTON_GAP)
            .items_center()
            .bg(background)
            .child(background_image_layer(
                BackgroundImageTarget::EditorAndTools,
                BackgroundImageArea::Window,
                background,
                false,
                gpui::Corners::default(),
            ))
            .children(top_buttons)
            .when_some(more_button, |this, more_button| {
                this.child(
                    div()
                        .w(separator_width)
                        .child(Divider::horizontal().color(DividerColor::Border)),
                )
                .child(more_button)
            })
            .child(div().flex_1())
            .children(bottom_buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_state_follows_panel_visibility_and_focus() {
        assert_eq!(
            ToolWindowButtonState::new(false, false),
            ToolWindowButtonState::Closed
        );
        assert_eq!(
            ToolWindowButtonState::new(false, true),
            ToolWindowButtonState::Closed,
            "a hidden panel cannot be shown as focused"
        );
        assert_eq!(
            ToolWindowButtonState::new(true, false),
            ToolWindowButtonState::Open
        );
        assert_eq!(
            ToolWindowButtonState::new(true, true),
            ToolWindowButtonState::Focused
        );
    }

    #[test]
    fn button_colors_distinguish_closed_open_and_focused() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            let closed = tool_window_button_colors(ToolWindowButtonState::Closed, &colors);
            let open = tool_window_button_colors(ToolWindowButtonState::Open, &colors);
            let focused = tool_window_button_colors(ToolWindowButtonState::Focused, &colors);

            assert_eq!(closed.background, None);
            assert_eq!(closed.icon, colors.icon);
            assert_eq!(open.background, Some(colors.ghost_element_active));
            assert_eq!(open.icon, colors.icon);
            assert_eq!(focused.background, Some(colors.text_accent));
            assert_eq!(focused.icon, contrasting_foreground(colors.text_accent));
        }
    }

    #[test]
    fn contrasting_foreground_picks_readable_icon_color() {
        assert_eq!(contrasting_foreground(gpui::black()), gpui::white());
        assert_eq!(contrasting_foreground(gpui::white()), gpui::black());
        assert_eq!(
            contrasting_foreground(gpui::hsla(0.6, 0.8, 0.45, 1.0)),
            gpui::white(),
            "a saturated mid blue accent takes a white icon"
        );
        assert_eq!(
            contrasting_foreground(gpui::hsla(0.15, 0.9, 0.7, 1.0)),
            gpui::black(),
            "a light yellow accent takes a black icon"
        );
    }
}
