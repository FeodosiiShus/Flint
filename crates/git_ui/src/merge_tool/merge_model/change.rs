use std::ops::Range;

use merge_diff::{MergeConflictType, MergeRange, Side, ThreeSide};

pub(crate) fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

pub(crate) fn three_side_of(side: Side) -> ThreeSide {
    match side {
        Side::Left => ThreeSide::Left,
        Side::Right => ThreeSide::Right,
    }
}

pub(crate) fn opposite_three_side_of(side: Side) -> ThreeSide {
    match side {
        Side::Left => ThreeSide::Right,
        Side::Right => ThreeSide::Left,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MergeChange {
    pub(super) index: usize,
    pub(super) fragment: MergeRange,
    pub(super) conflict_type: MergeConflictType,
    pub(super) resolved: [bool; 2],
    pub(super) oneside_applied_conflict: bool,
}

impl MergeChange {
    pub(super) fn new(
        index: usize,
        fragment: MergeRange,
        conflict_type: MergeConflictType,
    ) -> Self {
        Self {
            index,
            fragment,
            conflict_type,
            resolved: [false; 2],
            oneside_applied_conflict: false,
        }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn conflict_type(&self) -> &MergeConflictType {
        &self.conflict_type
    }

    pub(crate) fn is_conflict(&self) -> bool {
        self.conflict_type.is_conflict()
    }

    pub(crate) fn is_resolved(&self) -> bool {
        self.resolved[0] && self.resolved[1]
    }

    pub(crate) fn is_side_resolved(&self, side: Side) -> bool {
        self.resolved[side_index(side)]
    }

    pub(crate) fn is_resolved_for(&self, side: ThreeSide) -> bool {
        match side {
            ThreeSide::Left => self.resolved[0],
            ThreeSide::Base => self.is_resolved(),
            ThreeSide::Right => self.resolved[1],
        }
    }

    pub(crate) fn is_oneside_applied_conflict(&self) -> bool {
        self.oneside_applied_conflict
    }

    pub(crate) fn is_change(&self, side: ThreeSide) -> bool {
        self.conflict_type.is_change(side)
    }

    pub(crate) fn fragment_lines(&self, side: ThreeSide) -> Range<usize> {
        self.fragment.side(side)
    }

    pub(super) fn store_state(&self, start_line: i32, end_line: i32) -> ChangeState {
        ChangeState {
            index: self.index,
            start_line,
            end_line,
            resolved: self.resolved,
            oneside_applied_conflict: self.oneside_applied_conflict,
        }
    }

    pub(super) fn restore_state(&mut self, state: &ChangeState) {
        self.resolved = state.resolved;
        self.oneside_applied_conflict = state.oneside_applied_conflict;
    }

    pub(super) fn reset_state(&mut self) {
        self.resolved = [false; 2];
        self.oneside_applied_conflict = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChangeState {
    pub(crate) index: usize,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) resolved: [bool; 2],
    pub(crate) oneside_applied_conflict: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChangeCounters {
    pub(crate) changes: usize,
    pub(crate) conflicts: usize,
}

impl ChangeCounters {
    pub(crate) fn is_all_resolved(&self) -> bool {
        self.changes == 0 && self.conflicts == 0
    }
}

#[cfg(test)]
mod tests {
    use merge_diff::ConflictKind;

    use super::*;

    fn conflict() -> MergeChange {
        MergeChange::new(
            3,
            MergeRange::new(1..2, 1..3, 4..4),
            MergeConflictType::new(ConflictKind::Conflict, true, true),
        )
    }

    #[test]
    fn a_change_is_resolved_only_when_both_sides_are() {
        let mut change = conflict();
        change.resolved[side_index(Side::Left)] = true;

        assert!(!change.is_resolved());
        assert!(change.is_side_resolved(Side::Left));
        assert!(!change.is_side_resolved(Side::Right));
        assert!(change.is_resolved_for(ThreeSide::Left));
        assert!(!change.is_resolved_for(ThreeSide::Base));
        assert!(!change.is_resolved_for(ThreeSide::Right));

        change.resolved[side_index(Side::Right)] = true;
        assert!(change.is_resolved());
        assert!(change.is_resolved_for(ThreeSide::Base));
        assert!(change.is_resolved_for(ThreeSide::Right));
    }

    #[test]
    fn stored_state_restores_the_flags_after_a_reset() {
        let mut change = conflict();
        change.resolved = [true, false];
        change.oneside_applied_conflict = true;
        let state = change.store_state(5, 7);
        assert_eq!(
            state,
            ChangeState {
                index: 3,
                start_line: 5,
                end_line: 7,
                resolved: [true, false],
                oneside_applied_conflict: true,
            }
        );

        change.reset_state();
        assert!(!change.is_side_resolved(Side::Left));
        assert!(!change.is_oneside_applied_conflict());

        change.restore_state(&state);
        assert!(change.is_side_resolved(Side::Left));
        assert!(!change.is_side_resolved(Side::Right));
        assert!(change.is_oneside_applied_conflict());
    }

    #[test]
    fn sides_map_to_their_three_way_counterparts() {
        assert_eq!(three_side_of(Side::Left), ThreeSide::Left);
        assert_eq!(three_side_of(Side::Right), ThreeSide::Right);
        assert_eq!(opposite_three_side_of(Side::Left), ThreeSide::Right);
        assert_eq!(opposite_three_side_of(Side::Right), ThreeSide::Left);
    }

    #[test]
    fn counters_are_all_resolved_only_when_both_are_zero() {
        assert!(ChangeCounters::default().is_all_resolved());
        assert!(
            !ChangeCounters {
                changes: 1,
                conflicts: 0
            }
            .is_all_resolved()
        );
        assert!(
            !ChangeCounters {
                changes: 0,
                conflicts: 2
            }
            .is_all_resolved()
        );
    }
}
