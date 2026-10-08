use std::ops::Range;

use crate::by_word::compare_words;
use crate::merge_type::{LineTexts, MergeConflictType};
use crate::range::{MergeRange, ThreeSide};
use crate::text::{ComparisonPolicy, is_equal_texts};
use crate::{CancellationChecker, ComparisonError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MergeInnerDifferences {
    pub left: Option<Vec<Range<usize>>>,
    pub base: Option<Vec<Range<usize>>>,
    pub right: Option<Vec<Range<usize>>>,
}

pub fn get_inner_chunks<'a>(
    range: &MergeRange,
    texts: [&LineTexts<'a>; 3],
    merge_type: &MergeConflictType,
) -> [Option<&'a str>; 3] {
    ThreeSide::ALL.map(|side| {
        let lines = range.side(side);
        if !merge_type.is_change(side) || lines.start == lines.end {
            None
        } else {
            Some(texts[side.index()].lines_content(lines.start, lines.end))
        }
    })
}

fn is_chunks_equals(chunk1: Option<&str>, chunk2: Option<&str>, policy: ComparisonPolicy) -> bool {
    is_equal_texts(
        chunk1.unwrap_or_default().as_bytes(),
        chunk2.unwrap_or_default().as_bytes(),
        policy,
    )
}

pub fn compare_threeside_inner(
    chunks: [Option<&str>; 3],
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Option<MergeInnerDifferences>, ComparisonError> {
    let [left_chunk, base_chunk, right_chunk] = chunks;
    if left_chunk.is_none() && base_chunk.is_none() && right_chunk.is_none() {
        return Ok(None);
    }
    if policy == ComparisonPolicy::IgnoreWhitespaces
        && is_chunks_equals(left_chunk, base_chunk, policy)
        && is_chunks_equals(left_chunk, right_chunk, policy)
    {
        return Ok(Some(MergeInnerDifferences {
            left: Some(Vec::new()),
            base: Some(Vec::new()),
            right: Some(Vec::new()),
        }));
    }
    let missing_count = chunks.iter().filter(|chunk| chunk.is_none()).count();
    if missing_count >= 2 {
        return Ok(None);
    }
    if let (Some(left), Some(base), Some(right)) = (left_chunk, base_chunk, right_chunk) {
        let fragments1 = compare_words(base, left, policy, cancel)?;
        let fragments2 = compare_words(base, right, policy, cancel)?;
        let mut left_ranges = Vec::new();
        let mut base_ranges = Vec::new();
        let mut right_ranges = Vec::new();
        for fragment in &fragments1 {
            base_ranges.push(fragment.start1..fragment.end1);
            left_ranges.push(fragment.start2..fragment.end2);
        }
        for fragment in &fragments2 {
            base_ranges.push(fragment.start1..fragment.end1);
            right_ranges.push(fragment.start2..fragment.end2);
        }
        return Ok(Some(MergeInnerDifferences {
            left: Some(left_ranges),
            base: Some(base_ranges),
            right: Some(right_ranges),
        }));
    }
    let first_side = if left_chunk.is_some() {
        ThreeSide::Left
    } else {
        ThreeSide::Base
    };
    let second_side = if right_chunk.is_some() {
        ThreeSide::Right
    } else {
        ThreeSide::Base
    };
    let (Some(chunk1), Some(chunk2)) = (chunks[first_side.index()], chunks[second_side.index()])
    else {
        return Ok(None);
    };
    let word_conflicts = compare_words(chunk1, chunk2, policy, cancel)?;
    let ranges_for = |side: ThreeSide| {
        if side == first_side {
            Some(
                word_conflicts
                    .iter()
                    .map(|fragment| fragment.start1..fragment.end1)
                    .collect::<Vec<_>>(),
            )
        } else if side == second_side {
            Some(
                word_conflicts
                    .iter()
                    .map(|fragment| fragment.start2..fragment.end2)
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        }
    };
    Ok(Some(MergeInnerDifferences {
        left: ranges_for(ThreeSide::Left),
        base: ranges_for(ThreeSide::Base),
        right: ranges_for(ThreeSide::Right),
    }))
}
