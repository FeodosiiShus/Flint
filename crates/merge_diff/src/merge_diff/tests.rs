use crate::lcs::{Bits, Patience};
use crate::{
    ComparisonError, ComparisonPolicy, ConflictKind, DiffFragment, InnerFragmentsPolicy,
    LineFragment, LineTexts, MergeConflictType, MergeInnerDifferences, MergeRange, NeverCanceled,
    Utf16Offsets, compare_lines, compare_lines_inner, compare_lines_three_way,
    compare_lines_with_inner_policy, compare_threeside_inner, get_inner_chunks, line_merge_type,
    line_three_way_diff_type, merge_lines, try_greedy_resolve, try_resolve, try_resolve_conflict,
    word_merge,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangedSides {
    Left,
    Right,
    Both,
}

fn changed_sides(merge_type: &MergeConflictType) -> ChangedSides {
    match (merge_type.left_change, merge_type.right_change) {
        (true, true) => ChangedSides::Both,
        (true, false) => ChangedSides::Left,
        (false, _) => ChangedSides::Right,
    }
}

fn with_newlines(text: &str) -> String {
    text.replace('_', "\n")
}

fn merge_type_of(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> MergeConflictType {
    let Some(merge_type) = line_merge_type(range, texts, policy) else {
        panic!("merge type must exist for {range:?}");
    };
    merge_type
}

fn merge_summary(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
) -> Result<Vec<(ConflictKind, ChangedSides, usize, usize)>, ComparisonError> {
    let left = with_newlines(left);
    let base = with_newlines(base);
    let right = with_newlines(right);
    let ranges = merge_lines(&left, &base, &right, policy, &NeverCanceled)?;
    let left_texts = LineTexts::new(&left);
    let base_texts = LineTexts::new(&base);
    let right_texts = LineTexts::new(&right);
    Ok(ranges
        .iter()
        .map(|range| {
            let merge_type = merge_type_of(range, [&left_texts, &base_texts, &right_texts], policy);
            (
                merge_type.kind,
                changed_sides(&merge_type),
                range.base.start,
                range.base.end,
            )
        })
        .collect())
}

fn line_ranges(fragments: &[LineFragment]) -> Vec<(usize, usize, usize, usize)> {
    fragments
        .iter()
        .map(|fragment| {
            (
                fragment.start_line1,
                fragment.end_line1,
                fragment.start_line2,
                fragment.end_line2,
            )
        })
        .collect()
}

fn two_way_line_ranges(
    left: &str,
    right: &str,
    policy: ComparisonPolicy,
) -> Result<Vec<(usize, usize, usize, usize)>, ComparisonError> {
    let fragments = compare_lines(
        &with_newlines(left),
        &with_newlines(right),
        policy,
        &NeverCanceled,
    )?;
    Ok(line_ranges(&fragments))
}

fn three_way_ranges(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
) -> Result<Vec<MergeRange>, ComparisonError> {
    merge_lines(left, base, right, policy, &NeverCanceled)
}

fn utf16_offset(offsets: &Utf16Offsets, byte_offset: usize) -> usize {
    match offsets.to_utf16(byte_offset) {
        Some(utf16_offset) => utf16_offset,
        None => panic!("byte offset {byte_offset} must lie on a character boundary"),
    }
}

fn utf16_fragment_offsets(
    fragments: &[LineFragment],
    text1: &str,
    text2: &str,
) -> Vec<(usize, usize, usize, usize)> {
    let offsets1 = Utf16Offsets::new(text1);
    let offsets2 = Utf16Offsets::new(text2);
    fragments
        .iter()
        .map(|fragment| {
            (
                utf16_offset(&offsets1, fragment.start_offset1),
                utf16_offset(&offsets1, fragment.end_offset1),
                utf16_offset(&offsets2, fragment.start_offset2),
                utf16_offset(&offsets2, fragment.end_offset2),
            )
        })
        .collect()
}

const ALL_POLICIES: [ComparisonPolicy; 3] = [
    ComparisonPolicy::Default,
    ComparisonPolicy::TrimWhitespaces,
    ComparisonPolicy::IgnoreWhitespaces,
];

type MergeVector = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(ConflictKind, ChangedSides, usize, usize)],
);

const MERGE_VECTORS: [MergeVector; 29] = [
    (
        "x",
        "x",
        "y",
        &[(ConflictKind::Modified, ChangedSides::Right, 0, 1)],
    ),
    (
        "x",
        "y",
        "x",
        &[(ConflictKind::Modified, ChangedSides::Both, 0, 1)],
    ),
    (
        "x_Y",
        "Y",
        "Y_z",
        &[
            (ConflictKind::Inserted, ChangedSides::Left, 0, 0),
            (ConflictKind::Inserted, ChangedSides::Right, 1, 1),
        ],
    ),
    (
        "Y_z",
        "x_Y_z",
        "x_Y",
        &[
            (ConflictKind::Deleted, ChangedSides::Left, 0, 1),
            (ConflictKind::Deleted, ChangedSides::Right, 2, 3),
        ],
    ),
    (
        "X_Z",
        "X_y_Z",
        "X_Z",
        &[(ConflictKind::Deleted, ChangedSides::Both, 1, 2)],
    ),
    (
        "X_y_Z",
        "X_Z",
        "X_y_Z",
        &[(ConflictKind::Inserted, ChangedSides::Both, 1, 1)],
    ),
    (
        "x",
        "y",
        "z",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 1)],
    ),
    (
        "z_Y",
        "x_Y",
        "Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 1)],
    ),
    (
        "z_Y",
        "x_Y",
        "k_x_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 1)],
    ),
    (
        "x_Y",
        "Y",
        "z_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 0)],
    ),
    (
        "x_Y",
        "Y",
        "z_x_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 0)],
    ),
    (
        "x_Y",
        "x_z_Y",
        "z_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 0, 2)],
    ),
    (
        "x",
        "x_",
        "x",
        &[(ConflictKind::Deleted, ChangedSides::Both, 1, 2)],
    ),
    (
        "x_",
        "x_",
        "x",
        &[(ConflictKind::Deleted, ChangedSides::Right, 1, 2)],
    ),
    (
        "x_",
        "x_",
        "x_y",
        &[(ConflictKind::Modified, ChangedSides::Right, 1, 2)],
    ),
    (
        "x",
        "x_",
        "x_y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 1, 2)],
    ),
    (
        "x_",
        "x",
        "x_y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 1, 1)],
    ),
    ("", "", "", &[]),
    ("x", "x", "x", &[]),
    ("x_y_z a x", "x_y_z a x", "x_y_z a x", &[]),
    (
        "X_1_Y_2_Z_3_4_U_W_",
        "X_a_Y_b_Z_c_U_d_W_",
        "X_a_Y_B_Z_C_U_D_W_",
        &[
            (ConflictKind::Modified, ChangedSides::Left, 1, 2),
            (ConflictKind::Conflict, ChangedSides::Both, 3, 4),
            (ConflictKind::Conflict, ChangedSides::Both, 5, 6),
            (ConflictKind::Conflict, ChangedSides::Both, 7, 8),
        ],
    ),
    (
        "X_1_Y_2_Z",
        "X_a_Y_b_Z",
        "X_a_Y_B_Z",
        &[
            (ConflictKind::Modified, ChangedSides::Left, 1, 2),
            (ConflictKind::Conflict, ChangedSides::Both, 3, 4),
        ],
    ),
    (
        "X_1_2_3_Z",
        "X_1_b_3_d_e_f_Z",
        "X_a_b_c_Z",
        &[(ConflictKind::Conflict, ChangedSides::Both, 1, 7)],
    ),
    (
        "a_X_b_c",
        "A_X_B_c",
        "1_X_1_c",
        &[
            (ConflictKind::Conflict, ChangedSides::Both, 0, 1),
            (ConflictKind::Conflict, ChangedSides::Both, 2, 3),
        ],
    ),
    (
        "a_Ins1_b_ccc",
        "a_b_ccc",
        "a_b_dddd",
        &[
            (ConflictKind::Inserted, ChangedSides::Left, 1, 1),
            (ConflictKind::Modified, ChangedSides::Right, 2, 3),
        ],
    ),
    (
        "X_b_Y",
        "X_1_2_3_Y",
        "X_a_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 1, 4)],
    ),
    (
        "X_Y",
        "X_1_2_3_Y",
        "X_a_Y",
        &[(ConflictKind::Conflict, ChangedSides::Both, 1, 4)],
    ),
    (
        "X_1_2_Y",
        "X_1_Ins_2_Y",
        "X_1_2_Y",
        &[(ConflictKind::Deleted, ChangedSides::Both, 2, 3)],
    ),
    (
        "C_X",
        "C_",
        "C_",
        &[(ConflictKind::Modified, ChangedSides::Left, 1, 2)],
    ),
];

#[test]
fn merge_lines_reproduces_intellij_merge_test_vectors() -> Result<(), ComparisonError> {
    for (left, base, right, expected) in MERGE_VECTORS {
        let actual = merge_summary(left, base, right, ComparisonPolicy::Default)?;
        assert_eq!(
            actual.as_slice(),
            expected,
            "left={left:?} base={base:?} right={right:?}"
        );
    }
    Ok(())
}

const RESOLVE_VECTORS: [(&str, &str, &str, Option<&str>); 14] = [
    ("", "", "", Some("")),
    ("x x x", "x x x", "x x x", Some("x x x")),
    ("x x x", "x Y x", "x x x", Some("x Y x")),
    ("x x", "x x", "x Y x", Some("x Y x")),
    ("x X x", "x x", "x X x", Some("x x")),
    ("x x x", "x Y x", "x Y x", Some("x Y x")),
    ("x x", "x Y x", "x Y x", Some("x Y x")),
    ("x X x", "x x", "x x", Some("x x")),
    ("x x x", "x Y x x", "x x Z x", Some("x Y x Z x")),
    ("x", "x Y", "Z x", Some("Z x Y")),
    ("x x", "x", "Z x x", Some("Z x")),
    ("x x x", "x Y x", "x Z x", None),
    ("x x", "x Y x", "x Z x", None),
    (
        "version: 1.0.0",
        "version: 2.0.0",
        "version: 1.0.4",
        Some("version: 2.0.4"),
    ),
];

#[test]
fn try_resolve_and_greedy_resolve_reproduce_intellij_resolve_vectors() {
    for (base, left, right, expected) in RESOLVE_VECTORS {
        let expected = expected.map(str::to_owned);
        assert_eq!(
            try_resolve(left, base, right),
            expected,
            "simple base={base:?} left={left:?} right={right:?}"
        );
        assert_eq!(
            try_greedy_resolve(left, base, right),
            expected,
            "greedy base={base:?} left={left:?} right={right:?}"
        );
        assert_eq!(
            try_resolve_conflict(left, base, right),
            expected,
            "conflict base={base:?} left={left:?} right={right:?}"
        );
    }
}

const GREEDY_VECTORS: [(&str, &str, &str, &str); 9] = [
    ("x X x", "x x", "x X Y x", "x Y x"),
    ("x X x", "x x", "x Y X x", "x Y x"),
    ("x X Y x", "x X x", "x Y x", "x x"),
    ("x X Y Z x", "x X x", "x Z x", "x x"),
    ("x A B C D E F G H K x", "x C F K x", "x A D H x", "x x"),
    ("x X x", "x x", "x Z x", "xZ x"),
    ("x X X x", "x X Y X x", "x x", "x Y x"),
    ("x X x", "x x", "x Y x", "xY x"),
    ("x X X x", "x Y x", "x X Y x", "x Y x"),
];

#[test]
fn try_greedy_resolve_reproduces_intellij_greedy_vectors() {
    for (base, left, right, expected) in GREEDY_VECTORS {
        assert_eq!(
            try_greedy_resolve(left, base, right).as_deref(),
            Some(expected),
            "base={base:?} left={left:?} right={right:?}"
        );
    }
}

fn patience_changes(first: &[u32], second: &[u32]) -> Result<(Bits, Bits), ComparisonError> {
    let mut patience = Patience::new(first, second);
    patience.execute(false)?;
    Ok((patience.changes1, patience.changes2))
}

type PatienceVector = (&'static [u32], &'static [u32], &'static [u8], &'static [u8]);

const PATIENCE_VECTORS: [PatienceVector; 15] = [
    (&[1, 2, 3], &[1, 2, 3], &[0, 0, 0], &[0, 0, 0]),
    (&[1, 2], &[1, 3], &[0, 1], &[0, 1]),
    (&[1, 3], &[2, 3], &[1, 0], &[1, 0]),
    (&[1, 2], &[1, 2, 3], &[0, 0], &[0, 0, 1]),
    (&[1, 2, 3], &[2, 3], &[1, 0, 0], &[0, 0]),
    (&[1, 2, 3], &[4, 2, 5], &[1, 0, 1], &[1, 0, 1]),
    (&[1, 2], &[3, 4], &[1, 1], &[1, 1]),
    (&[1, 2, 3], &[4, 5, 6], &[1, 1, 1], &[1, 1, 1]),
    (
        &[1, 1, 2, 2, 10],
        &[10, 1, 1, 2, 2],
        &[1, 1, 1, 1, 0],
        &[0, 1, 1, 1, 1],
    ),
    (
        &[1, 2, 3, 4, 5, 6, 7, 8, 7, 9, 10, 2, 11, 12, 5, 13, 7, 8, 7],
        &[10, 2, 11, 12, 5, 13, 7, 8, 7, 9, 1, 2, 3, 4, 5, 6, 7, 8, 7],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    ),
    (
        &[2, 3, 4, 6, 7, 8, 9, 11, 12, 4, 6, 11, 15],
        &[2, 3, 6, 7, 8, 9, 6, 11, 12, 4, 11, 15, 6],
        &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0],
    ),
    (
        &[0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0],
        &[1, 2, 1, 3, 1, 4, 1, 5, 1, 6, 1, 7, 1],
        &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1],
        &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1],
    ),
    (
        &[0, 2, 3, 0, 4, 5, 0],
        &[1, 2, 1, 3, 4, 5, 1],
        &[1, 0, 0, 1, 0, 0, 1],
        &[1, 0, 1, 0, 0, 0, 1],
    ),
    (
        &[15, 1, 2, 3, 1, 4, 5, 1, 15],
        &[13, 1, 2, 1, 3, 4, 5, 1, 13],
        &[1, 0, 0, 0, 1, 0, 0, 0, 1],
        &[1, 0, 0, 1, 0, 0, 0, 0, 1],
    ),
    (
        &[15, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 15],
        &[13, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 13],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    ),
];

#[test]
fn patience_marks_the_same_changed_elements_as_intellij_patience_int_lcs_tests()
-> Result<(), ComparisonError> {
    for (first, second, expected1, expected2) in PATIENCE_VECTORS {
        let (changes1, changes2) = patience_changes(first, second)?;
        for (index, expected) in expected1.iter().enumerate() {
            assert_eq!(
                changes1.get(index),
                *expected == 1,
                "first={first:?} second={second:?} index={index}"
            );
        }
        for (index, expected) in expected2.iter().enumerate() {
            assert_eq!(
                changes2.get(index),
                *expected == 1,
                "first={first:?} second={second:?} index={index}"
            );
        }
    }
    Ok(())
}

const PATIENCE_ABORT_VECTORS: [(bool, &[u32], &[u32]); 7] = [
    (false, &[1, 2, 3], &[1, 2, 3]),
    (false, &[100, 1, 2, 3, 4, 5, 100], &[1, 2, 3, 4]),
    (true, &[4, 1, 2, 3], &[500, 100, 600]),
    (true, &[4, 1, 2, 3, 100], &[5, 1, 6, 8, 9, 10]),
    (
        false,
        &[100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 100],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0],
    ),
    (
        false,
        &[
            100, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 200, 1, 0, 100,
        ],
        &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2],
    ),
    (
        true,
        &[
            100, 3, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 200, 1, 0, 100,
        ],
        &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0, 2],
    ),
];

#[test]
fn patience_aborts_on_small_reduction_like_intellij() {
    for (expected_abort, first, second) in PATIENCE_ABORT_VECTORS {
        let mut patience = Patience::new(first, second);
        let aborted = patience.execute(true) == Err(ComparisonError::DiffTooBig);
        assert_eq!(aborted, expected_abort, "first={first:?} second={second:?}");
    }
}

#[test]
fn line_fragments_count_utf16_units_for_unimportant_line_threshold() -> Result<(), ComparisonError>
{
    let cases: [(
        &str,
        &str,
        [(usize, usize, usize, usize); 2],
        [(usize, usize, usize, usize); 2],
    ); 1] = [(
        "\n\u{1F9D2} ",
        "\u{1F9D2} \n\n",
        [(0, 0, 0, 1), (1, 2, 2, 3)],
        [(0, 0, 0, 4), (1, 4, 5, 5)],
    )];
    for (left, right, expected_lines, expected_offsets) in cases {
        for policy in ALL_POLICIES {
            let fragments = compare_lines(left, right, policy, &NeverCanceled)?;
            assert_eq!(line_ranges(&fragments), expected_lines, "{policy:?}");
            assert_eq!(
                utf16_fragment_offsets(&fragments, left, right),
                expected_offsets,
                "{policy:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn line_fragments_report_utf16_offsets_for_astral_lines() -> Result<(), ComparisonError> {
    let cases = [
        ("a\n\u{1F600}\nb", "a\nc\nb", (2, 5, 2, 4)),
        ("x\n\u{1F600} \ny", "x\n\ny", (2, 6, 2, 3)),
    ];
    for (left, right, expected_offsets) in cases {
        for policy in ALL_POLICIES {
            let fragments = compare_lines(left, right, policy, &NeverCanceled)?;
            assert_eq!(line_ranges(&fragments), [(1, 2, 1, 2)], "{policy:?}");
            assert_eq!(
                utf16_fragment_offsets(&fragments, left, right),
                [expected_offsets],
                "{policy:?}"
            );
        }
    }
    Ok(())
}

fn inner_differences_of(
    left: &str,
    base: &str,
    right: &str,
    range: &MergeRange,
    policy: ComparisonPolicy,
) -> Result<(MergeConflictType, Option<MergeInnerDifferences>), ComparisonError> {
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    let merge_type = merge_type_of(range, texts, policy);
    let chunks = get_inner_chunks(range, texts, &merge_type);
    let inner = compare_threeside_inner(chunks, policy, &NeverCanceled)?;
    Ok((merge_type, inner))
}

#[test]
fn merge_inner_differences_use_utf16_aligned_byte_ranges_for_astral_text()
-> Result<(), ComparisonError> {
    let left = "a\nc\nb";
    let base = "a\n\u{1F600}\nb";
    let right = "a\n\u{1F600}\nb\nq";
    let policy = ComparisonPolicy::Default;
    let ranges = three_way_ranges(left, base, right, policy)?;
    assert_eq!(
        ranges,
        [
            MergeRange::new(1..2, 1..2, 1..2),
            MergeRange::new(3..3, 3..3, 3..4)
        ]
    );
    let (first_type, first_inner) = inner_differences_of(left, base, right, &ranges[0], policy)?;
    assert_eq!(first_type.kind, ConflictKind::Modified);
    assert_eq!(changed_sides(&first_type), ChangedSides::Left);
    assert_eq!(
        first_inner,
        Some(MergeInnerDifferences {
            left: Some(vec![0..1]),
            base: Some(vec![0..4]),
            right: None,
        })
    );
    let (second_type, second_inner) = inner_differences_of(left, base, right, &ranges[1], policy)?;
    assert_eq!(second_type.kind, ConflictKind::Inserted);
    assert_eq!(changed_sides(&second_type), ChangedSides::Right);
    assert_eq!(second_inner, None);
    Ok(())
}

#[test]
fn merge_inner_differences_cover_trailing_space_after_astral_char() -> Result<(), ComparisonError> {
    let left = "x\n\ny";
    let base = "x\n\u{1F600} \ny";
    let right = "x\n\u{1F600} \ny\nq";
    let policy = ComparisonPolicy::Default;
    let ranges = three_way_ranges(left, base, right, policy)?;
    assert_eq!(
        ranges,
        [
            MergeRange::new(1..2, 1..2, 1..2),
            MergeRange::new(3..3, 3..3, 3..4)
        ]
    );
    let (merge_type, inner) = inner_differences_of(left, base, right, &ranges[0], policy)?;
    assert_eq!(merge_type.kind, ConflictKind::Modified);
    assert_eq!(
        inner,
        Some(MergeInnerDifferences {
            left: Some(vec![0..0]),
            base: Some(vec![0..5]),
            right: None,
        })
    );
    Ok(())
}

#[test]
fn compare_lines_three_way_reports_conflict_with_inner_differences_for_surrogate_pairs()
-> Result<(), ComparisonError> {
    let left = "\u{1F9D2} \n\n";
    let base = "\n\u{1F9D2} ";
    let right = "\n\u{1F9D2} \nq";
    let policy = ComparisonPolicy::Default;
    let compared = compare_lines_three_way(left, base, right, policy, &NeverCanceled)?;
    assert_eq!(
        compared,
        [
            MergeRange::new(0..1, 0..0, 0..0),
            MergeRange::new(2..3, 1..2, 1..3)
        ]
    );
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    for range in &compared {
        let merge_type = merge_type_of(range, texts, policy);
        let three_way_type = line_three_way_diff_type(range, texts, policy);
        assert_eq!(three_way_type, Some(merge_type));
    }
    let first_type = merge_type_of(&compared[0], texts, policy);
    assert_eq!(first_type.kind, ConflictKind::Inserted);
    assert_eq!(changed_sides(&first_type), ChangedSides::Left);
    let second_type = merge_type_of(&compared[1], texts, policy);
    assert_eq!(second_type.kind, ConflictKind::Conflict);
    assert!(!second_type.can_be_resolved());
    let merged = merge_lines(left, base, right, policy, &NeverCanceled)?;
    assert_eq!(merged, compared);
    let (_, inner) = inner_differences_of(left, base, right, &merged[1], policy)?;
    assert_eq!(
        inner,
        Some(MergeInnerDifferences {
            left: Some(vec![0..0]),
            base: Some(vec![0..5, 5..5]),
            right: Some(vec![5..7]),
        })
    );
    Ok(())
}

const TWO_WAY_GOLDEN_VECTORS: [(
    &str,
    &str,
    ComparisonPolicy,
    &[(usize, usize, usize, usize)],
); 12] = [
    ("A_B_C", "A_X_C", ComparisonPolicy::Default, &[(1, 2, 1, 2)]),
    (
        "a_b__c_d",
        "a_b_c_d",
        ComparisonPolicy::Default,
        &[(2, 3, 2, 2)],
    ),
    (
        "x__y_z__w",
        "x_y_z_w",
        ComparisonPolicy::Default,
        &[(1, 2, 1, 1), (4, 5, 3, 3)],
    ),
    (
        "{_a_}__{_b_}",
        "{_b_}",
        ComparisonPolicy::Default,
        &[(0, 4, 0, 0)],
    ),
    (
        "{_b_}",
        "{_a_}__{_b_}",
        ComparisonPolicy::Default,
        &[(0, 0, 0, 4)],
    ),
    (
        ".{_..{_...{",
        "..{_...{",
        ComparisonPolicy::Default,
        &[(0, 1, 0, 0)],
    ),
    ("  a_b", "a_b", ComparisonPolicy::Default, &[(0, 1, 0, 1)]),
    ("  a_b", "a_b", ComparisonPolicy::TrimWhitespaces, &[]),
    ("  a_b", "a_b", ComparisonPolicy::IgnoreWhitespaces, &[]),
    (
        "a_b_c_d_e_f",
        "a_c_d_x_f",
        ComparisonPolicy::Default,
        &[(1, 2, 1, 1), (4, 5, 3, 4)],
    ),
    ("", "x", ComparisonPolicy::Default, &[(0, 1, 0, 1)]),
    ("x_", "x", ComparisonPolicy::Default, &[(1, 2, 1, 1)]),
];

#[test]
fn compare_lines_reproduces_golden_two_way_vectors() -> Result<(), ComparisonError> {
    for (left, right, policy, expected) in TWO_WAY_GOLDEN_VECTORS {
        assert_eq!(
            two_way_line_ranges(left, right, policy)?.as_slice(),
            expected,
            "left={left:?} right={right:?} {policy:?}"
        );
    }
    Ok(())
}

type ThreeWayGoldenVector = (
    &'static str,
    &'static str,
    &'static str,
    ComparisonPolicy,
    &'static [(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        ConflictKind,
        ChangedSides,
    )],
);

const THREE_WAY_GOLDEN_VECTORS: [ThreeWayGoldenVector; 8] = [
    (
        "x_y_z",
        "x_y_z",
        "x_Y_z",
        ComparisonPolicy::Default,
        &[(
            1,
            2,
            1,
            2,
            1,
            2,
            ConflictKind::Modified,
            ChangedSides::Right,
        )],
    ),
    (
        " x_y",
        "x_y",
        "x_y",
        ComparisonPolicy::TrimWhitespaces,
        &[(0, 1, 0, 1, 0, 1, ConflictKind::Modified, ChangedSides::Left)],
    ),
    (
        " x_y",
        "x_y",
        "x_y",
        ComparisonPolicy::Default,
        &[(0, 1, 0, 1, 0, 1, ConflictKind::Modified, ChangedSides::Left)],
    ),
    (
        " x_y",
        "x_y",
        "x_y_z",
        ComparisonPolicy::TrimWhitespaces,
        &[
            (0, 1, 0, 1, 0, 1, ConflictKind::Modified, ChangedSides::Left),
            (
                2,
                2,
                2,
                2,
                2,
                3,
                ConflictKind::Inserted,
                ChangedSides::Right,
            ),
        ],
    ),
    (
        "a_b_c",
        "a_b_c",
        "a_c",
        ComparisonPolicy::Default,
        &[(1, 2, 1, 2, 1, 1, ConflictKind::Deleted, ChangedSides::Right)],
    ),
    (
        "",
        "",
        "x",
        ComparisonPolicy::Default,
        &[(
            0,
            1,
            0,
            1,
            0,
            1,
            ConflictKind::Modified,
            ChangedSides::Right,
        )],
    ),
    (
        "x",
        "",
        "x",
        ComparisonPolicy::Default,
        &[(0, 1, 0, 1, 0, 1, ConflictKind::Modified, ChangedSides::Both)],
    ),
    (
        "x",
        "",
        "y",
        ComparisonPolicy::Default,
        &[(0, 1, 0, 1, 0, 1, ConflictKind::Conflict, ChangedSides::Both)],
    ),
];

#[test]
fn merge_lines_reproduces_golden_three_way_vectors() -> Result<(), ComparisonError> {
    for (left, base, right, policy, expected) in THREE_WAY_GOLDEN_VECTORS {
        let left = with_newlines(left);
        let base = with_newlines(base);
        let right = with_newlines(right);
        let ranges = three_way_ranges(&left, &base, &right, policy)?;
        let left_texts = LineTexts::new(&left);
        let base_texts = LineTexts::new(&base);
        let right_texts = LineTexts::new(&right);
        let actual: Vec<_> = ranges
            .iter()
            .map(|range| {
                let merge_type =
                    merge_type_of(range, [&left_texts, &base_texts, &right_texts], policy);
                (
                    range.left.start,
                    range.left.end,
                    range.base.start,
                    range.base.end,
                    range.right.start,
                    range.right.end,
                    merge_type.kind,
                    changed_sides(&merge_type),
                )
            })
            .collect();
        assert_eq!(
            actual.as_slice(),
            expected,
            "{left:?} {base:?} {right:?} {policy:?}"
        );
    }
    Ok(())
}

#[test]
fn word_merge_reports_the_conflicting_word_in_all_three_texts() -> Result<(), ComparisonError> {
    assert_eq!(
        word_merge(
            "a b c",
            "a x c",
            "a y c",
            ComparisonPolicy::Default,
            &NeverCanceled
        )?,
        [MergeRange::new(2..3, 2..3, 2..3)]
    );
    Ok(())
}

#[test]
fn compare_lines_inner_reports_relative_word_fragments() -> Result<(), ComparisonError> {
    let fragments = compare_lines_inner(
        "a b\nc d\n",
        "a x\nc d\ne\n",
        ComparisonPolicy::Default,
        &NeverCanceled,
    )?;
    assert_eq!(fragments.len(), 2);
    assert_eq!(
        (
            fragments[0].start_offset1,
            fragments[0].end_offset1,
            fragments[0].start_offset2,
            fragments[0].end_offset2
        ),
        (0, 4, 0, 4)
    );
    assert_eq!(
        fragments[0].inner_fragments,
        Some(vec![DiffFragment {
            start_offset1: 2,
            end_offset1: 3,
            start_offset2: 2,
            end_offset2: 3,
        }])
    );
    assert_eq!(
        (
            fragments[1].start_offset1,
            fragments[1].end_offset1,
            fragments[1].start_offset2,
            fragments[1].end_offset2
        ),
        (8, 8, 8, 10)
    );
    assert_eq!(fragments[1].inner_fragments, None);
    Ok(())
}

#[test]
fn compare_lines_with_chars_policy_reports_relative_char_fragments() -> Result<(), ComparisonError>
{
    let fragments = compare_lines_with_inner_policy(
        "a b\nc d\n",
        "a x\nc d\ne\n",
        ComparisonPolicy::Default,
        InnerFragmentsPolicy::Chars,
        &NeverCanceled,
    )?;
    assert_eq!(
        fragments[0].inner_fragments,
        Some(vec![DiffFragment {
            start_offset1: 2,
            end_offset1: 3,
            start_offset2: 2,
            end_offset2: 3,
        }])
    );
    Ok(())
}

#[test]
fn compare_threeside_inner_is_empty_when_ignoring_whitespace_and_all_chunks_match()
-> Result<(), ComparisonError> {
    let inner = compare_threeside_inner(
        [Some("a  b"), Some("a b"), Some("ab")],
        ComparisonPolicy::IgnoreWhitespaces,
        &NeverCanceled,
    )?;
    assert_eq!(
        inner,
        Some(MergeInnerDifferences {
            left: Some(Vec::new()),
            base: Some(Vec::new()),
            right: Some(Vec::new()),
        })
    );
    assert_eq!(
        compare_threeside_inner(
            [None, None, None],
            ComparisonPolicy::Default,
            &NeverCanceled
        )?,
        None
    );
    assert_eq!(
        compare_threeside_inner(
            [Some("a"), None, None],
            ComparisonPolicy::Default,
            &NeverCanceled
        )?,
        None
    );
    Ok(())
}
