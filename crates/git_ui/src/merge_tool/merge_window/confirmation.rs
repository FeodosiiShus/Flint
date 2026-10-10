use gpui::{App, Div, FocusHandle, FontWeight, Img, Stateful, Window, px};
use ui::prelude::*;

use super::{
    chrome::{ThemedImage, dialog_button, themed_image},
    finish::{ConfirmationIcon, ConfirmationText},
};

pub(crate) const CONFIRMATION_KEY_CONTEXT: &str = "MergeConfirmation";
const CARD_WIDTH: f32 = 440.0;
const CARD_PADDING: f32 = 16.0;
const ICON_COLUMN_WIDTH: f32 = 32.0;
const ICON_SIZE: f32 = 28.0;

fn confirmation_icon(icon: ConfirmationIcon, cx: &App) -> Img {
    let image = match icon {
        ConfirmationIcon::Warning => ThemedImage::DialogWarning,
        ConfirmationIcon::Question => ThemedImage::DialogQuestion,
    };
    themed_image(image, ICON_SIZE, cx)
}

pub(crate) fn render_confirmation(
    text: &ConfirmationText,
    focus_handle: &FocusHandle,
    on_yes: impl Fn(&mut Window, &mut App) + 'static,
    on_no: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let yes_button = dialog_button(
        "merge-confirmation-yes",
        text.yes_label.clone(),
        true,
        false,
        cx,
    )
    .on_click(move |_, window, cx| on_yes(window, cx));
    let no_button = dialog_button(
        "merge-confirmation-no",
        text.no_label.clone(),
        false,
        false,
        cx,
    )
    .on_click(move |_, window, cx| on_no(window, cx));
    let message = h_flex()
        .gap_3()
        .items_start()
        .when_some(text.icon, |row, icon| {
            row.child(
                div()
                    .w(px(ICON_COLUMN_WIDTH))
                    .flex_none()
                    .child(confirmation_icon(icon, cx)),
            )
        })
        .child(div().flex_1().min_w_0().child(text.message.clone()));
    let card = v_flex()
        .w(px(CARD_WIDTH))
        .gap_3()
        .p(px(CARD_PADDING))
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().colors().border)
        .bg(cx.theme().colors().elevated_surface_background)
        .shadow_lg()
        .child(Label::new(text.title.clone()).weight(FontWeight::BOLD))
        .child(message)
        .child(
            h_flex()
                .justify_end()
                .gap(px(12.0))
                .child(no_button)
                .child(yes_button),
        );
    div()
        .id("merge-confirmation")
        .key_context(CONFIRMATION_KEY_CONTEXT)
        .track_focus(focus_handle)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .child(card)
}
