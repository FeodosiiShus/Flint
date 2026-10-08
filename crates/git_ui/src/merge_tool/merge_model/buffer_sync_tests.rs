use std::{ops::Range, sync::Arc, time::Duration};

use gpui::{AppContext as _, Entity, TestAppContext};
use merge_diff::{
    ComparisonPolicy, ConflictKind, MergeConflictType, MergeRange, ResolutionStrategy,
};

use super::{FileConflictType, MergeBuffers, MergeConflictModel, MergeDiffData, MergeTexts};

const BASE_TEXT: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";

fn modified_change() -> MergeConflictType {
    MergeConflictType {
        kind: ConflictKind::Modified,
        left_change: true,
        right_change: false,
        resolution: Some(ResolutionStrategy::Default),
    }
}

fn model_with_four_changes(cx: &mut TestAppContext) -> Entity<MergeConflictModel> {
    let texts = MergeTexts::new(BASE_TEXT, BASE_TEXT, BASE_TEXT);
    let data = MergeDiffData {
        fragments: [(1, 2), (4, 5), (7, 8), (10, 11)]
            .into_iter()
            .map(|(start, end)| MergeRange::new(start..end, start..end, start..end))
            .collect(),
        conflict_types: vec![modified_change(); 4],
        ignore_policy: ComparisonPolicy::Default,
    };
    let model = cx.new(|cx| {
        let buffers = MergeBuffers::detached(&texts, None, cx);
        MergeConflictModel::new(buffers, &texts, FileConflictType::Default, cx)
    });
    model.update(cx, |model, cx| {
        model
            .apply_diff_data(Arc::new(data), cx)
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

fn edit_result(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
    edits: Vec<(Range<usize>, String)>,
) {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, cx| {
        buffer.edit(edits, None, cx);
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

fn set_group_interval(
    cx: &mut TestAppContext,
    model: &Entity<MergeConflictModel>,
    interval: Duration,
) {
    let buffer = model.read_with(cx, |model, _| model.buffers().result.clone());
    buffer.update(cx, |buffer, _| buffer.set_group_interval(interval));
}

#[gpui::test]
async fn undoing_a_multi_edit_transaction_restores_every_change_range(cx: &mut TestAppContext) {
    let model = model_with_four_changes(cx);
    set_group_interval(cx, &model, Duration::ZERO);

    edit_result(
        cx,
        &model,
        vec![(4..4, "x\n".to_string()), (20..21, "K".to_string())],
    );

    assert_eq!(
        result_text(cx, &model),
        "a\nb\nx\nc\nd\ne\nf\ng\nh\ni\nj\nK\n"
    );
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (5, 6), (8, 9), (11, 12)]
    );

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );

    assert!(redo_result(cx, &model));
    assert_eq!(
        result_text(cx, &model),
        "a\nb\nx\nc\nd\ne\nf\ng\nh\ni\nj\nK\n"
    );
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (5, 6), (8, 9), (11, 12)]
    );
}

#[gpui::test]
async fn separate_user_edits_are_undone_one_at_a_time(cx: &mut TestAppContext) {
    let model = model_with_four_changes(cx);
    set_group_interval(cx, &model, Duration::ZERO);

    edit_result(cx, &model, vec![(0..0, "x\n".to_string())]);
    edit_result(cx, &model, vec![(8..8, "y\n".to_string())]);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(2, 3), (6, 7), (9, 10), (12, 13)]
    );

    assert!(undo_result(cx, &model));
    assert_eq!(
        result_ranges(cx, &model),
        vec![(2, 3), (5, 6), (8, 9), (11, 12)]
    );

    assert!(undo_result(cx, &model));
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    assert!(!undo_result(cx, &model));
}

#[gpui::test]
async fn user_edits_grouped_by_the_buffer_are_undone_together_to_the_earliest_ranges(
    cx: &mut TestAppContext,
) {
    let model = model_with_four_changes(cx);
    set_group_interval(cx, &model, Duration::from_secs(60));

    edit_result(cx, &model, vec![(0..0, "x\n".to_string())]);
    edit_result(cx, &model, vec![(8..8, "y\n".to_string())]);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(2, 3), (6, 7), (9, 10), (12, 13)]
    );

    assert!(undo_result(cx, &model));
    assert_eq!(result_text(cx, &model), BASE_TEXT);
    assert_eq!(
        result_ranges(cx, &model),
        vec![(1, 2), (4, 5), (7, 8), (10, 11)]
    );
    assert!(!undo_result(cx, &model));
}
