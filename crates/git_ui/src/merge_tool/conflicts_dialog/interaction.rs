use gpui::{
    AnyElement, App, Context, KeyDownEvent, Modifiers, Pixels, Point, ScrollStrategy, Window,
};
use ui::prelude::*;

use super::ConflictsDialog;
use super::actions::{
    ConflictsCollapseRow, ConflictsExpandRow, ConflictsExtendPageDown, ConflictsExtendPageUp,
    ConflictsExtendSelectionDown, ConflictsExtendSelectionUp, ConflictsExtendToFirstRow,
    ConflictsExtendToLastRow, ConflictsPageDown, ConflictsPageUp, ConflictsSelectAllRows,
};
use super::state::{DefaultButton, RowId, SelectionMove, TreeRowKind};
use super::tree_table::ROW_HEIGHT;
use crate::branch_operations::opaque_elevated_surface;

fn page_row_count(viewport_height: f32, forwards: bool) -> usize {
    if forwards {
        (viewport_height.max(ROW_HEIGHT) / ROW_HEIGHT).floor() as usize
    } else {
        (viewport_height / ROW_HEIGHT).ceil() as usize
    }
}

impl ConflictsDialog {
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() || self.context_menu.is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let mut query = self.speed_search.clone().unwrap_or_default();
        if keystroke.key == "backspace" {
            if query.pop().is_none() {
                return;
            }
        } else {
            let Some(text) = keystroke
                .key_char
                .as_deref()
                .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
            else {
                return;
            };
            query.push_str(text);
        }
        if query.is_empty() {
            self.speed_search = None;
            cx.notify();
            return;
        }
        self.state.select_file_matching(&query);
        self.speed_search = Some(query);
        self.scroll_selection_into_view();
        cx.notify();
    }

    pub(super) fn render_speed_search(&self, cx: &App) -> Option<AnyElement> {
        let query = self.speed_search.as_ref()?;
        let colors = cx.theme().colors();
        Some(
            div()
                .absolute()
                .bottom_2()
                .left_2()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(opaque_elevated_surface(cx))
                .child(Label::new(query.clone()))
                .into_any_element(),
        )
    }

    fn input_blocked(&self) -> bool {
        self.overlay.is_some() || !self.table_enabled() || self.closing
    }

    pub(super) fn scroll_selection_into_view(&self) {
        let visible = self.state.visible_indices();
        let rows = self.state.rows();
        let focus_position = self.state.lead_id().and_then(|id| {
            visible
                .iter()
                .position(|index| rows.get(*index).is_some_and(|row| &row.id == id))
        });
        if let Some(position) = focus_position {
            self.table_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
    }

    fn move_selection(&mut self, movement: SelectionMove, extend: bool, cx: &mut Context<Self>) {
        if self.input_blocked() {
            return;
        }
        self.state.move_selection(movement, extend);
        self.scroll_selection_into_view();
        cx.notify();
    }

    pub(super) fn select_next(
        &mut self,
        _: &menu::SelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Next, false, cx);
    }

    pub(super) fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Previous, false, cx);
    }

    pub(super) fn select_first(
        &mut self,
        _: &menu::SelectFirst,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::First, false, cx);
    }

    pub(super) fn select_last(
        &mut self,
        _: &menu::SelectLast,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Last, false, cx);
    }

    pub(super) fn extend_selection_up(
        &mut self,
        _: &ConflictsExtendSelectionUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Previous, true, cx);
    }

    pub(super) fn extend_selection_down(
        &mut self,
        _: &ConflictsExtendSelectionDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Next, true, cx);
    }

    fn move_page(&mut self, forwards: bool, extend: bool, cx: &mut Context<Self>) {
        let viewport_height = self
            .table_scroll
            .0
            .borrow()
            .last_item_size
            .map_or(0., |size| size.item.height.as_f32());
        let rows = page_row_count(viewport_height, forwards);
        let movement = if forwards {
            SelectionMove::PageNext(rows)
        } else {
            SelectionMove::PagePrevious(rows)
        };
        self.move_selection(movement, extend, cx);
    }

    pub(super) fn page_up(
        &mut self,
        _: &ConflictsPageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(false, false, cx);
    }

    pub(super) fn page_down(
        &mut self,
        _: &ConflictsPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(true, false, cx);
    }

    pub(super) fn extend_page_up(
        &mut self,
        _: &ConflictsExtendPageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(false, true, cx);
    }

    pub(super) fn extend_page_down(
        &mut self,
        _: &ConflictsExtendPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(true, true, cx);
    }

    pub(super) fn extend_to_first_row(
        &mut self,
        _: &ConflictsExtendToFirstRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::First, true, cx);
    }

    pub(super) fn extend_to_last_row(
        &mut self,
        _: &ConflictsExtendToLastRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(SelectionMove::Last, true, cx);
    }

    pub(super) fn select_all_rows(
        &mut self,
        _: &ConflictsSelectAllRows,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.state.select_all();
        cx.notify();
    }

    pub(super) fn collapse_row(
        &mut self,
        _: &ConflictsCollapseRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.state.collapse_or_expand_single_selection(false);
        self.scroll_selection_into_view();
        cx.notify();
    }

    pub(super) fn expand_row(
        &mut self,
        _: &ConflictsExpandRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.state.collapse_or_expand_single_selection(true);
        self.scroll_selection_into_view();
        cx.notify();
    }

    pub(super) fn confirm(
        &mut self,
        _: &menu::Confirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_default(window, cx);
    }

    pub(super) fn secondary_confirm(
        &mut self,
        _: &menu::SecondaryConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_default(window, cx);
    }

    fn activate_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.is_confirmation())
        {
            self.answer_confirmation(true, window, cx);
            return;
        }
        if self.input_blocked() {
            return;
        }
        match self.state.default_button() {
            DefaultButton::AcceptAndFinish => {
                if self.state.accept_and_finish_enabled() {
                    self.accept_and_finish(window, cx);
                }
            }
            DefaultButton::ReviewOrResolve => self.review_button_pressed(window, cx),
        }
    }

    pub(super) fn cancel(&mut self, _: &menu::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(overlay) = &self.overlay {
            if overlay.is_confirmation() {
                self.answer_confirmation(false, window, cx);
            } else {
                overlay.cancel_progress();
            }
            return;
        }
        if self.speed_search.take().is_some() {
            cx.notify();
            return;
        }
        if self.closing {
            return;
        }
        cx.propagate();
    }

    pub(super) fn handle_row_mouse_down(
        &mut self,
        id: RowId,
        modifiers: Modifiers,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.speed_search = None;
        window.focus(&self.focus_handle, cx);
        if click_count == 2 {
            self.auto_resolve_status = None;
            let expandable = self
                .state
                .rows()
                .iter()
                .find(|row| row.id == id)
                .is_some_and(|row| !matches!(row.kind, TreeRowKind::File { .. }));
            if expandable {
                self.toggle_row_expanded(id, cx);
            } else {
                self.open_merge_windows(window, cx);
            }
            return;
        }
        if modifiers.platform {
            self.state.toggle_selection(id);
        } else if modifiers.shift {
            self.state.extend_selection_to(id);
        } else {
            self.state.select_only(id);
        }
        cx.notify();
    }

    pub(super) fn handle_row_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.deploy_context_menu(position, window, cx);
    }

    pub(super) fn handle_inline_accept(
        &mut self,
        side: merge_diff::Side,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked() {
            return;
        }
        self.accept_for_resolution(side, window, cx);
    }

    pub(super) fn toggle_row_expanded(&mut self, id: RowId, cx: &mut Context<Self>) {
        let expanded = self.state.is_expanded(&id);
        self.state.set_expanded(&id, !expanded);
        cx.notify();
    }

    pub(super) fn set_hovered_row(&mut self, id: Option<RowId>, cx: &mut Context<Self>) {
        if self.hovered_row == id {
            return;
        }
        self.hovered_row = id;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_page_down_moves_by_the_viewport_rows_and_at_least_one_row() {
        assert_eq!(page_row_count(280., true), 10);
        assert_eq!(page_row_count(291., true), 10);
        assert_eq!(page_row_count(10., true), 1);
        assert_eq!(page_row_count(0., true), 1);
    }

    #[test]
    fn merge_tool_page_up_moves_by_the_rows_covering_the_viewport() {
        assert_eq!(page_row_count(280., false), 10);
        assert_eq!(page_row_count(281., false), 11);
        assert_eq!(page_row_count(0., false), 0);
    }
}
