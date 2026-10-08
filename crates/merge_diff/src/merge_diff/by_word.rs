use std::collections::HashMap;

use crate::by_char::compare_punctuation;
use crate::chunk_optimizer::optimize_word_chunks;
use crate::diff::{ChangeBuilder, FairDiff, diff};
use crate::merge_builder::build_merge;
use crate::range::{DiffRange, MergeRange};
use crate::text::{
    ComparisonPolicy, expand_whitespaces, expand_whitespaces_backward,
    expand_whitespaces_backward3, expand_whitespaces_forward, expand_whitespaces_forward3,
    expand_whitespaces3, is_alpha, is_continuous_script, is_equal_texts, is_range_equals,
    is_range_equals3, is_space_enter_or_tab, trim_end, trim_start, trim_text_range, trim3,
};
use crate::{CancellationChecker, ComparisonError};

#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineChunk {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) is_newline: bool,
}

pub(crate) fn get_inline_chunks(text: &str) -> Vec<InlineChunk> {
    let mut chunks = Vec::new();
    let mut word_start: Option<usize> = None;
    for (offset, character) in text.char_indices() {
        let code_point = character as u32;
        let char_count = character.len_utf8();
        let alpha = is_alpha(code_point);
        let is_word_part = alpha && !is_continuous_script(code_point);
        if is_word_part {
            if word_start.is_none() {
                word_start = Some(offset);
            }
        } else {
            if let Some(start) = word_start.take() {
                chunks.push(InlineChunk {
                    start,
                    end: offset,
                    is_newline: false,
                });
            }
            if alpha {
                chunks.push(InlineChunk {
                    start: offset,
                    end: offset + char_count,
                    is_newline: false,
                });
            } else if code_point == 10 {
                chunks.push(InlineChunk {
                    start: offset,
                    end: offset + 1,
                    is_newline: true,
                });
            }
        }
    }
    if let Some(start) = word_start {
        chunks.push(InlineChunk {
            start,
            end: text.len(),
            is_newline: false,
        });
    }
    chunks
}

pub(crate) fn inline_chunks_with_ids<'a>(
    text: &'a str,
    interner: &mut HashMap<&'a str, u32>,
) -> (Vec<InlineChunk>, Vec<u32>) {
    let chunks = get_inline_chunks(text);
    let ids = chunks
        .iter()
        .map(|chunk| {
            let next = interner.len() as u32;
            *interner
                .entry(&text[chunk.start..chunk.end])
                .or_insert(next)
        })
        .collect();
    (chunks, ids)
}

struct DelimiterMatcher<'a> {
    text1: &'a [u8],
    text2: &'a [u8],
    words1: &'a [InlineChunk],
    words2: &'a [InlineChunk],
    start_shift1: usize,
    start_shift2: usize,
    builder: ChangeBuilder<'a>,
    last: Option<(usize, usize, usize, usize)>,
    cancel: &'a dyn CancellationChecker,
}

impl DelimiterMatcher<'_> {
    fn start_offset1(&self, index: usize) -> usize {
        self.words1[index].start - self.start_shift1
    }

    fn start_offset2(&self, index: usize) -> usize {
        self.words2[index].start - self.start_shift2
    }

    fn end_offset1(&self, index: usize) -> usize {
        self.words1[index].end - self.start_shift1
    }

    fn end_offset2(&self, index: usize) -> usize {
        self.words2[index].end - self.start_shift2
    }

    fn match_range(
        &mut self,
        start1: usize,
        start2: usize,
        end1: usize,
        end2: usize,
    ) -> Result<(), ComparisonError> {
        if start1 == end1 && start2 == end2 {
            return Ok(());
        }
        let changes = compare_punctuation(
            &self.text1[start1..end1],
            &self.text2[start2..end2],
            self.cancel,
        )?;
        for unchanged in changes.unchanged() {
            self.builder.mark_equal_ranges(
                start1 + unchanged.start1,
                start2 + unchanged.start2,
                start1 + unchanged.end1,
                start2 + unchanged.end2,
            );
        }
        Ok(())
    }

    fn match_complex_range_left(
        &mut self,
        start1: usize,
        end1: usize,
        start12: usize,
        end12: usize,
        start22: usize,
        end22: usize,
    ) -> Result<(), ComparisonError> {
        let sequence1 = &self.text1[start1..end1];
        let sequence21 = &self.text2[start12..end12];
        let sequence22 = &self.text2[start22..end22];
        let (first, second) =
            compare_punctuation_two_side(sequence1, sequence21, sequence22, self.cancel)?;
        for unchanged in first.unchanged() {
            self.builder.mark_equal_ranges(
                start1 + unchanged.start1,
                start12 + unchanged.start2,
                start1 + unchanged.end1,
                start12 + unchanged.end2,
            );
        }
        for unchanged in second.unchanged() {
            self.builder.mark_equal_ranges(
                start1 + unchanged.start1,
                start22 + unchanged.start2,
                start1 + unchanged.end1,
                start22 + unchanged.end2,
            );
        }
        Ok(())
    }

    fn match_complex_range_right(
        &mut self,
        start2: usize,
        end2: usize,
        start11: usize,
        end11: usize,
        start21: usize,
        end21: usize,
    ) -> Result<(), ComparisonError> {
        let sequence11 = &self.text1[start11..end11];
        let sequence12 = &self.text1[start21..end21];
        let sequence2 = &self.text2[start2..end2];
        let (first, second) =
            compare_punctuation_two_side(sequence2, sequence11, sequence12, self.cancel)?;
        for unchanged in first.unchanged() {
            self.builder.mark_equal_ranges(
                start11 + unchanged.start2,
                start2 + unchanged.start1,
                start11 + unchanged.end2,
                start2 + unchanged.end1,
            );
        }
        for unchanged in second.unchanged() {
            self.builder.mark_equal_ranges(
                start21 + unchanged.start2,
                start2 + unchanged.start1,
                start21 + unchanged.end2,
                start2 + unchanged.end1,
            );
        }
        Ok(())
    }

    fn match_complex_range(
        &mut self,
        previous: (usize, usize, usize, usize),
        current: (usize, usize, usize, usize),
    ) -> Result<(), ComparisonError> {
        let (start11, start12, end11, end12) = previous;
        let (start21, start22, end21, end22) = current;
        if start11 == start21 && end11 == end21 {
            self.match_complex_range_left(start11, end11, start12, end12, start22, end22)
        } else if start12 == start22 && end12 == end22 {
            self.match_complex_range_right(start12, end12, start11, end11, start21, end21)
        } else {
            debug_assert!(
                start11 == start21 && end11 == end21 || start12 == start22 && end12 == end22
            );
            Ok(())
        }
    }

    fn match_forward_range(&mut self, start1: usize, start2: usize, end1: usize, end2: usize) {
        debug_assert!(self.last.is_none());
        self.last = Some((start1, start2, end1, end2));
    }

    fn match_backward_range(
        &mut self,
        start1: usize,
        start2: usize,
        end1: usize,
        end2: usize,
    ) -> Result<(), ComparisonError> {
        debug_assert!(self.last.is_some());
        let Some(last) = self.last else {
            return Ok(());
        };
        let (last_start1, last_start2, last_end1, last_end2) = last;
        if last_start1 == start1 && last_start2 == start2 {
            debug_assert!(last_end1 == end1 && last_end2 == end2);
            return self.match_range(start1, start2, end1, end2);
        }
        if last_start1 < start1 && last_start2 < start2 {
            debug_assert!(last_end1 <= start1 && last_end2 <= start2);
            self.match_range(last_start1, last_start2, last_end1, last_end2)?;
            return self.match_range(start1, start2, end1, end2);
        }
        self.match_complex_range(last, (start1, start2, end1, end2))
    }

    fn match_backward(&mut self, index1: usize, index2: usize) -> Result<(), ComparisonError> {
        let start1 = if index1 == 0 {
            0
        } else {
            self.end_offset1(index1 - 1)
        };
        let start2 = if index2 == 0 {
            0
        } else {
            self.end_offset2(index2 - 1)
        };
        let end1 = if index1 == self.words1.len() {
            self.text1.len()
        } else {
            self.start_offset1(index1)
        };
        let end2 = if index2 == self.words2.len() {
            self.text2.len()
        } else {
            self.start_offset2(index2)
        };
        self.match_backward_range(start1, start2, end1, end2)?;
        self.last = None;
        Ok(())
    }

    fn match_forward(&mut self, index1: i64, index2: i64) {
        let start1 = if index1 == -1 {
            0
        } else {
            self.end_offset1(index1 as usize)
        };
        let start2 = if index2 == -1 {
            0
        } else {
            self.end_offset2(index2 as usize)
        };
        let next1 = (index1 + 1) as usize;
        let next2 = (index2 + 1) as usize;
        let end1 = if next1 == self.words1.len() {
            self.text1.len()
        } else {
            self.start_offset1(next1)
        };
        let end2 = if next2 == self.words2.len() {
            self.text2.len()
        } else {
            self.start_offset2(next2)
        };
        self.match_forward_range(start1, start2, end1, end2);
    }
}

pub(crate) fn match_adjustment_delimiters(
    text1: &[u8],
    text2: &[u8],
    words1: &[InlineChunk],
    words2: &[InlineChunk],
    changes: &FairDiff,
    start_shift1: usize,
    start_shift2: usize,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let mut matcher = DelimiterMatcher {
        text1,
        text2,
        words1,
        words2,
        start_shift1,
        start_shift2,
        builder: ChangeBuilder::new(text1.len(), text2.len()),
        last: None,
        cancel,
    };
    matcher.match_forward(-1, -1);
    for unchanged in changes.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            let index1 = unchanged.start1 + offset;
            let index2 = unchanged.start2 + offset;
            let start1 = matcher.start_offset1(index1);
            let start2 = matcher.start_offset2(index2);
            let end1 = matcher.end_offset1(index1);
            let end2 = matcher.end_offset2(index2);
            matcher.match_backward(index1, index2)?;
            matcher
                .builder
                .mark_equal_ranges(start1, start2, end1, end2);
            matcher.match_forward(index1 as i64, index2 as i64);
        }
    }
    matcher.match_backward(words1.len(), words2.len())?;
    Ok(matcher.builder.finish())
}

fn compare_punctuation_two_side(
    text1: &[u8],
    text21: &[u8],
    text22: &[u8],
    cancel: &dyn CancellationChecker,
) -> Result<(FairDiff, FairDiff), ComparisonError> {
    let mut text2 = Vec::with_capacity(text21.len() + text22.len());
    text2.extend_from_slice(text21);
    text2.extend_from_slice(text22);
    let changes = compare_punctuation(text1, &text2, cancel)?;
    let (ranges1, ranges2) = split_iterable_two_side(&changes, text21.len());
    Ok((
        FairDiff::from_unchanged(&ranges1, text1.len(), text21.len()),
        FairDiff::from_unchanged(&ranges2, text1.len(), text22.len()),
    ))
}

fn split_iterable_two_side(changes: &FairDiff, offset: usize) -> (Vec<DiffRange>, Vec<DiffRange>) {
    let mut ranges1 = Vec::new();
    let mut ranges2 = Vec::new();
    for range in changes.unchanged() {
        if range.end2 <= offset {
            ranges1.push(range);
        } else if range.start2 >= offset {
            ranges2.push(DiffRange::new(
                range.start1,
                range.end1,
                range.start2 - offset,
                range.end2 - offset,
            ));
        } else {
            let length2 = offset - range.start2;
            ranges1.push(DiffRange::new(
                range.start1,
                range.start1 + length2,
                range.start2,
                offset,
            ));
            ranges2.push(DiffRange::new(
                range.start1 + length2,
                range.end1,
                0,
                range.end2 - offset,
            ));
        }
    }
    (ranges1, ranges2)
}

fn is_leading_space(text: &[u8], start: usize) -> bool {
    if start >= text.len() {
        return false;
    }
    if !is_space_enter_or_tab(text[start]) {
        return false;
    }
    let mut index = start;
    while index > 0 {
        index -= 1;
        let byte = text[index];
        if byte == b'\n' {
            return true;
        }
        if !is_space_enter_or_tab(byte) {
            return false;
        }
    }
    true
}

fn is_trailing_space(text: &[u8], end: usize) -> bool {
    if end >= text.len() {
        return false;
    }
    if !is_space_enter_or_tab(text[end]) {
        return false;
    }
    let mut index = end;
    while index < text.len() {
        let byte = text[index];
        if byte == b'\n' {
            return true;
        }
        if !is_space_enter_or_tab(byte) {
            return false;
        }
        index += 1;
    }
    true
}

fn is_leading_trailing_space(text: &[u8], index: Option<usize>) -> bool {
    match index {
        Some(index) => is_leading_space(text, index) || is_trailing_space(text, index),
        None => false,
    }
}

fn default_corrector(changes: &FairDiff, text1: &[u8], text2: &[u8]) -> FairDiff {
    let mut result = Vec::new();
    for change in &changes.changes {
        let end_cut = expand_whitespaces_backward(
            text1,
            text2,
            change.start1,
            change.start2,
            change.end1,
            change.end2,
        );
        let start_cut = expand_whitespaces_forward(
            text1,
            text2,
            change.start1,
            change.start2,
            change.end1 - end_cut,
            change.end2 - end_cut,
        );
        let expanded = DiffRange::new(
            change.start1 + start_cut,
            change.end1 - end_cut,
            change.start2 + start_cut,
            change.end2 - end_cut,
        );
        if !expanded.is_empty() {
            result.push(expanded);
        }
    }
    FairDiff::new(result, text1.len(), text2.len())
}

fn merge_default_corrector(texts: [&[u8]; 3], ranges: &[MergeRange]) -> Vec<MergeRange> {
    let mut result = Vec::new();
    for range in ranges {
        let end_cut = expand_whitespaces_backward3(texts, range);
        let start_cut = expand_whitespaces_forward3(texts, range, end_cut);
        let expanded = MergeRange::new(
            range.left.start + start_cut..range.left.end - end_cut,
            range.base.start + start_cut..range.base.end - end_cut,
            range.right.start + start_cut..range.right.end - end_cut,
        );
        if !expanded.is_empty() {
            result.push(expanded);
        }
    }
    result
}

fn ignore_spaces_corrector(changes: &FairDiff, text1: &[u8], text2: &[u8]) -> FairDiff {
    let mut result = Vec::new();
    for change in &changes.changes {
        let expanded = expand_whitespaces(
            text1,
            text2,
            change.start1,
            change.start2,
            change.end1,
            change.end2,
        );
        let (trimmed_start1, trimmed_end1) = trim_text_range(text1, expanded.start1, expanded.end1);
        let (trimmed_start2, trimmed_end2) = trim_text_range(text2, expanded.start2, expanded.end2);
        let trimmed = DiffRange::new(trimmed_start1, trimmed_end1, trimmed_start2, trimmed_end2);
        if !trimmed.is_empty()
            && !is_range_equals(text1, text2, &trimmed, ComparisonPolicy::IgnoreWhitespaces)
        {
            result.push(trimmed);
        }
    }
    FairDiff::new(result, text1.len(), text2.len())
}

fn merge_ignore_spaces_corrector(texts: [&[u8]; 3], ranges: &[MergeRange]) -> Vec<MergeRange> {
    let mut result = Vec::new();
    for range in ranges {
        let expanded = expand_whitespaces3(texts, range);
        let trimmed = trim3(texts, &expanded);
        if !trimmed.is_empty()
            && !is_range_equals3(texts, &trimmed, ComparisonPolicy::IgnoreWhitespaces)
        {
            result.push(trimmed);
        }
    }
    result
}

pub(crate) fn trim_spaces_corrector(changes: &FairDiff, text1: &[u8], text2: &[u8]) -> FairDiff {
    let mut result = Vec::new();
    for change in &changes.changes {
        let mut start1 = change.start1;
        let mut end1 = change.end1;
        let mut start2 = change.start2;
        let mut end2 = change.end2;
        let is_space1 = |index: usize| is_space_enter_or_tab(text1[index]);
        let is_space2 = |index: usize| is_space_enter_or_tab(text2[index]);
        if is_leading_trailing_space(text1, Some(start1)) {
            start1 = trim_start(start1, end1, is_space1);
        }
        if is_leading_trailing_space(text1, end1.checked_sub(1)) {
            end1 = trim_end(start1, end1, is_space1);
        }
        if is_leading_trailing_space(text2, Some(start2)) {
            start2 = trim_start(start2, end2, is_space2);
        }
        if is_leading_trailing_space(text2, end2.checked_sub(1)) {
            end2 = trim_end(start2, end2, is_space2);
        }
        let trimmed = DiffRange::new(start1, end1, start2, end2);
        if !trimmed.is_empty()
            && !is_range_equals(text1, text2, &trimmed, ComparisonPolicy::Default)
        {
            result.push(trimmed);
        }
    }
    FairDiff::new(result, text1.len(), text2.len())
}

fn merge_trim_spaces_corrector(texts: [&[u8]; 3], ranges: &[MergeRange]) -> Vec<MergeRange> {
    let [text1, text2, text3] = texts;
    let mut result = Vec::new();
    for range in ranges {
        let mut start1 = range.left.start;
        let mut end1 = range.left.end;
        let mut start2 = range.base.start;
        let mut end2 = range.base.end;
        let mut start3 = range.right.start;
        let mut end3 = range.right.end;
        let is_space1 = |index: usize| is_space_enter_or_tab(text1[index]);
        let is_space2 = |index: usize| is_space_enter_or_tab(text2[index]);
        let is_space3 = |index: usize| is_space_enter_or_tab(text3[index]);
        if is_leading_trailing_space(text1, Some(start1)) {
            start1 = trim_start(start1, end1, is_space1);
        }
        if is_leading_trailing_space(text1, end1.checked_sub(1)) {
            end1 = trim_end(start1, end1, is_space1);
        }
        if is_leading_trailing_space(text2, Some(start2)) {
            start2 = trim_start(start2, end2, is_space2);
        }
        if is_leading_trailing_space(text2, end2.checked_sub(1)) {
            end2 = trim_end(start2, end2, is_space2);
        }
        if is_leading_trailing_space(text3, Some(start3)) {
            start3 = trim_start(start3, end3, is_space3);
        }
        if is_leading_trailing_space(text3, end3.checked_sub(1)) {
            end3 = trim_end(start3, end3, is_space3);
        }
        let trimmed = MergeRange::new(start1..end1, start2..end2, start3..end3);
        if !trimmed.is_empty() && !is_range_equals3(texts, &trimmed, ComparisonPolicy::Default) {
            result.push(trimmed);
        }
    }
    result
}

fn match_adjustment_whitespaces(
    text1: &[u8],
    text2: &[u8],
    iterable: &FairDiff,
    policy: ComparisonPolicy,
) -> FairDiff {
    match policy {
        ComparisonPolicy::Default => default_corrector(iterable, text1, text2),
        ComparisonPolicy::TrimWhitespaces => {
            let default_iterable = default_corrector(iterable, text1, text2);
            trim_spaces_corrector(&default_iterable, text1, text2)
        }
        ComparisonPolicy::IgnoreWhitespaces => ignore_spaces_corrector(iterable, text1, text2),
    }
}

fn match_adjustment_whitespaces3(
    texts: [&[u8]; 3],
    conflicts: &[MergeRange],
    policy: ComparisonPolicy,
) -> Vec<MergeRange> {
    match policy {
        ComparisonPolicy::Default => merge_default_corrector(texts, conflicts),
        ComparisonPolicy::TrimWhitespaces => {
            let default_conflicts = merge_default_corrector(texts, conflicts);
            merge_trim_spaces_corrector(texts, &default_conflicts)
        }
        ComparisonPolicy::IgnoreWhitespaces => merge_ignore_spaces_corrector(texts, conflicts),
    }
}

struct WordSequence<'a> {
    text: &'a str,
    chunks: Vec<InlineChunk>,
    ids: Vec<u32>,
}

fn word_sequence<'a>(text: &'a str, interner: &mut HashMap<&'a str, u32>) -> WordSequence<'a> {
    let (chunks, ids) = inline_chunks_with_ids(text, interner);
    WordSequence { text, chunks, ids }
}

fn diff_optimized_words(
    first: &WordSequence,
    second: &WordSequence,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let word_changes = diff(&first.ids, &second.ids, cancel)?;
    Ok(optimize_word_chunks(
        first.text.as_bytes(),
        second.text.as_bytes(),
        &first.chunks,
        &second.chunks,
        &first.ids,
        &second.ids,
        &word_changes,
    ))
}

pub(crate) fn compare_words(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<DiffRange>, ComparisonError> {
    let mut interner = HashMap::new();
    let sequence1 = word_sequence(text1, &mut interner);
    let sequence2 = word_sequence(text2, &mut interner);
    let word_changes = diff_optimized_words(&sequence1, &sequence2, cancel)?;
    let delimiters = match_adjustment_delimiters(
        text1.as_bytes(),
        text2.as_bytes(),
        &sequence1.chunks,
        &sequence2.chunks,
        &word_changes,
        0,
        0,
        cancel,
    )?;
    let iterable =
        match_adjustment_whitespaces(text1.as_bytes(), text2.as_bytes(), &delimiters, policy);
    Ok(iterable.changes)
}

fn delimiters_of_base_against(
    base: &WordSequence,
    other: &WordSequence,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let word_changes = diff_optimized_words(base, other, cancel)?;
    match_adjustment_delimiters(
        base.text.as_bytes(),
        other.text.as_bytes(),
        &base.chunks,
        &other.chunks,
        &word_changes,
        0,
        0,
        cancel,
    )
}

pub(crate) fn compare_words_three_way(
    text1: &str,
    text2: &str,
    text3: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<MergeRange>, ComparisonError> {
    let mut interner = HashMap::new();
    let sequence1 = word_sequence(text1, &mut interner);
    let sequence2 = word_sequence(text2, &mut interner);
    let sequence3 = word_sequence(text3, &mut interner);
    let iterable1 = delimiters_of_base_against(&sequence2, &sequence1, cancel)?;
    let iterable2 = delimiters_of_base_against(&sequence2, &sequence3, cancel)?;
    let word_conflicts = build_merge(&iterable1, &iterable2, None);
    Ok(match_adjustment_whitespaces3(
        [text1.as_bytes(), text2.as_bytes(), text3.as_bytes()],
        &word_conflicts,
        policy,
    ))
}

struct WordBlock {
    words: DiffRange,
    offsets: DiffRange,
}

struct PendingChunk {
    block: WordBlock,
    has_equal_words: bool,
    has_words_inside: bool,
    is_equal_ignore_whitespaces: bool,
}

pub(crate) struct LineBlock {
    pub(crate) fragments: Vec<DiffRange>,
    pub(crate) offsets: DiffRange,
    pub(crate) newlines1: usize,
    pub(crate) newlines2: usize,
}

struct WordBlockSplitter<'a> {
    text1: &'a [u8],
    text2: &'a [u8],
    words1: &'a [InlineChunk],
    words2: &'a [InlineChunk],
    result: Vec<WordBlock>,
    last1: i64,
    last2: i64,
    pending: Option<PendingChunk>,
}

fn get_offset(words: &[InlineChunk], text_length: usize, index: i64) -> usize {
    if index == -1 {
        return 0;
    }
    if index as usize == words.len() {
        return text_length;
    }
    let chunk = &words[index as usize];
    debug_assert!(chunk.is_newline);
    chunk.end
}

fn should_merge_chunks(chunk1: &PendingChunk, chunk2: &PendingChunk) -> bool {
    if !chunk1.has_equal_words && !chunk2.has_equal_words {
        return true;
    }
    if chunk1.is_equal_ignore_whitespaces && chunk2.is_equal_ignore_whitespaces {
        return true;
    }
    !chunk1.has_words_inside || !chunk2.has_words_inside
}

fn merge_chunks(chunk1: PendingChunk, chunk2: PendingChunk) -> PendingChunk {
    let block1 = &chunk1.block;
    let block2 = &chunk2.block;
    let block = WordBlock {
        words: DiffRange::new(
            block1.words.start1,
            block2.words.end1,
            block1.words.start2,
            block2.words.end2,
        ),
        offsets: DiffRange::new(
            block1.offsets.start1,
            block2.offsets.end1,
            block1.offsets.start2,
            block2.offsets.end2,
        ),
    };
    PendingChunk {
        block,
        has_equal_words: chunk1.has_equal_words || chunk2.has_equal_words,
        has_words_inside: chunk1.has_words_inside || chunk2.has_words_inside,
        is_equal_ignore_whitespaces: chunk1.is_equal_ignore_whitespaces
            && chunk2.is_equal_ignore_whitespaces,
    }
}

impl WordBlockSplitter<'_> {
    fn has_words_inside(&self, block: &WordBlock) -> bool {
        (block.words.start1..block.words.end1).any(|index| !self.words1[index].is_newline)
            || (block.words.start2..block.words.end2).any(|index| !self.words2[index].is_newline)
    }

    fn is_equal_ignore_whitespaces(&self, block: &WordBlock) -> bool {
        is_equal_texts(
            &self.text1[block.offsets.start1..block.offsets.end1],
            &self.text2[block.offsets.start2..block.offsets.end2],
            ComparisonPolicy::IgnoreWhitespaces,
        )
    }

    fn create_chunk(
        &self,
        start1: i64,
        start2: i64,
        end1: i64,
        end2: i64,
        has_equal_words: bool,
    ) -> PendingChunk {
        let start_offset1 = get_offset(self.words1, self.text1.len(), start1);
        let start_offset2 = get_offset(self.words2, self.text2.len(), start2);
        let end_offset1 = get_offset(self.words1, self.text1.len(), end1);
        let end_offset2 = get_offset(self.words2, self.text2.len(), end2);
        let words_start1 = (start1 + 1).max(0) as usize;
        let words_start2 = (start2 + 1).max(0) as usize;
        let words_end1 = (end1 + 1).min(self.words1.len() as i64) as usize;
        let words_end2 = (end2 + 1).min(self.words2.len() as i64) as usize;
        let block = WordBlock {
            words: DiffRange::new(words_start1, words_end1, words_start2, words_end2),
            offsets: DiffRange::new(start_offset1, end_offset1, start_offset2, end_offset2),
        };
        let has_words_inside = self.has_words_inside(&block);
        let is_equal_ignore_whitespaces = self.is_equal_ignore_whitespaces(&block);
        PendingChunk {
            block,
            has_equal_words,
            has_words_inside,
            is_equal_ignore_whitespaces,
        }
    }

    fn add_line_chunk(&mut self, end1: i64, end2: i64, has_equal_words: bool) {
        if self.last1 > end1 || self.last2 > end2 {
            return;
        }
        let chunk = self.create_chunk(self.last1, self.last2, end1, end2, has_equal_words);
        let offsets = chunk.block.offsets;
        if offsets.is_empty() {
            return;
        }
        self.pending = match self.pending.take() {
            Some(pending) => {
                if should_merge_chunks(&pending, &chunk) {
                    Some(merge_chunks(pending, chunk))
                } else {
                    self.result.push(pending.block);
                    Some(chunk)
                }
            }
            None => Some(chunk),
        };
        self.last1 = end1;
        self.last2 = end2;
    }

    fn is_first_in_line(words: &[InlineChunk], index: usize) -> bool {
        index == 0 || words[index - 1].is_newline
    }
}

fn split_into_word_blocks(
    text1: &[u8],
    text2: &[u8],
    words1: &[InlineChunk],
    words2: &[InlineChunk],
    iterable: &FairDiff,
) -> Vec<WordBlock> {
    let mut splitter = WordBlockSplitter {
        text1,
        text2,
        words1,
        words2,
        result: Vec::new(),
        last1: -1,
        last2: -1,
        pending: None,
    };
    let mut has_equal_words = false;
    for unchanged in iterable.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            let index1 = unchanged.start1 + offset;
            let index2 = unchanged.start2 + offset;
            if words1[index1].is_newline && words2[index2].is_newline {
                splitter.add_line_chunk(index1 as i64, index2 as i64, has_equal_words);
                has_equal_words = false;
            } else {
                if WordBlockSplitter::is_first_in_line(words1, index1)
                    && WordBlockSplitter::is_first_in_line(words2, index2)
                {
                    splitter.add_line_chunk(index1 as i64 - 1, index2 as i64 - 1, has_equal_words);
                }
                has_equal_words = true;
            }
        }
    }
    splitter.add_line_chunk(words1.len() as i64, words2.len() as i64, has_equal_words);
    if let Some(pending) = splitter.pending.take() {
        splitter.result.push(pending.block);
    }
    splitter.result
}

fn subiterable(
    changed: &[DiffRange],
    start1: usize,
    end1: usize,
    start2: usize,
    end2: usize,
    first_index: usize,
) -> FairDiff {
    let mut result = Vec::new();
    let mut index = first_index;
    while index < changed.len() {
        let range = changed[index];
        index += 1;
        if range.end1 < start1 || range.end2 < start2 {
            continue;
        }
        if range.start1 > end1 || range.start2 > end2 {
            break;
        }
        let new_range = DiffRange::new(
            range.start1.max(start1) - start1,
            range.end1.min(end1) - start1,
            range.start2.max(start2) - start2,
            range.end2.min(end2) - start2,
        );
        if new_range.is_empty() {
            continue;
        }
        result.push(new_range);
    }
    FairDiff::new(result, end1 - start1, end2 - start2)
}

fn collect_word_block_sub_iterables(
    word_changes: &FairDiff,
    word_blocks: &[WordBlock],
) -> Vec<FairDiff> {
    let changed = &word_changes.changes;
    let mut index = 0;
    let mut sub_iterables = Vec::new();
    for block in word_blocks {
        let words = block.words;
        while index < changed.len() {
            let range = changed[index];
            if range.end1 < words.start1 || range.end2 < words.start2 {
                index += 1;
                continue;
            }
            break;
        }
        sub_iterables.push(subiterable(
            changed,
            words.start1,
            words.end1,
            words.start2,
            words.end2,
            index,
        ));
    }
    sub_iterables
}

fn count_newlines(words: &[InlineChunk]) -> usize {
    words.iter().filter(|word| word.is_newline).count()
}

pub(crate) fn compare_and_split(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<LineBlock>, ComparisonError> {
    let mut interner = HashMap::new();
    let sequence1 = word_sequence(text1, &mut interner);
    let sequence2 = word_sequence(text2, &mut interner);
    let word_changes = diff_optimized_words(&sequence1, &sequence2, cancel)?;
    let word_blocks = split_into_word_blocks(
        text1.as_bytes(),
        text2.as_bytes(),
        &sequence1.chunks,
        &sequence2.chunks,
        &word_changes,
    );
    let sub_iterables = collect_word_block_sub_iterables(&word_changes, &word_blocks);
    let mut line_blocks = Vec::with_capacity(word_blocks.len());
    for (block, sub_iterable) in word_blocks.iter().zip(&sub_iterables) {
        let offsets = block.offsets;
        let words = block.words;
        let sub_text1 = &text1.as_bytes()[offsets.start1..offsets.end1];
        let sub_text2 = &text2.as_bytes()[offsets.start2..offsets.end2];
        let sub_words1 = &sequence1.chunks[words.start1..words.end1];
        let sub_words2 = &sequence2.chunks[words.start2..words.end2];
        let delimiters = match_adjustment_delimiters(
            sub_text1,
            sub_text2,
            sub_words1,
            sub_words2,
            sub_iterable,
            offsets.start1,
            offsets.start2,
            cancel,
        )?;
        let iterable = match_adjustment_whitespaces(sub_text1, sub_text2, &delimiters, policy);
        line_blocks.push(LineBlock {
            fragments: iterable.changes,
            offsets,
            newlines1: count_newlines(sub_words1),
            newlines2: count_newlines(sub_words2),
        });
    }
    Ok(line_blocks)
}
