use gpui::{Action, IntoElement, ParentElement, RenderOnce};
use ui::{Divider, List, ListBulletItem, prelude::*};

#[derive(IntoElement)]
pub struct ApiKeysWithoutProviders;

impl ApiKeysWithoutProviders {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ApiKeysWithoutProviders {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .mt_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("API Keys")
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(Divider::horizontal()),
            )
            .child(List::new().child(ListBulletItem::new("Add your own keys to use AI.")))
            .child(
                Button::new("configure-providers", "Configure Providers")
                    .full_width()
                    .style(ButtonStyle::Outlined)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(zed_actions::agent::OpenSettings.boxed_clone(), cx);
                    }),
            )
    }
}
