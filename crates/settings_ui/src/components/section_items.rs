use gpui::{IntoElement, ParentElement, Role, Styled};
use ui::prelude::*;

const TITLE_TOP_INSET: Pixels = px(7.);
const TITLE_BOTTOM_INSET: Pixels = px(5.);
const SEPARATOR_LEFT_INSET: Pixels = px(6.);
const GROUP_VERTICAL_GAP: Pixels = px(20.);

#[derive(IntoElement)]
pub struct SettingsSectionHeader {
    label: SharedString,
    first: bool,
}

impl SettingsSectionHeader {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            first: false,
        }
    }

    pub fn first(mut self, first: bool) -> Self {
        self.first = first;
        self
    }
}

impl RenderOnce for SettingsSectionHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_padding = if self.first {
            TITLE_TOP_INSET
        } else {
            GROUP_VERTICAL_GAP + TITLE_TOP_INSET
        };

        h_flex()
            .id(self.label.clone())
            .role(Role::Heading)
            .aria_level(2)
            .aria_label(self.label.clone())
            .w_full()
            .min_w_0()
            .pt(top_padding)
            .pb(TITLE_BOTTOM_INSET)
            .child(Label::new(self.label).single_line())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ml(SEPARATOR_LEFT_INSET)
                    .h_px()
                    .bg(cx.theme().colors().border_variant),
            )
    }
}
