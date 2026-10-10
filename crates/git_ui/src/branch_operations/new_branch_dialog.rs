use collections::HashSet;
use editor::{Editor, EditorEvent};
use git::repository::Branch;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window,
};
use menu::{Cancel, Confirm, SelectNext, SelectPrevious};
use ui::{Checkbox, TintColor, prelude::*};
use workspace::ModalView;

use crate::branch_operations::BranchContext;
use crate::branch_operations::ref_suggestions::{
    HighlightMove, MAX_SUGGESTIONS, RefSuggestion, branch_name_candidates, highlighted_suggestion,
    move_highlight, render_suggestion_list, suggest, suggestion_for_tab,
};
use crate::branch_refs::RefTarget;

const HEAD_KEYWORD: &str = "HEAD";
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
            (Some('.'), '.' | '/') | (Some('/'), '/' | '.') => continue,
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
    if name == HEAD_KEYWORD {
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
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(options.initial_name.clone(), window, cx);
            editor.select_all(&Default::default(), window, cx);
            editor
        });
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
        let typed = editor.read(cx).text(cx);
        let cleaned = clean_up_branch_name_on_typing(&typed);
        if cleaned == typed {
            return;
        }
        editor.update(cx, |editor, cx| {
            editor.set_text(cleaned, window, cx);
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([editor::Anchor::Max..editor::Anchor::Max]);
            });
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
                    .child(Label::new("Branch Name:"))
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
                    )
                    .children(suggestion_list)
                    .children(problem_message.map(|message| {
                        Label::new(message)
                            .color(Color::Error)
                            .size(LabelSize::Small)
                    }))
                    .when(self.show_checkout_option, |this| {
                        this.child(
                            Checkbox::new("new-branch-checkout", self.checkout.into())
                                .label("Checkout branch")
                                .on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                                    this.checkout = state.selected();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Checkbox::new(
                            "new-branch-overwrite",
                            (self.overwrite && overwrite_available).into(),
                        )
                        .label("Overwrite existing branch")
                        .disabled(!overwrite_available)
                        .on_click(cx.listener(
                            |this, state: &ToggleState, _, cx| {
                                this.overwrite = state.selected();
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                h_flex()
                    .p_3()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("new-branch-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        Button::new("new-branch-confirm", self.confirm_label.clone())
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .disabled(!can_confirm)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(local: &[&str], remote: &[&str]) -> ExistingBranches {
        ExistingBranches {
            local: local.iter().map(|name| name.to_string()).collect(),
            remote: remote.iter().map(|name| name.to_string()).collect(),
        }
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
        assert_eq!(clean_up_branch_name_on_typing("a./b"), "a.b");
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
        assert_eq!(validate_branch_name("head", &existing, false), None);
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
