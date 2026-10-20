use editor::{Editor, EditorEvent};
use git::Oid;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window,
};
use menu::{Cancel, Confirm};
use ui::{DIALOG_METRICS, DialogButton, DialogWidth, Modal, ModalFooter, ModalHeader, prelude::*};
use workspace::ModalView;

use crate::branch_operations::{BranchContext, error_notice};

const TAG_NAME_LABEL: &str = "Enter the name of new tag";
const OK_LABEL: &str = "OK";
const CANCEL_LABEL: &str = "Cancel";

fn dialog_title(reference: &str) -> String {
    format!("Create New Tag On {reference}")
}

fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_whitespace)
}

pub(super) fn open(context: BranchContext, commit: Oid, window: &mut Window, cx: &mut App) {
    let dialog_context = context.clone();
    context.open_modal(window, cx, move |window, cx| {
        NewTagDialog::new(dialog_context, commit, window, cx)
    });
}

struct NewTagDialog {
    context: BranchContext,
    reference: String,
    title: SharedString,
    editor: Entity<Editor>,
    _editor_subscription: Subscription,
}

impl NewTagDialog {
    fn new(
        context: BranchContext,
        commit: Oid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let reference = commit.to_string();
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let editor_subscription =
            cx.subscribe_in(&editor, window, |_, _, event: &EditorEvent, _, cx| {
                if let EditorEvent::BufferEdited = event {
                    cx.notify();
                }
            });
        Self {
            context,
            title: dialog_title(&reference).into(),
            reference,
            editor,
            _editor_subscription: editor_subscription,
        }
    }

    fn tag_name(&self, cx: &App) -> String {
        self.editor.read(cx).text(cx)
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, _window: &mut Window, cx: &mut Context<Self>) {
        self.submit(cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let name = self.tag_name(cx);
        if !is_valid_tag_name(&name) {
            return;
        }
        cx.emit(DismissEvent);
        create_tag(self.context.clone(), name, self.reference.clone(), cx);
    }
}

fn create_tag(context: BranchContext, name: String, reference: String, cx: &mut App) {
    let receiver = context.repository.update(cx, |repository, _| {
        repository.create_tag(name.clone(), reference)
    });
    cx.spawn(async move |cx| {
        let Ok(Err(error)) = receiver.await else {
            return;
        };
        cx.update(|cx| {
            if let Some(notice) = error_notice("tag", &error) {
                context.notify(notice.title(format!("Could not create tag {name}")), cx);
            }
        });
    })
    .detach();
}

impl EventEmitter<DismissEvent> for NewTagDialog {}

impl ModalView for NewTagDialog {}

impl Focusable for NewTagDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for NewTagDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_confirm = is_valid_tag_name(&self.tag_name(cx));
        v_flex()
            .key_context("NewTagDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .dialog_surface(DialogWidth::Small, window, cx)
            .child(
                Modal::new("new-tag-dialog", None)
                    .header(
                        ModalHeader::new()
                            .headline(self.title.clone())
                            .show_dismiss_button(true),
                    )
                    .child(
                        v_flex()
                            .px(DIALOG_METRICS.padding_x)
                            .py_2()
                            .gap_2()
                            .child(Label::new(TAG_NAME_LABEL))
                            .child(
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .bg(cx.theme().colors().editor_background)
                                    .child(self.editor.clone()),
                            ),
                    )
                    .footer(
                        ModalFooter::new()
                            .end_button(
                                DialogButton::new("new-tag-cancel", CANCEL_LABEL)
                                    .default_button(false)
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                            )
                            .end_button(
                                DialogButton::new("new-tag-confirm", OK_LABEL)
                                    .default_button(true)
                                    .disabled(!can_confirm)
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_must_be_non_empty_and_free_of_whitespace() {
        assert!(is_valid_tag_name("v1.0.0"));
        assert!(!is_valid_tag_name(""));
        assert!(!is_valid_tag_name("release 1"));
        assert!(!is_valid_tag_name("v1\t"));
    }

    #[test]
    fn dialog_title_names_the_full_reference() {
        assert_eq!(
            dialog_title("0123456789abcdef0123456789abcdef01234567"),
            "Create New Tag On 0123456789abcdef0123456789abcdef01234567"
        );
    }
}
