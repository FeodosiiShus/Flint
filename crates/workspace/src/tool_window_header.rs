use crate::dock::{Dock, DockPosition, PanelHandle, PanelHeaderAction, panel_context_menu};
use gpui::{Action, Anchor, ElementId, Entity, FocusHandle, FontWeight};
use std::sync::Arc;
use ui::{ChromeRegion, IconButtonShape, PopoverMenu, Tooltip, prelude::*, right_click_menu};

pub(crate) const TOOL_WINDOW_GROUP: &str = "tool-window";
const PANEL_NAME_SUFFIX: &str = " Panel";

pub(crate) fn tool_window_title(name: &str) -> &str {
    name.strip_suffix(PANEL_NAME_SUFFIX).unwrap_or(name)
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
    let hide_tooltip_action = hide_action.boxed_clone();

    let options_button = PopoverMenu::new(("tool-window-header-options", panel_id))
        .anchor(Anchor::TopRight)
        .menu({
            let panel = panel.clone();
            let dock = dock.clone();
            move |window, cx| Some(panel_context_menu(panel.clone(), dock.clone(), window, cx))
        })
        .trigger_with_tooltip(
            IconButton::new(
                ("tool-window-header-options-button", panel_id),
                IconName::EllipsisVertical,
            )
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::Small)
            .chrome_region(ChromeRegion::Panel)
            .aria_label("Options"),
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
            let button = IconButton::new(
                (ElementId::from(("tool-window-header-action", panel_id)), id),
                icon,
            )
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::Small)
            .chrome_region(ChromeRegion::Panel)
            .aria_label(tooltip)
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

    let hide_button = IconButton::new(("tool-window-header-hide", panel_id), IconName::Dash)
        .shape(IconButtonShape::Square)
        .icon_size(IconSize::Small)
        .chrome_region(ChromeRegion::Panel)
        .aria_label("Hide")
        .tooltip(move |_window, cx| Tooltip::for_action("Hide", &*hide_tooltip_action, cx))
        .on_click(move |_, window, cx| {
            window.focus(&dock_focus_handle, cx);
            window.dispatch_action(hide_action.boxed_clone(), cx);
        });

    let actions = h_flex()
        .flex_none()
        .gap_0p5()
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

    let header = h_flex()
        .debug_selector(move || format!("tool_window_header_{position_key}"))
        .flex_none()
        .w_full()
        .h(ui::panel_header_height(cx))
        .pl_2()
        .pr_1()
        .gap_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Label::new(title).weight(FontWeight::SEMIBOLD).truncate()),
        )
        .child(actions);

    right_click_menu(("tool-window-header-menu", panel_id))
        .menu(move |window, cx| panel_context_menu(panel.clone(), dock.clone(), window, cx))
        .trigger(move |_is_menu_open, _window, _cx| header)
        .into_any_element()
}
