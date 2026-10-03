use super::*;
use rand::{Rng, SeedableRng, rngs::StdRng};

fn merge(base: &str, left: &str, right: &str) -> ThreeWayMerge {
    ThreeWayMerge::new(base, left, right, IgnorePolicy::None)
}

fn document(base: &str, left: &str, right: &str) -> MergeDocument {
    MergeDocument::new(merge(base, left, right))
}

fn kinds(merge: &ThreeWayMerge) -> Vec<ChunkKind> {
    merge.chunks().iter().map(|chunk| chunk.kind).collect()
}

const FIXTURE_BASE: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\n";
const FIXTURE_LEFT: &str = "A\nb\nc\nd\nE\nf\ng\nh\nI\n";
const FIXTURE_RIGHT: &str = "a\nb\nC\nd\nX\nf\ng\nh\nI\n";

fn fixture() -> MergeDocument {
    document(FIXTURE_BASE, FIXTURE_LEFT, FIXTURE_RIGHT)
}

#[test]
fn identical_texts_produce_no_chunks() {
    let merge = merge("a\nb\n", "a\nb\n", "a\nb\n");
    assert!(merge.chunks().is_empty());
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nb\n"));
}

#[test]
fn left_only_modification_is_a_left_change() {
    let merge = merge("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\n");
    assert_eq!(
        merge.chunks(),
        &[Chunk {
            base: 1..2,
            left: 1..2,
            right: 1..2,
            kind: ChunkKind::LeftChange,
        }]
    );
    let chunk = &merge.chunks()[0];
    assert_eq!(chunk.change_type(Side::Left), Some(ChangeType::Modified));
    assert_eq!(chunk.change_type(Side::Right), None);
    assert_eq!(merge.chunk_text(0, Revision::Left), "B\n");
    assert_eq!(merge.chunk_text(0, Revision::Base), "b\n");
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nB\nc\n"));
}

#[test]
fn right_only_insertion_is_an_added_right_change() {
    let merge = merge("a\nb\nc\n", "a\nb\nc\n", "a\nb\nx\nc\n");
    assert_eq!(
        merge.chunks(),
        &[Chunk {
            base: 2..2,
            left: 2..2,
            right: 2..3,
            kind: ChunkKind::RightChange,
        }]
    );
    assert_eq!(
        merge.chunks()[0].change_type(Side::Right),
        Some(ChangeType::Added)
    );
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nb\nx\nc\n"));
}

#[test]
fn deletion_is_reported_as_deleted() {
    let merge = merge("a\nb\nc\n", "a\nc\n", "a\nb\nc\n");
    let chunk = &merge.chunks()[0];
    assert_eq!(chunk.base, 1..2);
    assert_eq!(chunk.left, 1..1);
    assert_eq!(chunk.change_type(Side::Left), Some(ChangeType::Deleted));
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nc\n"));
}

#[test]
fn identical_changes_on_both_sides_do_not_conflict() {
    let merge = merge("a\nb\nc\n", "a\nB\nc\n", "a\nB\nc\n");
    assert_eq!(kinds(&merge), vec![ChunkKind::BothChangedEqually]);
    assert!(!merge.has_conflicts());
    assert_eq!(
        merge.chunks()[0].change_type(Side::Right),
        Some(ChangeType::Modified)
    );
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nB\nc\n"));
    assert_eq!(
        diffy::merge("a\nb\nc\n", "a\nB\nc\n", "a\nB\nc\n").as_deref(),
        Ok("a\nB\nc\n")
    );
}

#[test]
fn different_changes_to_the_same_line_conflict() {
    let (base, left, right) = ("a\nb\nc\n", "a\nL\nc\n", "a\nR\nc\n");
    let merge = merge(base, left, right);
    assert_eq!(kinds(&merge), vec![ChunkKind::Conflict]);
    let chunk = &merge.chunks()[0];
    assert_eq!(chunk.change_type(Side::Left), Some(ChangeType::Modified));
    assert_eq!(chunk.change_type(Side::Right), Some(ChangeType::Modified));
    assert_eq!(merge.clean_merge(), None);
    assert!(diffy::merge(base, left, right).is_err());
}

#[test]
fn changes_to_adjacent_lines_conflict_like_git() {
    let (base, left, right) = ("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n");
    let merge = merge(base, left, right);
    assert_eq!(
        merge.chunks(),
        &[Chunk {
            base: 1..3,
            left: 1..3,
            right: 1..3,
            kind: ChunkKind::Conflict,
        }]
    );
    assert_eq!(merge.chunk_text(0, Revision::Left), "B\nc\n");
    assert_eq!(merge.chunk_text(0, Revision::Right), "b\nC\n");
    assert!(diffy::merge(base, left, right).is_err());
}

#[test]
fn insertion_touching_a_modification_conflicts() {
    let (base, left, right) = ("a\nb\nc\n", "a\nB\nc\n", "a\nb\nnew\nc\n");
    let merge = merge(base, left, right);
    assert_eq!(kinds(&merge), vec![ChunkKind::Conflict]);
    assert!(diffy::merge(base, left, right).is_err());
}

#[test]
fn changes_separated_by_one_unchanged_line_merge_cleanly() {
    let (base, left, right) = ("a\nb\nc\n", "A\nb\nc\n", "a\nb\nC\n");
    let merge = merge(base, left, right);
    assert_eq!(
        kinds(&merge),
        vec![ChunkKind::LeftChange, ChunkKind::RightChange]
    );
    assert_eq!(merge.clean_merge().as_deref(), Some("A\nb\nC\n"));
    assert_eq!(diffy::merge(base, left, right).as_deref(), Ok("A\nb\nC\n"));
}

#[test]
fn add_add_with_different_content_conflicts() {
    let (base, left, right) = ("a\nz\n", "a\nleft\nz\n", "a\nright\nz\n");
    let merge = merge(base, left, right);
    assert_eq!(kinds(&merge), vec![ChunkKind::Conflict]);
    let chunk = &merge.chunks()[0];
    assert_eq!(chunk.base, 1..1);
    assert_eq!(chunk.change_type(Side::Left), Some(ChangeType::Added));
    assert_eq!(chunk.change_type(Side::Right), Some(ChangeType::Added));
    assert!(diffy::merge(base, left, right).is_err());
}

#[test]
fn add_add_with_identical_content_is_one_change() {
    let merge = merge("a\nz\n", "a\nsame\nz\n", "a\nsame\nz\n");
    assert_eq!(kinds(&merge), vec![ChunkKind::BothChangedEqually]);
    assert_eq!(merge.clean_merge().as_deref(), Some("a\nsame\nz\n"));
}

#[test]
fn delete_modify_conflicts() {
    let (base, left, right) = ("a\nb\nc\n", "a\nc\n", "a\nB\nc\n");
    let merge = merge(base, left, right);
    assert_eq!(kinds(&merge), vec![ChunkKind::Conflict]);
    let chunk = &merge.chunks()[0];
    assert_eq!(chunk.change_type(Side::Left), Some(ChangeType::Deleted));
    assert_eq!(chunk.change_type(Side::Right), Some(ChangeType::Modified));
    assert!(diffy::merge(base, left, right).is_err());
}

#[test]
fn empty_files() {
    let all_empty = merge("", "", "");
    assert!(all_empty.chunks().is_empty());
    assert_eq!(all_empty.clean_merge().as_deref(), Some(""));
    assert_eq!(all_empty.line_count(Revision::Base), 0);

    let added_on_left = merge("", "new\n", "");
    assert_eq!(kinds(&added_on_left), vec![ChunkKind::LeftChange]);
    assert_eq!(
        added_on_left.chunks()[0].change_type(Side::Left),
        Some(ChangeType::Added)
    );
    assert_eq!(added_on_left.clean_merge().as_deref(), Some("new\n"));

    let added_on_both = merge("", "left\n", "right\n");
    assert_eq!(kinds(&added_on_both), vec![ChunkKind::Conflict]);
    assert!(diffy::merge("", "left\n", "right\n").is_err());

    let deleted_on_both = merge("a\nb\n", "", "");
    assert_eq!(kinds(&deleted_on_both), vec![ChunkKind::BothChangedEqually]);
    assert_eq!(
        deleted_on_both.chunks()[0].change_type(Side::Left),
        Some(ChangeType::Deleted)
    );
    assert_eq!(deleted_on_both.clean_merge().as_deref(), Some(""));

    let deleted_and_modified = merge("a\n", "", "b\n");
    assert_eq!(kinds(&deleted_and_modified), vec![ChunkKind::Conflict]);
}

#[test]
fn missing_trailing_newline_is_preserved_and_compared() {
    let clean = merge("a\nb", "A\nb", "a\nb");
    assert_eq!(clean.line_count(Revision::Base), 2);
    assert_eq!(clean.clean_merge().as_deref(), Some("A\nb"));
    assert_eq!(diffy::merge("a\nb", "A\nb", "a\nb").as_deref(), Ok("A\nb"));

    let (base, left, right) = ("a\nb", "a\nB", "a\nb\nc");
    let conflicting = merge(base, left, right);
    assert_eq!(kinds(&conflicting), vec![ChunkKind::Conflict]);
    assert!(diffy::merge(base, left, right).is_err());

    let newline_added = merge("a\nb", "a\nb\n", "a\nb");
    assert_eq!(kinds(&newline_added), vec![ChunkKind::LeftChange]);
    assert_eq!(newline_added.clean_merge().as_deref(), Some("a\nb\n"));
}

#[test]
fn crlf_line_endings_are_kept() {
    let base = "one\r\ntwo\r\nthree\r\nfour\r\n";
    let left = "ONE\r\ntwo\r\nthree\r\nfour\r\n";
    let right = "one\r\ntwo\r\nthree\r\nFOUR\r\n";
    let merge = merge(base, left, right);
    assert_eq!(merge.line_count(Revision::Base), 4);
    let expected = "ONE\r\ntwo\r\nthree\r\nFOUR\r\n";
    assert_eq!(merge.clean_merge().as_deref(), Some(expected));
    assert_eq!(diffy::merge(base, left, right).as_deref(), Ok(expected));
}

#[test]
fn line_ending_changes_respect_the_ignore_policy() {
    let base = "one\r\ntwo\r\n";
    let left = "one\ntwo\n";
    let strict = ThreeWayMerge::new(base, left, base, IgnorePolicy::None);
    assert_eq!(kinds(&strict), vec![ChunkKind::LeftChange]);
    for policy in [
        IgnorePolicy::TrimWhitespaces,
        IgnorePolicy::IgnoreWhitespaces,
        IgnorePolicy::IgnoreWhitespacesAndEmptyLines,
    ] {
        assert!(
            ThreeWayMerge::new(base, left, base, policy)
                .chunks()
                .is_empty(),
            "{policy:?} must ignore line ending differences"
        );
    }
}

#[test]
fn ignore_policies_filter_whitespace_changes() {
    let base = "fn main() {\n    call(a, b);\n}\n";
    let reindented = "fn main() {\n\tcall(a, b);\n}\n";
    let inner_spacing = "fn main() {\n    call(a,  b);\n}\n";

    let none = ThreeWayMerge::new(base, reindented, base, IgnorePolicy::None);
    assert_eq!(kinds(&none), vec![ChunkKind::LeftChange]);
    let trim = ThreeWayMerge::new(base, reindented, base, IgnorePolicy::TrimWhitespaces);
    assert!(trim.chunks().is_empty());

    let trim_inner = ThreeWayMerge::new(base, inner_spacing, base, IgnorePolicy::TrimWhitespaces);
    assert_eq!(kinds(&trim_inner), vec![ChunkKind::LeftChange]);
    let ignore_inner =
        ThreeWayMerge::new(base, inner_spacing, base, IgnorePolicy::IgnoreWhitespaces);
    assert!(ignore_inner.chunks().is_empty());

    let both_whitespace = ThreeWayMerge::new(
        base,
        reindented,
        inner_spacing,
        IgnorePolicy::IgnoreWhitespaces,
    );
    assert!(both_whitespace.chunks().is_empty());
    let both_strict = ThreeWayMerge::new(base, reindented, inner_spacing, IgnorePolicy::None);
    assert_eq!(kinds(&both_strict), vec![ChunkKind::Conflict]);
}

#[test]
fn empty_lines_are_only_ignored_by_the_strongest_policy() {
    let base = "a\nb\nc\nd\n";
    let left = "\na\nb\nc\nd\n";
    let right = "a\nb\nc\nD\n";

    let whitespace = ThreeWayMerge::new(base, left, right, IgnorePolicy::IgnoreWhitespaces);
    assert_eq!(
        kinds(&whitespace),
        vec![ChunkKind::LeftChange, ChunkKind::RightChange]
    );

    let empty_lines = ThreeWayMerge::new(
        base,
        left,
        right,
        IgnorePolicy::IgnoreWhitespacesAndEmptyLines,
    );
    assert_eq!(
        empty_lines.chunks(),
        &[Chunk {
            base: 3..4,
            left: 4..5,
            right: 3..4,
            kind: ChunkKind::RightChange,
        }]
    );
    assert_eq!(empty_lines.chunk_text(0, Revision::Left), "d\n");
}

#[test]
fn fixture_counters_match_the_webstorm_legend() {
    let document = fixture();
    assert_eq!(
        kinds(document.model().merge()),
        vec![
            ChunkKind::LeftChange,
            ChunkKind::RightChange,
            ChunkKind::Conflict,
            ChunkKind::BothChangedEqually,
        ]
    );
    let counters = document.model().counters();
    assert_eq!(
        counters,
        Counters {
            changes: 3,
            conflicts: 1,
            total: 4,
        }
    );
    assert_eq!(counters.status_text(), "3 changes. 1 conflict.");
    assert_eq!(counters.resolved(), 0);
    assert_eq!(document.result().text(), FIXTURE_BASE);
}

#[test]
fn status_text_pluralizes_and_reports_completion() {
    let one = Counters {
        changes: 1,
        conflicts: 0,
        total: 3,
    };
    assert_eq!(one.status_text(), "1 change. 0 conflicts.");
    assert_eq!(one.resolved(), 2);
    let two_conflicts = Counters {
        changes: 0,
        conflicts: 2,
        total: 2,
    };
    assert_eq!(two_conflicts.status_text(), "0 changes. 2 conflicts.");
    let done = Counters {
        changes: 0,
        conflicts: 0,
        total: 4,
    };
    assert!(done.is_complete());
    assert_eq!(done.status_text(), "All changes have been processed.");
}

#[test]
fn apply_non_conflicting_all() {
    let mut document = fixture();
    assert_eq!(document.apply_non_conflicting(NonConflictingScope::All), 3);
    assert_eq!(document.result().text(), "A\nb\nC\nd\ne\nf\ng\nh\nI\n");
    let counters = document.model().counters();
    assert_eq!(counters.changes, 0);
    assert_eq!(counters.conflicts, 1);
    assert_eq!(counters.status_text(), "0 changes. 1 conflict.");
    assert_eq!(document.apply_non_conflicting(NonConflictingScope::All), 0);
}

#[test]
fn apply_non_conflicting_left_only_touches_left_changes() {
    let mut document = fixture();
    assert_eq!(document.apply_non_conflicting(NonConflictingScope::Left), 2);
    assert_eq!(document.result().text(), "A\nb\nc\nd\ne\nf\ng\nh\nI\n");
    assert!(document.model().is_pending(1, Side::Right));
    assert_eq!(document.model().counters().changes, 1);
    assert_eq!(
        document.model().chunk_state(3),
        Some(ChunkState {
            left: SideState::Applied,
            right: SideState::Applied,
        })
    );
}

#[test]
fn apply_non_conflicting_right_only_touches_right_changes() {
    let mut document = fixture();
    assert_eq!(
        document.apply_non_conflicting(NonConflictingScope::Right),
        2
    );
    assert_eq!(document.result().text(), "a\nb\nC\nd\ne\nf\ng\nh\nI\n");
    assert!(document.model().is_pending(0, Side::Left));
}

#[test]
fn accepting_both_sides_of_a_conflict_appends_the_second() {
    let mut document = fixture();
    assert!(document.apply(2, Side::Left, ApplyMode::Replace));
    assert_eq!(document.result().text(), "a\nb\nc\nd\nE\nf\ng\nh\ni\n");
    assert!(!document.model().is_resolved(2));
    assert_eq!(document.model().counters().conflicts, 1);

    assert!(document.apply(2, Side::Right, ApplyMode::Replace));
    assert_eq!(document.result().text(), "a\nb\nc\nd\nE\nX\nf\ng\nh\ni\n");
    assert!(document.model().is_resolved(2));
    assert_eq!(document.model().counters().conflicts, 0);
    assert!(!document.apply(2, Side::Right, ApplyMode::Replace));
}

#[test]
fn append_mode_keeps_the_current_text() {
    let mut document = fixture();
    assert!(document.apply(2, Side::Right, ApplyMode::Append));
    assert_eq!(document.result().text(), "a\nb\nc\nd\ne\nX\nf\ng\nh\ni\n");
    assert!(document.model().is_pending(2, Side::Left));
}

#[test]
fn ignoring_one_side_then_accepting_the_other() {
    let mut document = fixture();
    assert!(document.ignore(2, Side::Left));
    assert_eq!(document.result().text(), FIXTURE_BASE);
    assert!(!document.model().is_resolved(2));
    assert!(document.apply(2, Side::Right, ApplyMode::Replace));
    assert_eq!(document.result().text(), "a\nb\nc\nd\nX\nf\ng\nh\ni\n");
    assert!(document.model().is_resolved(2));
}

#[test]
fn ignoring_both_sides_keeps_the_base() {
    let mut document = fixture();
    assert!(document.ignore(2, Side::Left));
    assert!(document.ignore(2, Side::Right));
    assert!(!document.ignore(2, Side::Right));
    assert_eq!(document.result().text(), FIXTURE_BASE);
    assert!(document.model().is_resolved(2));
}

#[test]
fn ignoring_an_identical_change_resolves_it() {
    let mut document = fixture();
    assert!(document.ignore(3, Side::Right));
    assert!(document.model().is_resolved(3));
    assert_eq!(document.result().text(), FIXTURE_BASE);
    assert_eq!(
        document.model().chunk_state(3),
        Some(ChunkState {
            left: SideState::Ignored,
            right: SideState::Ignored,
        })
    );
}

#[test]
fn resolve_using_a_side_ignores_the_other() {
    let mut document = fixture();
    assert!(document.resolve_using(2, Side::Left));
    assert_eq!(document.result().text(), "a\nb\nc\nd\nE\nf\ng\nh\ni\n");
    assert_eq!(
        document.model().chunk_state(2),
        Some(ChunkState {
            left: SideState::Applied,
            right: SideState::Ignored,
        })
    );
    assert!(!document.resolve_using(2, Side::Right));

    let mut document = fixture();
    assert!(document.apply(2, Side::Left, ApplyMode::Replace));
    assert!(document.resolve_using(2, Side::Right));
    assert_eq!(document.result().text(), "a\nb\nc\nd\nX\nf\ng\nh\ni\n");
}

#[test]
fn unchanged_side_has_no_pending_action() {
    let mut document = fixture();
    assert!(!document.model().is_pending(0, Side::Right));
    assert!(!document.apply(0, Side::Right, ApplyMode::Replace));
    assert!(!document.ignore(0, Side::Right));
    assert!(document.apply(0, Side::Left, ApplyMode::Replace));
    assert!(document.model().is_resolved(0));
    assert!(!document.apply(99, Side::Left, ApplyMode::Replace));
}

#[test]
fn appending_after_text_without_trailing_newline_inserts_one() {
    let mut document = document("a\nb", "a\nleft", "a\nright");
    assert!(document.apply(0, Side::Left, ApplyMode::Replace));
    assert!(document.apply(0, Side::Right, ApplyMode::Replace));
    assert_eq!(document.result().text(), "a\nleft\nright");

    let mut crlf = document_crlf();
    assert!(crlf.apply(0, Side::Left, ApplyMode::Replace));
    assert!(crlf.apply(0, Side::Right, ApplyMode::Replace));
    assert_eq!(crlf.result().text(), "a\r\nleft\r\nright");
}

fn document_crlf() -> MergeDocument {
    document("a\r\nb", "a\r\nleft", "a\r\nright")
}

#[test]
fn accepting_whole_sides_reproduces_them() {
    let mut left_document = fixture();
    for chunk_index in 0..left_document.model().chunks().len() {
        left_document.resolve_using(chunk_index, Side::Left);
    }
    assert_eq!(left_document.result().text(), FIXTURE_LEFT);
    assert!(left_document.model().counters().is_complete());

    let mut right_document = fixture();
    for chunk_index in 0..right_document.model().chunks().len() {
        right_document.resolve_using(chunk_index, Side::Right);
    }
    assert_eq!(right_document.result().text(), FIXTURE_RIGHT);
}

#[test]
fn navigation_between_unresolved_chunks() {
    let mut document = fixture();
    let lines = document.result().chunk_line_ranges();
    assert_eq!(lines, vec![0..1, 2..3, 4..5, 8..9]);
    let model = document.model();
    assert_eq!(model.next_unresolved(&lines, 0), Some(1));
    assert_eq!(model.next_unresolved(&lines, 3), Some(2));
    assert_eq!(model.next_unresolved(&lines, 8), None);
    assert_eq!(model.previous_unresolved(&lines, 8), Some(2));
    assert_eq!(model.previous_unresolved(&lines, 0), None);
    assert_eq!(model.previous_unresolved(&lines, 1), Some(0));

    document.apply_non_conflicting(NonConflictingScope::All);
    let lines = document.result().chunk_line_ranges();
    assert_eq!(document.model().next_unresolved(&lines, 0), Some(2));
    assert_eq!(document.model().previous_unresolved(&lines, 8), Some(2));
    assert_eq!(document.model().next_unresolved(&lines, 4), None);
}

#[test]
fn restore_rolls_back_chunk_states() {
    let mut document = fixture();
    let before = document.model().chunk_states();
    document.apply_non_conflicting(NonConflictingScope::All);
    document.resolve_using(2, Side::Right);
    assert!(document.model().counters().is_complete());
    let after = document.model().chunk_states();

    let (mut model, _) = document.into_parts();
    model.restore(before.clone());
    assert_eq!(model.counters().remaining(), 4);
    assert_eq!(model.chunk_states(), before);
    model.restore(vec![ChunkState::default()]);
    assert_eq!(model.chunk_states(), before);
    model.restore(after);
    assert!(model.counters().is_complete());
}

#[test]
fn simple_conflict_with_edits_at_both_ends_of_a_line() {
    let base = "let value = compute(a, b);\n";
    let left = "let total = compute(a, b);\n";
    let right = "let value = compute(a, c);\n";
    let mut document = document(base, left, right);
    assert_eq!(kinds(document.model().merge()), vec![ChunkKind::Conflict]);
    assert!(document.model().is_simple_conflict(0));
    assert_eq!(
        document.model().simple_resolution(0).as_deref(),
        Some("let total = compute(a, c);\n")
    );
    assert!(document.resolve_simple(0));
    assert_eq!(document.result().text(), "let total = compute(a, c);\n");
    assert!(document.model().counters().is_complete());
}

#[test]
fn simple_conflict_with_an_extra_line() {
    let base = "a\nb\nc\n";
    let left = "a\nB\nc\n";
    let right = "a\nb\nextra\nc\n";
    let mut document = document(base, left, right);
    assert_eq!(kinds(document.model().merge()), vec![ChunkKind::Conflict]);
    assert_eq!(document.resolve_simple_conflicts(), 1);
    assert_eq!(document.result().text(), "a\nB\nextra\nc\n");
}

#[test]
fn overlapping_word_edits_are_not_simple() {
    let mut document = document("call(a, b);\n", "call(x, b);\n", "call(y, b);\n");
    assert!(!document.model().is_simple_conflict(0));
    assert_eq!(document.resolve_simple_conflicts(), 0);
    assert_eq!(document.result().text(), "call(a, b);\n");

    let same_point = document_with("a b\n", "a X b\n", "a Y b\n");
    assert!(!same_point.model().is_simple_conflict(0));

    let identical_word = document_with("a b c\n", "a B c\n", "a B c d\n");
    assert_eq!(
        identical_word.model().simple_resolution(0).as_deref(),
        Some("a B c d\n")
    );
}

fn document_with(base: &str, left: &str, right: &str) -> MergeDocument {
    document(base, left, right)
}

#[test]
fn simple_resolution_requires_untouched_conflicts() {
    let base = "let value = compute(a, b);\n";
    let left = "let total = compute(a, b);\n";
    let right = "let value = compute(a, c);\n";
    let mut document = document(base, left, right);
    assert!(document.ignore(0, Side::Right));
    assert!(!document.model().is_simple_conflict(0));
    assert!(!document.resolve_simple(0));

    let fixture_document = fixture();
    assert_eq!(fixture_document.model().simple_resolution(0), None);
    assert_eq!(fixture_document.model().simple_resolution(2), None);
}

#[test]
fn resolve_simple_conflicts_only_touches_simple_ones() {
    let base = "x = 1 + 2\nkeep\nname\n";
    let left = "y = 1 + 2\nkeep\nleft\n";
    let right = "x = 1 + 3\nkeep\nright\n";
    let mut document = document(base, left, right);
    assert_eq!(
        kinds(document.model().merge()),
        vec![ChunkKind::Conflict, ChunkKind::Conflict]
    );
    assert_eq!(document.resolve_simple_conflicts(), 1);
    assert_eq!(document.result().text(), "y = 1 + 3\nkeep\nname\n");
    assert_eq!(document.model().counters().conflicts, 1);
}

#[test]
fn tracker_shifts_chunks_after_user_edits() {
    let mut document = fixture();
    let original = document.result().chunk_ranges().to_vec();
    assert_eq!(original, vec![0..2, 4..6, 8..10, 16..18]);

    document.edit(2..2, "new\n");
    assert_eq!(
        document.result().chunk_ranges(),
        &[0..6, 8..10, 12..14, 20..22]
    );
    assert_eq!(document.result().chunk_text(0), "a\nnew\n");
    assert_eq!(document.result().chunk_text(1), "c\n");

    document.edit(9..9, "zz");
    assert_eq!(document.result().chunk_ranges()[1], 8..12);
    assert_eq!(document.result().chunk_text(1), "czz\n");

    assert!(document.apply(1, Side::Right, ApplyMode::Replace));
    assert_eq!(document.result().chunk_text(1), "C\n");
    assert_eq!(document.result().text(), "a\nnew\nb\nC\nd\ne\nf\ng\nh\ni\n");
}

#[test]
fn tracker_mirrors_anchor_biases() {
    let mut tracker = ResultTracker::from_parts("abcdef".to_string(), vec![2..4, 4..4]);
    tracker.edit(2..2, "X");
    assert_eq!(tracker.chunk_ranges(), &[2..5, 5..5]);
    tracker.edit(5..5, "Y");
    assert_eq!(tracker.chunk_ranges(), &[2..6, 5..6]);
    assert_eq!(tracker.text(), "abXcdYef");

    let mut deleting = ResultTracker::from_parts("0123456789".to_string(), vec![3..6]);
    deleting.edit(1..4, "");
    assert_eq!(deleting.chunk_ranges(), &[1..3]);
    deleting.edit(2..9, "Z");
    assert_eq!(deleting.chunk_ranges(), &[1..3]);
    assert_eq!(deleting.text(), "04Z");

    let mut replacing = ResultTracker::from_parts("abc".to_string(), vec![1..2]);
    replacing.apply(&ChunkEdit {
        chunk_index: 0,
        new_text: "long".to_string(),
    });
    assert_eq!(replacing.text(), "alongc");
    assert_eq!(replacing.chunk_ranges(), &[1..5]);
    replacing.edit(0..1, "\u{e9}");
    assert_eq!(replacing.chunk_ranges(), &[2..6]);
    replacing.edit(1..1, "x");
    assert_eq!(replacing.text(), "\u{e9}longc");
}

#[test]
fn applying_into_an_emptied_chunk_reinserts_text() {
    let mut document = document("a\nb\nc\n", "a\nc\n", "a\nb\nc\n");
    assert!(document.apply(0, Side::Left, ApplyMode::Replace));
    assert_eq!(document.result().text(), "a\nc\n");
    assert_eq!(document.result().chunk_ranges(), &[2..2]);
    assert_eq!(document.result().chunk_line_ranges(), vec![1..1]);
    document.edit(2..2, "typed\n");
    assert_eq!(document.result().chunk_text(0), "typed\n");
}

#[test]
fn line_ranges_for_bytes() {
    let text = "a\nbb\nccc";
    assert_eq!(line_range_for_bytes(text, 0..2), 0..1);
    assert_eq!(line_range_for_bytes(text, 2..5), 1..2);
    assert_eq!(line_range_for_bytes(text, 5..8), 2..3);
    assert_eq!(line_range_for_bytes(text, 2..2), 1..1);
    assert_eq!(line_range_for_bytes(text, 0..8), 0..3);
    assert_eq!(line_range_for_bytes(text, 7..100), 2..3);
    assert_eq!(line_range_for_bytes("", 0..0), 0..0);
}

#[test]
fn map_line_interpolates_inside_chunks_and_offsets_between() {
    let source = [2..4, 10..10];
    let target = [2..8, 14..16];
    assert_eq!(map_line(&source, &target, 0.0), 0.0);
    assert_eq!(map_line(&source, &target, 1.5), 1.5);
    assert_eq!(map_line(&source, &target, 2.0), 2.0);
    assert_eq!(map_line(&source, &target, 3.0), 5.0);
    assert_eq!(map_line(&source, &target, 4.0), 8.0);
    assert_eq!(map_line(&source, &target, 6.0), 10.0);
    assert_eq!(map_line(&source, &target, 10.0), 16.0);
    assert_eq!(map_line(&source, &target, 12.0), 18.0);
    assert_eq!(map_line(&[], &[], 7.0), 7.0);
}

#[test]
fn map_line_round_trips_between_panes_of_a_merge() {
    let merge = merge(FIXTURE_BASE, "x\ny\nA\nb\nc\nd\nE\nf\n", FIXTURE_RIGHT);
    let left: Vec<_> = merge
        .chunks()
        .iter()
        .map(|chunk| chunk.left.clone())
        .collect();
    let base: Vec<_> = merge
        .chunks()
        .iter()
        .map(|chunk| chunk.base.clone())
        .collect();
    for line in 0..merge.line_count(Revision::Left) {
        let mapped = map_line(&left, &base, f64::from(line));
        assert!(mapped >= 0.0);
        assert!(mapped <= f64::from(merge.line_count(Revision::Base)));
    }
    let first_unchanged_after_chunk = merge.chunks()[0].left.end;
    assert_eq!(
        map_line(&left, &base, f64::from(first_unchanged_after_chunk)),
        f64::from(merge.chunks()[0].base.end)
    );
}

#[test]
fn ignore_policy_labels_match_webstorm() {
    let labels: Vec<_> = IgnorePolicy::ALL
        .iter()
        .map(|policy| policy.label())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Do not ignore",
            "Trim whitespaces",
            "Ignore whitespaces",
            "Ignore whitespaces and empty lines",
        ]
    );
}

#[derive(Clone, Copy, Debug)]
enum RegionKind {
    LeftModify,
    RightModify,
    BothModify,
    LeftDelete,
    RightDelete,
    LeftInsert,
    RightInsert,
    BothInsert,
    Conflict,
}

struct Generated {
    base: String,
    left: String,
    right: String,
    expected_with_left_conflicts: String,
    conflicts: usize,
    changes: usize,
}

fn generate(rng: &mut StdRng, allow_conflicts: bool) -> Generated {
    let line_count = rng.random_range(0..24usize);
    let drop_trailing_newline = line_count > 0 && rng.random_bool(0.25);
    let base_lines: Vec<String> = (0..line_count)
        .map(|index| format!("base-{index}\n"))
        .collect();
    let mut fresh = 0usize;
    let mut fresh_lines = |rng: &mut StdRng, label: &str| -> Vec<String> {
        let count = rng.random_range(1..4usize);
        (0..count)
            .map(|_| {
                fresh += 1;
                format!("{label}-{fresh}\n")
            })
            .collect()
    };

    let mut base = Vec::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut expected = Vec::new();
    let mut conflicts = 0;
    let mut changes = 0;
    let last_changeable = if drop_trailing_newline {
        line_count.saturating_sub(1)
    } else {
        line_count
    };

    let mut index = 0;
    let mut previous_changed = false;
    while index <= line_count {
        let can_change = !previous_changed && rng.random_bool(0.35);
        if can_change {
            let mut options = vec![
                RegionKind::LeftInsert,
                RegionKind::RightInsert,
                RegionKind::BothInsert,
            ];
            if index < last_changeable {
                options.extend([
                    RegionKind::LeftModify,
                    RegionKind::RightModify,
                    RegionKind::BothModify,
                    RegionKind::LeftDelete,
                    RegionKind::RightDelete,
                ]);
                if allow_conflicts {
                    options.push(RegionKind::Conflict);
                }
            }
            if index == line_count && drop_trailing_newline {
                options.clear();
            }
            if let Some(&kind) = options.get(rng.random_range(0..options.len().max(1))) {
                let original = base_lines.get(index).cloned().unwrap_or_default();
                match kind {
                    RegionKind::LeftModify => {
                        let lines = fresh_lines(rng, "left");
                        left.extend(lines.clone());
                        right.push(original.clone());
                        expected.extend(lines);
                        base.push(original);
                        index += 1;
                    }
                    RegionKind::RightModify => {
                        let lines = fresh_lines(rng, "right");
                        right.extend(lines.clone());
                        left.push(original.clone());
                        expected.extend(lines);
                        base.push(original);
                        index += 1;
                    }
                    RegionKind::BothModify => {
                        let lines = fresh_lines(rng, "both");
                        left.extend(lines.clone());
                        right.extend(lines.clone());
                        expected.extend(lines);
                        base.push(original);
                        index += 1;
                    }
                    RegionKind::LeftDelete => {
                        right.push(original.clone());
                        base.push(original);
                        index += 1;
                    }
                    RegionKind::RightDelete => {
                        left.push(original.clone());
                        base.push(original);
                        index += 1;
                    }
                    RegionKind::LeftInsert => {
                        let lines = fresh_lines(rng, "left");
                        left.extend(lines.clone());
                        expected.extend(lines);
                    }
                    RegionKind::RightInsert => {
                        let lines = fresh_lines(rng, "right");
                        right.extend(lines.clone());
                        expected.extend(lines);
                    }
                    RegionKind::BothInsert => {
                        let lines = fresh_lines(rng, "both");
                        left.extend(lines.clone());
                        right.extend(lines.clone());
                        expected.extend(lines);
                    }
                    RegionKind::Conflict => {
                        let left_lines = fresh_lines(rng, "left");
                        let right_lines = fresh_lines(rng, "right");
                        left.extend(left_lines.clone());
                        right.extend(right_lines);
                        expected.extend(left_lines);
                        base.push(original);
                        index += 1;
                        conflicts += 1;
                    }
                }
                if !matches!(kind, RegionKind::Conflict) {
                    changes += 1;
                }
                previous_changed = true;
                continue;
            }
        }
        previous_changed = false;
        if let Some(line) = base_lines.get(index) {
            let line = if drop_trailing_newline && index + 1 == line_count {
                line.trim_end_matches('\n').to_string()
            } else {
                line.clone()
            };
            base.push(line.clone());
            left.push(line.clone());
            right.push(line.clone());
            expected.push(line);
        }
        index += 1;
    }

    Generated {
        base: base.concat(),
        left: left.concat(),
        right: right.concat(),
        expected_with_left_conflicts: expected.concat(),
        conflicts,
        changes,
    }
}

#[test]
fn random_clean_merges_match_diffy() {
    for seed in 0..500u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let generated = generate(&mut rng, false);
        let merge = merge(&generated.base, &generated.left, &generated.right);
        let context = format!(
            "seed {seed}\nbase:\n{}\nleft:\n{}\nright:\n{}",
            generated.base, generated.left, generated.right
        );
        assert!(!merge.has_conflicts(), "{context}");
        assert_eq!(merge.chunks().len(), generated.changes, "{context}");
        assert_eq!(
            merge.clean_merge().as_deref(),
            Some(generated.expected_with_left_conflicts.as_str()),
            "{context}"
        );
        assert_eq!(
            diffy::merge(&generated.base, &generated.left, &generated.right).as_deref(),
            Ok(generated.expected_with_left_conflicts.as_str()),
            "{context}"
        );

        let mut document = MergeDocument::new(merge);
        document.apply_non_conflicting(NonConflictingScope::All);
        assert_eq!(
            document.result().text(),
            generated.expected_with_left_conflicts,
            "{context}"
        );
        assert!(document.model().counters().is_complete(), "{context}");
    }
}

#[test]
fn random_conflicts_are_detected_and_resolvable() {
    let mut seen_conflicts = 0;
    for seed in 0..500u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let generated = generate(&mut rng, true);
        let merge = merge(&generated.base, &generated.left, &generated.right);
        let context = format!(
            "seed {seed}\nbase:\n{}\nleft:\n{}\nright:\n{}",
            generated.base, generated.left, generated.right
        );
        let conflict_count = merge
            .chunks()
            .iter()
            .filter(|chunk| chunk.is_conflict())
            .count();
        assert_eq!(conflict_count, generated.conflicts, "{context}");
        assert_eq!(
            merge.chunks().len(),
            generated.changes + generated.conflicts,
            "{context}"
        );
        assert_eq!(
            diffy::merge(&generated.base, &generated.left, &generated.right).is_err(),
            generated.conflicts > 0,
            "{context}"
        );
        seen_conflicts += conflict_count;

        let mut document = MergeDocument::new(merge);
        document.apply_non_conflicting(NonConflictingScope::All);
        assert_eq!(
            document.model().counters().conflicts,
            generated.conflicts,
            "{context}"
        );
        for chunk_index in 0..document.model().chunks().len() {
            if document.model().chunks()[chunk_index].is_conflict() {
                assert!(document.resolve_using(chunk_index, Side::Left), "{context}");
            }
        }
        assert_eq!(
            document.result().text(),
            generated.expected_with_left_conflicts,
            "{context}"
        );
        assert!(document.model().counters().is_complete(), "{context}");
    }
    assert!(seen_conflicts > 100);
}

fn random_text(rng: &mut StdRng) -> String {
    const LINES: [&str; 7] = ["a\n", "b\n", "c\n", "\n", "  a\n", "a\r\n", "a"];
    let count = rng.random_range(0..10usize);
    let mut text = String::new();
    for _ in 0..count {
        let line = LINES[rng.random_range(0..LINES.len())];
        if text.ends_with(|character: char| character != '\n') {
            text.push('\n');
        }
        text.push_str(line);
    }
    text
}

#[test]
fn random_texts_keep_structural_invariants() {
    for seed in 0..2000u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let base = random_text(&mut rng);
        let left = random_text(&mut rng);
        let right = random_text(&mut rng);
        for policy in IgnorePolicy::ALL {
            let merge = ThreeWayMerge::new(base.clone(), left.clone(), right.clone(), policy);
            let context = format!("seed {seed} {policy:?}\n{base:?}\n{left:?}\n{right:?}");
            let mut previous_end: Option<u32> = None;
            for chunk in merge.chunks() {
                if let Some(end) = previous_end {
                    assert!(chunk.base.start > end, "{context}");
                }
                previous_end = Some(chunk.base.end);
                assert!(
                    chunk.base.end <= merge.line_count(Revision::Base),
                    "{context}"
                );
                assert!(
                    chunk.left.end <= merge.line_count(Revision::Left),
                    "{context}"
                );
                assert!(
                    chunk.right.end <= merge.line_count(Revision::Right),
                    "{context}"
                );
                assert!(chunk.base.start <= chunk.base.end, "{context}");
            }

            if policy != IgnorePolicy::None {
                continue;
            }
            for side in [Side::Left, Side::Right] {
                let mut document = MergeDocument::new(merge.clone());
                for chunk_index in 0..merge.chunks().len() {
                    document.resolve_using(chunk_index, side);
                }
                assert_eq!(
                    document.result().text(),
                    merge.text(side.revision()),
                    "{context}"
                );
                assert!(document.model().counters().is_complete(), "{context}");
            }
            let mut ignored = MergeDocument::new(merge.clone());
            for chunk_index in 0..merge.chunks().len() {
                ignored.ignore(chunk_index, Side::Left);
                ignored.ignore(chunk_index, Side::Right);
            }
            assert_eq!(ignored.result().text(), base, "{context}");
            assert!(ignored.model().counters().is_complete(), "{context}");
            let mut auto_merged = MergeDocument::new(merge.clone());
            auto_merged.apply_non_conflicting(NonConflictingScope::All);
            match merge.clean_merge() {
                Some(clean) => {
                    assert!(!merge.has_conflicts(), "{context}");
                    assert_eq!(auto_merged.result().text(), clean, "{context}");
                    assert!(auto_merged.model().counters().is_complete(), "{context}");
                }
                None => {
                    assert!(merge.has_conflicts(), "{context}");
                    assert!(!auto_merged.model().counters().is_complete(), "{context}");
                }
            }
        }
    }
}
