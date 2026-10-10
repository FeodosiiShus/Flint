use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task,
    Window,
};
use menu::{Cancel, Confirm, SelectNext, SelectPrevious};
use project::git_store::Repository;
use ui::{TintColor, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use crate::branch_operations::BranchContext;
use crate::branch_operations::manage::settle;
use crate::branch_operations::ref_suggestions::{
    HighlightMove, MAX_SUGGESTIONS, RefSuggestion, highlighted_suggestion, move_highlight,
    render_suggestion_list, revision_candidates, suggest, suggestion_for_tab,
};

const DIALOG_TITLE: &str = "Checkout";
const PROMPT: &str = "Enter reference (branch, tag) name or commit hash:";
const CONFIRM_LABEL: &str = "OK";

pub fn normalize_revision_input(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn open(
    context: &BranchContext,
    on_submit: impl FnOnce(String, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let repository = context.repository.clone();
    context.open_modal(window, cx, move |window, cx| {
        CheckoutRevisionDialog::new(repository, on_submit, window, cx)
    });
}

type SubmitHandler = Box<dyn FnOnce(String, &mut Window, &mut App)>;

struct CheckoutRevisionDialog {
    editor: Entity<Editor>,
    on_submit: Option<SubmitHandler>,
    local_branches: Vec<String>,
    remote_branches: Vec<String>,
    tags: Vec<String>,
    candidates: Vec<String>,
    suggestions: Vec<RefSuggestion>,
    highlighted: Option<usize>,
    suppressed_text: Option<String>,
    _load_tags_task: Task<()>,
    _editor_subscription: Subscription,
}

impl CheckoutRevisionDialog {
    fn new(
        repository: Entity<Repository>,
        on_submit: impl FnOnce(String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let editor_subscription =
            cx.subscribe_in(&editor, window, |this, _, event: &EditorEvent, _, cx| {
                if let EditorEvent::BufferEdited = event {
                    this.refresh_suggestions(cx);
                    cx.notify();
                }
            });
        let (local_branches, remote_branches, tags_receiver) =
            repository.update(cx, |repository, _| {
                let mut local_branches = Vec::new();
                let mut remote_branches = Vec::new();
                for branch in repository.branch_list.iter() {
                    let name = branch.name().to_string();
                    if branch.is_remote() {
                        remote_branches.push(name);
                    } else {
                        local_branches.push(name);
                    }
                }
                (local_branches, remote_branches, repository.tags())
            });
        let load_tags_task = cx.spawn(async move |this, cx| {
            let Some(tags) = settle(tags_receiver).await.log_err() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.tags = tags.into_iter().map(|tag| tag.name.to_string()).collect();
                this.rebuild_candidates();
                this.refresh_suggestions(cx);
                cx.notify();
            })
            .ok();
        });
        let mut dialog = Self {
            editor,
            on_submit: Some(Box::new(on_submit)),
            local_branches,
            remote_branches,
            tags: Vec::new(),
            candidates: Vec::new(),
            suggestions: Vec::new(),
            highlighted: None,
            suppressed_text: None,
            _load_tags_task: load_tags_task,
            _editor_subscription: editor_subscription,
        };
        dialog.rebuild_candidates();
        dialog
    }

    fn rebuild_candidates(&mut self) {
        self.candidates = revision_candidates(
            self.local_branches.iter().map(String::as_str),
            self.remote_branches.iter().map(String::as_str),
            self.tags.iter().map(String::as_str),
        );
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
        self.suppressed_text = Some(suggestion.text.clone());
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

    fn revision(&self, cx: &App) -> Option<String> {
        normalize_revision_input(&self.editor.read(cx).text(cx))
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
        let Some(revision) = self.revision(cx) else {
            return;
        };
        let Some(on_submit) = self.on_submit.take() else {
            return;
        };
        cx.emit(DismissEvent);
        let app: &mut App = cx;
        on_submit(revision, window, app);
    }
}

impl EventEmitter<DismissEvent> for CheckoutRevisionDialog {}

impl ModalView for CheckoutRevisionDialog {}

impl Focusable for CheckoutRevisionDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for CheckoutRevisionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_confirm = self.revision(cx).is_some();
        let suggestion_list = (!self.suggestions.is_empty()).then(|| {
            render_suggestion_list(
                &self.suggestions,
                self.highlighted,
                "checkout-revision-suggestions",
                Self::accept_suggestion_at,
                cx,
            )
        });

        v_flex()
            .key_context("CheckoutRevisionDialog")
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
                    .child(Headline::new(DIALOG_TITLE).size(HeadlineSize::Small)),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Label::new(PROMPT))
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
                    .children(suggestion_list),
            )
            .child(
                h_flex()
                    .p_3()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("checkout-revision-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        Button::new("checkout-revision-confirm", CONFIRM_LABEL)
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

    #[test]
    fn revision_input_is_trimmed() {
        assert_eq!(
            normalize_revision_input("  v1.2.3\n"),
            Some("v1.2.3".to_string())
        );
        assert_eq!(
            normalize_revision_input("0123abc"),
            Some("0123abc".to_string())
        );
    }

    #[test]
    fn blank_revision_input_is_rejected() {
        assert_eq!(normalize_revision_input(""), None);
        assert_eq!(normalize_revision_input(" \t\n"), None);
    }
}
