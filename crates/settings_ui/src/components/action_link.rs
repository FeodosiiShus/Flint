use gpui::{AnyView, ClickEvent, ElementId, MouseButton, Role};
use ui::{COMPONENT_CORNER_RADII, prelude::*};

use crate::components::LINK_ICON_GAP;

#[derive(IntoElement)]
pub struct SettingsActionLink {
    id: ElementId,
    label: SharedString,
    end_icon: Option<IconName>,
    tab_index: Option<isize>,
    aria_label: Option<SharedString>,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl SettingsActionLink {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            end_icon: None,
            tab_index: None,
            aria_label: None,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn end_icon(mut self, icon: IconName) -> Self {
        self.end_icon = Some(icon);
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SettingsActionLink {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused_border = cx.theme().colors().border_focused;

        h_flex()
            .id(self.id)
            .role(Role::Link)
            .aria_label(self.aria_label.unwrap_or_else(|| self.label.clone()))
            .when_some(self.tab_index, |this, tab_index| {
                this.tab_index(tab_index).tab_stop(true)
            })
            .flex_none()
            .gap(LINK_ICON_GAP)
            .rounded(COMPONENT_CORNER_RADII.button)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(move |style| style.border_color(focused_border))
            .cursor_pointer()
            .child(Label::new(self.label).color(Color::Accent).single_line())
            .when_some(self.end_icon, |this, icon| {
                this.child(Icon::new(icon).size(IconSize::Small).color(Color::Accent))
            })
            .when_some(self.on_click, |this, on_click| {
                this.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_click(event, window, cx)
                    })
            })
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| tooltip(window, cx))
            })
    }
}
