use std::time::Duration;

use gpui::{DismissEvent, EventEmitter, FocusHandle, Focusable, Task};
use ui::{Tooltip, prelude::*};
use workspace::notifications::{Notification, SuppressEvent};

use super::{BranchNotice, NoticeSeverity};

const MAX_ACTIONS: usize = 4;
const AUTO_DISMISS_DELAY: Duration = Duration::from_secs(10);
const MAX_MESSAGE_HEIGHT_FRACTION: f32 = 0.4;

pub struct BranchNotification {
    focus_handle: FocusHandle,
    notice: BranchNotice,
    _auto_dismiss: Option<Task<()>>,
}

fn should_auto_dismiss(notice: &BranchNotice) -> bool {
    notice.severity == NoticeSeverity::Info && notice.actions.is_empty()
}

impl BranchNotification {
    pub fn new(mut notice: BranchNotice, cx: &mut Context<Self>) -> Self {
        notice.actions.truncate(MAX_ACTIONS);
        let auto_dismiss = should_auto_dismiss(&notice).then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTO_DISMISS_DELAY).await;
                this.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
            })
        });
        Self {
            focus_handle: cx.focus_handle(),
            notice,
            _auto_dismiss: auto_dismiss,
        }
    }
}

impl Focusable for BranchNotification {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for BranchNotification {}
impl EventEmitter<SuppressEvent> for BranchNotification {}

impl Notification for BranchNotification {}

impl Render for BranchNotification {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, icon_color) = match self.notice.severity {
            NoticeSeverity::Info => (IconName::Info, Color::Muted),
            NoticeSeverity::Warning => (IconName::Warning, Color::Warning),
            NoticeSeverity::Error => (IconName::XCircle, Color::Error),
        };
        let line_height = window.line_height();

        let action_buttons = self
            .notice
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let handler = action.handler.clone();
                Button::new(("branch-notification-action", index), action.label.clone())
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(move |_, _, window, cx| {
                        cx.emit(DismissEvent);
                        let app: &mut App = cx;
                        handler(window, app);
                    }))
            })
            .collect::<Vec<_>>();
        let has_actions = !action_buttons.is_empty();

        let close_button = IconButton::new("branch-notification-close", IconName::Close)
            .icon_size(IconSize::Small)
            .tooltip(Tooltip::text("Close"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)));

        let message = div()
            .id("branch-notification-message")
            .max_h(vh(MAX_MESSAGE_HEIGHT_FRACTION, window))
            .overflow_y_scroll()
            .child(Label::new(self.notice.message.clone()));

        div()
            .id("branch-notification")
            .occlude()
            .w_full()
            .elevation_3(cx)
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .items_start()
                    .child(
                        h_flex()
                            .h(line_height)
                            .justify_center()
                            .child(Icon::new(icon).size(IconSize::Small).color(icon_color)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .when_some(self.notice.title.clone(), |this, title| {
                                        this.child(Label::new(title).weight(gpui::FontWeight::BOLD))
                                    })
                                    .when(!self.notice.message.is_empty(), |this| {
                                        this.child(message)
                                    }),
                            )
                            .when(has_actions, |this| {
                                this.child(h_flex().gap_1().flex_wrap().children(action_buttons))
                            }),
                    )
                    .child(close_button),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::TestAppContext;

    use super::*;

    fn with_actions(notice: BranchNotice, count: usize) -> BranchNotice {
        (0..count).fold(notice, |notice, index| {
            notice.action(format!("Action {index}"), |_, _| {})
        })
    }

    #[test]
    fn only_info_notices_without_actions_dismiss_themselves() {
        assert!(should_auto_dismiss(&BranchNotice::info("done")));
        assert!(!should_auto_dismiss(&BranchNotice::warning("careful")));
        assert!(!should_auto_dismiss(&BranchNotice::error("broken")));
        assert!(!should_auto_dismiss(
            &BranchNotice::info("done").action("Undo", |_, _| {})
        ));
    }

    #[gpui::test]
    fn info_notice_dismisses_after_the_delay(cx: &mut TestAppContext) {
        let notification = cx.new(|cx| BranchNotification::new(BranchNotice::info("done"), cx));
        let dismissed = Rc::new(Cell::new(false));
        let _subscription = cx.update(|cx| {
            let dismissed = dismissed.clone();
            cx.subscribe(&notification, move |_, _: &DismissEvent, _| {
                dismissed.set(true)
            })
        });

        cx.executor()
            .advance_clock(AUTO_DISMISS_DELAY - Duration::from_millis(1));
        cx.run_until_parked();
        assert!(!dismissed.get(), "dismissed before the delay elapsed");

        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        assert!(dismissed.get(), "not dismissed after the delay elapsed");
    }

    #[gpui::test]
    fn notices_that_need_attention_stay_until_closed(cx: &mut TestAppContext) {
        let notifications = [
            cx.new(|cx| BranchNotification::new(BranchNotice::warning("careful"), cx)),
            cx.new(|cx| BranchNotification::new(BranchNotice::error("broken"), cx)),
            cx.new(|cx| {
                BranchNotification::new(BranchNotice::info("done").action("Undo", |_, _| {}), cx)
            }),
        ];
        let dismissed = Rc::new(Cell::new(false));
        let _subscriptions = cx.update(|cx| {
            notifications
                .iter()
                .map(|notification| {
                    let dismissed = dismissed.clone();
                    cx.subscribe(notification, move |_, _: &DismissEvent, _| {
                        dismissed.set(true)
                    })
                })
                .collect::<Vec<_>>()
        });

        cx.executor().advance_clock(AUTO_DISMISS_DELAY * 3);
        cx.run_until_parked();
        assert!(!dismissed.get());
    }

    #[gpui::test]
    fn at_most_four_actions_are_kept(cx: &mut TestAppContext) {
        let notification = cx
            .new(|cx| BranchNotification::new(with_actions(BranchNotice::error("broken"), 6), cx));
        notification.read_with(cx, |notification, _| {
            let labels: Vec<&str> = notification
                .notice
                .actions
                .iter()
                .map(|action| action.label.as_ref())
                .collect();
            assert_eq!(labels, vec!["Action 0", "Action 1", "Action 2", "Action 3"]);
        });
    }

    #[gpui::test]
    fn fewer_actions_than_the_limit_are_all_kept(cx: &mut TestAppContext) {
        let notification = cx.new(|cx| {
            BranchNotification::new(with_actions(BranchNotice::warning("careful"), 2), cx)
        });
        notification.read_with(cx, |notification, _| {
            assert_eq!(notification.notice.actions.len(), 2);
        });
    }
}
