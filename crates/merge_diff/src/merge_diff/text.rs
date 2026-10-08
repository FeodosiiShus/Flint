use std::borrow::Cow;

use crate::range::{DiffRange, MergeRange};
use crate::unicode_tables::{
    ALPHABETIC_BASIC_MULTILINGUAL_PLANE, DECIMAL_DIGIT, HIRAGANA_SCRIPT, IDEOGRAPHIC,
    JAVANESE_SCRIPT, KATAKANA_SCRIPT, THAI_SCRIPT,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ComparisonPolicy {
    Default,
    TrimWhitespaces,
    IgnoreWhitespaces,
}

pub(crate) fn is_space_enter_or_tab(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n')
}

pub(crate) fn is_punctuation(byte: u8) -> bool {
    if byte == 95 {
        return false;
    }
    matches!(byte, 33..=47 | 58..=64 | 91..=96 | 123..=126)
}

pub(crate) fn is_white_space_code_point(code_point: u32) -> bool {
    code_point < 128 && is_space_enter_or_tab(code_point as u8)
}

pub(crate) fn is_alpha(code_point: u32) -> bool {
    if is_white_space_code_point(code_point) {
        return false;
    }
    code_point >= 128 || !is_punctuation(code_point as u8)
}

fn is_in_ranges(code_point: u32, ranges: &[(u32, u32)]) -> bool {
    let index = ranges.partition_point(|&(start, _)| start < code_point);
    if let Some(&(start, _)) = ranges.get(index)
        && start == code_point
    {
        return true;
    }
    if index == 0 {
        return false;
    }
    code_point <= ranges[index - 1].1
}

fn is_basic_plane_alphabetic(code_point: u32) -> bool {
    is_in_ranges(code_point, &ALPHABETIC_BASIC_MULTILINGUAL_PLANE)
}

pub(crate) fn is_continuous_script(code_point: u32) -> bool {
    if code_point < 128 {
        return false;
    }
    if is_in_ranges(code_point, &DECIMAL_DIGIT) {
        return false;
    }
    if code_point >> 16 != 0 {
        return true;
    }
    if is_in_ranges(code_point, &IDEOGRAPHIC) {
        return true;
    }
    if !is_basic_plane_alphabetic(code_point) {
        return true;
    }
    is_in_ranges(code_point, &HIRAGANA_SCRIPT)
        || is_in_ranges(code_point, &KATAKANA_SCRIPT)
        || is_in_ranges(code_point, &THAI_SCRIPT)
        || is_in_ranges(code_point, &JAVANESE_SCRIPT)
}

fn whitespace_stripped_bounds(bytes: &[u8]) -> (usize, usize) {
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && is_space_enter_or_tab(bytes[start]) {
        start += 1;
    }
    while start < end && is_space_enter_or_tab(bytes[end - 1]) {
        end -= 1;
    }
    (start, end)
}

pub(crate) fn strip_whitespace(bytes: &[u8]) -> &[u8] {
    let (start, end) = whitespace_stripped_bounds(bytes);
    &bytes[start..end]
}

pub(crate) fn strip_whitespace_str(text: &str) -> &str {
    let (start, end) = whitespace_stripped_bounds(text.as_bytes());
    &text[start..end]
}

pub(crate) fn remove_whitespace(text: &str) -> Cow<'_, str> {
    if !text.bytes().any(is_space_enter_or_tab) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .filter(|character| !matches!(character, ' ' | '\t' | '\n'))
            .collect(),
    )
}

pub(crate) fn equals_ignore_whitespaces(first: &[u8], second: &[u8]) -> bool {
    let length1 = first.len();
    let length2 = second.len();
    let mut index1 = 0;
    let mut index2 = 0;
    while index1 < length1 && index2 < length2 {
        if first[index1] == second[index2] {
            index1 += 1;
            index2 += 1;
            continue;
        }
        let mut skipped = false;
        while index1 != length1 && is_space_enter_or_tab(first[index1]) {
            skipped = true;
            index1 += 1;
        }
        while index2 != length2 && is_space_enter_or_tab(second[index2]) {
            skipped = true;
            index2 += 1;
        }
        if !skipped {
            return false;
        }
    }
    while index1 != length1 {
        if !is_space_enter_or_tab(first[index1]) {
            return false;
        }
        index1 += 1;
    }
    while index2 != length2 {
        if !is_space_enter_or_tab(second[index2]) {
            return false;
        }
        index2 += 1;
    }
    true
}

fn equals_trim_whitespaces_single(first: &[u8], second: &[u8]) -> bool {
    strip_whitespace(first) == strip_whitespace(second)
}

fn find_newline(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map(|position| from + position)
}

fn equals_trim_whitespaces_multiline(first: &[u8], second: &[u8]) -> bool {
    let mut index1 = 0;
    let mut index2 = 0;
    loop {
        let newline1 = find_newline(first, index1);
        let newline2 = find_newline(second, index2);
        let last_line1 = newline1.is_none();
        let last_line2 = newline2.is_none();
        let end1 = newline1.map_or(first.len(), |position| position + 1);
        let end2 = newline2.map_or(second.len(), |position| position + 1);
        if last_line1 != last_line2 {
            return false;
        }
        if !equals_trim_whitespaces_single(&first[index1..end1], &second[index2..end2]) {
            return false;
        }
        index1 = end1;
        index2 = end2;
        if last_line1 {
            return true;
        }
    }
}

pub(crate) fn is_equal_texts(first: &[u8], second: &[u8], policy: ComparisonPolicy) -> bool {
    match policy {
        ComparisonPolicy::Default => first == second,
        ComparisonPolicy::TrimWhitespaces => equals_trim_whitespaces_multiline(first, second),
        ComparisonPolicy::IgnoreWhitespaces => equals_ignore_whitespaces(first, second),
    }
}

pub(crate) fn trim_start(start: usize, end: usize, ignored: impl Fn(usize) -> bool) -> usize {
    let mut start = start;
    while start < end {
        if !ignored(start) {
            break;
        }
        start += 1;
    }
    start
}

pub(crate) fn trim_end(start: usize, end: usize, ignored: impl Fn(usize) -> bool) -> usize {
    let mut end = end;
    while start < end {
        if !ignored(end - 1) {
            break;
        }
        end -= 1;
    }
    end
}

pub(crate) fn trim_text_range(text: &[u8], start: usize, end: usize) -> (usize, usize) {
    let ignored = |index: usize| is_space_enter_or_tab(text[index]);
    let trimmed_start = trim_start(start, end, ignored);
    let trimmed_end = trim_end(trimmed_start, end, ignored);
    (trimmed_start, trimmed_end)
}

fn expand_ignored_forward(
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
    equals: impl Fn(usize, usize) -> bool,
    ignored1: impl Fn(usize) -> bool,
) -> usize {
    let old_start1 = start1;
    let mut start1 = start1;
    let mut start2 = start2;
    while start1 < end1 && start2 < end2 {
        if !equals(start1, start2) {
            break;
        }
        if !ignored1(start1) {
            break;
        }
        start1 += 1;
        start2 += 1;
    }
    start1 - old_start1
}

fn expand_ignored_backward(
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
    equals: impl Fn(usize, usize) -> bool,
    ignored1: impl Fn(usize) -> bool,
) -> usize {
    let old_end1 = end1;
    let mut end1 = end1;
    let mut end2 = end2;
    while start1 < end1 && start2 < end2 {
        if !equals(end1 - 1, end2 - 1) {
            break;
        }
        if !ignored1(end1 - 1) {
            break;
        }
        end1 -= 1;
        end2 -= 1;
    }
    old_end1 - end1
}

pub(crate) fn expand_whitespaces_forward(
    text1: &[u8],
    text2: &[u8],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> usize {
    expand_ignored_forward(
        start1,
        start2,
        end1,
        end2,
        |index1, index2| text1[index1] == text2[index2],
        |index| is_space_enter_or_tab(text1[index]),
    )
}

pub(crate) fn expand_whitespaces_backward(
    text1: &[u8],
    text2: &[u8],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> usize {
    expand_ignored_backward(
        start1,
        start2,
        end1,
        end2,
        |index1, index2| text1[index1] == text2[index2],
        |index| is_space_enter_or_tab(text1[index]),
    )
}

pub(crate) fn expand_whitespaces(
    text1: &[u8],
    text2: &[u8],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> DiffRange {
    let count1 = expand_whitespaces_forward(text1, text2, start1, start2, end1, end2);
    let start1 = start1 + count1;
    let start2 = start2 + count1;
    let count2 = expand_whitespaces_backward(text1, text2, start1, start2, end1, end2);
    DiffRange::new(start1, end1 - count2, start2, end2 - count2)
}

fn expand_ignored_forward3(
    start1: usize,
    start2: usize,
    start3: usize,
    end1: usize,
    end2: usize,
    end3: usize,
    equals12: impl Fn(usize, usize) -> bool,
    equals13: impl Fn(usize, usize) -> bool,
    ignored1: impl Fn(usize) -> bool,
) -> usize {
    let old_start1 = start1;
    let mut start1 = start1;
    let mut start2 = start2;
    let mut start3 = start3;
    while start1 < end1 && start2 < end2 && start3 < end3 {
        if !equals12(start1, start2) {
            break;
        }
        if !equals13(start1, start3) {
            break;
        }
        if !ignored1(start1) {
            break;
        }
        start1 += 1;
        start2 += 1;
        start3 += 1;
    }
    start1 - old_start1
}

fn expand_ignored_backward3(
    start1: usize,
    start2: usize,
    start3: usize,
    end1: usize,
    end2: usize,
    end3: usize,
    equals12: impl Fn(usize, usize) -> bool,
    equals13: impl Fn(usize, usize) -> bool,
    ignored1: impl Fn(usize) -> bool,
) -> usize {
    let old_end1 = end1;
    let mut end1 = end1;
    let mut end2 = end2;
    let mut end3 = end3;
    while start1 < end1 && start2 < end2 && start3 < end3 {
        if !equals12(end1 - 1, end2 - 1) {
            break;
        }
        if !equals13(end1 - 1, end3 - 1) {
            break;
        }
        if !ignored1(end1 - 1) {
            break;
        }
        end1 -= 1;
        end2 -= 1;
        end3 -= 1;
    }
    old_end1 - end1
}

pub(crate) fn expand_whitespaces_forward3(
    texts: [&[u8]; 3],
    range: &MergeRange,
    end_cut: usize,
) -> usize {
    let [text1, text2, text3] = texts;
    expand_ignored_forward3(
        range.left.start,
        range.base.start,
        range.right.start,
        range.left.end - end_cut,
        range.base.end - end_cut,
        range.right.end - end_cut,
        |index1, index2| text1[index1] == text2[index2],
        |index1, index3| text1[index1] == text3[index3],
        |index| is_space_enter_or_tab(text1[index]),
    )
}

pub(crate) fn expand_whitespaces_backward3(texts: [&[u8]; 3], range: &MergeRange) -> usize {
    let [text1, text2, text3] = texts;
    expand_ignored_backward3(
        range.left.start,
        range.base.start,
        range.right.start,
        range.left.end,
        range.base.end,
        range.right.end,
        |index1, index2| text1[index1] == text2[index2],
        |index1, index3| text1[index1] == text3[index3],
        |index| is_space_enter_or_tab(text1[index]),
    )
}

pub(crate) fn expand_whitespaces3(texts: [&[u8]; 3], range: &MergeRange) -> MergeRange {
    let [text1, text2, text3] = texts;
    let count1 = expand_whitespaces_forward3(texts, range, 0);
    let start1 = range.left.start + count1;
    let start2 = range.base.start + count1;
    let start3 = range.right.start + count1;
    let count2 = expand_ignored_backward3(
        start1,
        start2,
        start3,
        range.left.end,
        range.base.end,
        range.right.end,
        |index1, index2| text1[index1] == text2[index2],
        |index1, index3| text1[index1] == text3[index3],
        |index| is_space_enter_or_tab(text1[index]),
    );
    MergeRange::new(
        start1..range.left.end - count2,
        start2..range.base.end - count2,
        start3..range.right.end - count2,
    )
}

pub(crate) fn trim3(texts: [&[u8]; 3], range: &MergeRange) -> MergeRange {
    let [text1, text2, text3] = texts;
    let (start1, end1) = trim_text_range(text1, range.left.start, range.left.end);
    let (start2, end2) = trim_text_range(text2, range.base.start, range.base.end);
    let (start3, end3) = trim_text_range(text3, range.right.start, range.right.end);
    MergeRange::new(start1..end1, start2..end2, start3..end3)
}

pub(crate) fn is_range_equals(
    text1: &[u8],
    text2: &[u8],
    range: &DiffRange,
    policy: ComparisonPolicy,
) -> bool {
    is_equal_texts(
        &text1[range.start1..range.end1],
        &text2[range.start2..range.end2],
        policy,
    )
}

pub(crate) fn is_range_equals3(
    texts: [&[u8]; 3],
    range: &MergeRange,
    policy: ComparisonPolicy,
) -> bool {
    let [text1, text2, text3] = texts;
    let sequence1 = &text1[range.left.clone()];
    let sequence2 = &text2[range.base.clone()];
    let sequence3 = &text3[range.right.clone()];
    is_equal_texts(sequence2, sequence1, policy) && is_equal_texts(sequence2, sequence3, policy)
}

#[derive(Clone, Debug)]
pub struct LineOffsets {
    text_length: usize,
    line_ends: Vec<usize>,
}

impl LineOffsets {
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut line_ends = Vec::new();
        let mut index = 0;
        loop {
            match find_newline(bytes, index) {
                Some(line_end) => {
                    line_ends.push(line_end);
                    index = line_end + 1;
                }
                None => {
                    line_ends.push(bytes.len());
                    break;
                }
            }
        }
        Self {
            text_length: bytes.len(),
            line_ends,
        }
    }

    pub fn text_length(&self) -> usize {
        self.text_length
    }

    pub fn line_count(&self) -> usize {
        self.line_ends.len()
    }

    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            0
        } else {
            self.line_ends[line - 1] + 1
        }
    }

    pub fn line_end(&self, line: usize, include_newline: bool) -> usize {
        let end = self.line_ends[line];
        if include_newline && line != self.line_ends.len() - 1 {
            end + 1
        } else {
            end
        }
    }

    pub fn lines_range(
        &self,
        first_line: usize,
        last_line: usize,
        include_newline: bool,
    ) -> (usize, usize) {
        if first_line == last_line {
            let line_start_offset = if first_line < self.line_count() {
                self.line_start(first_line)
            } else {
                self.text_length
            };
            return (line_start_offset, line_start_offset);
        }
        let start_offset = self.line_start(first_line);
        let mut end_offset = self.line_end(last_line - 1, false);
        if include_newline && end_offset < self.text_length {
            end_offset += 1;
        }
        (start_offset, end_offset)
    }
}

pub(crate) fn get_line_contents<'a>(text: &'a str, offsets: &LineOffsets) -> Vec<&'a str> {
    (0..offsets.line_count())
        .map(|line| &text[offsets.line_start(line)..offsets.line_end(line, false)])
        .collect()
}

#[derive(Clone, Debug)]
pub struct Utf16Offsets {
    boundaries: Vec<(usize, usize)>,
    ascii_only: bool,
    byte_length: usize,
}

impl Utf16Offsets {
    pub fn new(text: &str) -> Self {
        if text.is_ascii() {
            return Self {
                boundaries: Vec::new(),
                ascii_only: true,
                byte_length: text.len(),
            };
        }
        let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
        let mut utf16_offset = 0;
        for (byte_offset, character) in text.char_indices() {
            boundaries.push((byte_offset, utf16_offset));
            utf16_offset += character.len_utf16();
        }
        boundaries.push((text.len(), utf16_offset));
        Self {
            boundaries,
            ascii_only: false,
            byte_length: text.len(),
        }
    }

    pub fn to_utf16(&self, byte_offset: usize) -> Option<usize> {
        if self.ascii_only {
            return (byte_offset <= self.byte_length).then_some(byte_offset);
        }
        self.boundaries
            .binary_search_by_key(&byte_offset, |&(byte, _)| byte)
            .ok()
            .map(|index| self.boundaries[index].1)
    }

    pub fn to_byte(&self, utf16_offset: usize) -> Option<usize> {
        if self.ascii_only {
            return (utf16_offset <= self.byte_length).then_some(utf16_offset);
        }
        self.boundaries
            .binary_search_by_key(&utf16_offset, |&(_, utf16)| utf16)
            .ok()
            .map(|index| self.boundaries[index].0)
    }
}
