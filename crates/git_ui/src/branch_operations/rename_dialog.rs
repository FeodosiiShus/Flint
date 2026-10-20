use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window,
};
use menu::{Cancel, Confirm};
use ui::{
    Checkbox, DialogButton, DialogWidth, Modal, ModalFooter, ModalHeader, Section, prelude::*,
};
use workspace::ModalView;

use super::BranchContext;
use super::manage::perform_rename;
use super::new_branch_dialog::{
    branch_name_editor, branch_name_row, clean_up_branch_name_on_apply,
    clean_up_typed_name_in_editor, under_branch_name_field,
};

#[derive(Debug, PartialEq, Eq)]
pub enum RenameValidation {
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
    if name.eq_ignore_ascii_case("HEAD") {
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
    if let Some(branch) = name
        .match_indices('/')
        .map(|(index, _)| &name[..index])
        .find(|ancestor| {
            *ancestor != old_name
                && local_branches
                    .iter()
                    .any(|local| local.as_ref() == *ancestor)
        })
    {
        return RenameValidation::Invalid(format!(
            "Branch name {name} conflicts with local branch {branch}"
        ));
    }
    // IntelliJ validates a rename like a new branch, so the unchanged name is
    // reported as the existing branch it is.
    if name == old_name || local_branches.iter().any(|local| local.as_ref() == name) {
        return RenameValidation::Invalid(format!("Branch name {name} already exists"));
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

        let editor = cx.new(|cx| branch_name_editor(&old_name, window, cx));
        let editor_subscription = cx.subscribe_in(
            &editor,
            window,
            |_, editor, event: &EditorEvent, window, cx| {
                if !matches!(event, EditorEvent::BufferEdited) {
                    return;
                }
                editor.update(cx, |editor, cx| {
                    clean_up_typed_name_in_editor(editor, window, cx)
                });
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let validation = self.validation(cx);
        let error_message = match &validation {
            RenameValidation::Invalid(message) => Some(message.clone()),
            RenameValidation::Valid(_) => None,
        };
        let can_rename = matches!(validation, RenameValidation::Valid(_));

        v_flex()
            .key_context("RenameBranchDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .dialog_surface(DialogWidth::Medium, window, cx)
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
                            .child(branch_name_row(&self.editor, cx))
                            .children(error_message.map(|message| {
                                under_branch_name_field(
                                    Label::new(message)
                                        .size(LabelSize::Small)
                                        .color(Color::Error),
                                )
                            }))
                            .when(self.has_upstream, |this| {
                                this.child(under_branch_name_field(
                                    div()
                                        .debug_selector(|| "RENAME_UNSET_UPSTREAM".into())
                                        .child(
                                            Checkbox::new(
                                                "rename-unset-upstream",
                                                self.unset_upstream.into(),
                                            )
                                            .label("Unset upstream branch")
                                            .on_click(
                                                cx.listener(
                                                    |this, state: &ToggleState, _window, cx| {
                                                        this.unset_upstream = state.selected();
                                                        cx.notify();
                                                    },
                                                ),
                                            ),
                                        ),
                                ))
                            }),
                    )
                    .footer(
                        ModalFooter::new()
                            .end_button(
                                DialogButton::new("rename-cancel", "Cancel")
                                    .default_button(false)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    })),
                            )
                            .end_button(
                                DialogButton::new("rename-confirm", "Rename")
                                    .default_button(true)
                                    .disabled(!can_rename)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(&Confirm, window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use editor::MultiBufferOffset;
    use git::repository::{UpstreamTracking, UpstreamTrackingStatus};
    use gpui::{TestAppContext, VisualTestContext};
    use project::{FakeFs, Project};
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;
    use workspace::MultiWorkspace;

    use super::*;

    fn names(values: &[&str]) -> Vec<SharedString> {
        values
            .iter()
            .map(|value| SharedString::from(value.to_string()))
            .collect()
    }

    async fn open_rename_dialog<'a>(
        cx: &'a mut TestAppContext,
        root: &str,
        branch: &str,
    ) -> (Entity<RenameBranchDialog>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            cx.set_global(db::AppDatabase::test_new());
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(root, json!({ ".git": {}, "file.txt": "content" }))
            .await;
        let dot_git = Path::new(root).join(".git");
        fs.set_head_for_repo(&dot_git, &[("file.txt", "content".to_string())], "deadbeef");
        fs.insert_branches(&dot_git, &["main", branch]);
        fs.set_upstream_for_repo(
            &dot_git,
            branch,
            &format!("origin/{branch}"),
            UpstreamTracking::Tracked(UpstreamTrackingStatus {
                ahead: 0,
                behind: 0,
            }),
        );
        let project = Project::test(fs.clone(), [Path::new(root)], cx).await;
        cx.run_until_parked();
        let repository = project
            .read_with(cx, |project, cx| project.active_repository(cx))
            .expect("the project has an active repository");
        let workspace_window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = workspace_window
            .read_with(cx, |multi_workspace, _| {
                multi_workspace.workspace().downgrade()
            })
            .expect("window is alive");
        let context = BranchContext::new(workspace, repository);
        let old_name = SharedString::from(branch.to_string());
        cx.add_window_view(|window, cx| RenameBranchDialog::new(context, old_name, window, cx))
    }

    fn text_and_caret(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> (String, usize) {
        editor.update(cx, |editor, cx| {
            let caret = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx))
                .head()
                .0;
            (editor.text(cx), caret)
        })
    }

    #[gpui::test]
    async fn the_old_name_is_selected_and_cleanup_keeps_the_caret_in_place(
        cx: &mut TestAppContext,
    ) {
        let (dialog, cx) = open_rename_dialog(cx, path!("/rename-dialog-editing"), "feature").await;
        let editor = dialog.read_with(cx, |dialog, _| dialog.editor.clone());
        editor.update_in(cx, |editor, window, cx| editor.insert("topic", window, cx));
        cx.run_until_parked();
        assert_eq!(text_and_caret(&editor, cx), ("topic".to_string(), 5));

        editor.update_in(cx, |editor, window, cx| {
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([MultiBufferOffset(2)..MultiBufferOffset(2)]);
            });
            editor.insert(" ", window, cx);
        });
        cx.run_until_parked();
        assert_eq!(text_and_caret(&editor, cx), ("to-pic".to_string(), 3));
    }

    #[gpui::test]
    async fn the_branch_name_label_sits_left_of_the_field_with_unset_upstream_below_it(
        cx: &mut TestAppContext,
    ) {
        let (_dialog, cx) = open_rename_dialog(cx, path!("/rename-dialog-layout"), "feature").await;
        cx.run_until_parked();
        let label = cx
            .debug_bounds("BRANCH_NAME_LABEL")
            .expect("the label is painted");
        let field = cx
            .debug_bounds("BRANCH_NAME_FIELD")
            .expect("the field is painted");
        let unset_upstream = cx
            .debug_bounds("RENAME_UNSET_UPSTREAM")
            .expect("the unset upstream option is painted");
        assert!(label.right() <= field.left(), "{label:?} vs {field:?}");
        assert!(
            label.top() >= field.top() && label.bottom() <= field.bottom(),
            "the label shares the row of the field: {label:?} vs {field:?}"
        );
        assert!(unset_upstream.top() >= field.bottom());
        assert!(
            f32::from(unset_upstream.left() - field.left()).abs() < 0.5,
            "the option starts under the field: {unset_upstream:?} vs {field:?}"
        );
    }

    #[test]
    fn empty_name_asks_for_a_name() {
        assert_eq!(
            validate_rename("main", "  ", &names(&["main"]), &[]),
            RenameValidation::Invalid("Specify name for the new branch".to_string())
        );
    }

    #[test]
    fn unchanged_name_is_reported_as_the_existing_branch() {
        assert_eq!(
            validate_rename("main", "main", &names(&["main"]), &[]),
            RenameValidation::Invalid("Branch name main already exists".to_string())
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
    fn head_is_reserved_in_any_case() {
        for name in ["head", "Head", "hEaD"] {
            assert_eq!(
                validate_rename("main", name, &names(&["main"]), &[]),
                RenameValidation::Invalid("HEAD is a reserved keyword".to_string()),
                "{name}"
            );
        }
    }

    #[test]
    fn existing_local_branch_is_rejected() {
        assert_eq!(
            validate_rename("main", "dev", &names(&["main", "dev"]), &[]),
            RenameValidation::Invalid("Branch name dev already exists".to_string())
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

    #[test]
    fn name_nested_under_an_existing_local_branch_is_rejected() {
        assert_eq!(
            validate_rename("main", "feature/login", &names(&["main", "feature"]), &[]),
            RenameValidation::Invalid(
                "Branch name feature/login conflicts with local branch feature".to_string()
            )
        );
    }

    #[test]
    fn renaming_a_branch_into_its_own_directory_is_allowed() {
        assert_eq!(
            validate_rename(
                "feature",
                "feature/login",
                &names(&["main", "feature"]),
                &[]
            ),
            RenameValidation::Valid("feature/login".to_string())
        );
    }
}
