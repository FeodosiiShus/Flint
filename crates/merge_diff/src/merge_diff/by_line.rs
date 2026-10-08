use std::borrow::Cow;
use std::collections::HashMap;

use crate::chunk_optimizer::{UNIMPORTANT_LINE_CHAR_COUNT, optimize_line_chunks};
use crate::diff::{ChangeBuilder, FairDiff, diff, expand};
use crate::merge_builder::{TrueEquality, build_merge};
use crate::range::{DiffRange, MergeRange};
use crate::text::{ComparisonPolicy, remove_whitespace, strip_whitespace_str};
use crate::{CancellationChecker, ComparisonError};

const MAX_ALIGNMENT_SIZE: usize = 10;

pub(crate) struct Lines<'a> {
    pub(crate) contents: Vec<&'a str>,
    pub(crate) keys: Vec<u32>,
    pub(crate) ignore_whitespace_keys: Vec<u32>,
    pub(crate) non_space_chars: Vec<usize>,
}

pub(crate) struct LineInterner<'a> {
    policy: ComparisonPolicy,
    keys: HashMap<Cow<'a, str>, u32>,
    ignore_whitespace_keys: HashMap<Cow<'a, str>, u32>,
}

fn intern<'a>(map: &mut HashMap<Cow<'a, str>, u32>, key: Cow<'a, str>) -> u32 {
    let next = map.len() as u32;
    *map.entry(key).or_insert(next)
}

fn count_non_space_chars(content: &str) -> usize {
    content
        .chars()
        .filter(|character| !matches!(character, ' ' | '\t' | '\n'))
        .map(char::len_utf16)
        .sum()
}

impl<'a> LineInterner<'a> {
    pub(crate) fn new(policy: ComparisonPolicy) -> Self {
        Self {
            policy,
            keys: HashMap::new(),
            ignore_whitespace_keys: HashMap::new(),
        }
    }

    pub(crate) fn lines(&mut self, contents: Vec<&'a str>) -> Lines<'a> {
        let mut keys = Vec::with_capacity(contents.len());
        let mut ignore_whitespace_keys = Vec::with_capacity(contents.len());
        let mut non_space_chars = Vec::with_capacity(contents.len());
        for &content in &contents {
            let ignore_whitespace_key =
                intern(&mut self.ignore_whitespace_keys, remove_whitespace(content));
            let key = match self.policy {
                ComparisonPolicy::Default => intern(&mut self.keys, Cow::Borrowed(content)),
                ComparisonPolicy::TrimWhitespaces => {
                    intern(&mut self.keys, Cow::Borrowed(strip_whitespace_str(content)))
                }
                ComparisonPolicy::IgnoreWhitespaces => ignore_whitespace_key,
            };
            keys.push(key);
            ignore_whitespace_keys.push(ignore_whitespace_key);
            non_space_chars.push(count_non_space_chars(content));
        }
        Lines {
            contents,
            keys,
            ignore_whitespace_keys,
            non_space_chars,
        }
    }
}

fn match_gap(
    builder: &mut ChangeBuilder,
    lines1: &[u32],
    lines2: &[u32],
    range: &DiffRange,
    cancel: &dyn CancellationChecker,
) -> Result<(), ComparisonError> {
    let expanded = expand(
        lines1,
        lines2,
        range.start1,
        range.start2,
        range.end1,
        range.end2,
    );
    let inner = diff(
        &lines1[expanded.start1..expanded.end1],
        &lines2[expanded.start2..expanded.end2],
        cancel,
    )?;
    builder.mark_equal_ranges(range.start1, range.start2, expanded.start1, expanded.start2);
    for unchanged in inner.unchanged() {
        builder.mark_equal(
            expanded.start1 + unchanged.start1,
            expanded.start2 + unchanged.start2,
            unchanged.end1 - unchanged.start1,
        );
    }
    builder.mark_equal_ranges(expanded.end1, expanded.end2, range.end1, range.end2);
    Ok(())
}

fn smart_correct(
    indices1: &[usize],
    indices2: &[usize],
    lines1: &[u32],
    lines2: &[u32],
    changes: &FairDiff,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let mut builder = ChangeBuilder::new(lines1.len(), lines2.len());
    let mut last1 = 0;
    let mut last2 = 0;
    for unchanged in changes.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            let big1 = indices1[unchanged.start1 + offset];
            let big2 = indices2[unchanged.start2 + offset];
            match_gap(
                &mut builder,
                lines1,
                lines2,
                &DiffRange::new(last1, big1, last2, big2),
                cancel,
            )?;
            builder.mark_equal_ranges(big1, big2, big1 + 1, big2 + 1);
            last1 = big1 + 1;
            last2 = big2 + 1;
        }
    }
    match_gap(
        &mut builder,
        lines1,
        lines2,
        &DiffRange::new(last1, lines1.len(), last2, lines2.len()),
        cancel,
    )?;
    Ok(builder.finish())
}

fn big_line_indices(non_space_chars: &[usize]) -> Vec<usize> {
    non_space_chars
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > UNIMPORTANT_LINE_CHAR_COUNT)
        .map(|(index, _)| index)
        .collect()
}

fn compare_smart(
    keys1: &[u32],
    non_space_chars1: &[usize],
    keys2: &[u32],
    non_space_chars2: &[usize],
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let big_indices1 = big_line_indices(non_space_chars1);
    let big_indices2 = big_line_indices(non_space_chars2);
    let big_keys1: Vec<u32> = big_indices1.iter().map(|&index| keys1[index]).collect();
    let big_keys2: Vec<u32> = big_indices2.iter().map(|&index| keys2[index]).collect();
    let changes = diff(&big_keys1, &big_keys2, cancel)?;
    smart_correct(&big_indices1, &big_indices2, keys1, keys2, &changes, cancel)
}

fn combinations_in_lexicographic_order(universe: usize, size: usize) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    if size > universe {
        return result;
    }
    let mut indices: Vec<usize> = (0..size).collect();
    result.push(indices.clone());
    loop {
        let Some(pivot) = (0..size)
            .rev()
            .find(|&position| indices[position] != position + universe - size)
        else {
            return result;
        };
        indices[pivot] += 1;
        for position in pivot + 1..size {
            indices[position] = indices[position - 1] + 1;
        }
        result.push(indices.clone());
    }
}

struct SecondStepCorrector<'a> {
    lines1: &'a Lines<'a>,
    lines2: &'a Lines<'a>,
    builder: ChangeBuilder<'a>,
    sample: Option<u32>,
    last1: usize,
    last2: usize,
}

impl<'a> SecondStepCorrector<'a> {
    fn flush(&mut self, end1: usize, end2: usize) {
        let Some(sample) = self.sample else {
            return;
        };
        let start1 = self.last1.max(self.builder.index1());
        let start2 = self.last2.max(self.builder.index2());
        let mut matching1 = Vec::new();
        let mut matching2 = Vec::new();
        for index in start1..end1 {
            if self.lines1.ignore_whitespace_keys[index] == sample {
                matching1.push(index);
                self.last1 = index + 1;
            }
        }
        for index in start2..end2 {
            if self.lines2.ignore_whitespace_keys[index] == sample {
                matching2.push(index);
                self.last2 = index + 1;
            }
        }
        debug_assert!(!matching1.is_empty() && !matching2.is_empty());
        self.align(&matching1, &matching2);
        self.sample = None;
    }

    fn best_alignment(
        subset1: &[usize],
        subset2: &[usize],
        keys1: &[u32],
        keys2: &[u32],
    ) -> Vec<usize> {
        let size = subset1.len();
        let mut best_combination: Vec<usize> = (0..size).collect();
        let mut best_weight = 0;
        for combination in combinations_in_lexicographic_order(subset2.len(), size) {
            let weight = (0..size)
                .filter(|&index| keys1[subset1[index]] == keys2[subset2[combination[index]]])
                .count();
            if weight > best_weight {
                best_weight = weight;
                best_combination = combination;
            }
        }
        best_combination
    }

    fn mark_if_equal(&mut self, index1: usize, index2: usize) {
        if self.lines1.keys[index1] == self.lines2.keys[index2] {
            self.builder.mark_equal(index1, index2, 1);
        }
    }

    fn align(&mut self, subset1: &[usize], subset2: &[usize]) {
        let size = subset1.len().max(subset2.len());
        if size > MAX_ALIGNMENT_SIZE || subset1.len() == subset2.len() {
            for index in 0..subset1.len().min(subset2.len()) {
                self.mark_if_equal(subset1[index], subset2[index]);
            }
            return;
        }
        if subset1.len() < subset2.len() {
            let mapping =
                Self::best_alignment(subset1, subset2, &self.lines1.keys, &self.lines2.keys);
            for index in 0..subset1.len() {
                self.mark_if_equal(subset1[index], subset2[mapping[index]]);
            }
        } else {
            let mapping =
                Self::best_alignment(subset2, subset1, &self.lines2.keys, &self.lines1.keys);
            for index in 0..subset2.len() {
                self.mark_if_equal(subset1[mapping[index]], subset2[index]);
            }
        }
    }
}

pub(crate) fn correct_second_step<'a>(
    lines1: &'a Lines<'a>,
    lines2: &'a Lines<'a>,
    changes: &FairDiff,
) -> FairDiff {
    let mut corrector = SecondStepCorrector {
        lines1,
        lines2,
        builder: ChangeBuilder::expanding(
            lines1.keys.len(),
            lines2.keys.len(),
            &lines1.keys,
            &lines2.keys,
        ),
        sample: None,
        last1: 0,
        last2: 0,
    };
    for unchanged in changes.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            let index1 = unchanged.start1 + offset;
            let index2 = unchanged.start2 + offset;
            if corrector.sample.is_none()
                || corrector.sample != Some(lines1.ignore_whitespace_keys[index1])
            {
                corrector.flush(index1, index2);
                if lines1.keys[index1] == lines2.keys[index2] {
                    corrector.builder.mark_equal(index1, index2, 1);
                } else {
                    corrector.sample = Some(lines1.ignore_whitespace_keys[index1]);
                }
            }
        }
    }
    corrector.flush(changes.length1, changes.length2);
    corrector.builder.finish()
}

fn expand_ranges(keys1: &[u32], keys2: &[u32], changes: &FairDiff) -> FairDiff {
    let mut expanded_changes = Vec::new();
    for change in &changes.changes {
        let expanded = expand(
            keys1,
            keys2,
            change.start1,
            change.start2,
            change.end1,
            change.end2,
        );
        if !expanded.is_empty() {
            expanded_changes.push(expanded);
        }
    }
    FairDiff::new(expanded_changes, keys1.len(), keys2.len())
}

pub(crate) fn compare_lines_two_way(
    lines1: &Lines,
    lines2: &Lines,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let changes = compare_smart(
        &lines1.ignore_whitespace_keys,
        &lines1.non_space_chars,
        &lines2.ignore_whitespace_keys,
        &lines2.non_space_chars,
        cancel,
    )?;
    let changes = optimize_line_chunks(
        &lines1.keys,
        &lines2.keys,
        &lines1.non_space_chars,
        &lines2.non_space_chars,
        &changes,
    );
    if policy == ComparisonPolicy::IgnoreWhitespaces {
        return Ok(expand_ranges(&lines1.keys, &lines2.keys, &changes));
    }
    Ok(correct_second_step(lines1, lines2, &changes))
}

fn compare_against_base<'a>(
    base: &'a Lines<'a>,
    other: &'a Lines<'a>,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let changes = compare_smart(
        &base.ignore_whitespace_keys,
        &base.non_space_chars,
        &other.ignore_whitespace_keys,
        &other.non_space_chars,
        cancel,
    )?;
    let changes = optimize_line_chunks(
        &base.keys,
        &other.keys,
        &base.non_space_chars,
        &other.non_space_chars,
        &changes,
    );
    Ok(correct_second_step(base, other, &changes))
}

pub(crate) fn compare_lines_three_way_ranges<'a>(
    left: &'a Lines<'a>,
    base: &'a Lines<'a>,
    right: &'a Lines<'a>,
    policy: ComparisonPolicy,
    keep_ignored_changes: bool,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    let base_to_left = compare_against_base(base, left, cancel)?;
    let base_to_right = compare_against_base(base, right, cancel)?;
    let true_equality = if keep_ignored_changes && policy != ComparisonPolicy::Default {
        Some(TrueEquality {
            left: &left.contents,
            base: &base.contents,
            right: &right.contents,
        })
    } else {
        None
    };
    Ok(build_merge(&base_to_left, &base_to_right, true_equality))
}
