use gpui::{
    ClickEvent, CursorStyle, ElementId, FocusHandle, ManagedView, MouseButton, Role, accesskit,
};
use ui::{PopoverMenuHandle, component_corner_radii, prelude::*};

use crate::components::{
    COMBO_BOX_ARROW_AREA_WIDTH, COMBO_BOX_MIN_WIDTH, CONTROL_HEIGHT, CONTROL_TEXT_MARGIN_LEFT,
};

type A11yActionListener = Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ComboBoxTrigger {
    id: ElementId,
    label: SharedString,
    open: bool,
    tab_index: Option<isize>,
    focus_handle: Option<FocusHandle>,
    aria_label: Option<SharedString>,
    aria_description: Option<SharedString>,
    cursor_style: CursorStyle,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    a11y_actions: Vec<(accesskit::Action, A11yActionListener)>,
}

impl ComboBoxTrigger {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            open: false,
            tab_index: None,
            focus_handle: None,
            aria_label: None,
            aria_description: None,
            cursor_style: CursorStyle::PointingHand,
            on_click: None,
            a11y_actions: Vec::new(),
        }
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }

    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    pub fn aria_description(mut self, description: impl Into<SharedString>) -> Self {
        self.aria_description = Some(description.into());
        self
    }

    pub fn popover_handle<M: ManagedView>(mut self, handle: &PopoverMenuHandle<M>) -> Self {
        let show_handle = handle.clone();
        let hide_handle = handle.clone();
        self.a11y_actions.push((
            accesskit::Action::Expand,
            Box::new(move |_, window, cx| show_handle.show(window, cx)),
        ));
        self.a11y_actions.push((
            accesskit::Action::Collapse,
            Box::new(move |_, _, cx| hide_handle.hide(cx)),
        ));
        self
    }
}

impl Clickable for ComboBoxTrigger {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.cursor_style = cursor_style;
        self
    }
}

impl Toggleable for ComboBoxTrigger {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }
}

impl RenderOnce for ComboBoxTrigger {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let corner_radius = component_corner_radii(cx).input;
        let colors = cx.theme().colors();
        let focused_border = colors.border_focused;

        h_flex()
            .id(self.id)
            .role(Role::ComboBox)
            .aria_label(self.aria_label.unwrap_or_else(|| self.label.clone()))
            .when_some(self.aria_description, |this, description| {
                this.aria_description(description)
            })
            .aria_value(self.label.clone())
            .aria_expanded(self.open)
            .map(|this| match self.focus_handle {
                Some(focus_handle) => this.track_focus(&focus_handle),
                None => this.when_some(self.tab_index, |this, tab_index| {
                    this.tab_index(tab_index).tab_stop(true)
                }),
            })
            .flex_none()
            .h(CONTROL_HEIGHT)
            .min_w(COMBO_BOX_MIN_WIDTH)
            .rounded(corner_radius)
            .border_1()
            .border_color(if self.open {
                focused_border
            } else {
                colors.border
            })
            .bg(colors.element_background)
            .font_ui(cx)
            .cursor(self.cursor_style)
            .focus_visible(move |style| style.border_color(focused_border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(CONTROL_TEXT_MARGIN_LEFT)
                    .child(Label::new(self.label).single_line().truncate()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .w(COMBO_BOX_ARROW_AREA_WIDTH)
                    .h_full()
                    .justify_center()
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .when_some(self.on_click, |this, on_click| {
                this.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_click(event, window, cx)
                    })
            })
            .map(|mut this| {
                for (action, listener) in self.a11y_actions {
                    this = this.on_a11y_action(action, listener);
                }
                this
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext};

    struct ComboBoxProbe;

    impl Render for ComboBoxProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .p_4()
                .child(ComboBoxTrigger::new("combo-box", "Option"))
        }
    }

    fn painted_trigger_radius(density: settings::UiDensity, cx: &mut TestAppContext) -> f32 {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            cx.update_global::<settings::SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.theme.ui_density = Some(density);
                });
            });
        });
        let (_, cx) = cx.add_window_view(|_, _| ComboBoxProbe);
        cx.update(|window, cx| {
            let border_color = cx.theme().colors().border;
            let scale = window.scale_factor();
            // The fill and the border are painted as separate quads, so look for the border alone.
            window
                .painted_quads()
                .into_iter()
                .find(|quad| quad.border_color == border_color && quad.border_widths.top.0 > 0.)
                .map(|quad| quad.corner_radii.top_left.0 / scale)
                .expect("the combo box paints its bordered field")
        })
    }

    #[gpui::test]
    fn combo_box_uses_the_islands_component_arc(cx: &mut TestAppContext) {
        let radius = painted_trigger_radius(settings::UiDensity::Default, cx);
        assert!(
            (radius - 4.).abs() < 0.01,
            "Component.arc 8 is a 4 px radius, got {radius}"
        );
    }

    #[gpui::test]
    fn combo_box_uses_the_islands_compact_component_arc_in_compact_density(
        cx: &mut TestAppContext,
    ) {
        let radius = painted_trigger_radius(settings::UiDensity::Compact, cx);
        assert!(
            (radius - 3.).abs() < 0.01,
            "Component.arc.compact 6 is a 3 px radius, got {radius}"
        );
    }
}
