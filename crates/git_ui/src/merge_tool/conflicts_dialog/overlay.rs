use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use futures::channel::oneshot;
use gpui::{Hsla, SharedString, Window};
use ui::{ProgressBar, prelude::*};

use super::ConflictsDialog;
use super::messages;
use crate::merge_tool::merge_window::{ThemedImage, dialog_button, themed_image};

const OVERLAY_PANEL_WIDTH: f32 = 420.;
const OVERLAY_SCRIM_ALPHA: f32 = 0.4;
const QUESTION_IMAGE_SIZE: f32 = 28.;

pub(super) struct ConfirmOverlay {
    pub(super) title: SharedString,
    pub(super) message: SharedString,
    pub(super) yes_label: SharedString,
    pub(super) no_label: SharedString,
    pub(super) reply: Option<oneshot::Sender<bool>>,
}

pub(super) struct ProgressOverlay {
    pub(super) title: SharedString,
    pub(super) text: Option<SharedString>,
    pub(super) steps: Option<(usize, usize)>,
    pub(super) cancellation: Option<Arc<AtomicBool>>,
}

pub(super) enum Overlay {
    Confirm(ConfirmOverlay),
    Progress(ProgressOverlay),
}

impl Overlay {
    pub(super) fn is_confirmation(&self) -> bool {
        matches!(self, Overlay::Confirm(_))
    }

    pub(super) fn cancel_progress(&self) -> bool {
        match self {
            Overlay::Progress(ProgressOverlay {
                cancellation: Some(cancellation),
                ..
            }) => {
                cancellation.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }
}

impl ProgressOverlay {
    pub(super) fn indeterminate(title: &str) -> Self {
        Self {
            title: SharedString::from(title.to_string()),
            text: None,
            steps: None,
            cancellation: None,
        }
    }
}

impl ConflictsDialog {
    pub(super) fn show_progress(&mut self, overlay: ProgressOverlay, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Progress(overlay));
        cx.notify();
    }

    pub(super) fn hide_progress(&mut self, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Progress(_))) {
            self.overlay = None;
            cx.notify();
        }
    }

    pub(super) fn ask_confirmation(
        &mut self,
        title: &str,
        message: String,
        yes_label: &str,
        no_label: &str,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<bool> {
        let (sender, receiver) = oneshot::channel();
        self.overlay = Some(Overlay::Confirm(ConfirmOverlay {
            title: SharedString::from(title.to_string()),
            message: SharedString::from(message),
            yes_label: SharedString::from(yes_label.to_string()),
            no_label: SharedString::from(no_label.to_string()),
            reply: Some(sender),
        }));
        cx.notify();
        receiver
    }

    pub(super) fn answer_confirmation(
        &mut self,
        accepted: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.overlay, Some(Overlay::Confirm(_))) {
            return;
        }
        if let Some(Overlay::Confirm(mut confirmation)) = self.overlay.take()
            && let Some(reply) = confirmation.reply.take()
        {
            reply.send(accepted).ok();
        }
        cx.notify();
    }

    pub(super) fn render_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let overlay = self.overlay.as_ref()?;
        let panel = match overlay {
            Overlay::Confirm(confirmation) => self.render_confirmation(confirmation, cx),
            Overlay::Progress(progress) => self.render_progress(progress, cx),
        };
        let scrim = Hsla {
            h: 0.,
            s: 0.,
            l: 0.,
            a: OVERLAY_SCRIM_ALPHA,
        };
        Some(
            div()
                .id("conflicts-dialog-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .bg(scrim)
                .flex()
                .items_center()
                .justify_center()
                .child(panel)
                .into_any_element(),
        )
    }

    fn render_confirmation(
        &self,
        confirmation: &ConfirmOverlay,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.theme().colors();
        v_flex()
            .w(px(OVERLAY_PANEL_WIDTH))
            .p_4()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .child(Label::new(confirmation.title.clone()).weight(gpui::FontWeight::BOLD))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(themed_image(
                        ThemedImage::DialogQuestion,
                        QUESTION_IMAGE_SIZE,
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Label::new(confirmation.message.clone())),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_3()
                    .pt_2()
                    .child(
                        dialog_button(
                            "conflicts-confirm-no",
                            confirmation.no_label.clone(),
                            false,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.answer_confirmation(false, window, cx);
                        })),
                    )
                    .child(
                        dialog_button(
                            "conflicts-confirm-yes",
                            confirmation.yes_label.clone(),
                            true,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.answer_confirmation(true, window, cx);
                        })),
                    ),
            )
            .into_any_element()
    }

    fn render_progress(
        &self,
        progress: &ProgressOverlay,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.theme().colors();
        let cancellation = progress.cancellation.clone();
        v_flex()
            .w(px(OVERLAY_PANEL_WIDTH))
            .p_4()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .child(Label::new(progress.title.clone()).weight(gpui::FontWeight::BOLD))
            .when_some(progress.text.clone(), |this, text| {
                this.child(Label::new(text).color(Color::Muted))
            })
            .when_some(progress.steps, |this, (done, total)| {
                this.child(ProgressBar::new(
                    "conflicts-dialog-progress",
                    done as f32,
                    total.max(1) as f32,
                    cx,
                ))
            })
            .when_some(cancellation, |this, cancellation| {
                this.child(
                    h_flex().justify_end().child(
                        dialog_button(
                            "conflicts-progress-cancel",
                            messages::CANCEL,
                            false,
                            false,
                            cx,
                        )
                        .on_click(move |_, _, _| {
                            cancellation.store(true, Ordering::Relaxed);
                        }),
                    ),
                )
            })
            .into_any_element()
    }
}
