use gpui::{AnyView, ClickEvent, CursorStyle};

use crate::{
    ButtonCommon, ButtonSize, CLOSE_BUTTON_HOVER_ALPHA, CloseButtonSize, IconButton,
    IconButtonShape, IconName, exact_icon_size, prelude::*,
};

/// The one close (×) button of the UI: IntelliJ's `closeSmall` glyph on a circular hover fill.
///
/// Tabs, chips and fields use [`CloseButtonSize::Inline`]; dialog, popup and balloon headers use
/// [`CloseButtonSize::Header`].
#[derive(IntoElement)]
pub struct CloseButton {
    id: ElementId,
    size: CloseButtonSize,
    scale: f32,
    icon_color: Color,
    disabled: bool,
    cursor_style: CursorStyle,
    visible_on_hover: Option<SharedString>,
    aria_label: SharedString,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView + 'static>>,
}

impl CloseButton {
    pub fn new(id: impl Into<ElementId>, size: CloseButtonSize) -> Self {
        Self {
            id: id.into(),
            size,
            scale: 1.,
            icon_color: Color::Muted,
            disabled: false,
            cursor_style: CursorStyle::PointingHand,
            visible_on_hover: None,
            aria_label: SharedString::new_static("Close"),
            on_click: None,
            tooltip: None,
        }
    }

    /// Scales the button with a configured chrome icon size (see [`crate::chrome_icon_scale`]).
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn icon_color(mut self, color: Color) -> Self {
        self.icon_color = color;
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = label.into();
        self
    }

    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }

    pub fn debug_selector_for(id: &ElementId) -> String {
        format!("CLOSE_BUTTON-{id}")
    }
}

impl Clickable for CloseButton {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.cursor_style = cursor_style;
        self
    }
}

impl Disableable for CloseButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl VisibleOnHover for CloseButton {
    fn visible_on_hover(mut self, group_name: impl Into<SharedString>) -> Self {
        self.visible_on_hover = Some(group_name.into());
        self
    }
}

impl RenderOnce for CloseButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = self.size.metrics();
        let side = metrics.box_side * self.scale;
        let icon_side = metrics.icon_side * self.scale;
        let hover = cx.theme().colors().text.opacity(CLOSE_BUTTON_HOVER_ALPHA);
        let selector = Self::debug_selector_for(&self.id);

        IconButton::new(self.id, IconName::Close)
            .shape(IconButtonShape::Square)
            .size(ButtonSize::None)
            .icon_size(exact_icon_size(icon_side, window.rem_size()))
            .box_size(side)
            .corner_radius(side / 2.)
            .hover_background(hover)
            .active_background(hover)
            .icon_color(self.icon_color)
            .disabled(self.disabled)
            .cursor_style(self.cursor_style)
            .aria_label(self.aria_label)
            .debug_selector(move || selector)
            .when_some(self.on_click, |button, on_click| {
                button.on_click(move |event, window, cx| on_click(event, window, cx))
            })
            .when_some(self.tooltip, |button, tooltip| {
                button.tooltip(move |window, cx| tooltip(window, cx))
            })
            .when_some(self.visible_on_hover, |button, group| {
                button.visible_on_hover(group)
            })
    }
}
