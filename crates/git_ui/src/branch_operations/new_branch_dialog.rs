use collections::HashSet;
use editor::{Editor, EditorEvent, MultiBufferOffset};
use git::repository::Branch;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window,
};
use menu::{Cancel, Confirm, SelectNext, SelectPrevious};
use ui::{Checkbox, DialogButton, prelude::*};
use workspace::ModalView;

use crate::branch_operations::BranchContext;
use crate::branch_operations::ref_suggestions::{
    HighlightMove, MAX_SUGGESTIONS, RefSuggestion, branch_name_candidates, highlighted_suggestion,
    move_highlight, render_suggestion_list, suggest, suggestion_for_tab,
};
use crate::branch_refs::RefTarget;

const HEAD_KEYWORD: &str = "HEAD";
const BRANCH_NAME_LABEL: &str = "Branch Name:";
const NAME_REPLACEMENT: char = '-';
const INVALID_NAME_CHARACTERS: [char; 8] = ['~', ':', '^', '?', '*', '"', '[', '\\'];
const LOCK_SUFFIX: &str = ".lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewBranchRequest {
    pub name: String,
    pub checkout: bool,
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct NewBranchDialogOptions {
    pub title: SharedString,
    pub confirm_label: SharedString,
    pub initial_name: String,
    pub show_checkout_option: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExistingBranches {
    local: HashSet<String>,
    remote: HashSet<String>,
}

impl ExistingBranches {
    pub fn new(branches: &[Branch]) -> Self {
        let mut existing = Self::default();
        for branch in branches {
            let name = branch.name().to_string();
            if branch.is_remote() {
                existing.remote.insert(name);
            } else {
                existing.local.insert(name);
            }
        }
        existing
    }

    pub fn has_local(&self, name: &str) -> bool {
        self.local.contains(name)
    }

    fn has_remote(&self, name: &str) -> bool {
        self.remote.contains(name)
    }

    fn has_local_branch_inside(&self, directory: &str) -> bool {
        let prefix = format!("{directory}/");
        self.local.iter().any(|name| name.starts_with(&prefix))
    }

    fn local_branch_containing<'a>(&self, name: &'a str) -> Option<&'a str> {
        name.match_indices('/')
            .map(|(index, _)| &name[..index])
            .find(|ancestor| self.local.contains(*ancestor))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchNameProblem {
    Empty,
    ReservedHead,
    ConflictsWithRemoteBranch,
    ConflictsWithBranchDirectory,
    ConflictsWithBranchAsDirectory { branch: String },
    AlreadyExists,
}

impl BranchNameProblem {
    pub fn message(&self, name: &str) -> String {
        match self {
            BranchNameProblem::Empty => "Specify name for the new branch".to_string(),
            BranchNameProblem::ReservedHead => "HEAD is a reserved keyword".to_string(),
            BranchNameProblem::ConflictsWithRemoteBranch => {
                format!("Branch name {name} conflicts with remote branch with the same name")
            }
            BranchNameProblem::ConflictsWithBranchDirectory => {
                format!(
                    "Branch name {name} conflicts with local branch directory with the same name"
                )
            }
            BranchNameProblem::ConflictsWithBranchAsDirectory { branch } => {
                format!("Branch name {name} conflicts with local branch {branch}")
            }
            BranchNameProblem::AlreadyExists => {
                format!(
                    "Branch name {name} already exists. Change the name or overwrite existing \
                     branch"
                )
            }
        }
    }
}

pub fn initial_branch_name(start: &RefTarget) -> String {
    start.name_without_remote().to_string()
}

pub fn clean_up_branch_name_on_typing(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        let character = if character.is_whitespace() || character.is_control() {
            NAME_REPLACEMENT
        } else {
            character
        };
        if INVALID_NAME_CHARACTERS.contains(&character) {
            continue;
        }
        match (output.chars().last(), character) {
            (None, '.' | '-' | '/') => continue,
            // IntelliJ drops the dot before a slash (`\.(?=/)`) and keeps the slash.
            (Some('.'), '/') => {
                output.pop();
                if output.is_empty() || output.ends_with('/') {
                    continue;
                }
            }
            (Some('.'), '.') | (Some('/'), '/' | '.') => continue,
            (Some('@'), '{') => {
                output.pop();
                continue;
            }
            _ => {}
        }
        output.push(character);
    }
    output
}

/// Where the caret goes after cleaning up typed text: IntelliJ cleans the text before the caret
/// on its own and puts the caret at the end of it, so editing inside a name keeps the caret there.
pub fn caret_after_clean_up(typed: &str, caret: usize) -> usize {
    clean_up_branch_name_on_typing(typed.get(..caret).unwrap_or(typed)).len()
}

pub(crate) fn branch_name_editor(
    initial_name: &str,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Editor {
    let mut editor = Editor::single_line(window, cx);
    editor.set_text(initial_name.to_string(), window, cx);
    editor.select_all(&Default::default(), window, cx);
    editor
}

pub(crate) fn clean_up_typed_name_in_editor(
    editor: &mut Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let typed = editor.text(cx);
    let cleaned = clean_up_branch_name_on_typing(&typed);
    if cleaned == typed {
        return;
    }
    let caret = editor
        .selections
        .newest::<MultiBufferOffset>(&editor.display_snapshot(cx))
        .head()
        .0;
    let caret = MultiBufferOffset(caret_after_clean_up(&typed, caret));
    editor.set_text(cleaned, window, cx);
    editor.change_selections(Default::default(), window, cx, |selections| {
        selections.select_ranges([caret..caret]);
    });
}

pub(crate) fn branch_name_row(editor: &Entity<Editor>, cx: &App) -> Div {
    let colors = cx.theme().colors();
    h_flex()
        .w_full()
        .gap_1p5()
        .child(
            div()
                .flex_none()
                .debug_selector(|| "BRANCH_NAME_LABEL".into())
                .child(Label::new(BRANCH_NAME_LABEL)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .debug_selector(|| "BRANCH_NAME_FIELD".into())
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.editor_background)
                .child(editor.clone()),
        )
}

/// Puts `content` under the Branch Name field, where IntelliJ's UI DSL places the cells of a
/// `row("")`.
pub(crate) fn under_branch_name_field(content: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .gap_1p5()
        .items_start()
        // An invisible copy of the label takes exactly the width of the label column.
        .child(
            div()
                .flex_none()
                .invisible()
                .child(Label::new(BRANCH_NAME_LABEL)),
        )
        .child(div().flex_1().min_w_0().child(content))
}

pub fn clean_up_branch_name_on_apply(name: &str) -> String {
    let mut cleaned = name.trim().to_string();
    loop {
        let without_separators =
            cleaned.trim_end_matches(|character: char| character == '.' || character == '/');
        let without_lock = without_separators
            .strip_suffix(LOCK_SUFFIX)
            .unwrap_or(without_separators);
        if without_lock.len() == cleaned.len() {
            return cleaned;
        }
        cleaned = without_lock.to_string();
    }
}

pub fn validate_branch_name(
    name: &str,
    existing: &ExistingBranches,
    overwrite: bool,
) -> Option<BranchNameProblem> {
    if name.is_empty() {
        return Some(BranchNameProblem::Empty);
    }
    if name.eq_ignore_ascii_case(HEAD_KEYWORD) {
        return Some(BranchNameProblem::ReservedHead);
    }
    if existing.has_remote(name) {
        return Some(BranchNameProblem::ConflictsWithRemoteBranch);
    }
    if existing.has_local_branch_inside(name) {
        return Some(BranchNameProblem::ConflictsWithBranchDirectory);
    }
    if let Some(branch) = existing.local_branch_containing(name) {
        return Some(BranchNameProblem::ConflictsWithBranchAsDirectory {
            branch: branch.to_string(),
        });
    }
    if existing.has_local(name) && !overwrite {
        return Some(BranchNameProblem::AlreadyExists);
    }
    None
}

pub fn open(
    context: &BranchContext,
    options: NewBranchDialogOptions,
    on_submit: impl FnOnce(NewBranchRequest, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let existing = ExistingBranches::new(&context.repository.read(cx).branch_list);
    context.open_modal(window, cx, move |window, cx| {
        NewBranchDialog::new(options, existing, on_submit, window, cx)
    });
}

type SubmitHandler = Box<dyn FnOnce(NewBranchRequest, &mut Window, &mut App)>;

struct NewBranchDialog {
    title: SharedString,
    confirm_label: SharedString,
    show_checkout_option: bool,
    existing: ExistingBranches,
    editor: Entity<Editor>,
    checkout: bool,
    overwrite: bool,
    on_submit: Option<SubmitHandler>,
    candidates: Vec<String>,
    suggestions: Vec<RefSuggestion>,
    highlighted: Option<usize>,
    suppressed_text: Option<String>,
    _editor_subscription: Subscription,
}

impl NewBranchDialog {
    fn new(
        options: NewBranchDialogOptions,
        existing: ExistingBranches,
        on_submit: impl FnOnce(NewBranchRequest, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| branch_name_editor(&options.initial_name, window, cx));
        let editor_subscription = cx.subscribe_in(
            &editor,
            window,
            |this, editor, event: &EditorEvent, window, cx| {
                if let EditorEvent::BufferEdited = event {
                    this.clean_up_typed_name(editor, window, cx);
                    this.refresh_suggestions(cx);
                    cx.notify();
                }
            },
        );
        let candidates = branch_name_candidates(existing.local.iter().map(String::as_str));
        let suppressed_text = Some(clean_up_branch_name_on_typing(&options.initial_name));
        Self {
            title: options.title,
            confirm_label: options.confirm_label,
            show_checkout_option: options.show_checkout_option,
            existing,
            editor,
            checkout: true,
            overwrite: false,
            on_submit: Some(Box::new(on_submit)),
            candidates,
            suggestions: Vec::new(),
            highlighted: None,
            suppressed_text,
            _editor_subscription: editor_subscription,
        }
    }

    fn clean_up_typed_name(
        &mut self,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        editor.update(cx, |editor, cx| {
            clean_up_typed_name_in_editor(editor, window, cx)
        });
    }

    fn refresh_suggestions(&mut self, cx: &App) {
        let text = self.editor.read(cx).text(cx);
        self.suggestions = if self.suppressed_text.as_deref() == Some(text.as_str()) {
            Vec::new()
        } else {
            suggest(&text, &self.candidates, MAX_SUGGESTIONS)
        };
        self.highlighted = None;
    }

    fn accept_suggestion(
        &mut self,
        suggestion: RefSuggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !suggestion.is_directory() {
            self.suppressed_text = Some(suggestion.text.clone());
        }
        self.suggestions.clear();
        self.highlighted = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_text(suggestion.text, window, cx);
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([editor::Anchor::Max..editor::Anchor::Max]);
            });
        });
        window.focus(&self.editor.focus_handle(cx), cx);
        cx.notify();
    }

    fn accept_suggestion_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(suggestion) = self.suggestions.get(index).cloned() {
            self.accept_suggestion(suggestion, window, cx);
        }
    }

    fn move_suggestion_highlight(&mut self, direction: HighlightMove, cx: &mut Context<Self>) {
        if self.suggestions.is_empty() {
            cx.propagate();
            return;
        }
        self.highlighted = move_highlight(self.highlighted, self.suggestions.len(), direction);
        cx.notify();
    }

    fn select_next_suggestion(
        &mut self,
        _: &SelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_suggestion_highlight(HighlightMove::Next, cx);
    }

    fn select_previous_suggestion(
        &mut self,
        _: &SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_suggestion_highlight(HighlightMove::Previous, cx);
    }

    fn accept_first_suggestion(
        &mut self,
        _: &editor::actions::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(suggestion) = suggestion_for_tab(&self.suggestions, self.highlighted).cloned()
        else {
            cx.propagate();
            return;
        };
        self.accept_suggestion(suggestion, window, cx);
    }

    fn applied_name(&self, cx: &App) -> String {
        clean_up_branch_name_on_apply(&self.editor.read(cx).text(cx))
    }

    fn effective_overwrite(&self, name: &str) -> bool {
        self.overwrite && self.existing.has_local(name)
    }

    fn problem(&self, cx: &App) -> Option<BranchNameProblem> {
        let name = self.applied_name(cx);
        let overwrite = self.effective_overwrite(&name);
        validate_branch_name(&name, &self.existing, overwrite)
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(suggestion) =
            highlighted_suggestion(&self.suggestions, self.highlighted).cloned()
        {
            self.accept_suggestion(suggestion, window, cx);
            return;
        }
        self.submit(window, cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.problem(cx).is_some() {
            return;
        }
        let name = self.applied_name(cx);
        let request = NewBranchRequest {
            overwrite: self.effective_overwrite(&name),
            checkout: self.checkout || !self.show_checkout_option,
            name,
        };
        let Some(on_submit) = self.on_submit.take() else {
            return;
        };
        cx.emit(DismissEvent);
        let app: &mut App = cx;
        on_submit(request, window, app);
    }
}

impl EventEmitter<DismissEvent> for NewBranchDialog {}

impl ModalView for NewBranchDialog {}

impl Focusable for NewBranchDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for NewBranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.applied_name(cx);
        let problem = self.problem(cx);
        let overwrite_available = self.existing.has_local(&name);
        let can_confirm = problem.is_none();
        let problem_message = problem.map(|problem| problem.message(&name));
        let has_field_details = !self.suggestions.is_empty() || problem_message.is_some();
        let suggestion_list = (!self.suggestions.is_empty()).then(|| {
            render_suggestion_list(
                &self.suggestions,
                self.highlighted,
                "new-branch-suggestions",
                Self::accept_suggestion_at,
                cx,
            )
        });

        v_flex()
            .key_context("NewBranchDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::select_next_suggestion))
            .on_action(cx.listener(Self::select_previous_suggestion))
            .on_action(cx.listener(Self::accept_first_suggestion))
            .elevation_3(cx)
            .w(rems(30.))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(Headline::new(self.title.clone()).size(HeadlineSize::Small)),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(branch_name_row(&self.editor, cx))
                    .when(has_field_details, |this| {
                        this.child(under_branch_name_field(
                            v_flex().gap_2().children(suggestion_list).children(
                                problem_message.map(|message| {
                                    Label::new(message)
                                        .color(Color::Error)
                                        .size(LabelSize::Small)
                                }),
                            ),
                        ))
                    })
                    .child(under_branch_name_field(
                        h_flex()
                            .gap_4()
                            .when(self.show_checkout_option, |this| {
                                this.child(
                                    div().debug_selector(|| "NEW_BRANCH_CHECKOUT".into()).child(
                                        Checkbox::new("new-branch-checkout", self.checkout.into())
                                            .label("Checkout branch")
                                            .on_click(cx.listener(
                                                |this, state: &ToggleState, _, cx| {
                                                    this.checkout = state.selected();
                                                    cx.notify();
                                                },
                                            )),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .debug_selector(|| "NEW_BRANCH_OVERWRITE".into())
                                    .child(
                                        Checkbox::new(
                                            "new-branch-overwrite",
                                            (self.overwrite && overwrite_available).into(),
                                        )
                                        .label("Overwrite existing branch")
                                        .disabled(!overwrite_available)
                                        .on_click(
                                            cx.listener(|this, state: &ToggleState, _, cx| {
                                                this.overwrite = state.selected();
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            ),
                    )),
            )
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .justify_end()
                    .child(
                        DialogButton::new("new-branch-cancel", "Cancel")
                            .default_button(false)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        DialogButton::new("new-branch-confirm", self.confirm_label.clone())
                            .default_button(true)
                            .disabled(!can_confirm)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use settings::SettingsStore;

    fn existing(local: &[&str], remote: &[&str]) -> ExistingBranches {
        ExistingBranches {
            local: local.iter().map(|name| name.to_string()).collect(),
            remote: remote.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            cx.set_global(db::AppDatabase::test_new());
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
        });
    }

    fn open_dialog<'a>(
        cx: &'a mut TestAppContext,
        initial_name: &str,
    ) -> (Entity<NewBranchDialog>, &'a mut VisualTestContext) {
        init_test(cx);
        let options = NewBranchDialogOptions {
            title: SharedString::new_static("Create New Branch"),
            confirm_label: SharedString::new_static("Create"),
            initial_name: initial_name.to_string(),
            show_checkout_option: true,
        };
        cx.add_window_view(|window, cx| {
            NewBranchDialog::new(options, existing(&["main"], &[]), |_, _, _| {}, window, cx)
        })
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

    #[test]
    fn typing_replaces_whitespace_and_drops_characters_git_rejects() {
        assert_eq!(clean_up_branch_name_on_typing("feature x"), "feature-x");
        assert_eq!(clean_up_branch_name_on_typing("tab\tx"), "tab-x");
        assert_eq!(
            clean_up_branch_name_on_typing("a~b^c:d?e*f[g\"h\\i"),
            "abcdefghi"
        );
    }

    #[test]
    fn typing_drops_leading_separators_and_repeated_dots_and_slashes() {
        assert_eq!(clean_up_branch_name_on_typing(".hidden"), "hidden");
        assert_eq!(clean_up_branch_name_on_typing("-flag"), "flag");
        assert_eq!(clean_up_branch_name_on_typing("/root"), "root");
        assert_eq!(clean_up_branch_name_on_typing(" spaced"), "spaced");
        assert_eq!(clean_up_branch_name_on_typing("a..b"), "a.b");
        assert_eq!(clean_up_branch_name_on_typing("a//b"), "a/b");
        assert_eq!(clean_up_branch_name_on_typing("a/.b"), "a/b");
        assert_eq!(clean_up_branch_name_on_typing("a./b"), "a/b");
        assert_eq!(clean_up_branch_name_on_typing("release./x"), "release/x");
        assert_eq!(clean_up_branch_name_on_typing("a@{b"), "ab");
        assert_eq!(clean_up_branch_name_on_typing("a@{1}"), "a1}");
    }

    #[test]
    fn typing_keeps_valid_names_untouched() {
        for name in [
            "main",
            "feature/login",
            "release-1.2.3",
            "user/jane/fix_bug",
            "v1",
        ] {
            assert_eq!(clean_up_branch_name_on_typing(name), name);
        }
    }

    #[test]
    fn applying_strips_trailing_dots_slashes_and_lock_suffixes() {
        assert_eq!(clean_up_branch_name_on_apply("feature/"), "feature");
        assert_eq!(clean_up_branch_name_on_apply("feature."), "feature");
        assert_eq!(clean_up_branch_name_on_apply("topic.lock"), "topic");
        assert_eq!(clean_up_branch_name_on_apply("topic.lock."), "topic");
        assert_eq!(clean_up_branch_name_on_apply("topic.lock.lock"), "topic");
        assert_eq!(clean_up_branch_name_on_apply("  name  "), "name");
        assert_eq!(clean_up_branch_name_on_apply("."), "");
        assert_eq!(clean_up_branch_name_on_apply("./."), "");
        assert_eq!(clean_up_branch_name_on_apply(""), "");
        assert_eq!(clean_up_branch_name_on_apply("topic.lock/"), "topic");
    }

    #[test]
    fn empty_and_reserved_names_are_rejected_first() {
        let existing = existing(&[], &[]);
        assert_eq!(
            validate_branch_name("", &existing, false),
            Some(BranchNameProblem::Empty)
        );
        assert_eq!(
            validate_branch_name("HEAD", &existing, true),
            Some(BranchNameProblem::ReservedHead)
        );
        assert_eq!(
            validate_branch_name("head", &existing, false),
            Some(BranchNameProblem::ReservedHead)
        );
        assert_eq!(
            validate_branch_name("Head", &existing, false),
            Some(BranchNameProblem::ReservedHead)
        );
        assert_eq!(validate_branch_name("heads", &existing, false), None);
    }

    #[test]
    fn the_caret_stays_where_it_was_relative_to_the_cleaned_text() {
        assert_eq!(caret_after_clean_up("feature x", 8), 8);
        assert_eq!(caret_after_clean_up("ab//cd", 4), 3);
        assert_eq!(caret_after_clean_up("a..b.c", 3), 2);
        assert_eq!(caret_after_clean_up("feature x", 9), 9);
        assert_eq!(caret_after_clean_up("caf\u{e9} x", 6), 6);
    }

    #[gpui::test]
    fn cleaning_up_a_name_typed_in_the_middle_keeps_the_caret_there(cx: &mut TestAppContext) {
        let (dialog, cx) = open_dialog(cx, "featurex");
        let editor = dialog.read_with(cx, |dialog, _| dialog.editor.clone());
        editor.update_in(cx, |editor, window, cx| {
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([MultiBufferOffset(7)..MultiBufferOffset(7)]);
            });
            editor.insert(" ", window, cx);
        });
        cx.run_until_parked();
        assert_eq!(text_and_caret(&editor, cx), ("feature-x".to_string(), 8));
    }

    #[gpui::test]
    fn branch_name_editors_start_with_the_whole_name_selected(cx: &mut TestAppContext) {
        init_test(cx);
        let (editor, cx) =
            cx.add_window_view(|window, cx| branch_name_editor("feature", window, cx));
        editor.update_in(cx, |editor, window, cx| editor.insert("topic", window, cx));
        assert_eq!(text_and_caret(&editor, cx), ("topic".to_string(), 5));
    }

    #[gpui::test]
    fn the_branch_name_label_sits_left_of_the_field_with_the_options_in_one_row_below(
        cx: &mut TestAppContext,
    ) {
        let (_dialog, cx) = open_dialog(cx, "feature");
        cx.run_until_parked();
        let label = cx
            .debug_bounds("BRANCH_NAME_LABEL")
            .expect("the label is painted");
        let field = cx
            .debug_bounds("BRANCH_NAME_FIELD")
            .expect("the field is painted");
        let checkout = cx
            .debug_bounds("NEW_BRANCH_CHECKOUT")
            .expect("the checkout option is painted");
        let overwrite = cx
            .debug_bounds("NEW_BRANCH_OVERWRITE")
            .expect("the overwrite option is painted");

        assert!(label.right() <= field.left(), "{label:?} vs {field:?}");
        assert!(
            label.top() >= field.top() && label.bottom() <= field.bottom(),
            "the label shares the row of the field: {label:?} vs {field:?}"
        );
        assert!(checkout.top() >= field.bottom());
        assert_eq!(
            checkout.top(),
            overwrite.top(),
            "both options share one row"
        );
        assert!(checkout.right() <= overwrite.left());
        assert!(
            f32::from(checkout.left() - field.left()).abs() < 0.5,
            "the options start under the field: {checkout:?} vs {field:?}"
        );
    }

    #[test]
    fn names_equal_to_a_remote_branch_conflict_with_it() {
        let existing = existing(&["main"], &["origin/main"]);
        assert_eq!(
            validate_branch_name("origin/main", &existing, false),
            Some(BranchNameProblem::ConflictsWithRemoteBranch)
        );
    }

    #[test]
    fn a_name_that_is_a_directory_of_existing_branches_conflicts() {
        let existing = existing(&["feature/login"], &[]);
        assert_eq!(
            validate_branch_name("feature", &existing, false),
            Some(BranchNameProblem::ConflictsWithBranchDirectory)
        );
        assert_eq!(validate_branch_name("feat", &existing, false), None);
    }

    #[test]
    fn a_name_nested_below_an_existing_branch_conflicts_with_that_branch() {
        let existing = existing(&["main", "feature/login"], &[]);
        assert_eq!(
            validate_branch_name("main/fix", &existing, false),
            Some(BranchNameProblem::ConflictsWithBranchAsDirectory {
                branch: "main".to_string()
            })
        );
        assert_eq!(
            validate_branch_name("feature/login/retry", &existing, false),
            Some(BranchNameProblem::ConflictsWithBranchAsDirectory {
                branch: "feature/login".to_string()
            })
        );
        assert_eq!(
            validate_branch_name("feature/signup", &existing, false),
            None
        );
    }

    #[test]
    fn an_existing_local_name_is_only_accepted_when_overwriting() {
        let existing = existing(&["main"], &[]);
        assert_eq!(
            validate_branch_name("main", &existing, false),
            Some(BranchNameProblem::AlreadyExists)
        );
        assert_eq!(validate_branch_name("main", &existing, true), None);
        assert_eq!(validate_branch_name("other", &existing, false), None);
    }

    #[test]
    fn problem_messages_match_the_ide_wording() {
        assert_eq!(
            BranchNameProblem::Empty.message(""),
            "Specify name for the new branch"
        );
        assert_eq!(
            BranchNameProblem::ReservedHead.message("HEAD"),
            "HEAD is a reserved keyword"
        );
        assert_eq!(
            BranchNameProblem::ConflictsWithRemoteBranch.message("origin/main"),
            "Branch name origin/main conflicts with remote branch with the same name"
        );
        assert_eq!(
            BranchNameProblem::ConflictsWithBranchDirectory.message("feature"),
            "Branch name feature conflicts with local branch directory with the same name"
        );
        assert_eq!(
            BranchNameProblem::AlreadyExists.message("main"),
            "Branch name main already exists. Change the name or overwrite existing branch"
        );
    }

    #[test]
    fn initial_name_keeps_local_names_and_drops_the_remote_prefix() {
        assert_eq!(
            initial_branch_name(&RefTarget::local("feature/x")),
            "feature/x"
        );
        assert_eq!(
            initial_branch_name(&RefTarget::remote("origin/feature/x")),
            "feature/x"
        );
        assert_eq!(initial_branch_name(&RefTarget::tag("v1.0")), "v1.0");
    }
}
