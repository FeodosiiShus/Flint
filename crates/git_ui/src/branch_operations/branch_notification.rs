use std::time::{Duration, Instant};

use gpui::{DismissEvent, EventEmitter, FocusHandle, Focusable, Task};
use ui::{Tooltip, prelude::*};
use workspace::notifications::{Notification, SuppressEvent};

use super::{BranchNotice, NoticeSeverity};

const MAX_ACTIONS: usize = 4;
const AUTO_DISMISS_DELAY: Duration = Duration::from_secs(10);
const MINIMUM_RESUME_DELAY: Duration = Duration::from_millis(800);
const MAX_MESSAGE_HEIGHT_FRACTION: f32 = 0.4;
const BALLOON_WIDTH: Pixels = px(360.);
const BALLOON_VERTICAL_PADDING: Pixels = px(12.);
const BALLOON_HORIZONTAL_PADDING: Pixels = px(14.);
const ACTION_LINK_GAP: Pixels = px(16.);
const BALLOON_HOVER_GROUP: &str = "branch-notification-balloon";

pub struct BranchNotification {
    focus_handle: FocusHandle,
    notice: BranchNotice,
    auto_dismiss: Option<AutoDismiss>,
}

struct AutoDismiss {
    remaining: Duration,
    countdown: Option<Countdown>,
}

struct Countdown {
    started_at: Instant,
    _timer: Task<()>,
}

fn should_auto_dismiss(notice: &BranchNotice) -> bool {
    notice.severity == NoticeSeverity::Info
}

fn severity_icon(severity: NoticeSeverity) -> (IconName, Color) {
    match severity {
        NoticeSeverity::Info => (IconName::Info, Color::Accent),
        NoticeSeverity::Warning => (IconName::Warning, Color::Warning),
        NoticeSeverity::Error => (IconName::XCircle, Color::Error),
    }
}

impl BranchNotification {
    pub fn new(mut notice: BranchNotice, cx: &mut Context<Self>) -> Self {
        notice.actions.truncate(MAX_ACTIONS);
        let auto_dismiss = should_auto_dismiss(&notice).then_some(AutoDismiss {
            remaining: AUTO_DISMISS_DELAY,
            countdown: None,
        });
        let mut notification = Self {
            focus_handle: cx.focus_handle(),
            notice,
            auto_dismiss,
        };
        notification.resume_countdown(cx);
        notification
    }

    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.pause_countdown(cx);
        } else {
            self.resume_countdown(cx);
        }
    }

    fn resume_countdown(&mut self, cx: &mut Context<Self>) {
        let Some(auto_dismiss) = self.auto_dismiss.as_mut() else {
            return;
        };
        if auto_dismiss.countdown.is_some() {
            return;
        }
        let delay = auto_dismiss.remaining;
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
        });
        auto_dismiss.countdown = Some(Countdown {
            started_at: cx.background_executor().now(),
            _timer: timer,
        });
    }

    fn pause_countdown(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(auto_dismiss) = self.auto_dismiss.as_mut() else {
            return;
        };
        let Some(countdown) = auto_dismiss.countdown.take() else {
            return;
        };
        let elapsed = now.saturating_duration_since(countdown.started_at);
        auto_dismiss.remaining = auto_dismiss
            .remaining
            .saturating_sub(elapsed)
            .max(MINIMUM_RESUME_DELAY);
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
        let (icon, icon_color) = severity_icon(self.notice.severity);
        let line_height = window.line_height();
        let link_color = cx.theme().colors().text_accent;

        let action_links = self
            .notice
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let handler = action.handler.clone();
                div()
                    .id(("branch-notification-action", index))
                    .flex_none()
                    .cursor_pointer()
                    .text_ui(cx)
                    .text_color(link_color)
                    .hover(|style| style.underline())
                    .child(action.label.clone())
                    .on_click(cx.listener(move |_, _, window, cx| {
                        cx.emit(DismissEvent);
                        let app: &mut App = cx;
                        handler(window, app);
                    }))
            })
            .collect::<Vec<_>>();
        let has_actions = !action_links.is_empty();

        let close_button = IconButton::new("branch-notification-close", IconName::Close)
            .icon_size(IconSize::Small)
            .size(ButtonSize::Compact)
            .tooltip(Tooltip::text("Close"))
            .visible_on_hover(BALLOON_HOVER_GROUP)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)));

        let message = div()
            .id("branch-notification-message")
            .max_h(vh(MAX_MESSAGE_HEIGHT_FRACTION, window))
            .overflow_y_scroll()
            .child(Label::new(self.notice.message.clone()));

        h_flex().w_full().justify_end().child(
            h_flex()
                .id("branch-notification")
                .group(BALLOON_HOVER_GROUP)
                .occlude()
                .w(BALLOON_WIDTH)
                .items_start()
                .gap_2()
                .py(BALLOON_VERTICAL_PADDING)
                .px(BALLOON_HORIZONTAL_PADDING)
                .elevation_3(cx)
                .on_hover(
                    cx.listener(|this, hovered: &bool, _window, cx| this.set_hovered(*hovered, cx)),
                )
                .child(
                    h_flex()
                        .flex_none()
                        .h(line_height)
                        .child(Icon::new(icon).size(IconSize::Medium).color(icon_color)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(
                            v_flex()
                                .gap_1()
                                .when_some(self.notice.title.clone(), |this, title| {
                                    this.child(Label::new(title).weight(gpui::FontWeight::BOLD))
                                })
                                .when(!self.notice.message.is_empty(), |this| this.child(message)),
                        )
                        .when(has_actions, |this| {
                            this.child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_x(ACTION_LINK_GAP)
                                    .gap_y_1()
                                    .children(action_links),
                            )
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

    use gpui::{Entity, TestAppContext};

    use super::*;

    fn with_actions(notice: BranchNotice, count: usize) -> BranchNotice {
        (0..count).fold(notice, |notice, index| {
            notice.action(format!("Action {index}"), |_, _| {})
        })
    }

    fn track_dismissal(
        notification: &Entity<BranchNotification>,
        cx: &mut TestAppContext,
    ) -> (Rc<Cell<bool>>, gpui::Subscription) {
        let dismissed = Rc::new(Cell::new(false));
        let subscription = cx.update(|cx| {
            let dismissed = dismissed.clone();
            cx.subscribe(notification, move |_, _: &DismissEvent, _| {
                dismissed.set(true)
            })
        });
        (dismissed, subscription)
    }

    #[test]
    fn info_notices_dismiss_themselves_even_with_actions() {
        assert!(should_auto_dismiss(&BranchNotice::info("done")));
        assert!(should_auto_dismiss(
            &BranchNotice::info("done").action("Undo", |_, _| {})
        ));
        assert!(!should_auto_dismiss(&BranchNotice::warning("careful")));
        assert!(!should_auto_dismiss(&BranchNotice::error("broken")));
    }

    #[test]
    fn severity_icons_follow_the_intellij_balloon_colors() {
        assert!(matches!(
            severity_icon(NoticeSeverity::Info),
            (IconName::Info, Color::Accent)
        ));
        assert!(matches!(
            severity_icon(NoticeSeverity::Warning),
            (IconName::Warning, Color::Warning)
        ));
        assert!(matches!(
            severity_icon(NoticeSeverity::Error),
            (IconName::XCircle, Color::Error)
        ));
    }

    #[gpui::test]
    fn info_notice_dismisses_after_the_delay(cx: &mut TestAppContext) {
        let notification = cx.new(|cx| BranchNotification::new(BranchNotice::info("done"), cx));
        let (dismissed, _subscription) = track_dismissal(&notification, cx);

        cx.executor()
            .advance_clock(AUTO_DISMISS_DELAY - Duration::from_millis(1));
        cx.run_until_parked();
        assert!(!dismissed.get(), "dismissed before the delay elapsed");

        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        assert!(dismissed.get(), "not dismissed after the delay elapsed");
    }

    #[gpui::test]
    fn info_notice_with_actions_dismisses_after_the_delay(cx: &mut TestAppContext) {
        let notification =
            cx.new(|cx| BranchNotification::new(with_actions(BranchNotice::info("Pushed"), 2), cx));
        let (dismissed, _subscription) = track_dismissal(&notification, cx);

        cx.executor()
            .advance_clock(AUTO_DISMISS_DELAY - Duration::from_millis(1));
        cx.run_until_parked();
        assert!(!dismissed.get(), "dismissed before the delay elapsed");

        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        assert!(
            dismissed.get(),
            "an info balloon with actions stayed after the delay"
        );
    }

    #[gpui::test]
    fn warnings_and_errors_stay_until_closed(cx: &mut TestAppContext) {
        let notifications = [
            cx.new(|cx| BranchNotification::new(BranchNotice::warning("careful"), cx)),
            cx.new(|cx| BranchNotification::new(BranchNotice::error("broken"), cx)),
            cx.new(|cx| {
                BranchNotification::new(with_actions(BranchNotice::error("broken"), 1), cx)
            }),
        ];
        let trackers = notifications
            .iter()
            .map(|notification| track_dismissal(notification, cx))
            .collect::<Vec<_>>();

        cx.executor().advance_clock(AUTO_DISMISS_DELAY * 3);
        cx.run_until_parked();
        assert!(trackers.iter().all(|(dismissed, _)| !dismissed.get()));
    }

    #[gpui::test]
    fn hovering_pauses_the_auto_dismiss_countdown(cx: &mut TestAppContext) {
        let notification = cx.new(|cx| BranchNotification::new(BranchNotice::info("done"), cx));
        let (dismissed, _subscription) = track_dismissal(&notification, cx);

        cx.executor().advance_clock(AUTO_DISMISS_DELAY / 2);
        cx.run_until_parked();
        notification.update(cx, |notification, cx| notification.set_hovered(true, cx));
        cx.executor().advance_clock(AUTO_DISMISS_DELAY * 3);
        cx.run_until_parked();
        assert!(!dismissed.get(), "dismissed while hovered");

        notification.update(cx, |notification, cx| notification.set_hovered(false, cx));
        cx.executor()
            .advance_clock(AUTO_DISMISS_DELAY / 2 - Duration::from_millis(1));
        cx.run_until_parked();
        assert!(
            !dismissed.get(),
            "dismissed before the time left at hover elapsed"
        );

        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        assert!(
            dismissed.get(),
            "the countdown did not resume with the time left at hover"
        );
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
