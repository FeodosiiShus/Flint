use gpui::Context;
use merge_diff::{Side, ThreeSide};

use super::MergeConflictModel;

const COMMAND_ACCEPT_CHANGE: &str = "Accept change";
const COMMAND_IGNORE_CHANGE: &str = "Ignore change";
const COMMAND_RESOLVE_CONFLICT: &str = "Resolve conflict";
const COMMAND_RESET_CHANGE: &str = "Reset resolved change";
const COMMAND_RESOLVE_AUTOMATICALLY: &str = "Resolve Automatically";
const COMMAND_APPLY_NON_CONFLICTING: &str = "Apply non-conflicting changes";
const COMMAND_RESOLVE_SIMPLE_CONFLICTS: &str = "Resolve simple conflicted changes";
const COMMAND_REVERT_RESOLUTION: &str = "Revert conflict resolution";

impl MergeConflictModel {
    pub(crate) fn run_accept_change(
        &mut self,
        index: usize,
        side: Side,
        resolve_change: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(COMMAND_ACCEPT_CHANGE, Some(&[index]), cx, |model, cx| {
            model.replace_change(index, side, resolve_change, cx);
        })
    }

    pub(crate) fn run_ignore_change(
        &mut self,
        index: usize,
        side: Side,
        resolve_change: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(COMMAND_IGNORE_CHANGE, Some(&[index]), cx, |model, cx| {
            model.ignore_change(index, side, resolve_change, cx);
        })
    }

    pub(crate) fn run_resolve_change_automatically(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(COMMAND_RESOLVE_CONFLICT, Some(&[index]), cx, |model, cx| {
            model.resolve_change_automatically(index, ThreeSide::Base, cx);
        })
    }

    pub(crate) fn run_replace_changes(
        &mut self,
        command_name: &str,
        indices: &[usize],
        side: Side,
        resolve_changes: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(command_name, Some(indices), cx, |model, cx| {
            for index in indices.iter().copied() {
                model.replace_change(index, side, resolve_changes, cx);
            }
        })
    }

    pub(crate) fn run_ignore_changes_on_side(
        &mut self,
        command_name: &str,
        indices: &[usize],
        side: Side,
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(command_name, Some(indices), cx, |model, cx| {
            for index in indices.iter().copied() {
                model.ignore_change(index, side, false, cx);
            }
        })
    }

    pub(crate) fn run_mark_changes_resolved(
        &mut self,
        command_name: &str,
        indices: &[usize],
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(command_name, Some(indices), cx, |model, cx| {
            for index in indices.iter().copied() {
                model.mark_change_resolved(index, cx);
            }
        })
    }

    pub(crate) fn run_reset_resolved_changes(
        &mut self,
        command_name: &str,
        indices: &[usize],
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(command_name, Some(indices), cx, |model, cx| {
            for index in indices.iter().copied() {
                model.reset_resolved_change(index, false, cx);
            }
        })
    }

    pub(crate) fn run_resolve_changes_automatically(
        &mut self,
        command_name: &str,
        indices: &[usize],
        cx: &mut Context<Self>,
    ) -> bool {
        self.execute_merge_command(command_name, Some(indices), cx, |model, cx| {
            model.resolve_changes_automatically(indices, ThreeSide::Base, cx);
        })
    }

    pub(crate) fn run_apply_non_conflicted_changes(
        &mut self,
        side: ThreeSide,
        cx: &mut Context<Self>,
    ) -> bool {
        let non_conflicted = self.indices_where(|change| !change.is_conflict());
        self.execute_merge_command(COMMAND_APPLY_NON_CONFLICTING, None, cx, |model, cx| {
            model.resolve_changes_automatically(&non_conflicted, side, cx);
        })
    }

    pub(crate) fn run_apply_resolvable_conflicted_changes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let all_changes: Vec<usize> = (0..self.changes.len()).collect();
        self.execute_merge_command(COMMAND_RESOLVE_SIMPLE_CONFLICTS, None, cx, |model, cx| {
            model.resolve_changes_automatically(&all_changes, ThreeSide::Base, cx);
        })
    }

    pub(crate) fn run_revert_conflict_resolution(&mut self, cx: &mut Context<Self>) -> bool {
        let resolved = self.resolved_changes();
        self.execute_merge_command(
            COMMAND_REVERT_RESOLUTION,
            Some(&resolved),
            cx,
            |model, cx| {
                for index in resolved.iter().copied() {
                    model.reset_resolved_change(index, false, cx);
                }
            },
        )
    }

    pub(crate) fn run_accept_side(&mut self, side: Side, cx: &mut Context<Self>) -> bool {
        let all_changes: Vec<usize> = (0..self.changes.len()).collect();
        self.execute_merge_command(
            COMMAND_RESOLVE_CONFLICT,
            Some(&all_changes),
            cx,
            |model, cx| {
                model.reset_all_changes(cx);
                for index in all_changes.iter().copied() {
                    model.replace_change(index, side, true, cx);
                }
            },
        )
    }

    pub(crate) fn run_apply_changes(&mut self, cx: &mut Context<Self>) -> bool {
        let unresolved = self.unresolved_changes();
        self.execute_merge_command(
            COMMAND_APPLY_NON_CONFLICTING,
            Some(&unresolved),
            cx,
            |model, cx| {
                model.mark_all_changes_resolved(cx);
            },
        )
    }

    pub(crate) fn run_reset_all_changes(&mut self, cx: &mut Context<Self>) -> bool {
        let all_changes: Vec<usize> = (0..self.changes.len()).collect();
        self.execute_merge_command(COMMAND_RESET_CHANGE, Some(&all_changes), cx, |model, cx| {
            model.reset_all_changes(cx);
        })
    }

    pub(crate) fn resolve_all_changes_automatically(&mut self, cx: &mut Context<Self>) -> bool {
        let auto_resolvable = self.auto_resolvable_changes();
        if auto_resolvable.is_empty() {
            return false;
        }
        let success = self.execute_merge_command(
            COMMAND_RESOLVE_AUTOMATICALLY,
            Some(&auto_resolvable),
            cx,
            |model, cx| {
                model.resolve_changes_automatically(&auto_resolvable, ThreeSide::Base, cx);
            },
        );
        if success {
            self.was_reviewed = false;
        }
        success
    }

    pub(crate) fn accept_revision_for_side(&mut self, side: Side, cx: &mut Context<Self>) -> bool {
        let all_changes: Vec<usize> = (0..self.changes.len()).collect();
        self.execute_merge_command(
            COMMAND_RESOLVE_CONFLICT,
            Some(&all_changes),
            cx,
            |model, cx| {
                model.reset_all_changes(cx);
                model.replace_all_changes(side, cx);
                model.mark_reviewed();
            },
        )
    }
}
