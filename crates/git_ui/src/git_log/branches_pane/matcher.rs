use std::ops::Range;

const MAX_CAMEL_HUMP_MATCHING_LENGTH: usize = 100;
const MATCH_OFFSET: i32 = 10_000;
const NO_MATCH_DEGREE: i32 = 0;
const PARTIAL_MATCH_DEGREE: i32 = 1;
const WORD_START_SCORE: i32 = 1000;
const NOT_AFTER_SEPARATOR_SCORE: i32 = 2;
const SKIPPED_HUMP_PENALTY: i32 = 10;
const RUSSIAN_TO_ENGLISH_LAYOUT: [(char, char); 33] = [
    ('й', 'q'),
    ('ц', 'w'),
    ('у', 'e'),
    ('к', 'r'),
    ('е', 't'),
    ('н', 'y'),
    ('г', 'u'),
    ('ш', 'i'),
    ('щ', 'o'),
    ('з', 'p'),
    ('х', '['),
    ('ъ', ']'),
    ('ф', 'a'),
    ('ы', 's'),
    ('в', 'd'),
    ('а', 'f'),
    ('п', 'g'),
    ('р', 'h'),
    ('о', 'j'),
    ('л', 'k'),
    ('д', 'l'),
    ('ж', ';'),
    ('э', '\''),
    ('я', 'z'),
    ('ч', 'x'),
    ('с', 'c'),
    ('м', 'v'),
    ('и', 'b'),
    ('т', 'n'),
    ('ь', 'm'),
    ('б', ','),
    ('ю', '.'),
    ('.', '/'),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matching {
    None,
    Partial,
    Full,
}

pub(crate) fn classify(fragments: Option<&[Range<usize>]>) -> Matching {
    match fragments {
        None => Matching::None,
        Some([only]) if only.start == 0 => Matching::Full,
        Some(_) => Matching::Partial,
    }
}

pub(crate) struct BranchesTreeMatcher {
    matchers: Vec<LayoutFixingMatcher>,
}

impl BranchesTreeMatcher {
    pub(crate) fn new(raw_pattern: &str) -> Self {
        let words: Vec<&str> = if raw_pattern.trim().is_empty() {
            vec![""]
        } else {
            raw_pattern
                .split(' ')
                .filter(|word| !word.is_empty())
                .map(str::trim)
                .collect()
        };
        Self {
            matchers: words
                .into_iter()
                .map(|word| LayoutFixingMatcher::new(&format!("*{word}")))
                .collect(),
        }
    }

    pub(crate) fn match_fragments(&self, name: &str) -> Option<Vec<Range<usize>>> {
        let name: Vec<char> = name.chars().collect();
        let mut best: Option<(usize, Vec<Range<usize>>)> = None;
        for matcher in &self.matchers {
            let Some(fragments) = matcher.match_fragments(&name) else {
                continue;
            };
            let length: usize = fragments.iter().map(|fragment| fragment.len()).sum();
            if best
                .as_ref()
                .is_none_or(|(best_length, _)| length > *best_length)
            {
                best = Some((length, fragments));
            }
        }
        best.map(|(_, fragments)| fragments)
    }

    pub(crate) fn matching_degree(&self, name: &str, fragments: Option<&[Range<usize>]>) -> i32 {
        if let [matcher] = self.matchers.as_slice() {
            let name: Vec<char> = name.chars().collect();
            let degree = matcher.matching_degree(&name, false, fragments);
            return match fragments.and_then(|fragments| fragments.first()) {
                Some(first) => degree
                    .saturating_add(MATCH_OFFSET)
                    .saturating_sub(i32::try_from(first.start).unwrap_or(i32::MAX)),
                None => degree,
            };
        }
        if fragments.is_some_and(|fragments| !fragments.is_empty()) {
            PARTIAL_MATCH_DEGREE
        } else {
            NO_MATCH_DEGREE
        }
    }

    pub(crate) fn degree_of(&self, name: &str) -> i32 {
        let fragments = self.match_fragments(name);
        self.matching_degree(name, fragments.as_deref())
    }
}

struct LayoutFixingMatcher {
    main: MinusculeMatcher,
    fallback: Option<MinusculeMatcher>,
}

impl LayoutFixingMatcher {
    fn new(pattern: &str) -> Self {
        let fallback = fix_layout(pattern)
            .filter(|fixed| fixed != pattern)
            .map(|fixed| MinusculeMatcher::new(&fixed));
        Self {
            main: MinusculeMatcher::new(pattern),
            fallback,
        }
    }

    fn uses_main(&self, main_fragments: &Option<Vec<Range<usize>>>) -> bool {
        self.fallback.is_none()
            || main_fragments
                .as_ref()
                .is_some_and(|fragments| !fragments.is_empty())
    }

    fn match_fragments(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        let main_fragments = self.main.match_fragments(name);
        if self.uses_main(&main_fragments) {
            return main_fragments;
        }
        self.fallback
            .as_ref()
            .and_then(|fallback| fallback.match_fragments(name))
    }

    fn matching_degree(
        &self,
        name: &[char],
        value_start_case_match: bool,
        fragments: Option<&[Range<usize>]>,
    ) -> i32 {
        let main_fragments = self.main.match_fragments(name);
        match &self.fallback {
            Some(fallback) if !self.uses_main(&main_fragments) => {
                fallback.matching_degree(name, value_start_case_match, fragments)
            }
            _ => self
                .main
                .matching_degree(name, value_start_case_match, fragments),
        }
    }
}

fn fix_layout(pattern: &str) -> Option<String> {
    let mut has_letters = false;
    let mut only_wrong_letters = true;
    for character in pattern.chars() {
        if character.is_alphabetic() {
            has_letters = true;
            if character.is_ascii() {
                only_wrong_letters = false;
                break;
            }
        }
    }
    (has_letters && only_wrong_letters).then(|| {
        pattern
            .chars()
            .map(|character| ascii_for_char(character).unwrap_or(character))
            .collect()
    })
}

fn ascii_for_char(character: char) -> Option<char> {
    let lower = to_lower(character);
    let converted = RUSSIAN_TO_ENGLISH_LAYOUT
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| *to)?;
    Some(if lower == character {
        converted
    } else {
        to_upper(converted)
    })
}

fn single_case(mut mapping: impl Iterator<Item = char>, character: char) -> char {
    match (mapping.next(), mapping.next()) {
        (Some(mapped), None) => mapped,
        _ => character,
    }
}

fn to_upper(character: char) -> char {
    single_case(character.to_uppercase(), character)
}

fn to_lower(character: char) -> char {
    single_case(character.to_lowercase(), character)
}

fn equals_ignore_case(left: char, right: char) -> bool {
    if left == right {
        return true;
    }
    let left_upper = to_upper(left);
    let right_upper = to_upper(right);
    left_upper == right_upper || to_lower(left_upper) == to_lower(right_upper)
}

fn is_wildcard(character: char) -> bool {
    character == ' ' || character == '*'
}

fn is_word_separator(character: char) -> bool {
    character.is_whitespace() || matches!(character, '_' | '-' | ':' | '+' | '.')
}

fn is_ideographic(character: char) -> bool {
    matches!(
        character,
        '\u{3006}'
            | '\u{3007}'
            | '\u{3021}'..='\u{3029}'
            | '\u{3038}'..='\u{303A}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{3FFFF}'
    )
}

fn char_at(text: &[char], index: isize) -> Option<char> {
    usize::try_from(index)
        .ok()
        .and_then(|index| text.get(index).copied())
}

fn is_word_start_at(text: &[char], index: isize) -> bool {
    usize::try_from(index).is_ok_and(|index| is_word_start(text, index))
}

fn is_word_start(text: &[char], index: usize) -> bool {
    let Some(&current) = text.get(index) else {
        return false;
    };
    let previous = index
        .checked_sub(1)
        .and_then(|previous| text.get(previous).copied());
    if current.is_uppercase() {
        if previous.is_some_and(char::is_uppercase) {
            return text.get(index + 1).is_some_and(|next| next.is_lowercase());
        }
        return true;
    }
    if current.is_numeric() {
        return true;
    }
    if !current.is_alphabetic() {
        return false;
    }
    if is_ideographic(current) {
        return true;
    }
    previous.is_none_or(|previous| !previous.is_alphanumeric())
}

fn next_word(text: &[char], start: usize) -> usize {
    let Some(&first) = text.get(start) else {
        return start + 1;
    };
    if !first.is_alphanumeric() {
        return start + 1;
    }
    let mut index = start;
    while text
        .get(index)
        .is_some_and(|character| character.is_numeric())
    {
        index += 1;
    }
    if index > start {
        return index;
    }
    while text
        .get(index)
        .is_some_and(|character| character.is_uppercase())
    {
        index += 1;
    }
    if index > start + 1 {
        if text
            .get(index)
            .is_none_or(|character| !character.is_alphabetic())
        {
            return index;
        }
        return index - 1;
    }
    if index == start {
        index += 1;
    }
    while text
        .get(index)
        .is_some_and(|character| character.is_alphabetic())
        && !is_word_start(text, index)
    {
        index += 1;
    }
    index
}

fn index_of(text: &[char], character: char, from: usize) -> Option<usize> {
    text.iter()
        .enumerate()
        .skip(from)
        .find_map(|(index, candidate)| (*candidate == character).then_some(index))
}

fn contains_char_in_range(text: &[char], character: char, start: isize, end: isize) -> bool {
    let start = usize::try_from(start.max(0)).unwrap_or(0);
    let end = usize::try_from(end.max(0)).unwrap_or(0).min(text.len());
    start < end && text[start..end].contains(&character)
}

fn name_contains_all_meaningful_chars_in_order(name: &[char], meaningful: &[char]) -> bool {
    let mut meaningful_index = 0;
    let mut name_index = 0;
    while meaningful_index + 1 < meaningful.len() {
        if name_index >= name.len() {
            return false;
        }
        let first = meaningful[meaningful_index];
        let first_index = index_of(name, first, name_index);
        if first_index == Some(name_index) {
            name_index += 1;
        } else {
            let second = meaningful[meaningful_index + 1];
            if first == second {
                match first_index {
                    Some(found) => name_index = found + 1,
                    None => return false,
                }
            } else {
                let second_index = index_of(name, second, name_index);
                name_index = match (first_index, second_index) {
                    (Some(first_found), Some(second_found)) => first_found.min(second_found) + 1,
                    (Some(found), None) | (None, Some(found)) => found + 1,
                    (None, None) => return false,
                };
            }
        }
        meaningful_index += 2;
    }
    true
}

fn append_range(mut ranges: Vec<Range<usize>>, from: usize, length: usize) -> Vec<Range<usize>> {
    if let Some(last) = ranges.last_mut()
        && last.start == from + length
    {
        last.start = from;
    } else {
        ranges.push(from..from + length);
    }
    ranges
}

fn count_skipped_upper_case_inside_word(name: &[char], start: usize, end: usize) -> i32 {
    let count = (start..end.min(name.len()))
        .filter(|index| name[*index].is_uppercase() && !is_word_start(name, *index))
        .count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

struct MinusculeMatcher {
    pattern: Vec<char>,
    is_lower_case: Vec<bool>,
    is_upper_case: Vec<bool>,
    is_word_separator: Vec<bool>,
    to_upper_case: Vec<char>,
    to_lower_case: Vec<char>,
    mixed_case: bool,
    has_dots: bool,
    meaningful_characters: Vec<char>,
}

impl MinusculeMatcher {
    fn new(pattern: &str) -> Self {
        let mut pattern: Vec<char> = pattern.chars().collect();
        if pattern.ends_with(&['*', ' ']) {
            pattern.truncate(pattern.len() - 2);
        }
        let mut meaningful_characters = Vec::new();
        let mut seen_non_wildcard = false;
        let mut seen_lower_case = false;
        let mut seen_upper_case_not_immediately_after_wildcard = false;
        let mut has_dots = false;
        for &character in &pattern {
            if character.is_lowercase() {
                seen_lower_case = true;
            }
            if character == '.' {
                has_dots = true;
            }
            if seen_non_wildcard && character.is_uppercase() {
                seen_upper_case_not_immediately_after_wildcard = true;
            }
            if !is_wildcard(character) {
                seen_non_wildcard = true;
                meaningful_characters.push(to_lower(character));
                meaningful_characters.push(to_upper(character));
            }
        }
        Self {
            is_lower_case: pattern.iter().map(|c| c.is_lowercase()).collect(),
            is_upper_case: pattern.iter().map(|c| c.is_uppercase()).collect(),
            is_word_separator: pattern.iter().map(|c| is_word_separator(*c)).collect(),
            to_upper_case: pattern.iter().map(|c| to_upper(*c)).collect(),
            to_lower_case: pattern.iter().map(|c| to_lower(*c)).collect(),
            mixed_case: seen_lower_case && seen_upper_case_not_immediately_after_wildcard,
            has_dots,
            meaningful_characters,
            pattern,
        }
    }

    fn pattern_length(&self) -> isize {
        isize::try_from(self.pattern.len()).unwrap_or(isize::MAX)
    }

    fn pattern_char(&self, index: isize) -> Option<char> {
        char_at(&self.pattern, index)
    }

    fn flag(flags: &[bool], index: isize) -> bool {
        usize::try_from(index)
            .ok()
            .and_then(|index| flags.get(index).copied())
            .unwrap_or(false)
    }

    fn is_wildcard_at(&self, index: isize) -> bool {
        self.pattern_char(index).is_some_and(is_wildcard)
    }

    fn is_pattern_char(&self, index: isize, character: char) -> bool {
        self.pattern_char(index) == Some(character)
    }

    fn is_trailing_space_pattern(&self) -> bool {
        self.pattern.last() == Some(&' ')
    }

    fn is_upper_case_or_digit(&self, index: isize) -> bool {
        Self::flag(&self.is_upper_case, index)
            || self.pattern_char(index).is_some_and(char::is_numeric)
    }

    fn char_equals(&self, pattern_index: isize, character: char) -> bool {
        let Ok(index) = usize::try_from(pattern_index) else {
            return false;
        };
        self.pattern.get(index) == Some(&character)
            || self.to_lower_case.get(index) == Some(&character)
            || self.to_upper_case.get(index) == Some(&character)
    }

    fn match_fragments(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        if name.len() < self.meaningful_characters.len() / 2 {
            return None;
        }
        if self.pattern.len() > MAX_CAMEL_HUMP_MATCHING_LENGTH {
            return self.match_by_substring(name);
        }
        if !name_contains_all_meaningful_chars_in_order(name, &self.meaningful_characters) {
            return None;
        }
        self.match_wildcards(name, 0, 0).map(|mut fragments| {
            fragments.reverse();
            fragments
        })
    }

    fn match_by_substring(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        let count = self.meaningful_characters.len() / 2;
        if name.len() < count {
            return None;
        }
        if self.is_pattern_char(0, '*') {
            return (0..=name.len() - count).find_map(|index| {
                let length = self.meaningful_chars_match_at(name, index);
                (length != 0).then(|| vec![index..index + length])
            });
        }
        let length = self.meaningful_chars_match_at(name, 0);
        (length != 0).then(|| vec![0..length])
    }

    fn meaningful_chars_match_at(&self, name: &[char], name_index: usize) -> usize {
        let mut meaningful_index = 0;
        let mut position = name_index;
        while position < name.len() && meaningful_index + 1 < self.meaningful_characters.len() {
            let character = name[position];
            if character == self.meaningful_characters[meaningful_index]
                || character == self.meaningful_characters[meaningful_index + 1]
            {
                meaningful_index += 2;
                position += 1;
            } else if is_wildcard(character) {
                position += 1;
            } else {
                return 0;
            }
        }
        position - name_index
    }

    fn match_wildcards(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
    ) -> Option<Vec<Range<usize>>> {
        if name_index < 0 {
            return None;
        }
        let mut pattern_index = pattern_index;
        if !self.is_wildcard_at(pattern_index) {
            if pattern_index == self.pattern_length() {
                return Some(Vec::new());
            }
            return self.match_fragment(name, pattern_index, name_index);
        }
        loop {
            pattern_index += 1;
            if !self.is_wildcard_at(pattern_index) {
                break;
            }
        }
        if pattern_index == self.pattern_length() {
            let name_length = isize::try_from(name.len()).unwrap_or(isize::MAX);
            let matches_trailing_space = self.is_trailing_space_pattern()
                && name_index != name_length
                && (pattern_index < 2 || !self.is_upper_case_or_digit(pattern_index - 2));
            if matches_trailing_space {
                let start = usize::try_from(name_index).unwrap_or(0);
                return index_of(name, ' ', start).map(|space| vec![space..space + 1]);
            }
            return Some(Vec::new());
        }
        let next = self.find_next_pattern_char_occurrence(name, name_index, pattern_index);
        self.match_skipping_words(name, pattern_index, next, true)
    }

    fn match_skipping_words(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
        allow_special_chars: bool,
    ) -> Option<Vec<Range<usize>>> {
        let name_length = isize::try_from(name.len()).unwrap_or(isize::MAX);
        let mut name_index = name_index;
        let mut max_found_length = 0;
        while name_index >= 0 {
            let fragment_length = if self.seems_like_fragment_start(name, pattern_index, name_index)
            {
                self.max_matching_fragment(name, pattern_index, name_index)
            } else {
                0
            };
            if fragment_length > max_found_length
                || (name_index + fragment_length == name_length && self.is_trailing_space_pattern())
            {
                if !self.is_middle_match(name, pattern_index, name_index) {
                    max_found_length = fragment_length;
                }
                if let Some(ranges) =
                    self.match_inside_fragment(name, pattern_index, name_index, fragment_length)
                {
                    return Some(ranges);
                }
            }
            let next = self.find_next_pattern_char_occurrence(name, name_index + 1, pattern_index);
            name_index = if allow_special_chars {
                next
            } else {
                self.check_for_special_chars(name, name_index + 1, next, pattern_index)
            };
        }
        None
    }

    fn find_next_pattern_char_occurrence(
        &self,
        name: &[char],
        start_at: isize,
        pattern_index: isize,
    ) -> isize {
        if !self.is_pattern_char(pattern_index - 1, '*')
            && !Self::flag(&self.is_word_separator, pattern_index)
        {
            self.index_of_word_start(name, pattern_index, start_at)
        } else {
            self.index_of_ignore_case(name, start_at, pattern_index)
        }
    }

    fn check_for_special_chars(
        &self,
        name: &[char],
        start: isize,
        end: isize,
        pattern_index: isize,
    ) -> isize {
        if end < 0 {
            return -1;
        }
        if self.has_dots
            && !self.is_pattern_char(pattern_index - 1, '.')
            && contains_char_in_range(name, '.', start, end)
        {
            return -1;
        }
        end
    }

    fn seems_like_fragment_start(
        &self,
        name: &[char],
        pattern_index: isize,
        next_occurrence: isize,
    ) -> bool {
        !Self::flag(&self.is_upper_case, pattern_index)
            || char_at(name, next_occurrence).is_some_and(char::is_uppercase)
            || is_word_start_at(name, next_occurrence)
            || !self.mixed_case
    }

    fn match_fragment(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
    ) -> Option<Vec<Range<usize>>> {
        let fragment_length = self.max_matching_fragment(name, pattern_index, name_index);
        if fragment_length == 0 {
            return None;
        }
        self.match_inside_fragment(name, pattern_index, name_index, fragment_length)
    }

    fn max_matching_fragment(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
    ) -> isize {
        let first_matches = char_at(name, name_index)
            .is_some_and(|character| self.char_equals(pattern_index, character));
        if !first_matches {
            return 0;
        }
        let name_length = isize::try_from(name.len()).unwrap_or(isize::MAX);
        let mut length = 1;
        while name_index + length < name_length && pattern_index + length < self.pattern_length() {
            let Some(name_char) = char_at(name, name_index + length) else {
                break;
            };
            if !self.char_equals(pattern_index + length, name_char) {
                let skips_digit = self
                    .pattern_char(pattern_index + length)
                    .is_some_and(char::is_numeric)
                    && self
                        .pattern_char(pattern_index + length - 1)
                        .is_some_and(char::is_numeric)
                    && name_char.is_numeric();
                if skips_digit {
                    return 0;
                }
                break;
            }
            length += 1;
        }
        length
    }

    fn match_inside_fragment(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
        fragment_length: isize,
    ) -> Option<Vec<Range<usize>>> {
        let min_fragment = if self.is_middle_match(name, pattern_index, name_index) {
            3
        } else {
            1
        };
        self.improve_camel_humps(
            name,
            pattern_index,
            name_index,
            fragment_length,
            min_fragment,
        )
        .or_else(|| {
            self.find_longest_matching_prefix(
                name,
                pattern_index,
                name_index,
                fragment_length,
                min_fragment,
            )
        })
    }

    fn is_middle_match(&self, name: &[char], pattern_index: isize, name_index: isize) -> bool {
        self.is_pattern_char(pattern_index - 1, '*')
            && !self.is_wildcard_at(pattern_index + 1)
            && char_at(name, name_index).is_some_and(char::is_alphanumeric)
            && !is_word_start_at(name, name_index)
    }

    fn find_longest_matching_prefix(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
        fragment_length: isize,
        min_fragment: isize,
    ) -> Option<Vec<Range<usize>>> {
        let start = usize::try_from(name_index).ok()?;
        if pattern_index + fragment_length >= self.pattern_length() {
            let length = usize::try_from(fragment_length).ok()?;
            return Some(vec![start..start + length]);
        }
        let mut length = fragment_length;
        while length >= min_fragment || (length > 0 && self.is_wildcard_at(pattern_index + length))
        {
            let ranges = if self.is_wildcard_at(pattern_index + length) {
                self.match_wildcards(name, pattern_index + length, name_index + length)
            } else {
                let next = self.find_next_pattern_char_occurrence(
                    name,
                    name_index + length + 1,
                    pattern_index + length,
                );
                let next = self.check_for_special_chars(
                    name,
                    name_index + length,
                    next,
                    pattern_index + length,
                );
                if next >= 0 {
                    self.match_skipping_words(name, pattern_index + length, next, false)
                } else {
                    None
                }
            };
            if let Some(ranges) = ranges {
                return Some(append_range(ranges, start, usize::try_from(length).ok()?));
            }
            length -= 1;
        }
        None
    }

    fn improve_camel_humps(
        &self,
        name: &[char],
        pattern_index: isize,
        name_index: isize,
        max_fragment: isize,
        min_fragment: isize,
    ) -> Option<Vec<Range<usize>>> {
        let start = usize::try_from(name_index).ok()?;
        for length in min_fragment..max_fragment {
            let uppercase_against_lowercase =
                Self::flag(&self.is_upper_case, pattern_index + length)
                    && self.pattern_char(pattern_index + length)
                        != char_at(name, name_index + length);
            if !uppercase_against_lowercase {
                continue;
            }
            let word_start =
                self.index_of_word_start(name, pattern_index + length, name_index + length);
            if let Some(ranges) = self.match_wildcards(name, pattern_index + length, word_start) {
                return Some(append_range(ranges, start, usize::try_from(length).ok()?));
            }
        }
        None
    }

    fn index_of_word_start(&self, name: &[char], pattern_index: isize, start_from: isize) -> isize {
        let Some(pattern_char) = self.pattern_char(pattern_index) else {
            return -1;
        };
        let name_length = isize::try_from(name.len()).unwrap_or(isize::MAX);
        let follows_separator =
            pattern_index > 0 && Self::flag(&self.is_word_separator, pattern_index - 1);
        if start_from >= name_length
            || (self.mixed_case
                && Self::flag(&self.is_lower_case, pattern_index)
                && !follows_separator)
        {
            return -1;
        }
        let is_special_symbol = !pattern_char.is_alphanumeric();
        let mut index = start_from.max(0);
        loop {
            index = self.index_of_ignore_case(name, index, pattern_index);
            if index < 0 {
                return -1;
            }
            if is_special_symbol || is_word_start_at(name, index) {
                return index;
            }
            index += 1;
        }
    }

    fn index_of_ignore_case(&self, name: &[char], from: isize, pattern_index: isize) -> isize {
        let Some(pattern_char) = self.pattern_char(pattern_index) else {
            return -1;
        };
        let start = usize::try_from(from.max(0)).unwrap_or(0);
        let found = name
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, character)| {
                let matches = if pattern_char.is_ascii() {
                    self.char_equals(pattern_index, *character)
                        && (*character == to_upper(pattern_char)
                            || *character == to_lower(pattern_char))
                } else {
                    equals_ignore_case(*character, pattern_char)
                };
                matches.then_some(index)
            });
        found.map_or(-1, |index| isize::try_from(index).unwrap_or(-1))
    }

    fn matching_degree(
        &self,
        name: &[char],
        value_start_case_match: bool,
        fragments: Option<&[Range<usize>]>,
    ) -> i32 {
        let Some(fragments) = fragments else {
            return i32::MIN;
        };
        let (Some(first), Some(last)) = (fragments.first(), fragments.last()) else {
            return 0;
        };
        let start_match = first.start == 0;
        let valued_start_match = start_match && value_start_case_match;
        let mut matching_case = 0;
        let mut pattern_position: Option<usize> = None;
        let mut skipped_humps = 0;
        let mut next_hump_start = 0;
        let mut hump_start_matched_upper_case = false;
        let mut previous_end = first.start;
        for (fragment_index, range) in fragments.iter().enumerate() {
            skipped_humps += count_skipped_upper_case_inside_word(name, previous_end, range.start);
            previous_end = range.end;
            for index in range.clone() {
                let Some(&character) = name.get(index) else {
                    break;
                };
                let after_gap = index == range.start && fragment_index != 0;
                let mut is_hump_start = false;
                while next_hump_start <= index {
                    if next_hump_start == index {
                        is_hump_start = true;
                    } else if after_gap {
                        skipped_humps += 1;
                    }
                    next_hump_start = if name
                        .get(next_hump_start)
                        .is_some_and(|character| character.is_numeric())
                    {
                        next_hump_start + 1
                    } else {
                        next_word(name, next_hump_start)
                    };
                }
                let search_from = pattern_position.map_or(0, |position| position + 1);
                let Some(position) = self.pattern.iter().enumerate().skip(search_from).find_map(
                    |(position, pattern_character)| {
                        equals_ignore_case(*pattern_character, character).then_some(position)
                    },
                ) else {
                    break;
                };
                pattern_position = Some(position);
                let is_upper = self.is_upper_case[position];
                let is_lower = self.is_lower_case[position];
                if is_hump_start {
                    hump_start_matched_upper_case = character == self.pattern[position] && is_upper;
                }
                matching_case += if after_gap && is_hump_start && is_lower {
                    -10
                } else if character == self.pattern[position] {
                    if is_upper {
                        50
                    } else if index == 0 && valued_start_match {
                        150
                    } else if is_hump_start {
                        1
                    } else {
                        0
                    }
                } else if is_hump_start || (is_lower && hump_start_matched_upper_case) {
                    -1
                } else {
                    0
                };
            }
        }
        let start_index = first.start;
        let word_start = start_index == 0
            || (is_word_start(name, start_index) && !is_word_start(name, start_index - 1));
        let final_match = last.end == name.len();
        let fragment_count = i32::try_from(fragments.len()).unwrap_or(i32::MAX);
        (if word_start { WORD_START_SCORE } else { 0 }) + matching_case
            - fragment_count
            - skipped_humps * SKIPPED_HUMP_PENALTY
            + NOT_AFTER_SEPARATOR_SCORE
            + i32::from(start_match)
            + i32::from(final_match)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_matches_anywhere_in_the_name_ignoring_case() {
        let matcher = BranchesTreeMatcher::new("feat");
        assert_eq!(matcher.match_fragments("feature/login"), Some(vec![0..4]));
        assert_eq!(matcher.match_fragments("my-FEATURE"), Some(vec![3..7]));
        assert_eq!(
            BranchesTreeMatcher::new("login").match_fragments("feature/login"),
            Some(vec![8..13])
        );
        assert_eq!(matcher.match_fragments("main"), None);
    }

    #[test]
    fn words_are_split_on_spaces_and_the_longest_fragments_win() {
        let matcher = BranchesTreeMatcher::new("fea login");
        assert_eq!(matcher.match_fragments("feature/login"), Some(vec![8..13]));
        assert_eq!(matcher.match_fragments("feature/x"), Some(vec![0..3]));
        assert_eq!(matcher.match_fragments("main"), None);
    }

    #[test]
    fn a_blank_pattern_matches_everything_without_fragments() {
        let matcher = BranchesTreeMatcher::new("   ");
        assert_eq!(matcher.match_fragments("main"), Some(Vec::new()));
        assert_eq!(classify(Some(&[])), Matching::Partial);
    }

    #[test]
    fn a_pattern_typed_in_the_russian_layout_is_fixed() {
        let matcher = BranchesTreeMatcher::new("ьфшт");
        assert_eq!(matcher.match_fragments("main"), Some(vec![0..4]));
    }

    #[test]
    fn a_match_at_the_start_of_the_name_has_a_higher_degree() {
        let matcher = BranchesTreeMatcher::new("main");
        assert!(matcher.degree_of("main") > matcher.degree_of("x-main"));
        assert!(matcher.degree_of("x-main") > matcher.degree_of("zz"));
    }

    #[test]
    fn several_words_report_the_partial_degree() {
        let matcher = BranchesTreeMatcher::new("ma in");
        let fragments = matcher.match_fragments("main");
        assert_eq!(
            matcher.matching_degree("main", fragments.as_deref()),
            PARTIAL_MATCH_DEGREE
        );
        assert_eq!(matcher.matching_degree("zzz", None), NO_MATCH_DEGREE);
    }

    #[test]
    fn a_single_fragment_from_the_start_is_a_full_match() {
        assert_eq!(classify(Some(&[0..4])), Matching::Full);
        assert_eq!(classify(Some(&[2..4])), Matching::Partial);
        assert_eq!(classify(Some(&[0..1, 3..4])), Matching::Partial);
        assert_eq!(classify(None), Matching::None);
    }
}
