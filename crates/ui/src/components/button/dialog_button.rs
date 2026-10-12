use gpui::{AnyElement, AnyView, ClickEvent, Hsla, Role, transparent_black, white};
use theme::UiDensity;

use crate::{prelude::*, ui_density};

const HEIGHT: Pixels = px(28.);
const COMPACT_HEIGHT: Pixels = px(24.);
const CORNER_RADIUS: Pixels = px(4.);
const COMPACT_CORNER_RADIUS: Pixels = px(3.);
const MINIMUM_WIDTH: Pixels = px(72.);
const HORIZONTAL_PADDING: Pixels = px(14.);
const ICON_GAP: Pixels = px(6.);
const DEFAULT_HOVER_OPACITY: f32 = 0.9;
const DEFAULT_PRESSED_OPACITY: f32 = 0.8;

struct DialogButtonColors {
    background: Hsla,
    border: Hsla,
    hover: Hsla,
    pressed: Hsla,
    foreground: Color,
}

#[derive(IntoElement)]
pub struct DialogButton {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    start_slot: Option<AnyElement>,
    is_default: bool,
    disabled: bool,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl DialogButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            start_slot: None,
            is_default: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn default_button(mut self, is_default: bool) -> Self {
        self.is_default = is_default;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn start_slot(mut self, start_slot: impl IntoElement) -> Self {
        self.start_slot = Some(start_slot.into_any_element());
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

    fn colors(&self, cx: &App) -> DialogButtonColors {
        let theme_colors = cx.theme().colors();
        if self.disabled {
            DialogButtonColors {
                background: transparent_black(),
                border: theme_colors.border_disabled,
                hover: transparent_black(),
                pressed: transparent_black(),
                foreground: Color::Disabled,
            }
        } else if self.is_default {
            let fill = theme_colors.text_accent;
            DialogButtonColors {
                background: fill,
                border: fill,
                hover: fill.opacity(DEFAULT_HOVER_OPACITY),
                pressed: fill.opacity(DEFAULT_PRESSED_OPACITY),
                foreground: Color::Custom(white()),
            }
        } else {
            DialogButtonColors {
                background: transparent_black(),
                border: theme_colors.border,
                hover: theme_colors.ghost_element_hover,
                pressed: theme_colors.ghost_element_active,
                foreground: Color::Default,
            }
        }
    }
}

impl Disableable for DialogButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for DialogButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let compact = ui_density(cx) == UiDensity::Compact;
        let colors = self.colors(cx);
        let foreground = colors.foreground;
        let (height, corner_radius) = if compact {
            (COMPACT_HEIGHT, COMPACT_CORNER_RADIUS)
        } else {
            (HEIGHT, CORNER_RADIUS)
        };

        h_flex()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.label.clone())
            .flex_none()
            .h(height)
            .min_w(MINIMUM_WIDTH)
            .px(HORIZONTAL_PADDING)
            .justify_center()
            .gap(ICON_GAP)
            .rounded(corner_radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .font_ui(cx)
            .children(
                self.icon
                    .map(|icon| Icon::new(icon).size(IconSize::Small).color(foreground)),
            )
            .children(self.start_slot)
            .child(Label::new(self.label).color(foreground))
            .when(self.disabled, |this| this.cursor_not_allowed())
            .when(!self.disabled, |this| {
                let hover = colors.hover;
                let pressed = colors.pressed;
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
                    .when_some(self.on_click, |this, on_click| {
                        this.on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            on_click(event, window, cx)
                        })
                    })
            })
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| tooltip(window, cx))
            })
    }
}
