use crate::range::{MergeRange, ThreeSide};
use crate::resolve::try_resolve_conflict;
use crate::text::{ComparisonPolicy, LineOffsets, is_equal_texts};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictKind {
    Inserted,
    Deleted,
    Modified,
    Conflict,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResolutionStrategy {
    Default,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MergeConflictType {
    pub kind: ConflictKind,
    pub left_change: bool,
    pub right_change: bool,
    pub resolution: Option<ResolutionStrategy>,
}

impl MergeConflictType {
    pub fn new(kind: ConflictKind, left_change: bool, right_change: bool) -> Self {
        Self {
            kind,
            left_change,
            right_change,
            resolution: Some(ResolutionStrategy::Default),
        }
    }

    pub fn with_resolution(
        kind: ConflictKind,
        left_change: bool,
        right_change: bool,
        resolution: Option<ResolutionStrategy>,
    ) -> Self {
        Self {
            kind,
            left_change,
            right_change,
            resolution,
        }
    }

    pub fn can_be_resolved(&self) -> bool {
        self.resolution.is_some()
    }

    pub fn is_change(&self, side: ThreeSide) -> bool {
        match side {
            ThreeSide::Left => self.left_change,
            ThreeSide::Base => true,
            ThreeSide::Right => self.right_change,
        }
    }

    pub fn is_conflict(&self) -> bool {
        self.kind == ConflictKind::Conflict
    }
}

pub(crate) fn get_merge_type(
    emptiness: &dyn Fn(ThreeSide) -> bool,
    equality: &dyn Fn(ThreeSide, ThreeSide) -> bool,
    true_equality: Option<&dyn Fn(ThreeSide, ThreeSide) -> bool>,
    conflict_resolver: &dyn Fn() -> bool,
) -> Option<MergeConflictType> {
    let is_left_empty = emptiness(ThreeSide::Left);
    let is_base_empty = emptiness(ThreeSide::Base);
    let is_right_empty = emptiness(ThreeSide::Right);
    if is_left_empty && is_base_empty && is_right_empty {
        return None;
    }
    if is_base_empty {
        if is_left_empty {
            return Some(MergeConflictType::new(ConflictKind::Inserted, false, true));
        }
        if is_right_empty {
            return Some(MergeConflictType::new(ConflictKind::Inserted, true, false));
        }
        if equality(ThreeSide::Left, ThreeSide::Right) {
            return Some(MergeConflictType::new(ConflictKind::Inserted, true, true));
        }
        return Some(MergeConflictType::with_resolution(
            ConflictKind::Conflict,
            true,
            true,
            None,
        ));
    }
    if is_left_empty && is_right_empty {
        return Some(MergeConflictType::new(ConflictKind::Deleted, true, true));
    }
    let unchanged_left = equality(ThreeSide::Base, ThreeSide::Left);
    let unchanged_right = equality(ThreeSide::Base, ThreeSide::Right);
    if unchanged_left && unchanged_right {
        let true_equality = true_equality?;
        let true_unchanged_left = true_equality(ThreeSide::Base, ThreeSide::Left);
        let true_unchanged_right = true_equality(ThreeSide::Base, ThreeSide::Right);
        if true_unchanged_left && true_unchanged_right {
            return None;
        }
        return Some(MergeConflictType::new(
            ConflictKind::Modified,
            !true_unchanged_left,
            !true_unchanged_right,
        ));
    }
    if unchanged_left {
        let kind = if is_right_empty {
            ConflictKind::Deleted
        } else {
            ConflictKind::Modified
        };
        return Some(MergeConflictType::new(kind, false, true));
    }
    if unchanged_right {
        let kind = if is_left_empty {
            ConflictKind::Deleted
        } else {
            ConflictKind::Modified
        };
        return Some(MergeConflictType::new(kind, true, false));
    }
    if equality(ThreeSide::Left, ThreeSide::Right) {
        return Some(MergeConflictType::new(ConflictKind::Modified, true, true));
    }
    let can_be_resolved = !is_left_empty && !is_right_empty && conflict_resolver();
    Some(MergeConflictType::with_resolution(
        ConflictKind::Conflict,
        true,
        true,
        can_be_resolved.then_some(ResolutionStrategy::Text),
    ))
}

pub(crate) fn get_word_merge_type(
    range: &MergeRange,
    texts: [&[u8]; 3],
    policy: ComparisonPolicy,
) -> Option<MergeConflictType> {
    get_merge_type(
        &|side: ThreeSide| range.side(side).is_empty(),
        &|side1: ThreeSide, side2: ThreeSide| {
            compare_word_merge_contents(range, texts, policy, side1, side2)
        },
        None,
        &|| false,
    )
}

pub(crate) fn compare_word_merge_contents(
    range: &MergeRange,
    texts: [&[u8]; 3],
    policy: ComparisonPolicy,
    side1: ThreeSide,
    side2: ThreeSide,
) -> bool {
    let content1 = &texts[side1.index()][range.side(side1)];
    let content2 = &texts[side2.index()][range.side(side2)];
    is_equal_texts(content1, content2, policy)
}

#[derive(Clone, Debug)]
pub struct LineTexts<'a> {
    text: &'a str,
    offsets: LineOffsets,
}

impl<'a> LineTexts<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            offsets: LineOffsets::new(text),
        }
    }

    pub fn text(&self) -> &'a str {
        self.text
    }

    pub fn offsets(&self) -> &LineOffsets {
        &self.offsets
    }

    pub fn lines_content(&self, first_line: usize, last_line: usize) -> &'a str {
        let (start, end) = self.offsets.lines_range(first_line, last_line, false);
        &self.text[start..end]
    }
}

fn compare_line_merge_contents(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
    side1: ThreeSide,
    side2: ThreeSide,
) -> bool {
    let lines1 = range.side(side1);
    let lines2 = range.side(side2);
    if lines1.len() != lines2.len() {
        return false;
    }
    let texts1 = texts[side1.index()];
    let texts2 = texts[side2.index()];
    lines1.zip(lines2).all(|(line1, line2)| {
        is_equal_texts(
            texts1.lines_content(line1, line1 + 1).as_bytes(),
            texts2.lines_content(line2, line2 + 1).as_bytes(),
            policy,
        )
    })
}

fn can_resolve_line_conflict(range: &MergeRange, texts: [&LineTexts; 3]) -> bool {
    let content = |side: ThreeSide| {
        let lines = range.side(side);
        texts[side.index()].lines_content(lines.start, lines.end)
    };
    try_resolve_conflict(
        content(ThreeSide::Left),
        content(ThreeSide::Base),
        content(ThreeSide::Right),
    )
    .is_some()
}

pub fn line_three_way_diff_type(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> Option<MergeConflictType> {
    get_merge_type(
        &|side: ThreeSide| range.side(side).is_empty(),
        &|side1: ThreeSide, side2: ThreeSide| {
            compare_line_merge_contents(range, texts, policy, side1, side2)
        },
        None,
        &|| can_resolve_line_conflict(range, texts),
    )
}

pub fn line_merge_type(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> Option<MergeConflictType> {
    get_merge_type(
        &|side: ThreeSide| range.side(side).is_empty(),
        &|side1: ThreeSide, side2: ThreeSide| {
            compare_line_merge_contents(range, texts, policy, side1, side2)
        },
        Some(&|side1: ThreeSide, side2: ThreeSide| {
            compare_line_merge_contents(range, texts, ComparisonPolicy::Default, side1, side2)
        }),
        &|| can_resolve_line_conflict(range, texts),
    )
}
