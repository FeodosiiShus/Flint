use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

use git::repository::RepoPath;
use gpui::{AppContext as _, Entity, Subscription, TestAppContext};
use language::Capability;
use merge_diff::{
    ComparisonPolicy, ConflictKind, MergeConflictType, MergeRange, ResolutionStrategy, Side,
    ThreeSide,
};

use super::{
    FileConflictType, MergeBuffers, MergeConflictModel, MergeDiffData, MergeModelError,
    MergeModelEvent, MergeTexts,
    iterative_data_holder::{FileProgress, MergeConflictIterativeDataHolder, MergeModelRequest},
    messages::MergeStatus,
};

struct Fixture {
    texts: MergeTexts,
    data: MergeDiffData,
}

fn joined(lines: &[&str]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn conflict_type(
    kind: ConflictKind,
    left_change: bool,
    right_change: bool,
    resolution: Option<ResolutionStrategy>,
) -> MergeConflictType {
    MergeConflictType {
        kind,
        left_change,
        right_change,
        resolution,
    }
}

fn fragment(left: (usize, usize), base: (usize, usize), right: (usize, usize)) -> MergeRange {
    MergeRange::new(left.0..left.1, base.0..base.1, right.0..right.1)
}

fn many_changes() -> Fixture {
    let base = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"];
    let left = ["a", "B", "c", "d", "e1", "f", "g", "i", "j", "k"];
    let right = ["a", "b", "c", "d", "e2", "f", "g", "h", "i", "j", "k2"];
    Fixture {
        texts: MergeTexts::new(&joined(&left), &joined(&base), &joined(&right)),
        data: MergeDiffData {
            fragments: vec![
                fragment((1, 2), (1, 2), (1, 2)),
                fragment((4, 5), (4, 5), (4, 5)),
                fragment((7, 7), (7, 8), (7, 8)),
                fragment((9, 10), (10, 11), (10, 11)),
            ],
            conflict_types: vec![
                conflict_type(
                    ConflictKind::Modified,
                    true,
                    false,
                    Some(ResolutionStrategy::Default),
                ),
                conflict_type(ConflictKind::Conflict, true, true, None),
                conflict_type(
                    ConflictKind::Deleted,
                    true,
                    false,
                    Some(ResolutionStrategy::Default),
                ),
                conflict_type(
                    ConflictKind::Modified,
                    false,
                    true,
                    Some(ResolutionStrategy::Default),
                ),
            ],
            ignore_policy: ComparisonPolicy::Default,
        },
    }
}

fn two_line_conflict() -> Fixture {
    Fixture {
        texts: MergeTexts::new(
            &joined(&["a", "L1", "L2", "d"]),
            &joined(&["a", "b", "c", "d"]),
            &joined(&["a", "R1", "d"]),
        ),
        data: MergeDiffData {
            fragments: vec![fragment((1, 3), (1, 3), (1, 2))],
            conflict_types: vec![conflict_type(ConflictKind::Conflict, true, true, None)],
            ignore_policy: ComparisonPolicy::Default,
        },
    }
}

fn text_resolvable_conflict() -> Fixture {
    Fixture {
        texts: MergeTexts::new(
            &joined(&["a", "FOO bar baz", "c"]),
            &joined(&["a", "foo bar baz", "c"]),
            &joined(&["a", "foo bar BAZ", "c"]),
        ),
        data: MergeDiffData {
            fragments: vec![fragment((1, 2), (1, 2), (1, 2))],
            conflict_types: vec![conflict_type(
                ConflictKind::Conflict,
                true,
                true,
                Some(ResolutionStrategy::Text),
            )],
            ignore_policy: ComparisonPolicy::Default,
        },
    }
}

fn inserted_line() -> Fixture {
    Fixture {
        texts: MergeTexts::new(
            &joined(&["a", "X", "b"]),
            &joined(&["a", "b"]),
            &joined(&["a", "b"]),
        ),
        data: MergeDiffData {
            fragments: vec![fragment((1, 2), (1, 1), (1, 1))],
            conflict_types: vec![conflict_type(
                ConflictKind::Inserted,
                true,
                false,
                Some(ResolutionStrategy::Default),
            )],
            ignore_policy: ComparisonPolicy::Default,
        },
    }
}

fn modified_against_deleted_conflict() -> Fixture {
    Fixture {
        texts: MergeTexts::new(
            &joined(&["a", "B", "c"]),
            &joined(&["a", "b", "c"]),
            &joined(&["a", "c"]),
        ),
        data: MergeDiffData {
            fragments: vec![fragment((1, 2), (1, 2), (1, 1))],
            conflict_types: vec![conflict_type(ConflictKind::Conflict, true, true, None)],
            ignore_policy: ComparisonPolicy::Default,
        },
    }
}

fn new_uninitialized_model(
    cx: &mut TestAppContext,
    fixture: &Fixture,
    conflict_type: FileConflictType,
) -> Entity<MergeConflictModel> {
    cx.new(|cx| {
        let buffers = MergeBuffers::detached(&fixture.texts, None, cx);
        MergeConflictModel::new(buffers, &fixture.texts, conflict_type, cx)
    })
}

fn new_model(cx: &mut TestAppContext, fixture: &Fixture) -> Entity<MergeConflictModel> {
    let model = new_uninitialized_model(cx, fixture, FileConflictType::Default);
    model.update(cx, |model, cx| {
        model
            .apply_diff_data(Arc::new(fixture.data.clone()), cx)
            .expect("diff data applies to a writable result buffer");
    });
    model
}

fn result_text(cx: &mut TestAppContext, model: &Entity<MergeConflictModel>) -> String {
    model.read_with(cx, |model, cx| model.current_result_text(cx))
}

fn result_ranges(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
) -> Vec<(usize, usize)> {
    model.read_with(cx, |model, _| {
        (0..model.changes().len())
            .map(|index| {
                let range = model.result_lines(index);
                (range.start, range.end)
            })
            .collect()
    })
}

fn resolved_flags(cx: &mut TestAppContext, model: &Entity<MergeConflictModel>) -> Vec<bool> {
    model.read_with(cx, |model, _| {
        model
            .changes()
            .iter()
            .map(|change| change.is_resolved())
            .collect()
    })
}

fn side_resolved(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
    index: usize,
    side: Side,
) -> bool {
    model.read_with(cx, |model, _| {
        model
            .change(index)
            .is_some_and(|change| change.is_side_resolved(side))
    })
}

fn oneside_applied(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
    index: usize,
) -> bool {
    model.read_with(cx, |model, _| {
        model
            .change(index)
            .is_some_and(|change| change.is_oneside_applied_conflict())
    })
}

fn edit_result(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
    range: Range<usize>,
    new_text: &str,
) {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, cx| {
        buffer.edit([(range, new_text.to_string())], None, cx);
    });
}

fn undo_result(cx: &mut TestAppContext, model: &Entity<MergeConflictModel>) -> bool {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, cx| buffer.undo(cx)).is_some()
}

fn redo_result(cx: &mut TestAppContext, model: &Entity<MergeConflictModel>) -> bool {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, cx| buffer.redo(cx)).is_some()
}

fn make_result_read_only(cx: &mut TestAppContext, model: &Entity<MergeConflictModel>) {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, cx| {
        buffer.set_capability(Capability::ReadOnly, cx)
    });
}

fn record_events(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
) -> (Rc<RefCell<Vec<MergeModelEvent>>>, Subscription) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(model, move |_, event: &MergeModelEvent, _| {
            events.borrow_mut().push(*event);
        })
    });
    (events, subscription)
}

const BASE_TEXT: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";

#[gpui::test]
async fn initial_state_matches_the_base_text_and_counts_unresolved_changes(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    model.read_with(cx, |model, _| {
        assert_eq!(
            model.status(),
            MergeStatus::Differences("3 changes, 1 conflict".to_string())
        );
        assert!(!model.content_modified());
        assert!(!model.is_fully_resolved());
        assert_eq!(model.unresolved_changes(), vec![0, 1, 2, 3]);
    });
}

#[gpui::test]
async fn accepting_a_non_conflicting_change_replaces_its_lines_and_resolves_it(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    let (events, _subscription) = record_events(cx, &model);

    let applied = model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    assert!(applied);
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
    assert!(model.read_with(cx, |model, _| model.content_modified()));
    let events = events.borrow();
    assert!(events.contains(&MergeModelEvent::ChangeResolved(0)));
    assert!(events.contains(&MergeModelEvent::ChangeProcessed(0)));
    assert!(events.contains(&MergeModelEvent::ContentModifiedChanged(true)));
    assert!(events.contains(&MergeModelEvent::BulkProcessingFinished));
}

#[gpui::test]
async fn accepting_a_side_that_did_not_change_only_marks_the_change_resolved(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
    assert!(undo_result(cx, &model));
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
}

#[gpui::test]
async fn accepting_both_sides_of_a_conflict_applies_the_left_side_then_appends_the_right(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &two_line_conflict());

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 3)]);
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(!side_resolved(cx, &model, 0, Side::Right));
    assert!(oneside_applied(cx, &model, 0));
    assert_eq!(resolved_flags(cx, &model), vec![false]);

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nR1\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 4)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);
}

#[gpui::test]
async fn accepting_an_inserted_change_widens_its_empty_result_range(cx: &mut TestAppContext) {
    let model = new_model(cx, &inserted_line());
    assert_eq!(result_ranges(cx, &model), vec![(1, 1)]);

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    assert_eq!(result_text(cx, &model), "a\nX\nb\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 2)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nb\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 1)]);
    assert_eq!(resolved_flags(cx, &model), vec![false]);
}

#[gpui::test]
async fn accepting_a_side_against_a_deleted_opposite_side_resolves_the_conflict_at_once(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &modified_against_deleted_conflict());

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    assert_eq!(result_text(cx, &model), "a\nB\nc\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 2)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(side_resolved(cx, &model, 0, Side::Right));
    assert!(!oneside_applied(cx, &model, 0));
}

#[gpui::test]
async fn accepting_the_deleting_side_first_lets_the_other_side_append_into_the_emptied_range(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &modified_against_deleted_conflict());

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nc\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 1)]);
    assert_eq!(resolved_flags(cx, &model), vec![false]);
    assert!(side_resolved(cx, &model, 0, Side::Right));
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(oneside_applied(cx, &model, 0));

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nB\nc\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 2)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);
}

#[gpui::test]
async fn undo_and_redo_restore_text_and_per_change_state_together(cx: &mut TestAppContext) {
    let model = new_model(cx, &two_line_conflict());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 3)]);
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(!side_resolved(cx, &model, 0, Side::Right));
    assert!(oneside_applied(cx, &model, 0));

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nb\nc\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 3)]);
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(!oneside_applied(cx, &model, 0));

    assert!(redo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nd\n");
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(oneside_applied(cx, &model, 0));

    assert!(redo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nR1\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 4)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    assert!(!redo_result(cx, &model));
}

#[gpui::test]
async fn commands_without_text_changes_are_undoable_and_clear_the_redo_history(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &two_line_conflict());

    model.update(cx, |model, cx| {
        model.run_ignore_change(0, Side::Left, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nb\nc\nd\n");
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(!oneside_applied(cx, &model, 0));
    assert_eq!(resolved_flags(cx, &model), vec![false]);

    assert!(undo_result(cx, &model));
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(redo_result(cx, &model));
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(undo_result(cx, &model));
    assert!(!side_resolved(cx, &model, 0, Side::Left));

    model.update(cx, |model, cx| {
        model.run_ignore_change(0, Side::Right, true, cx)
    });
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    assert!(!redo_result(cx, &model));
    assert!(undo_result(cx, &model));
    assert_eq!(resolved_flags(cx, &model), vec![false]);
}

#[gpui::test]
async fn ignoring_one_side_of_a_conflict_reports_a_side_event(cx: &mut TestAppContext) {
    let model = new_model(cx, &two_line_conflict());
    let (events, _subscription) = record_events(cx, &model);

    model.update(cx, |model, cx| {
        model.run_ignore_change(0, Side::Left, false, cx)
    });

    let events = events.borrow();
    assert!(events.contains(&MergeModelEvent::ChangeSideResolved {
        index: 0,
        side: Side::Left
    }));
    assert!(!events.contains(&MergeModelEvent::ChangeResolved(0)));
}

#[gpui::test]
async fn deleting_the_lines_of_a_deleted_change_resolves_it_and_undo_unresolves_it(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());

    edit_result(cx, &model, 14..16, "");

    assert_eq!(result_text(cx, &model), "a\nb\nc\nd\ne\nf\ng\ni\nj\nk\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (8, 8), (9, 10)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![false, false, true, false]);

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
}

#[gpui::test]
async fn typing_before_changes_shifts_them_and_undo_shifts_them_back(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());

    edit_result(cx, &model, 0..0, "x\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(2, 3), (5, 6), (8, 9), (11, 12)]
    );
    assert!(model.read_with(cx, |model, _| model.content_modified()));

    assert!(undo_result(cx, &model));
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
}

#[gpui::test]
async fn typing_inside_a_change_extends_its_range(cx: &mut TestAppContext) {
    let model = new_model(cx, &two_line_conflict());

    edit_result(cx, &model, 4..4, "x\n");

    assert_eq!(result_text(cx, &model), "a\nb\nx\nc\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 4)]);
}

#[gpui::test]
async fn result_lines_follow_edits_while_left_and_right_lines_stay_fixed(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());

    edit_result(cx, &model, 0..0, "x\n");

    model.read_with(cx, |model, _| {
        let lines = |index: usize, side: ThreeSide| {
            (model.start_line(index, side), model.end_line(index, side))
        };
        assert_eq!(lines(2, ThreeSide::Left), (7, 7));
        assert_eq!(lines(2, ThreeSide::Base), (8, 9));
        assert_eq!(lines(2, ThreeSide::Right), (7, 8));
        assert_eq!(lines(3, ThreeSide::Left), (9, 10));
        assert_eq!(lines(3, ThreeSide::Base), (11, 12));
        assert_eq!(lines(3, ThreeSide::Right), (10, 11));
    });
}

#[gpui::test]
async fn undoing_a_deletion_spanning_a_change_restores_the_damaged_range(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());

    edit_result(cx, &model, 2..6, "");
    assert_eq!(result_text(cx, &model), "a\nd\ne\nf\ng\nh\ni\nj\nk\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(2, 2), (2, 3), (5, 6), (8, 9)]
    );

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
}

#[gpui::test]
async fn a_command_is_undone_separately_from_the_preceding_user_edit(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());
    edit_result(cx, &model, 0..0, "x");
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    assert_eq!(
        result_text(cx, &model),
        "xa\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n"
    );

    assert!(undo_result(cx, &model));
    assert_eq!(
        result_text(cx, &model),
        "xa\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n"
    );
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
}

#[gpui::test]
async fn a_user_edit_after_an_undo_discards_the_redo_history(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    assert!(undo_result(cx, &model));

    edit_result(cx, &model, 0..0, "x");

    assert!(!redo_result(cx, &model));
    assert_eq!(
        result_text(cx, &model),
        "xa\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n"
    );
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
}

#[gpui::test]
async fn applying_non_conflicting_changes_respects_the_requested_side(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());

    model.read_with(cx, |model, _| {
        assert!(model.has_non_conflicted_changes(ThreeSide::Left));
        assert!(model.has_non_conflicted_changes(ThreeSide::Right));
        assert!(model.has_non_conflicted_changes(ThreeSide::Base));
    });

    model.update(cx, |model, cx| {
        model.run_apply_non_conflicted_changes(ThreeSide::Left, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\ni\nj\nk\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 7), (9, 10)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![true, false, true, false]);

    model.update(cx, |model, cx| {
        model.run_apply_non_conflicted_changes(ThreeSide::Base, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\ni\nj\nk2\n");
    assert_eq!(resolved_flags(cx, &model), vec![true, false, true, true]);
    model.read_with(cx, |model, _| {
        assert_eq!(
            model.status(),
            MergeStatus::Differences("No changes, 1 conflict".to_string())
        );
        assert!(!model.has_non_conflicted_changes(ThreeSide::Base));
    });

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\ni\nj\nk\n");
    assert_eq!(resolved_flags(cx, &model), vec![true, false, true, false]);
}

#[gpui::test]
async fn resolve_automatically_applies_text_resolvable_conflicts_once(cx: &mut TestAppContext) {
    let model = new_model(cx, &text_resolvable_conflict());
    model.update(cx, |model, _| model.mark_reviewed());
    model.read_with(cx, |model, _| {
        assert_eq!(model.auto_resolvable_changes(), vec![0]);
        assert!(model.has_auto_resolvable_conflicted_changes());
    });

    let resolved = model.update(cx, |model, cx| model.resolve_all_changes_automatically(cx));

    assert!(resolved);
    assert_eq!(result_text(cx, &model), "a\nFOO bar BAZ\nc\n");
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    model.read_with(cx, |model, _| assert!(!model.was_reviewed()));
    assert!(!model.update(cx, |model, cx| model.resolve_all_changes_automatically(cx)));
}

#[gpui::test]
async fn manually_edited_text_conflicts_are_not_auto_resolvable(cx: &mut TestAppContext) {
    let model = new_model(cx, &text_resolvable_conflict());
    edit_result(cx, &model, 4..5, "x");

    model.read_with(cx, |model, _| {
        assert!(model.auto_resolvable_changes().is_empty());
    });
    assert!(!model.update(cx, |model, cx| model.resolve_all_changes_automatically(cx)));
}

#[gpui::test]
async fn file_level_modify_delete_conflicts_disable_automatic_resolution(cx: &mut TestAppContext) {
    let fixture = text_resolvable_conflict();
    let model = new_uninitialized_model(cx, &fixture, FileConflictType::ModifiedDeleted);
    model.update(cx, |model, cx| {
        model
            .apply_diff_data(Arc::new(fixture.data.clone()), cx)
            .expect("diff data applies");
    });

    model.read_with(cx, |model, _| {
        assert!(model.auto_resolvable_changes().is_empty());
        assert!(!model.has_auto_resolvable_conflicted_changes());
    });
}

#[gpui::test]
async fn a_read_only_result_rejects_diff_data_and_merge_commands(cx: &mut TestAppContext) {
    let fixture = two_line_conflict();
    let uninitialized = new_uninitialized_model(cx, &fixture, FileConflictType::Default);
    make_result_read_only(cx, &uninitialized);

    let applied = uninitialized.update(cx, |model, cx| {
        model.apply_diff_data(Arc::new(fixture.data.clone()), cx)
    });

    assert_eq!(applied, Err(MergeModelError::ReadOnlyResult));
    uninitialized.read_with(cx, |model, _| assert!(!model.is_initialized()));

    let model = new_model(cx, &fixture);
    make_result_read_only(cx, &model);

    let accepted = model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    assert!(!accepted);
    assert_eq!(result_text(cx, &model), "a\nb\nc\nd\n");
    assert_eq!(resolved_flags(cx, &model), vec![false]);
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(!model.read_with(cx, |model, _| model.content_modified()));
}

#[gpui::test]
async fn reverting_resolved_changes_restores_their_base_lines_in_one_undo_step(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    model.update(cx, |model, cx| {
        model.run_accept_change(3, Side::Right, false, cx)
    });
    assert_eq!(
        result_text(cx, &model),
        "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk2\n"
    );
    let (events, _subscription) = record_events(cx, &model);

    model.update(cx, |model, cx| model.run_revert_conflict_resolution(cx));

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
    assert!(events.borrow().contains(&MergeModelEvent::ChangeReset(0)));
    assert!(events.borrow().contains(&MergeModelEvent::ChangeReset(3)));

    assert!(undo_result(cx, &model));
    assert_eq!(
        result_text(cx, &model),
        "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk2\n"
    );
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, true]);
}

#[gpui::test]
async fn accepting_a_whole_side_yields_that_sides_text(cx: &mut TestAppContext) {
    let fixture = many_changes();
    let model = new_model(cx, &fixture);

    model.update(cx, |model, cx| model.run_accept_side(Side::Left, cx));

    assert_eq!(result_text(cx, &model), fixture.texts.left);
    assert_eq!(resolved_flags(cx, &model), vec![true; 4]);
    model.read_with(cx, |model, _| {
        assert_eq!(model.status(), MergeStatus::AllConflictsResolved);
    });

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
}

#[gpui::test]
async fn accept_revision_for_a_side_marks_the_file_reviewed_and_records_the_choice(
    cx: &mut TestAppContext,
) {
    let fixture = many_changes();
    let model = new_model(cx, &fixture);

    model.update(cx, |model, cx| {
        model.accept_revision_for_side(Side::Right, cx)
    });

    assert_eq!(result_text(cx, &model), fixture.texts.right);
    model.read_with(cx, |model, _| {
        assert!(model.was_reviewed());
        assert_eq!(model.chosen_side(), Some(Side::Right));
        assert!(model.is_fully_resolved());
        assert!(model.content_modified());
    });
}

#[gpui::test]
async fn applying_changes_marks_everything_resolved_without_touching_the_text(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    model.update(cx, |model, cx| model.run_apply_changes(cx));

    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n");
    assert_eq!(resolved_flags(cx, &model), vec![true; 4]);
    assert!(undo_result(cx, &model));
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
}

#[gpui::test]
async fn content_modified_is_overwritten_by_every_document_event(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());
    let (events, _subscription) = record_events(cx, &model);
    assert!(!model.read_with(cx, |model, _| model.content_modified()));

    edit_result(cx, &model, 0..0, "x");
    assert!(model.read_with(cx, |model, _| model.content_modified()));

    let length = result_text(cx, &model).len();
    edit_result(cx, &model, 0..length, "zzz");
    assert!(!model.read_with(cx, |model, _| model.content_modified()));

    model.update(cx, |model, cx| {
        model.run_ignore_change(0, Side::Left, true, cx)
    });
    assert!(model.read_with(cx, |model, _| model.content_modified()));
    let content_modified_changes: Vec<bool> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            MergeModelEvent::ContentModifiedChanged(value) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(content_modified_changes, vec![true, false, true]);
}

#[gpui::test]
async fn rediff_builds_changes_from_the_real_line_comparison(cx: &mut TestAppContext) {
    let fixture = many_changes();
    let model = new_uninitialized_model(cx, &fixture, FileConflictType::Default);
    assert_eq!(model.read_with(cx, |model, _| model.counters()), None);

    let task = model.update(cx, |model, cx| model.rediff(ComparisonPolicy::Default, cx));
    task.await.expect("rediff succeeds");

    model.read_with(cx, |model, _| {
        let fragments: Vec<MergeRange> = model
            .changes()
            .iter()
            .map(|change| change.fragment.clone())
            .collect();
        assert_eq!(fragments, fixture.data.fragments);
        let types: Vec<(ConflictKind, bool, bool, Option<ResolutionStrategy>)> = model
            .changes()
            .iter()
            .map(|change| {
                let conflict = change.conflict_type();
                (
                    conflict.kind,
                    conflict.left_change,
                    conflict.right_change,
                    conflict.resolution,
                )
            })
            .collect();
        assert_eq!(
            types,
            vec![
                (
                    ConflictKind::Modified,
                    true,
                    false,
                    Some(ResolutionStrategy::Default)
                ),
                (ConflictKind::Conflict, true, true, None),
                (
                    ConflictKind::Deleted,
                    true,
                    false,
                    Some(ResolutionStrategy::Default)
                ),
                (
                    ConflictKind::Modified,
                    false,
                    true,
                    Some(ResolutionStrategy::Default)
                ),
            ]
        );
        assert_eq!(
            model.status(),
            MergeStatus::Differences("3 changes, 1 conflict".to_string())
        );
    });
}

#[gpui::test]
async fn rediff_with_the_same_policy_keeps_the_resolution_state(cx: &mut TestAppContext) {
    let fixture = many_changes();
    let model = new_uninitialized_model(cx, &fixture, FileConflictType::Default);
    let task = model.update(cx, |model, cx| model.rediff(ComparisonPolicy::Default, cx));
    task.await.expect("rediff succeeds");
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    let task = model.update(cx, |model, cx| model.rediff(ComparisonPolicy::Default, cx));
    task.await.expect("cached rediff succeeds");

    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n");
}

#[gpui::test]
async fn restarting_resets_the_result_to_the_base_text_and_forgets_the_history(
    cx: &mut TestAppContext,
) {
    let fixture = many_changes();
    let model = new_uninitialized_model(cx, &fixture, FileConflictType::Default);
    let task = model.update(cx, |model, cx| model.rediff(ComparisonPolicy::Default, cx));
    task.await.expect("rediff succeeds");
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    edit_result(cx, &model, 0..0, "x");

    let task = model.update(cx, |model, cx| {
        model.restart_with_policy(ComparisonPolicy::IgnoreWhitespaces, cx)
    });
    task.await.expect("restart succeeds");

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
    model.read_with(cx, |model, _| {
        assert!(!model.content_modified());
        assert_eq!(model.ignore_policy(), ComparisonPolicy::IgnoreWhitespaces);
    });
    assert!(!undo_result(cx, &model));
}

#[gpui::test]
async fn a_newer_rediff_cancels_the_pending_one(cx: &mut TestAppContext) {
    let fixture = many_changes();
    let model = new_uninitialized_model(cx, &fixture, FileConflictType::Default);

    let superseded = model.update(cx, |model, cx| {
        model.rediff(ComparisonPolicy::IgnoreWhitespaces, cx)
    });
    let current = model.update(cx, |model, cx| {
        model.rediff(ComparisonPolicy::TrimWhitespaces, cx)
    });

    assert_eq!(superseded.await, Err(MergeModelError::Canceled));
    current.await.expect("the newer rediff succeeds");
    model.read_with(cx, |model, _| {
        assert_eq!(model.ignore_policy(), ComparisonPolicy::TrimWhitespaces);
        assert_eq!(model.changes().len(), fixture.data.fragments.len());
    });
}

#[gpui::test]
async fn ignoring_one_side_then_accepting_the_other_replaces_the_result_and_resolves_the_change(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &two_line_conflict());

    model.update(cx, |model, cx| {
        model.run_ignore_change(0, Side::Left, false, cx)
    });
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });

    assert_eq!(result_text(cx, &model), "a\nR1\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 2)]);
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    assert!(side_resolved(cx, &model, 0, Side::Left));
    assert!(side_resolved(cx, &model, 0, Side::Right));
    assert!(oneside_applied(cx, &model, 0));
}

#[gpui::test]
async fn replacing_several_changes_from_one_side_is_a_single_undo_step(cx: &mut TestAppContext) {
    let model = new_model(cx, &many_changes());

    model.update(cx, |model, cx| {
        model.run_replace_changes("Accept", &[0, 2, 3], Side::Left, true, cx)
    });

    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\ni\nj\nk\n");
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 7), (9, 10)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![true, false, true, true]);

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
    assert!(!undo_result(cx, &model));
}

#[gpui::test]
async fn a_command_reports_each_touched_change_once_and_finishes_with_one_bulk_event(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    let (events, _subscription) = record_events(cx, &model);

    model.update(cx, |model, cx| {
        model.run_replace_changes("Accept", &[0, 2, 3], Side::Left, true, cx)
    });

    let mut processed: Vec<usize> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            MergeModelEvent::ChangeProcessed(index) => Some(*index),
            _ => None,
        })
        .collect();
    processed.sort_unstable();
    assert_eq!(processed, vec![0, 2, 3]);
    let bulk_finished = events
        .borrow()
        .iter()
        .filter(|event| **event == MergeModelEvent::BulkProcessingFinished)
        .count();
    assert_eq!(bulk_finished, 1);
}

#[gpui::test]
async fn ignoring_changes_on_one_side_resolves_non_conflicts_and_only_that_side_of_conflicts(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());

    model.update(cx, |model, cx| {
        model.run_ignore_changes_on_side("Ignore", &[0, 1], Side::Left, cx)
    });

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
    assert!(side_resolved(cx, &model, 1, Side::Left));
    assert!(!side_resolved(cx, &model, 1, Side::Right));
}

#[gpui::test]
async fn resolving_one_change_automatically_merges_both_sides_into_the_result(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &text_resolvable_conflict());

    let applied = model.update(cx, |model, cx| {
        model.run_resolve_change_automatically(0, cx)
    });

    assert!(applied);
    assert_eq!(result_text(cx, &model), "a\nFOO bar BAZ\nc\n");
    assert_eq!(resolved_flags(cx, &model), vec![true]);
    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nfoo bar baz\nc\n");
    assert_eq!(resolved_flags(cx, &model), vec![false]);
}

#[gpui::test]
async fn resetting_a_resolved_change_restores_its_base_lines_and_clears_its_state(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    let (events, _subscription) = record_events(cx, &model);

    model.update(cx, |model, cx| model.run_reset_resolved_change(0, cx));

    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(resolved_flags(cx, &model), vec![false; 4]);
    assert!(events.borrow().contains(&MergeModelEvent::ChangeReset(0)));

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n");
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, false]);
}

#[gpui::test]
async fn resetting_listed_changes_skips_unresolved_ones_and_leaves_other_resolved_changes_alone(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &many_changes());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    model.update(cx, |model, cx| {
        model.run_accept_change(3, Side::Right, false, cx)
    });
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, true]);
    assert_eq!(
        result_text(cx, &model),
        "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk2\n"
    );

    let reset = model.update(cx, |model, cx| {
        model.run_reset_resolved_changes("Revert in merge", &[0, 2], cx)
    });

    assert!(reset);
    assert_eq!(resolved_flags(cx, &model), vec![false, false, false, true]);
    assert_eq!(
        result_text(cx, &model),
        "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk2\n"
    );

    assert!(undo_result(cx, &model));
    assert_eq!(resolved_flags(cx, &model), vec![true, false, false, true]);
    assert_eq!(
        result_text(cx, &model),
        "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk2\n"
    );
}

#[gpui::test]
async fn reverting_a_two_sided_resolution_makes_the_next_accept_replace_instead_of_append(
    cx: &mut TestAppContext,
) {
    let model = new_model(cx, &two_line_conflict());
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });
    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });
    assert_eq!(result_text(cx, &model), "a\nL1\nL2\nR1\nd\n");

    model.update(cx, |model, cx| model.run_reset_resolved_change(0, cx));

    assert_eq!(result_text(cx, &model), "a\nb\nc\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 3)]);
    assert_eq!(resolved_flags(cx, &model), vec![false]);
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(!side_resolved(cx, &model, 0, Side::Right));
    assert!(!oneside_applied(cx, &model, 0));

    model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Right, false, cx)
    });

    assert_eq!(result_text(cx, &model), "a\nR1\nd\n");
    assert_eq!(result_ranges(cx, &model), vec![(1, 2)]);
    assert!(side_resolved(cx, &model, 0, Side::Right));
    assert!(!side_resolved(cx, &model, 0, Side::Left));
    assert!(oneside_applied(cx, &model, 0));
}

fn repo_path(path: &str) -> RepoPath {
    RepoPath::new(path).expect("valid repo path")
}

#[gpui::test]
async fn the_data_holder_tracks_resolution_and_review_per_file(cx: &mut TestAppContext) {
    let holder = cx.new(|_| MergeConflictIterativeDataHolder::new());
    let resolved_file = repo_path("resolved.txt");
    let pending_file = repo_path("pending.txt");
    let resolved_model = new_model(cx, &many_changes());
    let pending_model = new_model(cx, &two_line_conflict());
    holder.update(cx, |holder, _| {
        holder.insert_model(resolved_file.clone(), resolved_model.clone());
        holder.insert_model(pending_file.clone(), pending_model.clone());
    });

    holder.read_with(cx, |holder, cx| {
        assert!(!holder.is_file_resolved(&resolved_file, cx));
        assert!(!holder.is_file_resolved(&repo_path("unknown.txt"), cx));
        assert!(holder.resolved_files_and_models(cx).is_empty());
    });

    resolved_model.update(cx, |model, cx| {
        model.run_accept_side(Side::Left, cx);
        model.mark_reviewed();
    });
    pending_model.update(cx, |model, cx| {
        model.run_accept_change(0, Side::Left, false, cx)
    });

    holder.read_with(cx, |holder, cx| {
        assert!(holder.is_file_resolved(&resolved_file, cx));
        assert!(holder.is_file_reviewed(&resolved_file, cx));
        assert!(!holder.is_file_resolved(&pending_file, cx));
        assert!(!holder.is_file_reviewed(&pending_file, cx));
        let resolved_files: Vec<RepoPath> = holder
            .resolved_files_and_models(cx)
            .into_iter()
            .map(|(file, _)| file)
            .collect();
        assert_eq!(resolved_files, vec![resolved_file.clone()]);
        assert_eq!(
            holder.file_progress(&resolved_file, cx),
            Some(FileProgress {
                resolved: 4,
                total: 4
            })
        );
        assert_eq!(
            holder.file_progress(&pending_file, cx),
            Some(FileProgress {
                resolved: 0,
                total: 1
            })
        );
        assert_eq!(holder.file_progress(&repo_path("unknown.txt"), cx), None);
    });
}

#[gpui::test]
async fn the_data_holder_prepares_models_once_and_resolves_them_automatically(
    cx: &mut TestAppContext,
) {
    let holder = cx.new(|_| MergeConflictIterativeDataHolder::new());
    let file = repo_path("file.txt");
    let fixture = text_resolvable_conflict();
    let request = MergeModelRequest {
        texts: fixture.texts.clone(),
        conflict_type: FileConflictType::Default,
        ignore_policy: ComparisonPolicy::Default,
    };

    let task = holder.update(cx, |holder, cx| {
        holder.prepare_model_if_supported(
            file.clone(),
            request.clone(),
            |texts, cx| MergeBuffers::detached(texts, None, cx),
            cx,
        )
    });
    let model = task.await.expect("model is prepared");
    let again = holder
        .update(cx, |holder, cx| {
            holder.prepare_model_if_supported(
                file.clone(),
                request.clone(),
                |texts, cx| MergeBuffers::detached(texts, None, cx),
                cx,
            )
        })
        .await
        .expect("cached model is returned");
    assert_eq!(model.entity_id(), again.entity_id());

    let resolved = holder.update(cx, |holder, cx| {
        holder.resolve_auto_resolvable_conflicts(&file, cx)
    });
    assert!(resolved);
    assert_eq!(result_text(cx, &model), "a\nFOO bar BAZ\nc\n");
    holder.read_with(cx, |holder, cx| {
        assert!(holder.is_file_resolved(&file, cx));
    });

    holder.update(cx, |holder, cx| {
        holder.remove_files(std::slice::from_ref(&file), cx)
    });
    holder.read_with(cx, |holder, cx| {
        assert!(!holder.is_file_resolved(&file, cx));
        assert!(holder.merge_conflict_model(&file).is_none());
    });
    assert_eq!(result_text(cx, &model), "a\nfoo bar baz\nc\n");
}
