use gpui::{AnyView, ClickEvent, CursorStyle, Hsla, MouseButton, Role, Toggled, hsla};
use ui::{COMPONENT_CORNER_RADII, ChromeRegion, CommonAnimationExt, prelude::*};

pub(crate) const TOOLBAR_WIDGET_HEIGHT: Pixels = px(28.);
pub(crate) const TOOLBAR_WIDGET_PADDING_LEFT: Pixels = px(10.);
pub(crate) const TOOLBAR_WIDGET_PADDING_RIGHT: Pixels = px(6.);
pub(crate) const TOOLBAR_WIDGET_ICON_TEXT_GAP: Pixels = px(6.);
const TOOLBAR_WIDGET_CHEVRON_GAP: Pixels = px(2.);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ToolbarWidgetColors {
    pub(crate) rest: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) open: Hsla,
    pub(crate) border: Hsla,
    pub(crate) hover_border: Hsla,
}

impl ToolbarWidgetColors {
    pub(crate) fn current(cx: &App) -> Self {
        if cx.theme().appearance().is_light() {
            Self::with_tint(0., [0.05, 0.07, 0.11, 0.10, 0.16])
        } else {
            Self::with_tint(1., [0.07, 0.10, 0.16, 0.10, 0.16])
        }
    }

    fn with_tint(lightness: f32, [rest, hover, open, border, hover_border]: [f32; 5]) -> Self {
        Self {
            rest: hsla(0., 0., lightness, rest),
            hover: hsla(0., 0., lightness, hover),
            open: hsla(0., 0., lightness, open),
            border: hsla(0., 0., lightness, border),
            hover_border: hsla(0., 0., lightness, hover_border),
        }
    }
}

#[derive(IntoElement)]
pub(crate) struct ToolbarWidget {
    id: ElementId,
    icon: Icon,
    label: SharedString,
    loading: bool,
    toggled: bool,
    tab_index: Option<isize>,
    cursor_style: CursorStyle,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl ToolbarWidget {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        icon: Icon,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            loading: false,
            toggled: false,
            tab_index: None,
            cursor_style: CursorStyle::PointingHand,
            tooltip: None,
            on_click: None,
        }
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub(crate) fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }

    pub(crate) fn tooltip(
        mut self,
        tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }
}

impl Clickable for ToolbarWidget {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.cursor_style = cursor_style;
        self
    }
}

impl Toggleable for ToolbarWidget {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.toggled = selected;
        self
    }
}

impl RenderOnce for ToolbarWidget {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = ToolbarWidgetColors::current(cx);
        let toggled = self.toggled;
        let icon_size = ui::chrome_icon_size(ChromeRegion::TitleBar, IconSize::Medium, cx);
        let leading_icon = if self.loading {
            Icon::new(IconName::LoadCircle)
                .size(icon_size)
                .color(Color::Muted)
                .with_keyed_rotate_animation((self.id.clone(), "loading"), 2)
                .into_any_element()
        } else {
            self.icon.size(icon_size).into_any_element()
        };

        h_flex()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.label.clone())
            .aria_toggled(if toggled {
                Toggled::True
            } else {
                Toggled::False
            })
            .when_some(self.tab_index, |this, tab_index| this.tab_index(tab_index))
            .font_ui(cx)
            .flex_none()
            .h(TOOLBAR_WIDGET_HEIGHT)
            .pl(TOOLBAR_WIDGET_PADDING_LEFT)
            .pr(TOOLBAR_WIDGET_PADDING_RIGHT)
            .gap(TOOLBAR_WIDGET_ICON_TEXT_GAP)
            .rounded(COMPONENT_CORNER_RADII.toolbar_widget)
            .bg(if toggled { colors.open } else { colors.rest })
            .cursor(self.cursor_style)
            .when(!toggled, |this| {
                this.hover(move |style| style.bg(colors.hover))
            })
            .focus_visible(move |style| style.bg(colors.hover))
            .active(move |style| style.bg(colors.open))
            .when_some(self.on_click, |this, on_click| {
                this.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_click(event, window, cx)
                    })
            })
            .when_some(self.tooltip.filter(|_| !toggled), |this, tooltip| {
                this.tooltip(move |window, cx| tooltip(window, cx))
            })
            .child(leading_icon)
            .child(
                h_flex()
                    .gap(TOOLBAR_WIDGET_CHEVRON_GAP)
                    .child(Label::new(self.label))
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    ),
            )
    }
}

pub(crate) fn upstream_tracking_label(ahead: u32, behind: u32) -> Option<SharedString> {
    match (behind, ahead) {
        (0, 0) => None,
        (behind, 0) => Some(format!("↓{behind}").into()),
        (0, ahead) => Some(format!("↑{ahead}").into()),
        (behind, ahead) => Some(format!("↓{behind} ↑{ahead}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upstream_label_is_hidden_when_in_sync() {
        assert_eq!(upstream_tracking_label(0, 0), None);
    }

    #[test]
    fn test_upstream_label_shows_only_nonzero_directions() {
        assert_eq!(
            upstream_tracking_label(0, 2).map(|label| label.to_string()),
            Some("↓2".to_string())
        );
        assert_eq!(
            upstream_tracking_label(3, 0).map(|label| label.to_string()),
            Some("↑3".to_string())
        );
    }

    #[test]
    fn test_upstream_label_shows_incoming_before_outgoing() {
        assert_eq!(
            upstream_tracking_label(3, 2).map(|label| label.to_string()),
            Some("↓2 ↑3".to_string())
        );
        assert_eq!(
            upstream_tracking_label(u32::MAX, 1).map(|label| label.to_string()),
            Some(format!("↓1 ↑{}", u32::MAX))
        );
    }
}
