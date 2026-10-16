use crate::WorkspaceSettings;
use crate::dock::{Dock, PanelHandle, panel_context_menu};
use crate::workspace_settings::ToolWindowBarsSettings;
use gpui::{
    Action, Anchor, AnyElement, App, Context, Edges, Entity, FocusHandle, Focusable, Hsla,
    IntoElement, ParentElement, Render, Role, SharedString, Styled, Subscription, Window, div,
    hsla, px, rems,
};
use settings::{Settings, SettingsStore, ToolWindowIconStyle};
use std::sync::Arc;
use theme::ThemeColors;
use ui::{
    BackgroundImageArea, BackgroundImageTarget, ButtonLike, ContextMenu, CountBadge, PopoverMenu,
    Tooltip, background_image_layer, prelude::*, right_click_menu,
};

const BAR_VERTICAL_PADDING: Pixels = px(4.);
const BUTTON_GAP: Pixels = px(0.);
const NAMED_BUTTON_VERTICAL_PADDING: Pixels = px(4.);
const LIGHT_BACKGROUND_LUMINANCE: f32 = 0.55;

// IntelliJ New UI stripe metrics (Islands `StripeToolbar.Button` and `Button.ToolWindow.arc`):
// the button box is `StripeToolbar.Button.size`, "37,40" normal and "32,32" compact; the fill
// pill is the box minus the icon padding, `leftStripeIcon.padding` "5,5,5,2" and
// `rightStripeIcon.padding` "5,2,5,5" (compact "4,5,4,3" and "4,3,4,5": 5 px to the window
// edge, 2 px, 3 px compact, to the side facing the editor); the corner radius is half the arc
// of 12, or 8 compact.
const FILL_ICON_PADDING: Pixels = px(10.);
const FILL_ICON_PADDING_COMPACT: Pixels = px(8.);
const FILL_OUTER_INSET: Pixels = px(5.);
const FILL_INNER_INSET: Pixels = px(2.);
const FILL_OUTER_INSET_COMPACT: Pixels = px(5.);
const FILL_INNER_INSET_COMPACT: Pixels = px(3.);
const FILL_VERTICAL_INSET: Pixels = px(5.);
const FILL_VERTICAL_INSET_COMPACT: Pixels = px(4.);
const BUTTON_RADIUS: Pixels = px(6.);
const BUTTON_RADIUS_COMPACT: Pixels = px(4.);
// `StripeButtonSeparator`: a 11 px slot holding a 24x1 line.
const SEPARATOR_HEIGHT: Pixels = px(11.);
const SEPARATOR_LINE_WIDTH: Pixels = px(24.);

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

/// The IntelliJ New UI geometry of one stripe button: the clickable box, the fill pill inside
/// it and the pill's corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolWindowButtonMetrics {
    pub box_width: Pixels,
    pub box_height: Pixels,
    pub fill_width: Pixels,
    pub fill_height: Pixels,
    pub fill_insets: Edges<Pixels>,
    pub radius: Pixels,
}

impl ToolWindowButtonMetrics {
    pub fn new(side: ToolWindowBarSide, settings: &ToolWindowBarsSettings) -> Self {
        let (icon_padding, outer_inset, inner_inset, vertical_inset, radius) =
            if settings.uses_compact_icons() {
                (
                    FILL_ICON_PADDING_COMPACT,
                    FILL_OUTER_INSET_COMPACT,
                    FILL_INNER_INSET_COMPACT,
                    FILL_VERTICAL_INSET_COMPACT,
                    BUTTON_RADIUS_COMPACT,
                )
            } else {
                (
                    FILL_ICON_PADDING,
                    FILL_OUTER_INSET,
                    FILL_INNER_INSET,
                    FILL_VERTICAL_INSET,
                    BUTTON_RADIUS,
                )
            };
        let (left_inset, right_inset) = match side {
            ToolWindowBarSide::Left => (outer_inset, inner_inset),
            ToolWindowBarSide::Right => (inner_inset, outer_inset),
        };
        let fill_width = settings.icon_size + icon_padding;
        let fill_height = fill_width;

        Self {
            box_width: fill_width + left_inset + right_inset,
            box_height: fill_height + vertical_inset * 2.,
            fill_width,
            fill_height,
            fill_insets: Edges {
                top: vertical_inset,
                bottom: vertical_inset,
                left: left_inset,
                right: right_inset,
            },
            radius,
        }
    }
}

/// `ToolWindow.Stripe.separatorColor` (`toolbar-border`): `transparent-white-30`, `#FFFFFF21`,
/// in Islands Dark and `transparent-black-30`, `#00000020`, in Islands Light.
fn stripe_separator_color(is_light: bool) -> Hsla {
    if is_light {
        hsla(0., 0., 0., 32. / 255.)
    } else {
        hsla(0., 0., 1., 33. / 255.)
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
        let position_key = dock_state.position().key();
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
        let metrics = ToolWindowButtonMetrics::new(self.side, settings);
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
        let icon = tool_window_icon(button.icon, settings);
        let pressed_background = colors.ghost_element_active;
        let fill_group: SharedString = format!("tool-window-button-fill-{panel_id}").into();
        let trigger_group = fill_group.clone();

        let fill = div()
            .absolute()
            .top(metrics.fill_insets.top)
            .bottom(metrics.fill_insets.bottom)
            .left(metrics.fill_insets.left)
            .right(metrics.fill_insets.right)
            .rounded(metrics.radius)
            .map(|this| match button_colors.background {
                Some(background) => this.bg(background),
                None => this
                    .group_hover(fill_group.clone(), move |style| style.bg(hover_background))
                    .group_active(fill_group.clone(), move |style| {
                        style.bg(pressed_background)
                    }),
            });

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
            .relative()
            .cursor_pointer()
            .map(|this| {
                if show_names {
                    this.w(settings.bar_width() - BUTTON_GAP)
                        .py(NAMED_BUTTON_VERTICAL_PADDING)
                        .rounded(BUTTON_RADIUS)
                        .map(|this| match button_colors.background {
                            Some(background) => this.bg(background),
                            None => this.hover(move |style| style.bg(hover_background)),
                        })
                } else {
                    this.w(metrics.box_width)
                        .h(metrics.box_height)
                        .group(trigger_group)
                        .child(fill)
                }
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
        let metrics = ToolWindowButtonMetrics::new(self.side, settings);
        let (menu_anchor, menu_attach) = self.side.menu_placement();
        let has_panels = docks.iter().any(|dock| dock.read(cx).panels_len() > 0);
        let more_icon = tool_window_icon(IconName::Ellipsis, settings);

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
                        ButtonLike::new("tool-window-more-button")
                            .width(metrics.box_width)
                            .height(metrics.box_height)
                            .aria_label("More Tool Windows")
                            .disabled(!has_panels)
                            .child(Icon::new(more_icon).size(icon_size).color(Color::Muted)),
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

impl Render for ToolWindowBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = WorkspaceSettings::get_global(cx).tool_window_bars;
        let content_opacity = WorkspaceSettings::get_global(cx)
            .islands
            .frame_content_opacity(window.is_window_active());
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
            .rev()
            .map(|button| self.render_button(button, &settings, window, cx))
            .collect::<Vec<_>>();
        let more_button = (side == ToolWindowBarSide::Left)
            .then(|| self.render_more_button(&settings, window, cx));
        if top_buttons.is_empty() && bottom_buttons.is_empty() && more_button.is_none() {
            return div().into_any_element();
        }
        let separator_width = settings.bar_width();

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
            .bg(background)
            .child(background_image_layer(
                BackgroundImageTarget::EditorAndTools,
                BackgroundImageArea::Window,
                background,
                false,
                gpui::Corners::default(),
            ))
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .gap(BUTTON_GAP)
                    .items_center()
                    .opacity(content_opacity)
                    .children(top_buttons)
                    .when_some(more_button, |this, more_button| {
                        this.child(
                            div()
                                .debug_selector(|| "tool_window_bar_separator".into())
                                .w(separator_width)
                                .h(SEPARATOR_HEIGHT)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(div().w(SEPARATOR_LINE_WIDTH).h(px(1.)).bg(
                                    stripe_separator_color(cx.theme().appearance().is_light()),
                                )),
                        )
                        .child(more_button)
                    })
                    .child(div().flex_1())
                    .children(bottom_buttons),
            )
            .into_any_element()
    }
}

fn tool_window_icon(icon: IconName, settings: &ToolWindowBarsSettings) -> IconName {
    if settings.icon_style == ToolWindowIconStyle::Zed {
        return icon;
    }
    let compact = settings.uses_compact_icons();
    match (icon, compact) {
        (IconName::FileTree, false) => IconName::ToolWindowProject,
        (IconName::FileTree, true) => IconName::ToolWindowProjectCompact,
        (IconName::GitBranch, false) => IconName::ToolWindowVcs,
        (IconName::GitBranch, true) => IconName::ToolWindowVcsCompact,
        (IconName::TerminalAlt, false) => IconName::ToolWindowTerminal,
        (IconName::TerminalAlt, true) => IconName::ToolWindowTerminalCompact,
        (IconName::MagnifyingGlass, false) => IconName::ToolWindowSearch,
        (IconName::MagnifyingGlass, true) => IconName::ToolWindowSearchCompact,
        (IconName::Warning, false) => IconName::ToolWindowProblems,
        (IconName::Warning, true) => IconName::ToolWindowProblemsCompact,
        (IconName::BoltOutlined, false) => IconName::ToolWindowServices,
        (IconName::BoltOutlined, true) => IconName::ToolWindowServicesCompact,
        (IconName::Ellipsis, false) => IconName::ToolWindowMore,
        (IconName::Ellipsis, true) => IconName::ToolWindowMoreCompact,
        _ => icon,
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

    fn bars_settings(icon_size: f32, icon_style: ToolWindowIconStyle) -> ToolWindowBarsSettings {
        ToolWindowBarsSettings {
            show: true,
            icon_size: px(icon_size),
            show_names: false,
            icon_style,
        }
    }

    #[test]
    fn tool_window_icon_maps_zed_icons_to_standard_artwork_at_default_size() {
        let settings = bars_settings(20., ToolWindowIconStyle::Jetbrains);
        for (zed_icon, expected) in [
            (IconName::FileTree, IconName::ToolWindowProject),
            (IconName::GitBranch, IconName::ToolWindowVcs),
            (IconName::TerminalAlt, IconName::ToolWindowTerminal),
            (IconName::MagnifyingGlass, IconName::ToolWindowSearch),
            (IconName::Warning, IconName::ToolWindowProblems),
            (IconName::BoltOutlined, IconName::ToolWindowServices),
            (IconName::Ellipsis, IconName::ToolWindowMore),
        ] {
            assert_eq!(
                tool_window_icon(zed_icon, &settings),
                expected,
                "{zed_icon:?} at 20px maps to its standard artwork"
            );
        }
    }

    #[test]
    fn tool_window_icon_switches_to_compact_artwork_below_the_size_limit() {
        let standard_and_compact = [
            (
                IconName::FileTree,
                IconName::ToolWindowProject,
                IconName::ToolWindowProjectCompact,
            ),
            (
                IconName::GitBranch,
                IconName::ToolWindowVcs,
                IconName::ToolWindowVcsCompact,
            ),
            (
                IconName::TerminalAlt,
                IconName::ToolWindowTerminal,
                IconName::ToolWindowTerminalCompact,
            ),
            (
                IconName::MagnifyingGlass,
                IconName::ToolWindowSearch,
                IconName::ToolWindowSearchCompact,
            ),
            (
                IconName::Warning,
                IconName::ToolWindowProblems,
                IconName::ToolWindowProblemsCompact,
            ),
            (
                IconName::BoltOutlined,
                IconName::ToolWindowServices,
                IconName::ToolWindowServicesCompact,
            ),
            (
                IconName::Ellipsis,
                IconName::ToolWindowMore,
                IconName::ToolWindowMoreCompact,
            ),
        ];
        for (zed_icon, standard, compact) in standard_and_compact {
            for icon_size in [16., 17.] {
                let settings = bars_settings(icon_size, ToolWindowIconStyle::Jetbrains);
                assert_eq!(
                    tool_window_icon(zed_icon, &settings),
                    compact,
                    "{zed_icon:?} at {icon_size}px maps to its compact artwork"
                );
            }
            for icon_size in [18., 20., 32.] {
                let settings = bars_settings(icon_size, ToolWindowIconStyle::Jetbrains);
                assert_eq!(
                    tool_window_icon(zed_icon, &settings),
                    standard,
                    "{zed_icon:?} at {icon_size}px maps to its standard artwork"
                );
            }
        }
    }

    #[test]
    fn tool_window_icon_keeps_zed_icons_in_zed_style() {
        for icon_size in [12., 16., 18., 20., 32.] {
            let settings = bars_settings(icon_size, ToolWindowIconStyle::Zed);
            for zed_icon in [
                IconName::FileTree,
                IconName::GitBranch,
                IconName::TerminalAlt,
                IconName::Ellipsis,
                IconName::Plus,
            ] {
                assert_eq!(
                    tool_window_icon(zed_icon, &settings),
                    zed_icon,
                    "{zed_icon:?} at {icon_size}px stays unchanged in zed style"
                );
            }
        }
    }

    #[test]
    fn tool_window_icon_keeps_icons_outside_the_mapping_in_jetbrains_style() {
        for icon_size in [16., 20.] {
            let settings = bars_settings(icon_size, ToolWindowIconStyle::Jetbrains);
            assert_eq!(
                tool_window_icon(IconName::Plus, &settings),
                IconName::Plus,
                "an unmapped icon at {icon_size}px stays unchanged in jetbrains style"
            );
        }
    }

    #[test]
    fn tool_window_button_metrics_match_intellij_at_the_default_size() {
        let settings = bars_settings(20., ToolWindowIconStyle::Jetbrains);

        let left = ToolWindowButtonMetrics::new(ToolWindowBarSide::Left, &settings);
        assert_eq!(left.box_width, px(37.));
        assert_eq!(left.box_height, px(40.));
        assert_eq!(left.fill_width, px(30.));
        assert_eq!(left.fill_height, px(30.));
        assert_eq!(
            left.fill_insets,
            Edges {
                top: px(5.),
                bottom: px(5.),
                left: px(5.),
                right: px(2.),
            }
        );
        assert_eq!(left.radius, px(6.));
        assert_eq!(
            left.box_width,
            settings.bar_width(),
            "the bar is exactly as wide as its buttons"
        );

        let right = ToolWindowButtonMetrics::new(ToolWindowBarSide::Right, &settings);
        assert_eq!(
            right.fill_insets,
            Edges {
                top: px(5.),
                bottom: px(5.),
                left: px(2.),
                right: px(5.),
            },
            "the right bar mirrors the 2 px inset towards the editor"
        );
    }

    #[test]
    fn tool_window_button_metrics_use_the_compact_geometry_below_the_size_limit() {
        let settings = bars_settings(16., ToolWindowIconStyle::Jetbrains);

        let left = ToolWindowButtonMetrics::new(ToolWindowBarSide::Left, &settings);
        assert_eq!(left.box_width, px(32.));
        assert_eq!(left.box_height, px(32.));
        assert_eq!(left.fill_width, px(24.));
        assert_eq!(left.fill_height, px(24.));
        assert_eq!(
            left.fill_insets,
            Edges {
                top: px(4.),
                bottom: px(4.),
                left: px(5.),
                right: px(3.),
            }
        );
        assert_eq!(left.radius, px(4.));
        assert_eq!(
            left.box_width,
            settings.bar_width(),
            "the compact bar is exactly as wide as its buttons"
        );
    }

    #[test]
    fn stripe_separator_uses_the_intellij_stripe_border_color() {
        assert_eq!(
            stripe_separator_color(false),
            gpui::hsla(0., 0., 1., 33. / 255.),
            "Islands Dark draws the separator in transparent-white-30"
        );
        assert_eq!(
            stripe_separator_color(true),
            gpui::hsla(0., 0., 0., 32. / 255.),
            "Islands Light draws the separator in transparent-black-30"
        );
    }
}
