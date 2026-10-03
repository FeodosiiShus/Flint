use anyhow::Context as _;
use copilot_chat::{CopilotChat, CopilotChatStatus, DeviceFlow};
use gpui::{
    App, ClipboardItem, Context, DismissEvent, Element, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, MouseDownEvent, ParentElement, Render, Styled,
    Subscription, TaskExt, Window, WindowBounds, WindowOptions, point,
};
use ui::{ButtonLike, CommonAnimationExt, ConfiguredApiCard, Vector, VectorName, prelude::*};
use util::ResultExt as _;

fn open_copilot_chat_code_verification_window(
    copilot_chat: &Entity<CopilotChat>,
    window: &Window,
    cx: &mut App,
) {
    let current_window_center = window.bounds().center();
    let height = px(450.);
    let width = px(350.);
    let window_bounds = WindowBounds::Windowed(gpui::bounds(
        current_window_center - point(height / 2.0, width / 2.0),
        gpui::size(height, width),
    ));
    cx.open_window(
        WindowOptions {
            kind: gpui::WindowKind::Floating,
            window_bounds: Some(window_bounds),
            is_resizable: false,
            is_movable: true,
            titlebar: Some(gpui::TitlebarOptions {
                appears_transparent: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| CopilotChatCodeVerification::new(&copilot_chat, window, cx)),
    )
    .context("Failed to open Copilot Chat code verification window")
    .log_err();
}

pub struct CopilotChatCodeVerification {
    status: CopilotChatStatus,
    connect_clicked: bool,
    focus_handle: FocusHandle,
    copilot_chat: Entity<CopilotChat>,
    _subscription: Subscription,
}

impl Focusable for CopilotChatCodeVerification {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for CopilotChatCodeVerification {}

impl CopilotChatCodeVerification {
    pub fn new(
        copilot_chat: &Entity<CopilotChat>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.on_window_should_close(cx, |window, cx| {
            if let Some(this) = window.root::<CopilotChatCodeVerification>().flatten() {
                this.update(cx, |this, cx| {
                    this.before_dismiss(cx);
                });
            }
            true
        });
        cx.subscribe_in(
            &cx.entity(),
            window,
            |this, _, _: &DismissEvent, window, cx| {
                window.remove_window();
                this.before_dismiss(cx);
            },
        )
        .detach();

        let status = copilot_chat.read(cx).status();
        Self {
            status,
            connect_clicked: false,
            focus_handle: cx.focus_handle(),
            copilot_chat: copilot_chat.clone(),
            _subscription: cx.observe(copilot_chat, |this, copilot_chat, cx| {
                let status = copilot_chat.read(cx).status();
                let should_dismiss = matches!(
                    status,
                    CopilotChatStatus::Authorized | CopilotChatStatus::Error(_)
                );
                this.status = status;
                cx.notify();
                if should_dismiss {
                    cx.emit(DismissEvent);
                }
            }),
        }
    }

    fn render_device_code(user_code: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let copied = cx
            .read_from_clipboard()
            .map(|item| item.text().as_deref() == Some(user_code))
            .unwrap_or(false);
        let user_code = user_code.to_owned();

        ButtonLike::new("copy-button")
            .full_width()
            .style(ButtonStyle::Tinted(ui::TintColor::Accent))
            .size(ButtonSize::Medium)
            .child(
                h_flex()
                    .w_full()
                    .p_1()
                    .justify_between()
                    .child(Label::new(user_code.clone()))
                    .child(Label::new(if copied { "Copied!" } else { "Copy" })),
            )
            .on_click(move |_, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(user_code.clone()));
                window.refresh();
            })
    }

    fn render_prompting_modal(
        connect_clicked: bool,
        device_flow: &DeviceFlow,
        cx: &mut Context<Self>,
    ) -> impl Element {
        let connect_button_label = if connect_clicked {
            "Waiting for connection…"
        } else {
            "Connect to GitHub"
        };

        v_flex()
            .flex_1()
            .gap_2p5()
            .items_center()
            .text_center()
            .child(Headline::new("Use GitHub Copilot Chat in Zed").size(HeadlineSize::Large))
            .child(
                Label::new("Using Copilot Chat requires an active subscription on GitHub.")
                    .color(Color::Muted),
            )
            .child(Self::render_device_code(&device_flow.user_code, cx))
            .child(
                Label::new("Paste this code into GitHub after clicking the button below.")
                    .color(Color::Muted),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new("connect-button", connect_button_label)
                            .full_width()
                            .style(ButtonStyle::Outlined)
                            .size(ButtonSize::Medium)
                            .on_click({
                                let verification_uri = device_flow.verification_uri.clone();
                                cx.listener(move |this, _, _window, cx| {
                                    cx.open_url(&verification_uri);
                                    this.connect_clicked = true;
                                })
                            }),
                    )
                    .child(
                        Button::new("copilot-chat-enable-cancel-button", "Cancel")
                            .full_width()
                            .size(ButtonSize::Medium)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(DismissEvent);
                            })),
                    ),
            )
    }

    fn render_enabled_modal(cx: &mut Context<Self>) -> impl Element {
        v_flex()
            .gap_2()
            .text_center()
            .justify_center()
            .child(Headline::new("Copilot Chat Enabled!").size(HeadlineSize::Large))
            .child(Label::new("You're all set to use Copilot Chat.").color(Color::Muted))
            .child(
                Button::new("copilot-chat-enabled-done-button", "Done")
                    .full_width()
                    .style(ButtonStyle::Outlined)
                    .size(ButtonSize::Medium)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
            )
    }

    fn render_error_modal(message: &str, cx: &mut Context<Self>) -> impl Element {
        v_flex()
            .gap_2()
            .text_center()
            .justify_center()
            .child(Headline::new("An Error Happened").size(HeadlineSize::Large))
            .child(Label::new(message.to_owned()).color(Color::Muted))
            .child(
                Button::new("copilot-chat-error-cancel-button", "Cancel")
                    .full_width()
                    .style(ButtonStyle::Outlined)
                    .size(ButtonSize::Medium)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
            )
    }

    fn before_dismiss(&mut self, cx: &mut Context<Self>) -> workspace::DismissDecision {
        self.copilot_chat.update(cx, |chat, cx| {
            if matches!(
                chat.status(),
                CopilotChatStatus::SigningIn { .. } | CopilotChatStatus::Starting
            ) {
                chat.sign_out(cx).detach_and_log_err(cx);
            }
        });
        workspace::DismissDecision::Dismiss(true)
    }
}

impl Render for CopilotChatCodeVerification {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prompt = match self.status.clone() {
            CopilotChatStatus::Starting | CopilotChatStatus::SignedOut => {
                Icon::new(IconName::ArrowCircle)
                    .color(Color::Muted)
                    .with_rotate_animation(2)
                    .into_any_element()
            }
            CopilotChatStatus::SigningIn { device_flow } => {
                Self::render_prompting_modal(self.connect_clicked, &device_flow, cx)
                    .into_any_element()
            }
            CopilotChatStatus::Authorized => {
                self.connect_clicked = false;
                Self::render_enabled_modal(cx).into_any_element()
            }
            CopilotChatStatus::Error(message) => {
                self.connect_clicked = false;
                Self::render_error_modal(&message, cx).into_any_element()
            }
        };

        v_flex()
            .id("copilot_chat_code_verification")
            .track_focus(&self.focus_handle(cx))
            .size_full()
            .px_4()
            .py_8()
            .gap_2()
            .items_center()
            .justify_center()
            .elevation_3(cx)
            .on_action(cx.listener(|_, _: &menu::Cancel, _, cx| {
                cx.emit(DismissEvent);
            }))
            .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                window.focus(&this.focus_handle, cx);
            }))
            .child(
                Vector::new(VectorName::ZedXCopilot, rems(8.), rems(4.))
                    .color(Color::Custom(cx.theme().colors().icon)),
            )
            .child(prompt)
    }
}

pub struct ConfigurationView {
    chat_status: Option<CopilotChatStatus>,
    is_authenticated: Box<dyn Fn(&mut App) -> bool + 'static>,
    compact: bool,
    _subscription: Option<Subscription>,
}

impl ConfigurationView {
    pub fn new(
        is_authenticated: impl Fn(&mut App) -> bool + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let copilot_chat = CopilotChat::global(cx);

        Self {
            chat_status: copilot_chat
                .as_ref()
                .map(|copilot_chat| copilot_chat.read(cx).status()),
            is_authenticated: Box::new(is_authenticated),
            _subscription: copilot_chat.as_ref().map(|copilot_chat| {
                cx.observe(copilot_chat, |this, model, cx| {
                    this.chat_status = Some(model.read(cx).status());
                    cx.notify();
                })
            }),
            compact: false,
        }
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    fn is_starting(&self) -> bool {
        matches!(&self.chat_status, Some(CopilotChatStatus::Starting))
    }

    fn is_signing_in(&self) -> bool {
        matches!(
            &self.chat_status,
            Some(CopilotChatStatus::SigningIn { .. } | CopilotChatStatus::Starting)
        )
    }

    fn is_error(&self) -> bool {
        matches!(&self.chat_status, Some(CopilotChatStatus::Error(_)))
    }

    fn has_no_status(&self) -> bool {
        self.chat_status.is_none()
    }

    fn loading_message(&self) -> Option<SharedString> {
        if self.is_starting() {
            Some("Starting Copilot Chat…".into())
        } else if self.is_signing_in() {
            Some("Signing into Copilot Chat…".into())
        } else {
            None
        }
    }

    fn render_loading_button(&self, label: impl Into<SharedString>) -> impl IntoElement {
        Button::new("loading_button", label)
            .map(|this| {
                if self.compact {
                    this.size(ButtonSize::Medium)
                } else {
                    this.full_width()
                }
            })
            .disabled(true)
            .loading(true)
            .style(ButtonStyle::Outlined)
    }

    fn render_sign_in_button(&self) -> impl IntoElement {
        Button::new("sign_in", "Sign In")
            .map(|this| {
                if self.compact {
                    this.size(ButtonSize::Medium)
                } else {
                    this.full_width()
                }
            })
            .style(ButtonStyle::Outlined)
            .on_click(move |_, window, cx| {
                if let Some(copilot_chat) = CopilotChat::global(cx) {
                    copilot_chat.update(cx, |copilot_chat, cx| copilot_chat.sign_in(cx));
                    open_copilot_chat_code_verification_window(&copilot_chat, window, cx);
                }
            })
    }

    fn render_for_chat(&self) -> impl IntoElement {
        let start_label = "To use Zed's agent with GitHub Copilot Chat, you need to be logged in to GitHub. Note that your GitHub account must have an active Copilot Chat subscription.";
        let no_status_label = "Copilot Chat requires an active GitHub Copilot subscription. Please ensure Copilot Chat is configured and try again, or use a different LLM provider.";

        let (label, button) = if let Some(message) = self.loading_message() {
            (
                start_label,
                self.render_loading_button(message).into_any_element(),
            )
        } else if self.is_error() {
            (
                "Copilot Chat had an issue signing in. Please try again.",
                self.render_sign_in_button().into_any_element(),
            )
        } else if self.has_no_status() {
            (
                no_status_label,
                self.render_sign_in_button().into_any_element(),
            )
        } else {
            (start_label, self.render_sign_in_button().into_any_element())
        };

        v_flex()
            .gap_2()
            .when(!self.compact, |this| this.child(Label::new(label)))
            .child(button)
    }
}

impl Render for ConfigurationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_authenticated = &self.is_authenticated;

        if is_authenticated(cx) {
            return ConfiguredApiCard::new("copilot-authorized", "Authorized")
                .button_label("Sign Out")
                .on_click(move |_, _window, cx| {
                    if let Some(copilot_chat) = CopilotChat::global(cx) {
                        copilot_chat
                            .update(cx, |copilot_chat, cx| copilot_chat.sign_out(cx))
                            .detach_and_log_err(cx);
                    }
                })
                .into_any_element();
        }

        self.render_for_chat().into_any_element()
    }
}
