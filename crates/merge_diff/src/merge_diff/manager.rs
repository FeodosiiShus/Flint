use crate::by_char::compare_chars as compare_char_ranges;
use crate::by_line::{LineInterner, compare_lines_three_way_ranges, compare_lines_two_way};
use crate::by_word::{
    compare_and_split, compare_words as compare_word_ranges, compare_words_three_way,
};
use crate::range::{DiffFragment, MergeRange};
use crate::text::{ComparisonPolicy, LineOffsets, get_line_contents, is_equal_texts};
use crate::{CancellationChecker, ComparisonError};

const MAX_BAD_LINES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InnerFragmentsPolicy {
    Words,
    Chars,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LineFragment {
    pub start_line1: usize,
    pub end_line1: usize,
    pub start_line2: usize,
    pub end_line2: usize,
    pub start_offset1: usize,
    pub end_offset1: usize,
    pub start_offset2: usize,
    pub end_offset2: usize,
    pub inner_fragments: Option<Vec<DiffFragment>>,
}

fn drop_whole_changed_fragments(
    fragments: Option<Vec<DiffFragment>>,
    length1: usize,
    length2: usize,
) -> Option<Vec<DiffFragment>> {
    let fragments = fragments?;
    if let [fragment] = fragments.as_slice()
        && fragment.start_offset1 == 0
        && fragment.start_offset2 == 0
        && fragment.end_offset1 == length1
        && fragment.end_offset2 == length2
    {
        return None;
    }
    Some(fragments)
}

impl LineFragment {
    fn new(
        lines1: (usize, usize),
        lines2: (usize, usize),
        offsets1: (usize, usize),
        offsets2: (usize, usize),
        inner_fragments: Option<Vec<DiffFragment>>,
    ) -> Self {
        let inner_fragments = drop_whole_changed_fragments(
            inner_fragments,
            offsets1.1 - offsets1.0,
            offsets2.1 - offsets2.0,
        );
        Self {
            start_line1: lines1.0,
            end_line1: lines1.1,
            start_line2: lines2.0,
            end_line2: lines2.1,
            start_offset1: offsets1.0,
            end_offset1: offsets1.1,
            start_offset2: offsets2.0,
            end_offset2: offsets2.1,
            inner_fragments,
        }
    }

    fn with_inner(&self, inner_fragments: Vec<DiffFragment>) -> Self {
        Self::new(
            (self.start_line1, self.end_line1),
            (self.start_line2, self.end_line2),
            (self.start_offset1, self.end_offset1),
            (self.start_offset2, self.end_offset2),
            Some(inner_fragments),
        )
    }
}

fn get_offsets(line_offsets: &LineOffsets, start_index: usize, end_index: usize) -> (usize, usize) {
    if start_index == end_index {
        let offset = if start_index < line_offsets.line_count() {
            line_offsets.line_start(start_index)
        } else {
            line_offsets.line_end(line_offsets.line_count() - 1, true)
        };
        return (offset, offset);
    }
    (
        line_offsets.line_start(start_index),
        line_offsets.line_end(end_index - 1, true),
    )
}

pub fn compare_lines(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    let line_offsets1 = LineOffsets::new(text1);
    let line_offsets2 = LineOffsets::new(text2);
    let mut interner = LineInterner::new(policy);
    let lines1 = interner.lines(get_line_contents(text1, &line_offsets1));
    let lines2 = interner.lines(get_line_contents(text2, &line_offsets2));
    let changes = compare_lines_two_way(&lines1, &lines2, policy, cancel)?;
    Ok(changes
        .changes
        .iter()
        .map(|change| {
            LineFragment::new(
                (change.start1, change.end1),
                (change.start2, change.end2),
                get_offsets(&line_offsets1, change.start1, change.end1),
                get_offsets(&line_offsets2, change.start2, change.end2),
                None,
            )
        })
        .collect())
}

fn create_inner_fragments_all(
    line_fragments: Vec<LineFragment>,
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    fragments_policy: InnerFragmentsPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    let mut result = Vec::new();
    let mut too_big_chunks_count = 0;
    for fragment in line_fragments {
        debug_assert!(fragment.inner_fragments.is_none());
        let try_compute_differences = too_big_chunks_count < MAX_BAD_LINES;
        match create_inner_fragments(
            &fragment,
            text1,
            text2,
            policy,
            fragments_policy,
            try_compute_differences,
            cancel,
        ) {
            Ok(fragments) => result.extend(fragments),
            Err(ComparisonError::DiffTooBig) => {
                result.push(fragment);
                too_big_chunks_count += 1;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

fn create_inner_fragments(
    fragment: &LineFragment,
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    fragments_policy: InnerFragmentsPolicy,
    try_compute_differences: bool,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    let sub_sequence1 = &text1[fragment.start_offset1..fragment.end_offset1];
    let sub_sequence2 = &text2[fragment.start_offset2..fragment.end_offset2];
    if fragment.start_line1 == fragment.end_line1 || fragment.start_line2 == fragment.end_line2 {
        if is_equal_texts(sub_sequence1.as_bytes(), sub_sequence2.as_bytes(), policy) {
            return Ok(vec![fragment.with_inner(Vec::new())]);
        }
        return Ok(vec![fragment.clone()]);
    }
    if !try_compute_differences {
        return Ok(vec![fragment.clone()]);
    }
    match fragments_policy {
        InnerFragmentsPolicy::Words => {
            create_inner_word_fragments(fragment, sub_sequence1, sub_sequence2, policy, cancel)
        }
        InnerFragmentsPolicy::Chars => {
            let inner = compare_char_ranges(sub_sequence1, sub_sequence2, policy, cancel)?;
            Ok(vec![fragment.with_inner(
                inner.into_iter().map(DiffFragment::from).collect(),
            )])
        }
    }
}

fn create_inner_word_fragments(
    fragment: &LineFragment,
    sub_sequence1: &str,
    sub_sequence2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    let line_blocks = compare_and_split(sub_sequence1, sub_sequence2, policy, cancel)?;
    debug_assert!(!line_blocks.is_empty());
    if line_blocks.is_empty() {
        return Ok(vec![fragment.clone()]);
    }
    let block_count = line_blocks.len();
    let mut current_start_line1 = fragment.start_line1;
    let mut current_start_line2 = fragment.start_line2;
    let mut chunks = Vec::with_capacity(block_count);
    for (index, block) in line_blocks.into_iter().enumerate() {
        let is_last = index == block_count - 1;
        let current_end_line1 = if is_last {
            fragment.end_line1
        } else {
            current_start_line1 + block.newlines1
        };
        let current_end_line2 = if is_last {
            fragment.end_line2
        } else {
            current_start_line2 + block.newlines2
        };
        chunks.push(LineFragment::new(
            (current_start_line1, current_end_line1),
            (current_start_line2, current_end_line2),
            (
                block.offsets.start1 + fragment.start_offset1,
                block.offsets.end1 + fragment.start_offset1,
            ),
            (
                block.offsets.start2 + fragment.start_offset2,
                block.offsets.end2 + fragment.start_offset2,
            ),
            Some(
                block
                    .fragments
                    .into_iter()
                    .map(DiffFragment::from)
                    .collect(),
            ),
        ));
        current_start_line1 = current_end_line1;
        current_start_line2 = current_end_line2;
    }
    Ok(chunks)
}

pub fn compare_lines_inner(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    compare_lines_with_inner_policy(text1, text2, policy, InnerFragmentsPolicy::Words, cancel)
}

pub fn compare_lines_with_inner_policy(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    fragments_policy: InnerFragmentsPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineFragment>, ComparisonError> {
    let line_fragments = compare_lines(text1, text2, policy, cancel)?;
    create_inner_fragments_all(
        line_fragments,
        text1,
        text2,
        policy,
        fragments_policy,
        cancel,
    )
}

pub fn compare_words(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<DiffFragment>, ComparisonError> {
    Ok(compare_word_ranges(text1, text2, policy, cancel)?
        .into_iter()
        .map(DiffFragment::from)
        .collect())
}

pub fn compare_chars(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<DiffFragment>, ComparisonError> {
    Ok(compare_char_ranges(text1, text2, policy, cancel)?
        .into_iter()
        .map(DiffFragment::from)
        .collect())
}

fn three_way_line_ranges(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
    keep_ignored_changes: bool,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    let left_offsets = LineOffsets::new(left);
    let base_offsets = LineOffsets::new(base);
    let right_offsets = LineOffsets::new(right);
    let mut interner = LineInterner::new(policy);
    let left_lines = interner.lines(get_line_contents(left, &left_offsets));
    let base_lines = interner.lines(get_line_contents(base, &base_offsets));
    let right_lines = interner.lines(get_line_contents(right, &right_offsets));
    compare_lines_three_way_ranges(
        &left_lines,
        &base_lines,
        &right_lines,
        policy,
        keep_ignored_changes,
        cancel,
    )
}

pub fn compare_lines_three_way(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    three_way_line_ranges(left, base, right, policy, false, cancel)
}

pub fn merge_lines(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    three_way_line_ranges(left, base, right, policy, true, cancel)
}

pub fn word_merge(
    left: &str,
    base: &str,
    right: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    compare_words_three_way(left, base, right, policy, cancel)
}
