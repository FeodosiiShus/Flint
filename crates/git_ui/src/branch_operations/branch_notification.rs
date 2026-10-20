use std::time::{Duration, Instant};

use gpui::{Anchor, DismissEvent, EventEmitter, FocusHandle, Focusable, Task};
use ui::{
    BALLOON_MAX_VISIBLE_ACTIONS, Balloon, Button, ButtonSize, ButtonStyle, ContextMenu,
    ContextMenuEntry, PopoverMenu, prelude::*,
};
use util::ResultExt as _;
use workspace::notifications::{Notification, SuppressEvent};

use super::{BranchNotice, NoticeSeverity};

const AUTO_DISMISS_DELAY: Duration = Duration::from_secs(10);
const MINIMUM_RESUME_DELAY: Duration = Duration::from_millis(800);
const MORE_ACTIONS_LABEL: &str = "More";

// IntelliJ keeps every branch notification in the Notifications tool window, so
// actions like Restore stay reachable; without that history a notice with
// actions stays until it is clicked or closed.
fn should_auto_dismiss(notice: &BranchNotice) -> bool {
    notice.severity == NoticeSeverity::Info && notice.actions.is_empty()
}

fn severity(severity: NoticeSeverity) -> Severity {
    match severity {
        NoticeSeverity::Info => Severity::Info,
        NoticeSeverity::Warning => Severity::Warning,
        NoticeSeverity::Error => Severity::Error,
    }
}

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

impl BranchNotification {
    pub fn new(notice: BranchNotice, cx: &mut Context<Self>) -> Self {
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

    #[cfg(test)]
    pub(crate) fn notice(&self) -> &BranchNotice {
        &self.notice
    }

    pub(crate) fn invoke_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.notice.actions.get(index).cloned() else {
            return;
        };
        if action.expires {
            cx.emit(DismissEvent);
        }
        let app: &mut App = cx;
        (action.handler)(window, app);
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
            this.update(cx, |_, cx| cx.emit(DismissEvent)).log_err();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hidden_actions = self
            .notice
            .actions
            .iter()
            .skip(BALLOON_MAX_VISIBLE_ACTIONS)
            .map(|action| (action.label.clone(), action.handler.clone(), action.expires))
            .collect::<Vec<_>>();
        let more_actions = (!hidden_actions.is_empty()).then(|| {
            let notification = cx.entity().downgrade();
            PopoverMenu::new("branch-notification-more-actions")
                .anchor(Anchor::BottomLeft)
                .menu(move |window, cx| {
                    let hidden_actions = hidden_actions.clone();
                    let notification = notification.clone();
                    Some(ContextMenu::build(window, cx, move |menu, _, _| {
                        hidden_actions
                            .iter()
                            .fold(menu, |menu, (label, handler, expires)| {
                                let handler = handler.clone();
                                let notification = notification.clone();
                                let expires = *expires;
                                menu.item(ContextMenuEntry::new(label.clone()).handler(
                                    move |window, cx| {
                                        if expires {
                                            notification
                                                .update(cx, |_, cx| cx.emit(DismissEvent))
                                                .log_err();
                                        }
                                        handler(window, cx);
                                    },
                                ))
                            })
                    }))
                })
                .trigger(
                    Button::new(
                        "branch-notification-more-actions-trigger",
                        MORE_ACTIONS_LABEL,
                    )
                    .color(Color::Accent)
                    .style(ButtonStyle::Transparent)
                    .size(ButtonSize::None),
                )
        });

        let balloon = Balloon::new("branch-notification", severity(self.notice.severity))
            .when_some(self.notice.title.clone(), |balloon, title| {
                balloon.title(title)
            })
            .message(self.notice.message.clone())
            .on_close(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .on_hover(
                cx.listener(|this, hovered: &bool, _window, cx| this.set_hovered(*hovered, cx)),
            );
        let balloon = self
            .notice
            .actions
            .iter()
            .take(BALLOON_MAX_VISIBLE_ACTIONS)
            .enumerate()
            .fold(balloon, |balloon, (index, action)| {
                balloon.action(
                    action.label.clone(),
                    cx.listener(move |this, _, window, cx| {
                        this.invoke_action(index, window, cx);
                    }),
                )
            });
        balloon.when_some(more_actions, |balloon, more_actions| {
            balloon.more_actions(more_actions)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{Entity, TestAppContext};
    use settings::SettingsStore;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

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
    fn only_info_notices_without_actions_dismiss_themselves() {
        assert!(should_auto_dismiss(&BranchNotice::info("done")));
        assert!(!should_auto_dismiss(
            &BranchNotice::info("done").action("Undo", |_, _| {})
        ));
        assert!(!should_auto_dismiss(&BranchNotice::warning("careful")));
        assert!(!should_auto_dismiss(&BranchNotice::error("broken")));
    }

    #[test]
    fn notice_severities_pick_the_matching_balloon_severity() {
        assert_eq!(severity(NoticeSeverity::Info), Severity::Info);
        assert_eq!(severity(NoticeSeverity::Warning), Severity::Warning);
        assert_eq!(severity(NoticeSeverity::Error), Severity::Error);
    }

    #[gpui::test]
    fn info_notice_dismisses_after_the_delay(cx: &mut TestAppContext) {
        init_test(cx);
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
    fn info_notice_with_actions_stays_until_clicked_or_closed(cx: &mut TestAppContext) {
        init_test(cx);
        let notification = cx.new(|cx| {
            BranchNotification::new(with_actions(BranchNotice::info("Deleted Branch: x"), 2), cx)
        });
        let (dismissed, _subscription) = track_dismissal(&notification, cx);

        cx.executor().advance_clock(AUTO_DISMISS_DELAY * 3);
        cx.run_until_parked();
        assert!(
            !dismissed.get(),
            "a balloon whose actions would be lost disappeared on its own"
        );
    }

    #[gpui::test]
    fn an_action_that_keeps_the_notice_leaves_it_open(cx: &mut TestAppContext) {
        init_test(cx);
        let invoked = Rc::new(Cell::new(0));
        let notice = BranchNotice::info("Deleted Branch: x")
            .action("Restore", |_, _| {})
            .action_keeping_notice("View Commits", {
                let invoked = invoked.clone();
                move |_, _| invoked.set(invoked.get() + 1)
            });
        let (notification, cx) = cx.add_window_view(|_, cx| BranchNotification::new(notice, cx));
        let dismissed = Rc::new(Cell::new(false));
        let _subscription = cx.update(|_, cx| {
            let dismissed = dismissed.clone();
            cx.subscribe(&notification, move |_, _: &DismissEvent, _| {
                dismissed.set(true)
            })
        });

        notification.update_in(cx, |notification, window, cx| {
            notification.invoke_action(1, window, cx)
        });
        assert_eq!(invoked.get(), 1);
        assert!(!dismissed.get(), "View Commits closed the notification");

        notification.update_in(cx, |notification, window, cx| {
            notification.invoke_action(0, window, cx)
        });
        assert!(dismissed.get(), "Restore left the notification open");
    }

    #[gpui::test]
    fn warnings_and_errors_stay_until_closed(cx: &mut TestAppContext) {
        init_test(cx);
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
        init_test(cx);
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
    fn three_actions_show_two_links_and_a_more_link(cx: &mut TestAppContext) {
        init_test(cx);
        let (_notification, cx) = cx.add_window_view(|_, cx| {
            BranchNotification::new(with_actions(BranchNotice::error("broken"), 3), cx)
        });

        assert!(
            cx.debug_bounds("BALLOON_ACTION_0").is_some(),
            "the first action stays a visible link"
        );
        assert!(
            cx.debug_bounds("BALLOON_ACTION_1").is_some(),
            "the second action stays a visible link"
        );
        assert_eq!(
            cx.debug_bounds("BALLOON_ACTION_2"),
            None,
            "the third action is not shown as a link"
        );
        assert!(
            cx.debug_bounds("BALLOON_MORE_ACTIONS").is_some(),
            "everything past the first two actions is reachable through More"
        );
    }

    #[gpui::test]
    fn two_actions_need_no_more_link(cx: &mut TestAppContext) {
        init_test(cx);
        let (_notification, cx) = cx.add_window_view(|_, cx| {
            BranchNotification::new(with_actions(BranchNotice::warning("careful"), 2), cx)
        });

        assert!(cx.debug_bounds("BALLOON_ACTION_1").is_some());
        assert_eq!(
            cx.debug_bounds("BALLOON_MORE_ACTIONS"),
            None,
            "a balloon with two actions has nothing to hide"
        );
    }

    #[gpui::test]
    fn every_action_past_the_first_two_is_kept_for_the_more_menu(cx: &mut TestAppContext) {
        init_test(cx);
        let notification = cx
            .new(|cx| BranchNotification::new(with_actions(BranchNotice::error("broken"), 5), cx));
        notification.read_with(cx, |notification, _| {
            let labels: Vec<&str> = notification
                .notice
                .actions
                .iter()
                .map(|action| action.label.as_ref())
                .collect();
            assert_eq!(
                labels,
                vec!["Action 0", "Action 1", "Action 2", "Action 3", "Action 4"]
            );
        });
    }

    #[gpui::test]
    fn fewer_actions_than_the_visible_budget_are_all_kept(cx: &mut TestAppContext) {
        init_test(cx);
        let notification = cx.new(|cx| {
            BranchNotification::new(with_actions(BranchNotice::warning("careful"), 2), cx)
        });
        notification.read_with(cx, |notification, _| {
            assert_eq!(notification.notice.actions.len(), 2);
        });
    }
}
