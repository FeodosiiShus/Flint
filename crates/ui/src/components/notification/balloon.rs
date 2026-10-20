use gpui::{ClickEvent, FontWeight};
use smallvec::SmallVec;

use crate::{
    BALLOON_CONTENT_INSETS, BALLOON_METRICS, BalloonLayoutMetrics, CloseButton, CloseButtonSize,
    prelude::*,
};

const MAX_MESSAGE_HEIGHT_FRACTION: f32 = 0.4;
const BALLOON_HOVER_GROUP: &str = "notification-balloon";

/// IntelliJ's notification balloon, the one notification surface of the app: a severity icon,
/// a title that wraps, an optional message, action links and a close (×) that shows on hover.
#[derive(IntoElement)]
pub struct Balloon {
    id: ElementId,
    icon: Icon,
    title: Option<SharedString>,
    message: Option<AnyElement>,
    actions: SmallVec<[AnyElement; 2]>,
    more_actions: Option<AnyElement>,
    header_buttons: SmallVec<[AnyElement; 1]>,
    on_close: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_hover: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
}

fn severity_glyph(severity: Severity) -> (IconName, Color) {
    match severity {
        Severity::Info => (IconName::Info, Color::Accent),
        Severity::Success => (IconName::Check, Color::Success),
        Severity::Warning => (IconName::Warning, Color::Warning),
        Severity::Error => (IconName::XCircle, Color::Error),
    }
}

pub fn severity_icon(severity: Severity) -> Icon {
    let (name, color) = severity_glyph(severity);
    Icon::new(name).size(IconSize::Medium).color(color)
}

fn title_weight(has_message: bool) -> FontWeight {
    if has_message {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    }
}

impl Balloon {
    pub fn new(id: impl Into<ElementId>, severity: Severity) -> Self {
        Self {
            id: id.into(),
            icon: severity_icon(severity),
            title: None,
            message: None,
            actions: SmallVec::new(),
            more_actions: None,
            header_buttons: SmallVec::new(),
            on_close: None,
            on_hover: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon.size(IconSize::Medium);
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        let message: SharedString = message.into();
        if !message.is_empty() {
            self.message = Some(Label::new(message).into_any_element());
        }
        self
    }

    /// A message that needs more than a label, such as Markdown.
    pub fn message_element(mut self, message: impl IntoElement) -> Self {
        self.message = Some(message.into_any_element());
        self
    }

    /// Adds an action link. IntelliJ shows two; put the rest in [`Self::more_actions`].
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let index = self.actions.len();
        let label: SharedString = label.into();
        self.actions.push(
            BalloonLink::new(("balloon-action", index), label)
                .debug_selector(format!("BALLOON_ACTION_{index}"))
                .on_click(on_click)
                .into_any_element(),
        );
        self
    }

    pub fn more_actions(mut self, trigger: impl IntoElement) -> Self {
        self.more_actions = Some(
            div()
                .debug_selector(|| "BALLOON_MORE_ACTIONS".into())
                .child(trigger)
                .into_any_element(),
        );
        self
    }

    /// A button shown next to the close button, such as Copy.
    pub fn header_button(mut self, button: impl IntoElement) -> Self {
        self.header_buttons.push(button.into_any_element());
        self
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(handler));
        self
    }

    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Balloon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let has_title = self.title.is_some();
        let has_message = self.message.is_some();
        let has_actions = !self.actions.is_empty() || self.more_actions.is_some();
        let layout = BalloonLayoutMetrics::for_content(has_title, has_message, has_actions);
        let title_weight = title_weight(has_message);

        let action_row = has_actions.then(|| {
            h_flex()
                .debug_selector(|| "BALLOON_ACTIONS".into())
                .flex_wrap()
                .items_center()
                .gap_x(BALLOON_METRICS.action_gap)
                .children(self.actions)
                .children(self.more_actions)
        });

        let message = self.message.map(|message| {
            div()
                .id("balloon-message")
                .max_h(vh(MAX_MESSAGE_HEIGHT_FRACTION, window))
                .overflow_y_scroll()
                .child(message)
        });

        let close_column = h_flex()
            .flex_none()
            .items_start()
            .gap_0p5()
            .pt(BALLOON_METRICS.close_inset
                - BALLOON_METRICS.border_stroke_width
                - BALLOON_CONTENT_INSETS.top)
            .pr(BALLOON_METRICS.close_inset
                - BALLOON_METRICS.border_stroke_width
                - BALLOON_CONTENT_INSETS.right)
            .children(self.header_buttons.into_iter().map(|button| {
                div()
                    .invisible()
                    .group_hover(BALLOON_HOVER_GROUP, |style| style.visible())
                    .child(button)
            }))
            .when_some(self.on_close, |this, on_close| {
                this.child(
                    CloseButton::new("balloon-close", CloseButtonSize::Header)
                        .visible_on_hover(BALLOON_HOVER_GROUP)
                        .on_click(move |event, window, cx| on_close(event, window, cx)),
                )
            });

        h_flex()
            .id(self.id)
            .group(BALLOON_HOVER_GROUP)
            .occlude()
            .w(BALLOON_METRICS.width)
            .items_start()
            .gap_x(BALLOON_CONTENT_INSETS.gap)
            .pt(BALLOON_CONTENT_INSETS.top)
            .pb(BALLOON_CONTENT_INSETS.bottom)
            .pl(BALLOON_CONTENT_INSETS.left)
            .pr(BALLOON_CONTENT_INSETS.right)
            .balloon_surface(cx)
            .when_some(self.on_hover, |this, on_hover| {
                this.on_hover(move |hovered, window, cx| on_hover(hovered, window, cx))
            })
            .child(
                h_flex()
                    .flex_none()
                    .w(BALLOON_METRICS.icon_column_width)
                    .pt(layout.icon_offset_y)
                    .pl(BALLOON_METRICS.icon_offset_x)
                    .child(self.icon),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pt(layout.top)
                    .pb(layout.bottom)
                    .child(
                        v_flex()
                            .gap(layout.title_to_content)
                            .when_some(self.title, |this, title| {
                                this.child(
                                    div()
                                        .debug_selector(|| "BALLOON_TITLE".into())
                                        .child(Label::new(title).weight(title_weight)),
                                )
                            })
                            .children(message),
                    )
                    .when_some(action_row, |this, action_row| {
                        this.child(
                            div()
                                .pt(layout.actions_top(has_title, has_message))
                                .child(action_row),
                        )
                    }),
            )
            .child(close_column)
    }
}

/// An accent text link, as IntelliJ draws notification actions.
#[derive(IntoElement)]
pub struct BalloonLink {
    id: ElementId,
    label: SharedString,
    debug_selector: Option<String>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl BalloonLink {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            debug_selector: None,
            on_click: None,
        }
    }

    fn debug_selector(mut self, selector: String) -> Self {
        self.debug_selector = Some(selector);
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

impl RenderOnce for BalloonLink {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .when_some(self.debug_selector, |this, selector| {
                this.debug_selector(move || selector)
            })
            .flex_none()
            .cursor_pointer()
            .text_ui(cx)
            .text_color(cx.theme().colors().text_accent)
            .hover(|style| style.underline())
            .child(self.label)
            .when_some(self.on_click, |this, on_click| {
                this.on_click(move |event, window, cx| on_click(event, window, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_icons_follow_the_intellij_balloon_colors() {
        assert!(matches!(
            severity_glyph(Severity::Info),
            (IconName::Info, Color::Accent)
        ));
        assert!(matches!(
            severity_glyph(Severity::Warning),
            (IconName::Warning, Color::Warning)
        ));
        assert!(matches!(
            severity_glyph(Severity::Error),
            (IconName::XCircle, Color::Error)
        ));
    }

    #[test]
    fn a_title_is_bold_only_when_a_message_accompanies_it() {
        assert_eq!(title_weight(true), FontWeight::BOLD);
        assert_eq!(title_weight(false), FontWeight::NORMAL);
    }
}
