use std::ops::Range;

use gpui::Context;
use merge_diff::{ResolutionStrategy, Side, ThreeSide};

use super::{
    MergeConflictModel, MergeModelEvent,
    change::{opposite_three_side_of, side_index, three_side_of},
    line_text::tokenize_lines,
};

impl MergeConflictModel {
    pub(crate) fn mark_change_resolved(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get_mut(index) else {
            return;
        };
        if change.is_resolved() {
            return;
        }
        change.resolved = [true, true];
        cx.emit(MergeModelEvent::ChangeResolved(index));
        self.invalidate_change(index, cx);
    }

    fn mark_side_resolved(&mut self, index: usize, side: Side, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get_mut(index) else {
            return;
        };
        if change.is_resolved() {
            return;
        }
        change.resolved[side_index(side)] = true;
        let is_resolved = change.is_resolved();
        cx.emit(MergeModelEvent::ChangeSideResolved { index, side });
        if is_resolved {
            cx.emit(MergeModelEvent::ChangeResolved(index));
        }
        self.invalidate_change(index, cx);
    }

    pub(crate) fn mark_all_changes_resolved(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.changes.len() {
            self.mark_change_resolved(index, cx);
        }
    }

    pub(crate) fn ignore_change(
        &mut self,
        index: usize,
        side: Side,
        resolve_change: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(change) = self.changes.get(index) else {
            return;
        };
        if !change.is_conflict() || resolve_change {
            self.mark_change_resolved(index, cx);
        } else {
            self.mark_side_resolved(index, side, cx);
        }
    }

    pub(crate) fn replace_change(
        &mut self,
        index: usize,
        side: Side,
        resolve_change: bool,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let change = self.changes.get(index)?;
        if change.is_side_resolved(side) {
            return None;
        }
        let source_side = three_side_of(side);
        if !change.is_change(source_side) {
            self.mark_change_resolved(index, cx);
            return None;
        }

        let opposite_range = change.fragment_lines(opposite_three_side_of(side));
        let opposite_is_empty = opposite_range.start == opposite_range.end;
        let is_conflict = change.is_conflict();
        let append = change.is_oneside_applied_conflict();
        let source_range = change.fragment_lines(source_side);
        let new_content = self
            .sides
            .side(source_side)
            .lines(source_range.start, source_range.end);

        let new_line_start;
        if is_conflict {
            if append {
                new_line_start = self.result_lines(index).end;
                self.append_result_lines(index, &new_content, cx);
            } else {
                self.replace_result_lines(index, &new_content, cx);
                new_line_start = self.result_lines(index).start;
            }
            if resolve_change || opposite_is_empty {
                self.mark_change_resolved(index, cx);
            } else {
                if let Some(change) = self.changes.get_mut(index) {
                    change.oneside_applied_conflict = true;
                }
                self.mark_side_resolved(index, side, cx);
            }
        } else {
            self.replace_result_lines(index, &new_content, cx);
            new_line_start = self.result_lines(index).start;
            self.mark_change_resolved(index, cx);
        }
        Some(new_line_start..self.result_lines(index).end)
    }

    pub(crate) fn reset_resolved_change(
        &mut self,
        index: usize,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(change) = self.changes.get(index) else {
            return;
        };
        if !force && !change.is_resolved() {
            return;
        }
        let base_range = change.fragment_lines(ThreeSide::Base);
        let base_content = self.sides.base.lines(base_range.start, base_range.end);
        self.replace_result_lines(index, &base_content, cx);
        cx.emit(MergeModelEvent::ChangeReset(index));
        if let Some(change) = self.changes.get_mut(index) {
            change.reset_state();
        }
        self.invalidate_change(index, cx);
    }

    pub(crate) fn reset_all_changes(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.changes.len() {
            self.reset_resolved_change(index, true, cx);
        }
        self.reset_output_content(cx);
    }

    pub(crate) fn replace_all_changes(&mut self, side: Side, cx: &mut Context<Self>) {
        for index in 0..self.changes.len() {
            self.replace_change(index, side, true, cx);
        }
        self.chosen_side = Some(side);
    }

    pub(crate) fn can_resolve_change_automatically(&self, index: usize, side: ThreeSide) -> bool {
        if self.conflict_type.is_modify_delete() {
            return false;
        }
        let Some(change) = self.changes.get(index) else {
            return false;
        };
        if change.is_conflict() {
            side == ThreeSide::Base
                && change.conflict_type.can_be_resolved()
                && !change.is_side_resolved(Side::Left)
                && !change.is_side_resolved(Side::Right)
                && (change.conflict_type.resolution != Some(ResolutionStrategy::Text)
                    || !self.is_change_range_modified(index))
        } else {
            !change.is_resolved() && change.is_change(side) && !self.is_change_range_modified(index)
        }
    }

    fn is_change_range_modified(&self, index: usize) -> bool {
        let Some(change) = self.changes.get(index) else {
            return false;
        };
        let base_range = change.fragment_lines(ThreeSide::Base);
        let result_range = self.result_lines(index);
        self.sides
            .base
            .lines_content(base_range.start, base_range.end)
            != self
                .result
                .lines_content(result_range.start, result_range.end)
    }

    pub(crate) fn resolve_change_automatically(
        &mut self,
        index: usize,
        side: ThreeSide,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if !self.can_resolve_change_automatically(index, side) {
            return None;
        }
        let change = self.changes.get(index)?;
        if change.is_conflict() {
            match change.conflict_type.resolution {
                Some(ResolutionStrategy::Text) => {
                    let new_content = self.resolve_text_conflict(index)?;
                    Some(self.replace_with_new_content(index, &new_content, cx))
                }
                _ => None,
            }
        } else {
            let master_side = match side {
                ThreeSide::Left => Side::Left,
                ThreeSide::Base if change.is_change(ThreeSide::Left) => Side::Left,
                ThreeSide::Base | ThreeSide::Right => Side::Right,
            };
            self.replace_change(index, master_side, false, cx)
        }
    }

    fn resolve_text_conflict(&self, index: usize) -> Option<String> {
        let change = self.changes.get(index)?;
        let left_range = change.fragment_lines(ThreeSide::Left);
        let right_range = change.fragment_lines(ThreeSide::Right);
        let result_range = self.result_lines(index);
        let resolved = merge_diff::try_resolve_conflict(
            self.sides
                .left
                .lines_content(left_range.start, left_range.end),
            self.result
                .lines_content(result_range.start, result_range.end),
            self.sides
                .right
                .lines_content(right_range.start, right_range.end),
        );
        if resolved.is_none() {
            log::warn!("Can't resolve conflicting change {index}");
        }
        resolved
    }

    fn replace_with_new_content(
        &mut self,
        index: usize,
        new_content: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let lines = tokenize_lines(new_content);
        self.replace_result_lines(index, &lines, cx);
        self.mark_change_resolved(index, cx);
        self.result_lines(index)
    }

    pub(super) fn resolve_changes_automatically(
        &mut self,
        indices: &[usize],
        side: ThreeSide,
        cx: &mut Context<Self>,
    ) {
        for index in indices.iter().copied() {
            self.resolve_change_automatically(index, side, cx);
        }
    }
}
