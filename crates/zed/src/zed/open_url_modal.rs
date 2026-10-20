use editor::Editor;
use gpui::{AppContext as _, DismissEvent, Entity, EventEmitter, Focusable, ReadGlobal, Styled};
use ui::{
    ActiveTheme, App, Color, Context, DialogWidth, FluentBuilder, InteractiveElement, IntoElement,
    Label, LabelCommon, LabelSize, Modal, ModalHeader, ParentElement, Render, Section,
    SharedString, StyledSurface, Window, component_corner_radii, div, v_flex,
};
use workspace::ModalView;

use super::{OpenListener, RawOpenRequest};

pub struct OpenUrlModal {
    editor: Entity<Editor>,
    last_error: Option<SharedString>,
}

impl EventEmitter<DismissEvent> for OpenUrlModal {}
impl ModalView for OpenUrlModal {}

impl Focusable for OpenUrlModal {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl OpenUrlModal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("flint://...", window, cx);
            editor
        });

        Self {
            editor,
            last_error: None,
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.editor.update(cx, |editor, cx| {
            let text = editor.text(cx).trim().to_string();
            editor.clear(window, cx);
            text
        });

        if url.is_empty() {
            cx.emit(DismissEvent);
            return;
        }

        // Handle flint:// and legacy zed:// URLs internally.
        if url.starts_with("flint://") || url.starts_with("zed://") || url.starts_with("zed-cli://")
        {
            OpenListener::global(cx).open(RawOpenRequest {
                urls: vec![url],
                ..Default::default()
            });
            cx.emit(DismissEvent);
            return;
        }

        match url::Url::parse(&url) {
            Ok(parsed_url) => {
                cx.open_url(parsed_url.as_str());
                cx.emit(DismissEvent);
            }
            Err(e) => {
                self.last_error = Some(format!("Invalid URL: {}", e).into());
                cx.notify();
            }
        }
    }
}

impl Render for OpenUrlModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();

        v_flex()
            .key_context("OpenUrlModal")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .dialog_surface(DialogWidth::Small, window, cx)
            .child(
                Modal::new("open-url-modal", None)
                    .header(
                        ModalHeader::new()
                            .headline("Open URL")
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new()
                            .child(
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .rounded(component_corner_radii(cx).input)
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.editor_background)
                                    .child(self.editor.clone()),
                            )
                            .when_some(self.last_error.clone(), |this, error| {
                                this.child(
                                    Label::new(error).size(LabelSize::Small).color(Color::Error),
                                )
                            })
                            .when(self.last_error.is_none(), |this| {
                                this.child(
                                    Label::new("Paste a URL to open.")
                                        .color(Color::Muted)
                                        .size(LabelSize::Small),
                                )
                            }),
                    ),
            )
    }
}
