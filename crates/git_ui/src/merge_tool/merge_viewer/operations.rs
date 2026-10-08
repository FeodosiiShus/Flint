use gpui::{Context, Window};
use merge_diff::{ComparisonPolicy, Side, ThreeSide};

use super::{
    MergeViewer, MergeViewerEvent,
    change_view::GutterAction,
    navigation::{DifferenceNavigation, NavigableChange},
    selection::{EditorSelection, read_editor_selection},
    viewer_settings,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PopupAction {
    Accept(Side),
    ResolveUsing(Side),
    IgnoreSide(Side),
    ResolveAutomatically,
    IgnoreAll,
    Revert,
}

fn pane_of(side: Side) -> ThreeSide {
    match side {
        Side::Left => ThreeSide::Left,
        Side::Right => ThreeSide::Right,
    }
}

impl PopupAction {
    pub(crate) fn is_visible(self, editor_side: ThreeSide) -> bool {
        match self {
            Self::Accept(side) | Self::ResolveUsing(side) => {
                editor_side == ThreeSide::Base || editor_side == pane_of(side)
            }
            Self::IgnoreSide(side) => editor_side == pane_of(side),
            Self::ResolveAutomatically | Self::IgnoreAll => editor_side == ThreeSide::Base,
            Self::Revert => true,
        }
    }

    pub(crate) fn text(self, editor_side: ThreeSide) -> &'static str {
        match self {
            Self::Accept(side) if editor_side == ThreeSide::Base => match side {
                Side::Left => "Accept Left Side",
                Side::Right => "Accept Right Side",
            },
            Self::Accept(_) => "Accept",
            Self::ResolveUsing(Side::Left) => "Resolve using Left",
            Self::ResolveUsing(Side::Right) => "Resolve using Right",
            Self::IgnoreSide(_) | Self::IgnoreAll => "Ignore",
            Self::ResolveAutomatically => "Resolve Automatically",
            Self::Revert => "Revert",
        }
    }

    pub(crate) fn command_name(self, editor_side: ThreeSide) -> String {
        format!("{} in merge", self.text(editor_side))
    }
}

impl MergeViewer {
    pub(super) fn run_gutter_action(
        &mut self,
        action: GutterAction,
        index: usize,
        resolve_whole_change: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.update(cx, |model, cx| match action {
            GutterAction::Accept(side) => {
                model.run_accept_change(index, side, resolve_whole_change, cx)
            }
            GutterAction::Ignore(side) => {
                model.run_ignore_change(index, side, resolve_whole_change, cx)
            }
            GutterAction::Resolve => model.run_resolve_change_automatically(index, cx),
        });
    }

    fn popup_change_enabled(&self, action: PopupAction, index: usize, cx: &gpui::App) -> bool {
        let model = self.model.read(cx);
        let Some(change) = model.change(index) else {
            return false;
        };
        match action {
            PopupAction::Accept(side)
            | PopupAction::ResolveUsing(side)
            | PopupAction::IgnoreSide(side) => !change.is_side_resolved(side),
            PopupAction::ResolveAutomatically => {
                model.can_resolve_change_automatically(index, ThreeSide::Base)
            }
            PopupAction::IgnoreAll => !change.is_resolved(),
            PopupAction::Revert => false,
        }
    }

    fn is_change_selected(
        &self,
        action: PopupAction,
        index: usize,
        editor_side: ThreeSide,
        selection: &EditorSelection,
        cx: &gpui::App,
    ) -> bool {
        let model = self.model.read(cx);
        self.popup_change_enabled(action, index, cx)
            && selection.lines.selects_range(
                model.start_line(index, editor_side),
                model.end_line(index, editor_side),
            )
    }

    pub(crate) fn selected_changes(
        &self,
        action: PopupAction,
        editor_side: ThreeSide,
        selection: &EditorSelection,
        cx: &gpui::App,
    ) -> Vec<usize> {
        let model = self.model.read(cx);
        let candidates: Vec<usize> = match action {
            PopupAction::Revert => (0..model.changes().len()).collect(),
            _ => model.unresolved_changes(),
        };
        candidates
            .into_iter()
            .filter(|index| self.is_change_selected(action, *index, editor_side, selection, cx))
            .collect()
    }

    pub(crate) fn popup_action_enabled(
        &self,
        action: PopupAction,
        editor_side: ThreeSide,
        selection: &EditorSelection,
        cx: &gpui::App,
    ) -> bool {
        if !action.is_visible(editor_side) {
            return false;
        }
        let change_count = self.model.read(cx).changes().len();
        selection.is_some_range_selected(|_| {
            (0..change_count)
                .any(|index| self.is_change_selected(action, index, editor_side, selection, cx))
        })
    }

    pub(crate) fn perform_popup_action(
        &mut self,
        action: PopupAction,
        editor_side: ThreeSide,
        cx: &mut Context<Self>,
    ) {
        let selection = read_editor_selection(self.editor(editor_side), cx);
        let selected = self.selected_changes(action, editor_side, &selection, cx);
        if selected.is_empty() {
            return;
        }
        let command_name = action.command_name(editor_side);
        self.model.update(cx, |model, cx| match action {
            PopupAction::Accept(side) => {
                model.run_replace_changes(&command_name, &selected, side, false, cx)
            }
            PopupAction::ResolveUsing(side) => {
                model.run_replace_changes(&command_name, &selected, side, true, cx)
            }
            PopupAction::IgnoreSide(side) => {
                model.run_ignore_changes_on_side(&command_name, &selected, side, cx)
            }
            PopupAction::ResolveAutomatically => {
                model.run_resolve_changes_automatically(&command_name, &selected, cx)
            }
            PopupAction::IgnoreAll => model.run_mark_changes_resolved(&command_name, &selected, cx),
            PopupAction::Revert => model.run_reset_resolved_changes(&command_name, &selected, cx),
        });
    }

    pub(crate) fn apply_non_conflicting(
        &mut self,
        side: ThreeSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.model.read(cx).has_non_conflicted_changes(side) {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.run_apply_non_conflicted_changes(side, cx)
        });
        self.scroll_to_first_unresolved_change(window, cx);
    }

    pub(crate) fn resolve_simple_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.read(cx).has_auto_resolvable_conflicted_changes() {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.run_apply_resolvable_conflicted_changes(cx)
        });
        self.scroll_to_first_unresolved_change(window, cx);
    }

    pub(crate) fn can_revert_conflict_resolution(&self, cx: &gpui::App) -> bool {
        !self.model.read(cx).resolved_changes().is_empty()
    }

    pub(crate) fn request_revert_conflict_resolution(&mut self, cx: &mut Context<Self>) {
        if self.can_revert_conflict_resolution(cx) {
            cx.emit(MergeViewerEvent::RevertConfirmationRequested);
        }
    }

    pub(crate) fn revert_conflict_resolution(&mut self, cx: &mut Context<Self>) {
        self.model
            .update(cx, |model, cx| model.run_revert_conflict_resolution(cx));
    }

    pub(crate) fn request_ignore_policy(
        &mut self,
        policy: ComparisonPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if policy == self.ignore_policy {
            return;
        }
        if self.model.read(cx).content_modified() {
            cx.emit(MergeViewerEvent::RestartConfirmationRequested(policy));
            return;
        }
        self.restart_merge(policy, window, cx);
    }

    pub(crate) fn set_highlight_by_word(&mut self, by_word: bool, cx: &mut Context<Self>) {
        if self.highlight_by_word == by_word {
            return;
        }
        self.highlight_by_word = by_word;
        viewer_settings::store_highlight_by_word(by_word, cx);
        self.inner_diff_settings_changed(cx);
        cx.notify();
    }

    pub(crate) fn highlight_by_word(&self) -> bool {
        self.highlight_by_word
    }

    pub(crate) fn ignore_policy(&self) -> ComparisonPolicy {
        self.ignore_policy
    }

    pub(crate) fn sync_scroll_enabled(&self) -> bool {
        self.sync_scroll_enabled
    }

    pub(crate) fn toggle_synchronize_scrolling(&mut self, cx: &mut Context<Self>) {
        self.sync_scroll_enabled = !self.sync_scroll_enabled;
        cx.notify();
    }

    pub(crate) fn navigable_changes(
        &self,
        conflicts_only: bool,
        cx: &gpui::App,
    ) -> Vec<NavigableChange> {
        let model = self.model.read(cx);
        let side = self.current_side;
        model
            .changes()
            .iter()
            .filter(|change| !change.is_resolved())
            .filter(|change| {
                if conflicts_only {
                    change.is_conflict()
                } else {
                    side == ThreeSide::Base || change.is_change(side)
                }
            })
            .map(|change| NavigableChange {
                index: change.index(),
                start_line: model.start_line(change.index(), side) as i64,
                end_line: model.end_line(change.index(), side) as i64,
            })
            .collect()
    }

    pub(crate) fn can_navigate(
        &self,
        conflicts_only: bool,
        forward: bool,
        cx: &mut gpui::App,
    ) -> bool {
        let changes = self.navigable_changes(conflicts_only, cx);
        let (caret_line, line_count) = self.caret_line_and_line_count(self.current_side, cx);
        let navigation = DifferenceNavigation {
            changes: &changes,
            caret_line,
            line_count,
        };
        if forward {
            navigation.can_go_next()
        } else {
            navigation.can_go_previous()
        }
    }

    pub(crate) fn navigate(
        &mut self,
        conflicts_only: bool,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changes = self.navigable_changes(conflicts_only, cx);
        let (caret_line, line_count) = self.caret_line_and_line_count(self.current_side, cx);
        let navigation = DifferenceNavigation {
            changes: &changes,
            caret_line,
            line_count,
        };
        let target = if forward {
            navigation
                .can_go_next()
                .then(|| navigation.next_change())
                .flatten()
        } else {
            navigation
                .can_go_previous()
                .then(|| navigation.previous_change())
                .flatten()
        };
        if let Some(target) = target {
            self.scroll_to_change(target.index, window, cx);
        }
    }
}
