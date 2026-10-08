use crate::by_word::InlineChunk;
use crate::diff::FairDiff;
use crate::range::DiffRange;
use crate::text::is_space_enter_or_tab;

pub(crate) const UNIMPORTANT_LINE_CHAR_COUNT: usize = 3;

fn expand_forward(
    data1: &[u32],
    data2: &[u32],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> usize {
    let old_start1 = start1;
    let mut start1 = start1;
    let mut start2 = start2;
    while start1 < end1 && start2 < end2 {
        if data1[start1] != data2[start2] {
            break;
        }
        start1 += 1;
        start2 += 1;
    }
    start1 - old_start1
}

fn expand_backward(
    data1: &[u32],
    data2: &[u32],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> usize {
    let old_end1 = end1;
    let mut end1 = end1;
    let mut end2 = end2;
    while start1 < end1 && start2 < end2 {
        if data1[end1 - 1] != data2[end2 - 1] {
            break;
        }
        end1 -= 1;
        end2 -= 1;
    }
    old_end1 - end1
}

fn shifted(value: usize, shift: i64) -> usize {
    (value as i64 + shift) as usize
}

fn process_last_ranges(
    data1: &[u32],
    data2: &[u32],
    ranges: &mut Vec<DiffRange>,
    get_shift: &impl Fn(bool, usize, usize, &DiffRange, &DiffRange) -> i64,
) {
    loop {
        if ranges.len() < 2 {
            return;
        }
        let range1 = ranges[ranges.len() - 2];
        let range2 = ranges[ranges.len() - 1];
        if range1.end1 != range2.start1 && range1.end2 != range2.start2 {
            return;
        }
        let count1 = range1.end1 - range1.start1;
        let count2 = range2.end1 - range2.start1;
        let equal_forward = expand_forward(
            data1,
            data2,
            range1.end1,
            range1.end2,
            range1.end1 + count2,
            range1.end2 + count2,
        );
        let equal_backward = expand_backward(
            data1,
            data2,
            range2.start1 - count1,
            range2.start2 - count1,
            range2.start1,
            range2.start2,
        );
        if equal_forward == 0 && equal_backward == 0 {
            return;
        }
        if equal_forward == count2 {
            ranges.pop();
            ranges.pop();
            ranges.push(DiffRange::new(
                range1.start1,
                range1.end1 + count2,
                range1.start2,
                range1.end2 + count2,
            ));
            continue;
        }
        if equal_backward == count1 {
            ranges.pop();
            ranges.pop();
            ranges.push(DiffRange::new(
                range2.start1 - count1,
                range2.end1,
                range2.start2 - count1,
                range2.end2,
            ));
            continue;
        }
        let touch_left = range1.end1 == range2.start1;
        let shift = get_shift(touch_left, equal_forward, equal_backward, &range1, &range2);
        if shift != 0 {
            ranges.pop();
            ranges.pop();
            ranges.push(DiffRange::new(
                range1.start1,
                shifted(range1.end1, shift),
                range1.start2,
                shifted(range1.end2, shift),
            ));
            ranges.push(DiffRange::new(
                shifted(range2.start1, shift),
                range2.end1,
                shifted(range2.start2, shift),
                range2.end2,
            ));
        }
        return;
    }
}

pub(crate) fn optimize_chunks(
    data1: &[u32],
    data2: &[u32],
    iterable: &FairDiff,
    get_shift: impl Fn(bool, usize, usize, &DiffRange, &DiffRange) -> i64,
) -> FairDiff {
    let mut ranges: Vec<DiffRange> = Vec::new();
    for range in iterable.unchanged() {
        ranges.push(range);
        process_last_ranges(data1, data2, &mut ranges, &get_shift);
    }
    FairDiff::from_unchanged(&ranges, data1.len(), data2.len())
}

fn non_space_at(non_space: &[usize], index: i64) -> usize {
    usize::try_from(index)
        .ok()
        .and_then(|position| non_space.get(position).copied())
        .unwrap_or(usize::MAX)
}

fn find_next_unimportant_line(
    non_space: &[usize],
    offset: usize,
    count: usize,
    threshold: usize,
) -> i64 {
    for index in 0..count {
        if non_space_at(non_space, (offset + index) as i64) <= threshold {
            return index as i64;
        }
    }
    -1
}

fn find_prev_unimportant_line(
    non_space: &[usize],
    offset: i64,
    count: usize,
    threshold: usize,
) -> i64 {
    for index in 0..count {
        if non_space_at(non_space, offset - index as i64) <= threshold {
            return index as i64;
        }
    }
    -1
}

fn shift_from_boundaries(shift_forward: i64, shift_backward: i64) -> Option<i64> {
    if shift_forward == -1 && shift_backward == -1 {
        return None;
    }
    if shift_forward == 0 || shift_backward == 0 {
        return Some(0);
    }
    Some(if shift_forward != -1 {
        shift_forward
    } else {
        -shift_backward
    })
}

pub(crate) fn optimize_line_chunks(
    keys1: &[u32],
    keys2: &[u32],
    non_space1: &[usize],
    non_space2: &[usize],
    iterable: &FairDiff,
) -> FairDiff {
    let unchanged_boundary_shift = |touch_left: bool,
                                    equal_forward: usize,
                                    equal_backward: usize,
                                    range2: &DiffRange,
                                    threshold: usize| {
        let touch_lines = if touch_left { non_space1 } else { non_space2 };
        let touch_start = if touch_left {
            range2.start1
        } else {
            range2.start2
        };
        let shift_forward =
            find_next_unimportant_line(touch_lines, touch_start, equal_forward + 1, threshold);
        let shift_backward = find_prev_unimportant_line(
            touch_lines,
            touch_start as i64 - 1,
            equal_backward + 1,
            threshold,
        );
        shift_from_boundaries(shift_forward, shift_backward)
    };
    let changed_boundary_shift = |touch_left: bool,
                                  equal_forward: usize,
                                  equal_backward: usize,
                                  range1: &DiffRange,
                                  range2: &DiffRange,
                                  threshold: usize| {
        let non_touch_lines = if touch_left { non_space2 } else { non_space1 };
        let change_start = if touch_left { range1.end2 } else { range1.end1 };
        let change_end = if touch_left {
            range2.start2
        } else {
            range2.start1
        };
        let shift_forward =
            find_next_unimportant_line(non_touch_lines, change_start, equal_forward + 1, threshold);
        let shift_backward = find_prev_unimportant_line(
            non_touch_lines,
            change_end as i64 - 1,
            equal_backward + 1,
            threshold,
        );
        shift_from_boundaries(shift_forward, shift_backward)
    };
    let get_shift = |touch_left: bool,
                     equal_forward: usize,
                     equal_backward: usize,
                     range1: &DiffRange,
                     range2: &DiffRange| {
        let attempts = [
            unchanged_boundary_shift(touch_left, equal_forward, equal_backward, range2, 0),
            changed_boundary_shift(touch_left, equal_forward, equal_backward, range1, range2, 0),
            unchanged_boundary_shift(
                touch_left,
                equal_forward,
                equal_backward,
                range2,
                UNIMPORTANT_LINE_CHAR_COUNT,
            ),
            changed_boundary_shift(
                touch_left,
                equal_forward,
                equal_backward,
                range1,
                range2,
                UNIMPORTANT_LINE_CHAR_COUNT,
            ),
        ];
        attempts.into_iter().flatten().next().unwrap_or(0)
    };
    optimize_chunks(keys1, keys2, iterable, get_shift)
}

fn is_separated_with_whitespace(text: &[u8], word1: &InlineChunk, word2: &InlineChunk) -> bool {
    if word1.is_newline || word2.is_newline {
        return true;
    }
    (word1.end..word2.start).any(|index| is_space_enter_or_tab(text[index]))
}

fn is_separated_at(text: &[u8], words: &[InlineChunk], index1: i64, index2: i64) -> bool {
    let word1 = usize::try_from(index1)
        .ok()
        .and_then(|index| words.get(index));
    let word2 = usize::try_from(index2)
        .ok()
        .and_then(|index| words.get(index));
    match (word1, word2) {
        (Some(word1), Some(word2)) => is_separated_with_whitespace(text, word1, word2),
        _ => false,
    }
}

fn find_sequence_edge_shift(
    text: &[u8],
    words: &[InlineChunk],
    offset: i64,
    count: usize,
    left_to_right: bool,
) -> i64 {
    for index in 0..count as i64 {
        let separated = if left_to_right {
            is_separated_at(text, words, offset + index, offset + index + 1)
        } else {
            is_separated_at(text, words, offset - index - 1, offset - index)
        };
        if separated {
            return index + 1;
        }
    }
    -1
}

pub(crate) fn optimize_word_chunks(
    text1: &[u8],
    text2: &[u8],
    words1: &[InlineChunk],
    words2: &[InlineChunk],
    ids1: &[u32],
    ids2: &[u32],
    iterable: &FairDiff,
) -> FairDiff {
    let get_shift = |touch_left: bool,
                     equal_forward: usize,
                     equal_backward: usize,
                     _range1: &DiffRange,
                     range2: &DiffRange| {
        let touch_words = if touch_left { words1 } else { words2 };
        let touch_text = if touch_left { text1 } else { text2 };
        let touch_start = (if touch_left {
            range2.start1
        } else {
            range2.start2
        }) as i64;
        if is_separated_at(touch_text, touch_words, touch_start - 1, touch_start) {
            return 0;
        }
        let left_shift =
            find_sequence_edge_shift(touch_text, touch_words, touch_start, equal_forward, true);
        if left_shift > 0 {
            return left_shift;
        }
        let right_shift = find_sequence_edge_shift(
            touch_text,
            touch_words,
            touch_start - 1,
            equal_backward,
            false,
        );
        if right_shift > 0 {
            return -right_shift;
        }
        0
    };
    optimize_chunks(ids1, ids2, iterable, get_shift)
}
