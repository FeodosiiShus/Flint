use std::rc::Rc;

use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement};
use ui::{Balloon, prelude::*};
use workspace::{ToastAction, ToastView};

#[derive(RegisterComponent)]
pub struct StatusToast {
    icon: Option<Icon>,
    text: SharedString,
    action: Option<ToastAction>,
    auto_dismiss: bool,
    this_handle: Entity<Self>,
    focus_handle: FocusHandle,
}

impl StatusToast {
    pub fn new(
        text: impl Into<SharedString>,
        cx: &mut App,
        f: impl FnOnce(Self, &mut Context<Self>) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let focus_handle = cx.focus_handle();

            f(
                Self {
                    text: text.into(),
                    icon: None,
                    action: None,
                    auto_dismiss: true,
                    this_handle: cx.entity(),
                    focus_handle,
                },
                cx,
            )
        })
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn auto_dismiss(mut self, auto_dismiss: bool) -> Self {
        self.auto_dismiss = auto_dismiss;
        self
    }

    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let this_handle = self.this_handle.clone();
        self.action = Some(ToastAction::new(
            label.into(),
            Some(Rc::new(move |window, cx| {
                this_handle.update(cx, |_, cx| {
                    cx.emit(DismissEvent);
                });
                f(window, cx);
            })),
        ));
        self
    }
}

impl Render for StatusToast {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.this_handle.clone();
        Balloon::new("status-toast", Severity::Info)
            .when_some(self.icon.clone(), |balloon, icon| balloon.icon(icon))
            .title(self.text.clone())
            .when_some(self.action.as_ref(), |balloon, action| {
                let handler = action.on_click.clone();
                balloon.action(action.label.clone(), move |_, window, cx| {
                    if let Some(handler) = handler.as_ref() {
                        handler(window, cx);
                    }
                })
            })
            .on_close(move |_, _, cx| {
                handle.update(cx, |_, cx| cx.emit(DismissEvent));
            })
    }
}

impl ToastView for StatusToast {
    fn action(&self) -> Option<ToastAction> {
        self.action.clone()
    }

    fn auto_dismiss(&self) -> bool {
        self.auto_dismiss
    }
}

impl Focusable for StatusToast {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for StatusToast {}

impl Component for StatusToast {
    fn scope() -> ComponentScope {
        ComponentScope::Notification
    }

    fn description() -> &'static str {
        "A compact, transient toast used to surface status updates \
        such as completed operations or pending updates, with optional icon, \
        action, and dismiss affordances."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let text_example = StatusToast::new("Operation completed", cx, |this, _| this);

        let action_example = StatusToast::new("Update ready to install", cx, |this, _cx| {
            this.action("Restart", |_, _| {})
        });

        let icon_example = StatusToast::new(
            "Nathan Sobo accepted your contact request",
            cx,
            |this, _| {
                this.icon(
                    Icon::new(IconName::Check)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
            },
        );

        let success_example = StatusToast::new("Pushed 4 changes to `zed/main`", cx, |this, _| {
            this.icon(
                Icon::new(IconName::Check)
                    .size(IconSize::Small)
                    .color(Color::Success),
            )
        });

        let error_example = StatusToast::new(
            "git push: Couldn't find remote origin `iamnbutler/zed`",
            cx,
            |this, _cx| {
                this.icon(
                    Icon::new(IconName::XCircle)
                        .size(IconSize::Small)
                        .color(Color::Error),
                )
                .action("More Info", |_, _| {})
            },
        );

        let warning_example = StatusToast::new("You have outdated settings", cx, |this, _cx| {
            this.icon(
                Icon::new(IconName::Warning)
                    .size(IconSize::Small)
                    .color(Color::Warning),
            )
            .action("More Info", |_, _| {})
        });

        let pr_example =
            StatusToast::new("`zed/new-notification-system` created!", cx, |this, _cx| {
                this.icon(
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .action("Open Pull Request", |_, cx| {
                    cx.open_url("https://github.com/")
                })
            });

        v_flex()
            .gap_6()
            .p_4()
            .children(vec![
                example_group_with_title(
                    "Basic Toast",
                    vec![
                        single_example("Text", div().child(text_example).into_any_element()),
                        single_example("Action", div().child(action_example).into_any_element()),
                        single_example("Icon", div().child(icon_example).into_any_element()),
                    ],
                ),
                example_group_with_title(
                    "Examples",
                    vec![
                        single_example("Success", div().child(success_example).into_any_element()),
                        single_example("Error", div().child(error_example).into_any_element()),
                        single_example("Warning", div().child(warning_example).into_any_element()),
                        single_example("Create PR", div().child(pr_example).into_any_element()),
                    ],
                )
                .vertical(),
            ])
            .into_any_element()
    }
}
