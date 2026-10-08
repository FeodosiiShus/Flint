use merge_diff::{
    CancellationChecker, ComparisonError, ComparisonPolicy, ConflictKind, DiffFragment,
    LineOffsets, LineTexts, MergeConflictType, MergeInnerDifferences, MergeRange, NeverCanceled,
    ResolutionStrategy, Utf16Offsets, compare_chars, compare_lines, compare_lines_inner,
    compare_lines_three_way, compare_threeside_inner, compare_words, get_inner_chunks,
    line_merge_type, line_three_way_diff_type, merge_lines, try_greedy_resolve, try_resolve,
    word_merge,
};

struct AlwaysCanceled;

impl CancellationChecker for AlwaysCanceled {
    fn is_canceled(&self) -> bool {
        true
    }
}

struct CountingChecker {
    checks: std::cell::Cell<usize>,
}

impl CountingChecker {
    fn new() -> Self {
        Self {
            checks: std::cell::Cell::new(0),
        }
    }
}

impl CancellationChecker for CountingChecker {
    fn is_canceled(&self) -> bool {
        self.checks.set(self.checks.get() + 1);
        false
    }
}

struct CanceledAfter {
    remaining_checks: std::cell::Cell<usize>,
}

impl CanceledAfter {
    fn new(allowed_checks: usize) -> Self {
        Self {
            remaining_checks: std::cell::Cell::new(allowed_checks),
        }
    }
}

impl CancellationChecker for CanceledAfter {
    fn is_canceled(&self) -> bool {
        let remaining = self.remaining_checks.get();
        if remaining == 0 {
            return true;
        }
        self.remaining_checks.set(remaining - 1);
        false
    }
}

fn merge_range(left: (usize, usize), base: (usize, usize), right: (usize, usize)) -> MergeRange {
    MergeRange::new(left.0..left.1, base.0..base.1, right.0..right.1)
}

fn merged_ranges(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
) -> Result<Vec<MergeRange>, ComparisonError> {
    merge_lines(left, base, right, policy, &NeverCanceled)
}

fn compared_ranges(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
) -> Result<Vec<MergeRange>, ComparisonError> {
    compare_lines_three_way(left, base, right, policy, &NeverCanceled)
}

fn merge_type(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> MergeConflictType {
    let Some(conflict_type) = line_merge_type(range, texts, policy) else {
        panic!("range {range:?} differs on at least one side");
    };
    conflict_type
}

fn inner_differences(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> Result<Option<MergeInnerDifferences>, ComparisonError> {
    let conflict_type = merge_type(range, texts, policy);
    let chunks = get_inner_chunks(range, texts, &conflict_type);
    compare_threeside_inner(chunks, policy, &NeverCanceled)
}

#[test]
fn one_set_of_line_texts_classifies_every_range_of_a_document() -> Result<(), ComparisonError> {
    let left = "keep\nL1\nmid\nA\nend\n";
    let base = "keep\nB1\nmid\nB\nend\n";
    let right = "keep\nB1\nmid\nC\nend\n";
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    let ranges = merged_ranges(left, base, right, ComparisonPolicy::Default)?;
    assert_eq!(
        ranges,
        vec![
            merge_range((1, 2), (1, 2), (1, 2)),
            merge_range((3, 4), (3, 4), (3, 4)),
        ]
    );
    assert_eq!(
        merge_type(&ranges[0], texts, ComparisonPolicy::Default),
        MergeConflictType::new(ConflictKind::Modified, true, false)
    );
    let unresolvable = merge_type(&ranges[1], texts, ComparisonPolicy::Default);
    assert_eq!(
        unresolvable,
        MergeConflictType::with_resolution(ConflictKind::Conflict, true, true, None)
    );
    assert!(!unresolvable.can_be_resolved());
    for range in &ranges {
        assert_eq!(
            line_three_way_diff_type(range, texts, ComparisonPolicy::Default),
            Some(merge_type(range, texts, ComparisonPolicy::Default))
        );
    }
    Ok(())
}

#[test]
fn word_level_conflict_is_resolvable_by_text_strategy() -> Result<(), ComparisonError> {
    let left = "a\nx1 b\nc\n";
    let base = "a\nx b\nc\n";
    let right = "a\nx b y\nc\n";
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    let ranges = merged_ranges(left, base, right, ComparisonPolicy::Default)?;
    assert_eq!(ranges, vec![merge_range((1, 2), (1, 2), (1, 2))]);
    let conflict_type = merge_type(&ranges[0], texts, ComparisonPolicy::Default);
    assert_eq!(
        conflict_type,
        MergeConflictType::with_resolution(
            ConflictKind::Conflict,
            true,
            true,
            Some(ResolutionStrategy::Text)
        )
    );
    assert!(conflict_type.is_conflict());
    let chunks = get_inner_chunks(&ranges[0], texts, &conflict_type);
    assert_eq!(chunks, [Some("x1 b"), Some("x b"), Some("x b y")]);
    let Some(inner) = inner_differences(&ranges[0], texts, ComparisonPolicy::Default)? else {
        panic!("three changed sides produce inner differences");
    };
    assert_eq!(inner.left, Some(vec![0..2]));
    assert_eq!(inner.base, Some(vec![0..1, 3..3]));
    assert_eq!(inner.right, Some(vec![3..5]));
    Ok(())
}

#[test]
fn inner_difference_ranges_are_utf8_bytes_convertible_to_utf16() -> Result<(), ComparisonError> {
    let left = "a\n😀 B\nc\n";
    let base = "a\nB\nc\n";
    let right = "a\nB x\nc\n";
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    let ranges = merged_ranges(left, base, right, ComparisonPolicy::Default)?;
    assert_eq!(ranges, vec![merge_range((1, 2), (1, 2), (1, 2))]);
    let Some(inner) = inner_differences(&ranges[0], texts, ComparisonPolicy::Default)? else {
        panic!("three changed sides produce inner differences");
    };
    assert_eq!(inner.left, Some(vec![0..5]));
    assert_eq!(inner.base, Some(vec![0..0, 1..1]));
    assert_eq!(inner.right, Some(vec![1..3]));
    let chunk_offsets = Utf16Offsets::new("😀 B");
    let left_ranges_in_utf16: Vec<(Option<usize>, Option<usize>)> = inner
        .left
        .unwrap_or_default()
        .iter()
        .map(|range| {
            (
                chunk_offsets.to_utf16(range.start),
                chunk_offsets.to_utf16(range.end),
            )
        })
        .collect();
    assert_eq!(left_ranges_in_utf16, vec![(Some(0), Some(3))]);
    Ok(())
}

#[test]
fn whitespace_only_change_is_kept_by_merge_lines_but_dropped_by_compare()
-> Result<(), ComparisonError> {
    let left = "keep\n  B1  \nmid\nend\n";
    let base = "keep\nB1\nmid\nend\n";
    let right = "keep\nB1\nmid\nend\nnew\n";
    let whitespace_range = merge_range((1, 2), (1, 2), (1, 2));
    let insertion_range = merge_range((4, 4), (4, 4), (4, 5));
    assert_eq!(
        compared_ranges(left, base, right, ComparisonPolicy::Default)?,
        vec![whitespace_range.clone(), insertion_range.clone()]
    );
    for policy in [
        ComparisonPolicy::TrimWhitespaces,
        ComparisonPolicy::IgnoreWhitespaces,
    ] {
        assert_eq!(
            compared_ranges(left, base, right, policy)?,
            vec![insertion_range.clone()]
        );
        assert_eq!(
            merged_ranges(left, base, right, policy)?,
            vec![whitespace_range.clone(), insertion_range.clone()]
        );
        let left_texts = LineTexts::new(left);
        let base_texts = LineTexts::new(base);
        let right_texts = LineTexts::new(right);
        let texts = [&left_texts, &base_texts, &right_texts];
        assert_eq!(
            merge_type(&whitespace_range, texts, policy),
            MergeConflictType::new(ConflictKind::Modified, true, false)
        );
        assert_eq!(
            inner_differences(&whitespace_range, texts, policy)?,
            Some(MergeInnerDifferences {
                left: Some(Vec::new()),
                base: Some(Vec::new()),
                right: None,
            })
        );
        assert_eq!(inner_differences(&insertion_range, texts, policy)?, None);
    }
    Ok(())
}

#[test]
fn line_fragments_carry_line_indexes_byte_offsets_and_relative_inner_fragments()
-> Result<(), ComparisonError> {
    let left = "one\ntwo\nthree\n";
    let right = "one\n2\nthree\n";
    let plain = compare_lines(left, right, ComparisonPolicy::Default, &NeverCanceled)?;
    assert_eq!(plain.len(), 1);
    let fragment = &plain[0];
    assert_eq!(
        (
            fragment.start_line1,
            fragment.end_line1,
            fragment.start_line2,
            fragment.end_line2
        ),
        (1, 2, 1, 2)
    );
    assert_eq!(
        (
            fragment.start_offset1,
            fragment.end_offset1,
            fragment.start_offset2,
            fragment.end_offset2
        ),
        (4, 8, 4, 6)
    );
    assert_eq!(fragment.inner_fragments, None);
    let detailed = compare_lines_inner(left, right, ComparisonPolicy::Default, &NeverCanceled)?;
    assert_eq!(detailed.len(), 1);
    assert_eq!(
        detailed[0].inner_fragments,
        Some(vec![DiffFragment {
            start_offset1: 0,
            end_offset1: 3,
            start_offset2: 0,
            end_offset2: 1,
        }])
    );
    Ok(())
}

#[test]
fn word_and_character_diffs_use_absolute_byte_offsets() -> Result<(), ComparisonError> {
    let words = compare_words(
        "alpha beta",
        "alpha gamma",
        ComparisonPolicy::Default,
        &NeverCanceled,
    )?;
    assert_eq!(
        words,
        vec![DiffFragment {
            start_offset1: 6,
            end_offset1: 10,
            start_offset2: 6,
            end_offset2: 11,
        }]
    );
    let characters = compare_chars(
        "alpha beta",
        "alpha gamma",
        ComparisonPolicy::Default,
        &NeverCanceled,
    )?;
    assert_eq!(
        characters,
        vec![DiffFragment {
            start_offset1: 6,
            end_offset1: 9,
            start_offset2: 6,
            end_offset2: 10,
        }]
    );
    Ok(())
}

#[test]
fn utf16_offsets_round_trip_every_character_boundary() {
    let text = "a😀éb";
    let offsets = Utf16Offsets::new(text);
    let boundaries = [(0, 0), (1, 1), (5, 3), (7, 4), (8, 5)];
    for (byte_offset, utf16_offset) in boundaries {
        assert_eq!(offsets.to_utf16(byte_offset), Some(utf16_offset));
        assert_eq!(offsets.to_byte(utf16_offset), Some(byte_offset));
    }
}

#[test]
fn utf16_offsets_reject_offsets_that_do_not_start_a_character() {
    let offsets = Utf16Offsets::new("a😀b");
    for inside_character in [2, 3, 4] {
        assert_eq!(offsets.to_utf16(inside_character), None);
    }
    assert_eq!(offsets.to_utf16(7), None);
    assert_eq!(offsets.to_byte(2), None);
    assert_eq!(offsets.to_byte(5), None);

    let ascii = Utf16Offsets::new("abc");
    assert_eq!(ascii.to_utf16(3), Some(3));
    assert_eq!(ascii.to_byte(3), Some(3));
    assert_eq!(ascii.to_utf16(4), None);
    assert_eq!(ascii.to_byte(4), None);

    let empty = Utf16Offsets::new("");
    assert_eq!(empty.to_utf16(0), Some(0));
    assert_eq!(empty.to_utf16(1), None);
}

#[test]
fn line_offsets_split_only_on_line_feed_and_keep_the_trailing_empty_line() {
    assert_eq!(LineOffsets::new("").line_count(), 1);
    assert_eq!(LineOffsets::new("a\r\nb").line_count(), 2);
    let offsets = LineOffsets::new("a\nb\n");
    assert_eq!(offsets.line_count(), 3);
    assert_eq!(offsets.text_length(), 4);
    assert_eq!(offsets.line_start(2), 4);
    assert_eq!(offsets.line_end(0, false), 1);
    assert_eq!(offsets.line_end(0, true), 2);
    assert_eq!(offsets.line_end(2, true), 4);
    assert_eq!(offsets.lines_range(1, 2, false), (2, 3));
    assert_eq!(offsets.lines_range(1, 2, true), (2, 4));
    assert_eq!(offsets.lines_range(3, 3, false), (4, 4));
}

#[test]
fn canceled_checker_aborts_every_diff_entry_point() {
    let left = "one\ntwo\nthree\n";
    let right = "one\n2\nthree\n";
    let policy = ComparisonPolicy::Default;
    assert_eq!(
        compare_lines(left, right, policy, &AlwaysCanceled),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        compare_words("alpha beta", "alpha gamma", policy, &AlwaysCanceled),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        compare_chars("alpha beta", "alpha gamma", policy, &AlwaysCanceled),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        compare_lines_three_way(
            "a\nL\nc\n",
            "a\nB\nc\n",
            "a\nB\nc\n",
            policy,
            &AlwaysCanceled
        ),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        merge_lines(
            "a\nL\nc\n",
            "a\nB\nc\n",
            "a\nB\nc\n",
            policy,
            &AlwaysCanceled
        ),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        word_merge("a b c", "a x c", "a y c", policy, &AlwaysCanceled),
        Err(ComparisonError::Canceled)
    );
    assert_eq!(
        compare_threeside_inner(
            [Some("a b"), Some("a x"), Some("a y")],
            policy,
            &AlwaysCanceled
        ),
        Err(ComparisonError::Canceled)
    );
}

#[test]
fn cancellation_after_the_line_diff_is_reported_by_the_inner_diff() -> Result<(), ComparisonError> {
    let left = "one\ntwo\nthree\n";
    let right = "one\n2\nthree\n";
    let policy = ComparisonPolicy::Default;
    let counting = CountingChecker::new();
    compare_lines(left, right, policy, &counting)?;
    let line_diff_checks = counting.checks.get();
    assert!(line_diff_checks > 0);
    assert!(compare_lines(left, right, policy, &CanceledAfter::new(line_diff_checks)).is_ok());
    assert_eq!(
        compare_lines_inner(left, right, policy, &CanceledAfter::new(line_diff_checks)),
        Err(ComparisonError::Canceled)
    );
    Ok(())
}

#[test]
fn text_resolution_merges_non_overlapping_word_edits() {
    assert_eq!(
        try_resolve("x1 b", "x b", "x b y"),
        Some("x1 b y".to_string())
    );
    assert_eq!(
        try_greedy_resolve("x1 b", "x b", "x b y"),
        Some("x1 b y".to_string())
    );
    assert_eq!(try_resolve("A", "B", "C"), None);
    assert_eq!(try_resolve("a\n", "a\n", "a\n"), Some("a\n".to_string()));
}
