use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window, rems,
};
use menu::{Cancel, Confirm};
use ui::{Checkbox, Modal, ModalFooter, ModalHeader, Section, prelude::*};
use workspace::ModalView;

use super::manage::perform_rename;
use super::new_branch_dialog::{clean_up_branch_name_on_apply, clean_up_branch_name_on_typing};
use super::{BranchContext, opaque_elevated_surface};

const DIALOG_WIDTH: f32 = 34.;

#[derive(Debug, PartialEq, Eq)]
pub enum RenameValidation {
    Unchanged,
    Valid(String),
    Invalid(String),
}

pub fn validate_rename(
    old_name: &str,
    input: &str,
    local_branches: &[SharedString],
    remote_branches: &[SharedString],
) -> RenameValidation {
    let name = clean_up_branch_name_on_apply(input);
    if name.is_empty() {
        return RenameValidation::Invalid("Specify name for the new branch".to_string());
    }
    if name == old_name {
        return RenameValidation::Unchanged;
    }
    if name == "HEAD" {
        return RenameValidation::Invalid("HEAD is a reserved keyword".to_string());
    }
    if remote_branches.iter().any(|remote| remote.as_ref() == name) {
        return RenameValidation::Invalid(format!(
            "Branch name {name} conflicts with remote branch with the same name"
        ));
    }
    let directory_prefix = format!("{name}/");
    if local_branches
        .iter()
        .any(|local| &**local != old_name && local.starts_with(&directory_prefix))
    {
        return RenameValidation::Invalid(format!(
            "Branch name {name} conflicts with local branch directory with the same name"
        ));
    }
    if local_branches.iter().any(|local| local.as_ref() == name) {
        return RenameValidation::Invalid(format!("Branch name {name} already exists."));
    }
    RenameValidation::Valid(name)
}

pub struct RenameBranchDialog {
    context: BranchContext,
    old_name: SharedString,
    editor: Entity<Editor>,
    has_upstream: bool,
    unset_upstream: bool,
    local_branches: Vec<SharedString>,
    remote_branches: Vec<SharedString>,
    _editor_subscription: Subscription,
}

impl RenameBranchDialog {
    pub fn new(
        context: BranchContext,
        old_name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = context.snapshot(cx);
        let has_upstream = snapshot.branch_list.iter().any(|branch| {
            !branch.is_remote() && branch.name() == &*old_name && branch.upstream.is_some()
        });
        let local_branches: Vec<SharedString> = snapshot
            .branch_list
            .iter()
            .filter(|branch| !branch.is_remote())
            .map(|branch| SharedString::from(branch.name().to_string()))
            .collect();
        let remote_branches: Vec<SharedString> = snapshot
            .branch_list
            .iter()
            .filter(|branch| branch.is_remote())
            .map(|branch| SharedString::from(branch.name().to_string()))
            .collect();

        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(old_name.to_string(), window, cx);
            editor
        });
        let editor_subscription = cx.subscribe_in(
            &editor,
            window,
            |_, editor, event: &EditorEvent, window, cx| {
                if !matches!(event, EditorEvent::BufferEdited) {
                    return;
                }
                let text = editor.read(cx).text(cx);
                let cleaned = clean_up_branch_name_on_typing(&text);
                if cleaned != text {
                    editor.update(cx, |editor, cx| editor.set_text(cleaned, window, cx));
                }
                cx.notify();
            },
        );

        Self {
            context,
            old_name,
            editor,
            has_upstream,
            unset_upstream: false,
            local_branches,
            remote_branches,
            _editor_subscription: editor_subscription,
        }
    }

    fn validation(&self, cx: &App) -> RenameValidation {
        validate_rename(
            &self.old_name,
            &self.editor.read(cx).text(cx),
            &self.local_branches,
            &self.remote_branches,
        )
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        match self.validation(cx) {
            RenameValidation::Valid(new_name) => {
                perform_rename(
                    self.context.clone(),
                    self.old_name.clone(),
                    new_name.into(),
                    self.has_upstream && self.unset_upstream,
                    window,
                    cx,
                );
                cx.emit(DismissEvent);
            }
            RenameValidation::Unchanged => cx.emit(DismissEvent),
            RenameValidation::Invalid(_) => {}
        }
    }
}

impl EventEmitter<DismissEvent> for RenameBranchDialog {}

impl ModalView for RenameBranchDialog {}

impl Focusable for RenameBranchDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for RenameBranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let validation = self.validation(cx);
        let error_message = match &validation {
            RenameValidation::Invalid(message) => Some(message.clone()),
            RenameValidation::Unchanged | RenameValidation::Valid(_) => None,
        };
        let can_rename = matches!(validation, RenameValidation::Valid(_));
        let colors = cx.theme().colors();

        v_flex()
            .key_context("RenameBranchDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .w(rems(DIALOG_WIDTH))
            .elevation_3(cx)
            .bg(opaque_elevated_surface(cx))
            .overflow_hidden()
            .child(
                Modal::new("rename-branch-dialog", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .headline(format!("Rename Branch {}", self.old_name))
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new()
                            .child(Label::new("Branch Name:").size(LabelSize::Small))
                            .child(
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.editor_background)
                                    .child(self.editor.clone()),
                            )
                            .children(error_message.map(|message| {
                                Label::new(message)
                                    .size(LabelSize::Small)
                                    .color(Color::Error)
                            }))
                            .when(self.has_upstream, |this| {
                                this.child(
                                    Checkbox::new(
                                        "rename-unset-upstream",
                                        self.unset_upstream.into(),
                                    )
                                    .label("Unset upstream branch")
                                    .on_click(cx.listener(
                                        |this, state: &ToggleState, _window, cx| {
                                            this.unset_upstream = state.selected();
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(Button::new("rename-cancel", "Cancel").on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    }),
                                ))
                                .child(
                                    Button::new("rename-confirm", "Rename")
                                        .style(ButtonStyle::Filled)
                                        .disabled(!can_rename)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm(&Confirm, window, cx);
                                        })),
                                ),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<SharedString> {
        values
            .iter()
            .map(|value| SharedString::from(value.to_string()))
            .collect()
    }

    #[test]
    fn empty_name_asks_for_a_name() {
        assert_eq!(
            validate_rename("main", "  ", &names(&["main"]), &[]),
            RenameValidation::Invalid("Specify name for the new branch".to_string())
        );
    }

    #[test]
    fn unchanged_name_is_not_reported_as_existing() {
        assert_eq!(
            validate_rename("main", "main", &names(&["main"]), &[]),
            RenameValidation::Unchanged
        );
    }

    #[test]
    fn head_is_reserved() {
        assert_eq!(
            validate_rename("main", "HEAD", &names(&["main"]), &[]),
            RenameValidation::Invalid("HEAD is a reserved keyword".to_string())
        );
    }

    #[test]
    fn existing_local_branch_is_rejected() {
        assert_eq!(
            validate_rename("main", "dev", &names(&["main", "dev"]), &[]),
            RenameValidation::Invalid("Branch name dev already exists.".to_string())
        );
    }

    #[test]
    fn name_equal_to_a_remote_branch_is_rejected() {
        assert_eq!(
            validate_rename(
                "main",
                "origin/main",
                &names(&["main"]),
                &names(&["origin/main"])
            ),
            RenameValidation::Invalid(
                "Branch name origin/main conflicts with remote branch with the same name"
                    .to_string()
            )
        );
    }

    #[test]
    fn name_equal_to_a_local_branch_directory_is_rejected() {
        assert_eq!(
            validate_rename("main", "feature", &names(&["main", "feature/login"]), &[]),
            RenameValidation::Invalid(
                "Branch name feature conflicts with local branch directory with the same name"
                    .to_string()
            )
        );
    }

    #[test]
    fn renamed_branch_does_not_make_its_own_directory_conflict() {
        assert_eq!(
            validate_rename(
                "feature/login",
                "feature",
                &names(&["main", "feature/login"]),
                &[]
            ),
            RenameValidation::Valid("feature".to_string())
        );
    }

    #[test]
    fn sibling_of_the_renamed_branch_still_makes_the_directory_conflict() {
        assert_eq!(
            validate_rename(
                "feature/login",
                "feature",
                &names(&["feature/login", "feature/signup"]),
                &[]
            ),
            RenameValidation::Invalid(
                "Branch name feature conflicts with local branch directory with the same name"
                    .to_string()
            )
        );
    }

    #[test]
    fn fresh_name_is_valid_and_cleaned_for_apply() {
        assert_eq!(
            validate_rename("main", "release/1.0.", &names(&["main"]), &[]),
            RenameValidation::Valid("release/1.0".to_string())
        );
    }
}
