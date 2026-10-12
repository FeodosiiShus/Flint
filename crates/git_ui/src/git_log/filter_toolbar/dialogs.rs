use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    MouseDownEvent, SharedString, Subscription, Task, Window, px,
};
use menu::{Cancel, Confirm, SecondaryConfirm, SelectNext, SelectPrevious};
use project::git_store::Repository;
use time::{Date, Month, OffsetDateTime, UtcOffset};
use ui::{Checkbox, DialogButton, ToggleState, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use super::model::{self, DateField, REFERENCE_DATE};
use crate::branch_operations::manage::settle;
use crate::branch_operations::ref_suggestions::{
    HighlightMove, MAX_SUGGESTIONS, RefSuggestion, highlighted_suggestion, move_highlight,
    render_suggestion_list, revision_candidates, suggest, suggestion_for_tab,
};
use crate::git_log::log_dates;
use crate::git_log::properties::HEAD_REVISION;

const VALUES_KEY_CONTEXT: &str = "GitLogFilterValuesPopup";
const VALUES_POPUP_WIDTH: f32 = 200.;
const VALUES_POPUP_MINIMUM_HEIGHT: f32 = 90.;
const VALUES_MINIMUM_LINES: usize = 3;
const VALUES_MAXIMUM_LINES: usize = 12;
const SELECT_PERIOD_TITLE: &str = "Select Period";
const AFTER_LABEL: &str = "After";
const BEFORE_LABEL: &str = "Before";
const OK_LABEL: &str = "OK";
const AFTER_ERROR: &str = "Date After must be a valid date";
const BEFORE_ERROR: &str = "Date Before must be a valid date";
const GO_TO_LABEL: &str = "Enter hash or branch/tag name:";
const NOON_HOUR: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValuesCompletion {
    None,
    Values,
    Branches,
}

pub(super) enum ValuesPopupEvent {
    Confirmed(Vec<String>),
}

pub(super) struct ValuesPopup {
    editor: Entity<Editor>,
    completion: ValuesCompletion,
    candidates: Vec<String>,
    separators: &'static [char],
    suggestions: Vec<RefSuggestion>,
    highlighted: Option<usize>,
    suppressed_text: Option<String>,
    advertisement: SharedString,
    _subscription: Subscription,
}

impl ValuesPopup {
    pub(super) fn new(
        initial_text: String,
        candidates: Vec<String>,
        completion: ValuesCompletion,
        separators: &'static [char],
        advertisement: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut editor =
                Editor::auto_height(VALUES_MINIMUM_LINES, VALUES_MAXIMUM_LINES, window, cx);
            editor.set_text(initial_text, window, cx);
            editor
        });
        let subscription =
            cx.subscribe_in(&editor, window, |this, _, event: &EditorEvent, _, cx| {
                if let EditorEvent::BufferEdited = event {
                    this.refresh_suggestions(cx);
                    cx.notify();
                }
            });
        Self {
            editor,
            completion,
            candidates,
            separators,
            suggestions: Vec::new(),
            highlighted: None,
            suppressed_text: None,
            advertisement,
            _subscription: subscription,
        }
    }

    fn completion_query(&self, text: &str) -> Option<(usize, String)> {
        match self.completion {
            ValuesCompletion::None => None,
            ValuesCompletion::Values => {
                let start = model::completion_token_start(text, &["|", "\n"]);
                let token = text.get(start..)?.trim_start();
                Some((text.len() - token.len(), token.to_string()))
            }
            ValuesCompletion::Branches => {
                let start = model::completion_token_start(text, &["|", "\n", ".."]);
                let token = text.get(start..)?.trim_start().trim_start_matches('-');
                Some((text.len() - token.len(), token.to_string()))
            }
        }
    }

    fn refresh_suggestions(&mut self, cx: &App) {
        let text = self.editor.read(cx).text(cx);
        self.suggestions = match self.completion_query(&text) {
            Some((_, query)) if self.suppressed_text.as_deref() != Some(text.as_str()) => {
                suggest(&query, &self.candidates, MAX_SUGGESTIONS)
            }
            _ => Vec::new(),
        };
        self.highlighted = None;
    }

    fn accept_suggestion_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(suggestion) = self.suggestions.get(index).cloned() else {
            return;
        };
        let text = self.editor.read(cx).text(cx);
        let Some((start, _)) = self.completion_query(&text) else {
            return;
        };
        let mut new_text = text.get(..start).unwrap_or_default().to_string();
        new_text.push_str(&suggestion.text);
        self.suppressed_text = Some(new_text.clone());
        self.suggestions.clear();
        self.highlighted = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_text(new_text, window, cx);
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([editor::Anchor::Max..editor::Anchor::Max]);
            });
        });
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggestions.is_empty() {
            cx.propagate();
            return;
        }
        let index = self.highlighted.unwrap_or(0);
        self.accept_suggestion_at(index, window, cx);
    }

    fn finish(&mut self, _: &SecondaryConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text(cx);
        cx.emit(ValuesPopupEvent::Confirmed(model::parse_values(
            &text,
            self.separators,
        )));
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn dismiss_on_click_outside(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for ValuesPopup {}
impl EventEmitter<ValuesPopupEvent> for ValuesPopup {}

impl Focusable for ValuesPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for ValuesPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let suggestion_list = (!self.suggestions.is_empty()).then(|| {
            render_suggestion_list(
                &self.suggestions,
                self.highlighted,
                "git-log-filter-values-suggestions",
                Self::accept_suggestion_at,
                cx,
            )
        });
        let colors = cx.theme().colors();
        v_flex()
            .key_context(VALUES_KEY_CONTEXT)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::finish))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down_out(cx.listener(Self::dismiss_on_click_outside))
            .elevation_2(cx)
            .w(px(VALUES_POPUP_WIDTH))
            .min_h(px(VALUES_POPUP_MINIMUM_HEIGHT))
            .child(
                div()
                    .w_full()
                    .p(px(2.))
                    .px_2()
                    .py_1()
                    .child(self.editor.clone()),
            )
            .children(suggestion_list)
            .child(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new(self.advertisement.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
    }
}

type DateHandler = Box<dyn FnOnce(Option<i64>, Option<i64>, &mut Window, &mut App)>;

pub(super) struct DatePeriodDialog {
    focus_handle: FocusHandle,
    after_enabled: bool,
    before_enabled: bool,
    after_editor: Entity<Editor>,
    before_editor: Entity<Editor>,
    order: [DateField; 3],
    error: Option<SharedString>,
    on_confirm: Option<DateHandler>,
}

fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

fn local_unix_seconds(year: i32, month: u8, day: u8, hour: u8) -> Option<i64> {
    let month = Month::try_from(month).ok()?;
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let date_time = date.with_hms(hour, 0, 0).ok()?;
    Some(date_time.assume_offset(local_offset()).unix_timestamp())
}

impl DatePeriodDialog {
    pub(super) fn new(
        after: Option<i64>,
        before: Option<i64>,
        on_confirm: impl FnOnce(Option<i64>, Option<i64>, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (reference_year, reference_month, reference_day) = REFERENCE_DATE;
        let order = local_unix_seconds(reference_year, reference_month, reference_day, NOON_HOUR)
            .map(|seconds| model::date_field_order(&log_dates::format_date(seconds)))
            .unwrap_or_else(|| model::date_field_order(""));
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let create_editor = |value: Option<i64>, window: &mut Window, cx: &mut Context<Self>| {
            let text = log_dates::format_date(value.unwrap_or(now)).to_string();
            cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_text(text, window, cx);
                editor
            })
        };
        let after_editor = create_editor(after, window, cx);
        let before_editor = create_editor(before, window, cx);
        Self {
            focus_handle: cx.focus_handle(),
            after_enabled: after.is_some(),
            before_enabled: before.is_some(),
            after_editor,
            before_editor,
            order,
            error: None,
            on_confirm: Some(Box::new(on_confirm)),
        }
    }

    fn parse(&self, editor: &Entity<Editor>, cx: &App) -> Option<i64> {
        let text = editor.read(cx).text(cx);
        let current_year = OffsetDateTime::now_utc().to_offset(local_offset()).year();
        let (year, month, day) = model::parse_date(&text, self.order, current_year)?;
        local_unix_seconds(year, month, day, 0)
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let after = if self.after_enabled {
            match self.parse(&self.after_editor, cx) {
                Some(after) => Some(after),
                None => {
                    self.error = Some(AFTER_ERROR.into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let before = if self.before_enabled {
            match self.parse(&self.before_editor, cx) {
                Some(before) => Some(before),
                None => {
                    self.error = Some(BEFORE_ERROR.into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        cx.emit(DismissEvent);
        if after.is_none() && before.is_none() {
            return;
        }
        if let Some(on_confirm) = self.on_confirm.take() {
            on_confirm(after, before, window, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(window, cx);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn render_date_field(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        editor: &Entity<Editor>,
        set_enabled: fn(&mut Self, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let dialog = cx.weak_entity();
        let colors = cx.theme().colors();
        h_flex()
            .gap_2()
            .child(
                Checkbox::new(id, ToggleState::from(enabled))
                    .label(label)
                    .on_click(move |state: &ToggleState, _, cx| {
                        let selected = state.selected();
                        dialog
                            .update(cx, |dialog, cx| {
                                set_enabled(dialog, selected);
                                dialog.error = None;
                                cx.notify();
                            })
                            .ok();
                    }),
            )
            .child(
                div()
                    .w(px(110.))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.editor_background)
                    .when(!enabled, |this| this.opacity(0.5))
                    .child(editor.clone()),
            )
    }
}

impl EventEmitter<DismissEvent> for DatePeriodDialog {}

impl ModalView for DatePeriodDialog {}

impl Focusable for DatePeriodDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DatePeriodDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let after = self.render_date_field(
            "git-log-date-after",
            AFTER_LABEL,
            self.after_enabled,
            &self.after_editor,
            |dialog, enabled| dialog.after_enabled = enabled,
            cx,
        );
        let before = self.render_date_field(
            "git-log-date-before",
            BEFORE_LABEL,
            self.before_enabled,
            &self.before_editor,
            |dialog, enabled| dialog.before_enabled = enabled,
            cx,
        );
        v_flex()
            .key_context("GitLogSelectPeriodDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .elevation_3(cx)
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(Headline::new(SELECT_PERIOD_TITLE).size(HeadlineSize::Small)),
            )
            .child(h_flex().px_3().py_2().gap_3().child(after).child(before))
            .when_some(self.error.clone(), |this, error| {
                this.child(div().px_3().child(Label::new(error).color(Color::Error)))
            })
            .child(
                h_flex().p_3().justify_end().child(
                    DialogButton::new("git-log-date-ok", OK_LABEL)
                        .default_button(true)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}

type RevisionHandler = Box<dyn FnOnce(String, &mut Window, &mut App)>;

pub(super) struct GoToPopup {
    editor: Entity<Editor>,
    branches: Vec<String>,
    tags: Vec<String>,
    candidates: Vec<String>,
    suggestions: Vec<RefSuggestion>,
    highlighted: Option<usize>,
    on_submit: Option<RevisionHandler>,
    _load_tags_task: Task<()>,
    _subscription: Subscription,
}

impl GoToPopup {
    pub(super) fn new(
        repository: Entity<Repository>,
        branches: Vec<String>,
        on_submit: impl FnOnce(String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let subscription =
            cx.subscribe_in(&editor, window, |this, _, event: &EditorEvent, _, cx| {
                if let EditorEvent::BufferEdited = event {
                    this.refresh_suggestions(cx);
                    cx.notify();
                }
            });
        let tags_receiver = repository.update(cx, |repository, _| repository.tags());
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
        let mut popup = Self {
            editor,
            branches,
            tags: Vec::new(),
            candidates: Vec::new(),
            suggestions: Vec::new(),
            highlighted: None,
            on_submit: Some(Box::new(on_submit)),
            _load_tags_task: load_tags_task,
            _subscription: subscription,
        };
        popup.rebuild_candidates();
        popup
    }

    fn rebuild_candidates(&mut self) {
        let mut candidates = vec![HEAD_REVISION.to_string()];
        candidates.extend(revision_candidates(
            self.branches.iter().map(String::as_str),
            std::iter::empty(),
            self.tags.iter().map(String::as_str),
        ));
        candidates.dedup();
        self.candidates = candidates;
    }

    fn refresh_suggestions(&mut self, cx: &App) {
        let text = self.editor.read(cx).text(cx);
        self.suggestions = suggest(&text, &self.candidates, MAX_SUGGESTIONS);
        self.highlighted = None;
    }

    fn submit_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(on_submit) = self.on_submit.take() else {
            return;
        };
        cx.emit(DismissEvent);
        on_submit(text, window, cx);
    }

    fn accept_suggestion_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(suggestion) = self.suggestions.get(index).cloned() {
            self.submit_text(suggestion.text, window, cx);
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(HighlightMove::Next, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(HighlightMove::Previous, cx);
    }

    fn move_highlight(&mut self, direction: HighlightMove, cx: &mut Context<Self>) {
        if self.suggestions.is_empty() {
            cx.propagate();
            return;
        }
        self.highlighted = move_highlight(self.highlighted, self.suggestions.len(), direction);
        cx.notify();
    }

    fn accept_first(
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
        self.submit_text(suggestion.text, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let text = highlighted_suggestion(&self.suggestions, self.highlighted)
            .map(|suggestion| suggestion.text.clone())
            .unwrap_or_else(|| self.editor.read(cx).text(cx));
        self.submit_text(text, window, cx);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for GoToPopup {}

impl ModalView for GoToPopup {}

impl Focusable for GoToPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for GoToPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let suggestion_list = (!self.suggestions.is_empty()).then(|| {
            render_suggestion_list(
                &self.suggestions,
                self.highlighted,
                "git-log-go-to-suggestions",
                Self::accept_suggestion_at,
                cx,
            )
        });
        let colors = cx.theme().colors();
        v_flex()
            .key_context("GitLogGoToPopup")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::accept_first))
            .elevation_3(cx)
            .w(rems(30.))
            .p(px(2.))
            .gap_1()
            .child(
                div()
                    .px_1()
                    .child(Label::new(GO_TO_LABEL).weight(FontWeight::BOLD)),
            )
            .child(
                div()
                    .w_full()
                    .p(px(3.))
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.editor_background)
                    .child(self.editor.clone()),
            )
            .children(suggestion_list)
    }
}
