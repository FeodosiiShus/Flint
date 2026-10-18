use crate::dock::{Dock, DockPosition, PanelHandle, PanelHeaderAction, panel_context_menu};
use gpui::{Action, Anchor, ElementId, Entity, FocusHandle, FontWeight};
use std::sync::Arc;
use theme::UiDensity;
use ui::{ChromeRegion, IconButtonShape, PopoverMenu, Tooltip, prelude::*, right_click_menu};

pub(crate) const TOOL_WINDOW_GROUP: &str = "tool-window";
const PANEL_NAME_SUFFIX: &str = " Panel";
const HEADER_LABEL_LEFT_INSET: Pixels = px(12.);
const HEADER_LABEL_RIGHT_INSET: Pixels = px(16.);
const HEADER_TOOLBAR_LEFT_INSET: Pixels = px(12.);
const HEADER_TOOLBAR_RIGHT_INSET: Pixels = px(8.);
const HEADER_ACTION_BUTTON_SIZE: Pixels = px(22.);
const HEADER_ACTION_ICON_SIZE: Pixels = px(16.);
const HEADER_ACTION_ICON_MARGIN: Pixels = px(3.);
const HEADER_LABEL_COMPACT_FONT_SIZE_OFFSET: Pixels = px(-1.);
const HEADER_BORDER_WIDTH: Pixels = px(1.);
// The New UI creates the header id label as `BaseLabel(ui, ExperimentalUI.isNewUI())`, which
// derives `Font.BOLD`.
const HEADER_TITLE_WEIGHT: FontWeight = FontWeight::BOLD;

pub(crate) fn tool_window_title(name: &str) -> &str {
    name.strip_suffix(PANEL_NAME_SUFFIX).unwrap_or(name)
}

// The header's Hide button copies IntelliJ's `HideActiveWindow` (⇧⎋), so its tooltip shows that
// shortcut rather than the dock toggle's.
fn header_hide_tooltip_action() -> Box<dyn Action> {
    crate::CloseActiveDock.boxed_clone()
}

// At rest IntelliJ draws the header's bottom line only for bottom-anchored tool windows and for
// tabbed ones with more than one content (`ToolWindowHeader.paintComponent`). Header content is
// where a panel puts such tabs, like the Git tool window's.
fn draws_bottom_line(position: DockPosition, has_header_content: bool) -> bool {
    position == DockPosition::Bottom || has_header_content
}

fn header_label_size(window: &Window, cx: &App) -> LabelSize {
    match theme::theme_settings(cx).ui_density(cx) {
        UiDensity::Compact => LabelSize::Custom(rems(
            TextSize::Default.rems(cx).0
                + HEADER_LABEL_COMPACT_FONT_SIZE_OFFSET / window.rem_size(),
        )),
        UiDensity::Default | UiDensity::Comfortable => LabelSize::Default,
    }
}

fn header_action_side(icon_size: IconSize, rem_size: Pixels) -> Pixels {
    let icon_pixels = icon_size.rems().to_pixels(rem_size);
    HEADER_ACTION_BUTTON_SIZE.max(icon_pixels + HEADER_ACTION_ICON_MARGIN * 2.)
}

fn header_action_metrics(window: &Window, cx: &App) -> (IconSize, Pixels) {
    let icon_size = match ui::chrome_icon_scale(ChromeRegion::Panel, cx) {
        Some(_) => ui::chrome_icon_size(ChromeRegion::Panel, IconSize::Small, cx),
        None => IconSize::Custom(rems(
            f32::from(HEADER_ACTION_ICON_SIZE) / f32::from(window.rem_size()),
        )),
    };
    (icon_size, header_action_side(icon_size, window.rem_size()))
}

fn header_action_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    icon_size: IconSize,
    side: Pixels,
) -> IconButton {
    IconButton::new(id, icon)
        .shape(IconButtonShape::Square)
        .icon_size(icon_size)
        .box_size(side)
        .aria_label(label)
}

pub(crate) fn render_tool_window_header(
    position: DockPosition,
    panel: Arc<dyn PanelHandle>,
    dock: Entity<Dock>,
    dock_focus_handle: FocusHandle,
    hide_action: Box<dyn Action>,
    show_actions: bool,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let position_key = position.key();
    let panel_id = panel.panel_id();
    let title = tool_window_title(
        panel
            .icon_tooltip(window, cx)
            .unwrap_or_else(|| panel.persistent_name()),
    );
    let hide_tooltip_action = header_hide_tooltip_action();
    let label_size = header_label_size(window, cx);
    let (icon_size, button_side) = header_action_metrics(window, cx);
    // `ToolWindow.Header.borderColor` resolves to the main window background, gray-30 in Islands
    // Dark and gray-150 in Islands Light.
    let bottom_border_color = cx.theme().colors().background;

    let options_button = PopoverMenu::new(("tool-window-header-options", panel_id))
        .anchor(Anchor::TopRight)
        .menu({
            let panel = panel.clone();
            let dock = dock.clone();
            move |window, cx| {
                Some(panel_context_menu(
                    panel.clone(),
                    dock.clone(),
                    false,
                    window,
                    cx,
                ))
            }
        })
        .trigger_with_tooltip(
            header_action_button(
                ("tool-window-header-options-button", panel_id),
                IconName::EllipsisVertical,
                "Options",
                icon_size,
                button_side,
            ),
            Tooltip::text("Options"),
        );

    let header_action_buttons = panel
        .header_actions(window, cx)
        .into_iter()
        .map(|header_action| {
            let PanelHeaderAction {
                id,
                icon,
                tooltip,
                action,
            } = header_action;
            let tooltip_action = action.boxed_clone();
            let dock_focus_handle = dock_focus_handle.clone();
            let button = header_action_button(
                (ElementId::from(("tool-window-header-action", panel_id)), id),
                icon,
                tooltip,
                icon_size,
                button_side,
            )
            .tooltip(move |_window, cx| Tooltip::for_action(tooltip, &*tooltip_action, cx))
            .on_click(move |_, window, cx| {
                window.focus(&dock_focus_handle, cx);
                window.dispatch_action(action.boxed_clone(), cx);
            });
            div()
                .debug_selector(move || format!("tool_window_header_action_{id}_{position_key}"))
                .child(button)
        })
        .collect::<Vec<_>>();

    let hide_button = header_action_button(
        ("tool-window-header-hide", panel_id),
        IconName::GeneralHide,
        "Hide",
        icon_size,
        button_side,
    )
    .tooltip(move |_window, cx| Tooltip::for_action("Hide", &*hide_tooltip_action, cx))
    .on_click(move |_, window, cx| {
        window.focus(&dock_focus_handle, cx);
        window.dispatch_action(hide_action.boxed_clone(), cx);
    });

    let actions = h_flex()
        .flex_none()
        .when(!show_actions, |this| {
            this.visible_on_hover(TOOL_WINDOW_GROUP)
        })
        .children(header_action_buttons)
        .child(
            div()
                .debug_selector(move || format!("tool_window_header_options_{position_key}"))
                .child(options_button),
        )
        .child(
            div()
                .debug_selector(move || format!("tool_window_header_hide_{position_key}"))
                .child(hide_button),
        );

    let header_content = panel.header_content(window, cx);
    let has_bottom_line = draws_bottom_line(position, header_content.is_some());
    let leading = match header_content {
        Some(content) => h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(HEADER_LABEL_LEFT_INSET)
            .gap(HEADER_LABEL_RIGHT_INSET)
            .child(
                div()
                    .debug_selector(move || format!("tool_window_header_title_{position_key}"))
                    .flex_none()
                    .child(
                        Label::new(title)
                            .size(label_size)
                            .weight(HEADER_TITLE_WEIGHT),
                    ),
            )
            .child(
                div()
                    .debug_selector(move || format!("tool_window_header_content_{position_key}"))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(content),
            )
            .into_any_element(),
        None => div()
            .flex_1()
            .min_w_0()
            .pl(HEADER_LABEL_LEFT_INSET)
            .child(
                div()
                    .debug_selector(move || format!("tool_window_header_title_{position_key}"))
                    .child(
                        Label::new(title)
                            .size(label_size)
                            .weight(HEADER_TITLE_WEIGHT)
                            .truncate(),
                    ),
            )
            .into_any_element(),
    };

    let header = h_flex()
        .debug_selector(move || format!("tool_window_header_{position_key}"))
        .relative()
        .flex_none()
        .w_full()
        .h(ui::panel_header_height(cx))
        .pr(HEADER_TOOLBAR_RIGHT_INSET)
        .gap(HEADER_TOOLBAR_LEFT_INSET)
        .child(leading)
        .child(actions)
        .when(has_bottom_line, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(HEADER_BORDER_WIDTH)
                    .bg(bottom_border_color),
            )
        });

    right_click_menu(("tool-window-header-menu", panel_id))
        .menu(move |window, cx| panel_context_menu(panel.clone(), dock.clone(), false, window, cx))
        .trigger(move |_is_menu_open, _window, _cx| header)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_window_header_action_side_keeps_intellij_minimum_around_the_icon() {
        let rem_size = px(16.);
        assert_eq!(
            header_action_side(IconSize::Custom(rems(1.)), rem_size),
            px(22.),
            "a 16 px icon sits in IntelliJ's 22 px toolbar button"
        );
        assert_eq!(
            header_action_side(IconSize::Custom(rems(1.5)), rem_size),
            px(30.),
            "a larger icon keeps 3 px around it"
        );
    }

    #[test]
    fn tool_window_header_title_is_bold_like_the_new_ui_id_label() {
        assert_eq!(HEADER_TITLE_WEIGHT, FontWeight::BOLD);
    }

    #[test]
    fn tool_window_header_hide_tooltip_shows_the_hide_active_window_shortcut() {
        let action = header_hide_tooltip_action();
        assert!(
            action.partial_eq(&crate::CloseActiveDock),
            "the Hide tooltip shows the binding of IntelliJ's HideActiveWindow, got {}",
            action.name()
        );
        for toggle in [
            crate::ToggleLeftDock.boxed_clone(),
            crate::ToggleBottomDock.boxed_clone(),
            crate::ToggleRightDock.boxed_clone(),
        ] {
            assert!(
                !action.partial_eq(toggle.as_ref()),
                "the Hide tooltip does not show the {} shortcut",
                toggle.name()
            );
        }
    }

    #[test]
    fn tool_window_header_bottom_line_is_drawn_only_for_the_bottom_dock_or_header_tabs() {
        assert!(draws_bottom_line(DockPosition::Bottom, false));
        assert!(draws_bottom_line(DockPosition::Bottom, true));
        assert!(!draws_bottom_line(DockPosition::Left, false));
        assert!(!draws_bottom_line(DockPosition::Right, false));
        assert!(
            draws_bottom_line(DockPosition::Left, true),
            "a side tool window with header tabs keeps the line, like IntelliJ's tabbed headers"
        );
        assert!(draws_bottom_line(DockPosition::Right, true));
    }
}
