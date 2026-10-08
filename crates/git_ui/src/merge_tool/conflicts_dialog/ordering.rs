use std::cmp::Ordering;

use git::repository::RepoPath;

const PATH_SEPARATOR: u16 = b'/' as u16;
const SPACE: u16 = b' ' as u16;
const ZERO: u16 = b'0' as u16;
const NINE: u16 = b'9' as u16;
const HYPHEN: u16 = b'-' as u16;
const UNDERSCORE: u16 = b'_' as u16;

fn is_decimal_digit(unit: u16) -> bool {
    (ZERO..=NINE).contains(&unit)
}

fn single_character(mut characters: impl Iterator<Item = char>) -> Option<char> {
    let first = characters.next()?;
    characters.next().is_none().then_some(first)
}

fn to_unit(character: char) -> Option<u16> {
    u16::try_from(u32::from(character)).ok()
}

fn upper_case(unit: u16) -> u16 {
    char::from_u32(u32::from(unit))
        .and_then(|character| single_character(character.to_uppercase()))
        .and_then(to_unit)
        .unwrap_or(unit)
}

fn lower_case(unit: u16) -> u16 {
    char::from_u32(u32::from(unit))
        .and_then(|character| single_character(character.to_lowercase()))
        .and_then(to_unit)
        .unwrap_or(unit)
}

fn compare_units(left: u16, right: u16, ignore_case: bool) -> i32 {
    let difference = i32::from(left) - i32::from(right);
    if difference == 0 || !ignore_case {
        return difference;
    }
    let upper_difference = i32::from(upper_case(left)) - i32::from(upper_case(right));
    if upper_difference == 0 {
        return 0;
    }
    i32::from(lower_case(upper_case(left))) - i32::from(lower_case(upper_case(right)))
}

fn units_match(left: u16, right: u16, ignore_case: bool) -> bool {
    compare_units(left, right, ignore_case) == 0
}

fn compare_natural_units(left: u16, right: u16, ignore_case: bool) -> i32 {
    if left == SPACE && right > SPACE && right < ZERO {
        return 1;
    }
    if right == SPACE && left > SPACE && left < ZERO {
        return -1;
    }
    compare_units(left, right, ignore_case)
}

fn skip_units(units: &[u16], start: usize, end: usize, skipped: u16) -> usize {
    let mut index = start;
    while index < end && units[index] == skipped {
        index += 1;
    }
    index
}

fn skip_digits(units: &[u16], start: usize, end: usize) -> usize {
    let mut index = start;
    while index < end && is_decimal_digit(units[index]) {
        index += 1;
    }
    index
}

fn compare_unit_range(
    left: &[u16],
    right: &[u16],
    left_offset: usize,
    right_offset: usize,
    left_end: usize,
) -> i32 {
    let mut left_index = left_offset;
    let mut right_index = right_offset;
    while left_index < left_end {
        let difference = i32::from(left[left_index]) - i32::from(right[right_index]);
        if difference != 0 {
            return difference;
        }
        left_index += 1;
        right_index += 1;
    }
    0
}

fn natural_compare_units(
    left: &[u16],
    right: &[u16],
    ignore_case: bool,
    like_file_names: bool,
) -> i32 {
    let left_length = left.len();
    let right_length = right.len();
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left_length && right_index < right_length {
        let left_unit = left[left_index];
        let right_unit = right[right_index];
        let left_numeric = is_decimal_digit(left_unit) || left_unit == SPACE;
        let right_numeric = is_decimal_digit(right_unit) || right_unit == SPACE;
        if left_numeric && right_numeric {
            let left_start = skip_units(
                left,
                skip_units(left, left_index, left_length, SPACE),
                left_length,
                ZERO,
            );
            let right_start = skip_units(
                right,
                skip_units(right, right_index, right_length, SPACE),
                right_length,
                ZERO,
            );
            let left_end = skip_digits(left, left_start, left_length);
            let right_end = skip_digits(right, right_start, right_length);

            let digit_count_difference =
                (left_end - left_start) as i32 - (right_end - right_start) as i32;
            if digit_count_difference != 0 {
                return digit_count_difference;
            }

            let number_difference =
                compare_unit_range(left, right, left_start, right_start, left_end);
            if number_difference != 0 {
                return number_difference;
            }

            let full_length_difference =
                (left_end - left_index) as i32 - (right_end - right_index) as i32;
            if full_length_difference != 0 {
                return full_length_difference;
            }

            let leading_difference =
                compare_unit_range(left, right, left_index, right_index, left_start);
            if leading_difference != 0 {
                return leading_difference;
            }

            left_index = left_end - 1;
            right_index = right_end - 1;
        } else if like_file_names {
            if left_unit != right_unit {
                let difference = if left_unit == HYPHEN && right_unit != UNDERSCORE {
                    compare_natural_units(UNDERSCORE, right_unit, ignore_case)
                } else if right_unit == HYPHEN && left_unit != UNDERSCORE {
                    compare_natural_units(left_unit, UNDERSCORE, ignore_case)
                } else {
                    compare_natural_units(left_unit, right_unit, ignore_case)
                };
                if difference != 0 {
                    return difference;
                }
            }
        } else {
            let difference = compare_natural_units(left_unit, right_unit, ignore_case);
            if difference != 0 {
                return difference;
            }
        }
        left_index += 1;
        right_index += 1;
    }
    if left_index < left_length {
        return 1;
    }
    if right_index < right_length {
        return -1;
    }
    if left_length != right_length {
        return left_length as i32 - right_length as i32;
    }
    if ignore_case {
        natural_compare_units(left, right, false, like_file_names)
    } else {
        0
    }
}

pub(crate) fn natural_compare(left: &str, right: &str) -> Ordering {
    let left_units: Vec<u16> = left.encode_utf16().collect();
    let right_units: Vec<u16> = right.encode_utf16().collect();
    natural_compare_units(&left_units, &right_units, true, true).cmp(&0)
}

fn common_prefix_length(left: &[u16], right: &[u16]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left_unit, right_unit)| units_match(**left_unit, **right_unit, true))
        .count()
}

fn index_of_separator(units: &[u16], start: usize) -> Option<usize> {
    units
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, unit)| **unit == PATH_SEPARATOR)
        .map(|(index, _)| index)
}

fn segment_start(units: &[u16], common_prefix: usize) -> usize {
    units[..common_prefix]
        .iter()
        .rposition(|unit| *unit == PATH_SEPARATOR)
        .map_or(0, |index| index + 1)
}

pub(crate) fn hierarchical_compare(left: &RepoPath, right: &RepoPath) -> Ordering {
    let left_units: Vec<u16> = left.as_unix_str().encode_utf16().collect();
    let right_units: Vec<u16> = right.as_unix_str().encode_utf16().collect();
    let common_prefix = common_prefix_length(&left_units, &right_units);

    if common_prefix == left_units.len() && common_prefix == right_units.len() {
        return Ordering::Equal;
    }
    if common_prefix == left_units.len() && right_units[common_prefix] == PATH_SEPARATOR {
        return Ordering::Greater;
    }
    if common_prefix == right_units.len() && left_units[common_prefix] == PATH_SEPARATOR {
        return Ordering::Less;
    }

    let start = segment_start(&left_units, common_prefix);
    let left_end = index_of_separator(&left_units, start);
    let right_end = index_of_separator(&right_units, start);
    match (left_end.is_some(), right_end.is_some()) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let left_name = &left_units[start..left_end.unwrap_or(left_units.len())];
    let right_name = &right_units[start..right_end.unwrap_or(right_units.len())];
    natural_compare_units(left_name, right_name, true, true).cmp(&0)
}

pub(crate) fn file_compare(left: &RepoPath, right: &RepoPath, flattened: bool) -> Ordering {
    if flattened {
        let name_ordering = natural_compare(
            left.file_name().unwrap_or_default(),
            right.file_name().unwrap_or_default(),
        );
        if name_ordering != Ordering::Equal {
            return name_ordering;
        }
    }
    hierarchical_compare(left, right)
}

pub(crate) fn stable_sort_by<T>(items: &mut Vec<T>, compare: &mut dyn FnMut(&T, &T) -> Ordering) {
    if items.len() < 2 {
        return;
    }
    let middle = items.len() / 2;
    let mut right_half = items.split_off(middle);
    stable_sort_by(items, compare);
    stable_sort_by(&mut right_half, compare);
    let left_half = std::mem::take(items);
    let mut merged = Vec::with_capacity(left_half.len() + right_half.len());
    let mut left_iter = left_half.into_iter().peekable();
    let mut right_iter = right_half.into_iter().peekable();
    loop {
        let take_right = match (left_iter.peek(), right_iter.peek()) {
            (Some(left), Some(right)) => compare(right, left) == Ordering::Less,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => break,
        };
        let next = if take_right {
            right_iter.next()
        } else {
            left_iter.next()
        };
        merged.extend(next);
    }
    *items = merged;
}

pub(crate) fn sorted_files(files: &[RepoPath], flattened: bool) -> Vec<RepoPath> {
    let mut sorted = files.to_vec();
    stable_sort_by(&mut sorted, &mut |left, right| {
        file_compare(left, right, flattened)
    });
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).expect("valid repo path")
    }

    #[test]
    fn merge_tool_natural_compare_orders_digit_runs_numerically() {
        assert_eq!(natural_compare("file2.rs", "file10.rs"), Ordering::Less);
        assert_eq!(natural_compare("file10.rs", "file2.rs"), Ordering::Greater);
    }

    #[test]
    fn merge_tool_natural_compare_puts_shorter_leading_zero_run_first() {
        assert_eq!(natural_compare("a7", "a007"), Ordering::Less);
        assert_eq!(natural_compare("a007", "a7"), Ordering::Greater);
        assert_eq!(natural_compare("a07", "a7"), Ordering::Greater);
    }

    #[test]
    fn merge_tool_natural_compare_ignores_case_before_breaking_ties() {
        assert_eq!(natural_compare("Beta", "alpha"), Ordering::Greater);
        assert_eq!(natural_compare("alpha", "Beta"), Ordering::Less);
        assert_eq!(natural_compare("Alpha", "alpha"), Ordering::Less);
        assert_eq!(natural_compare("alpha", "Alpha"), Ordering::Greater);
        assert_eq!(natural_compare("alpha", "alpha"), Ordering::Equal);
    }

    #[test]
    fn merge_tool_natural_compare_puts_prefix_first() {
        assert_eq!(natural_compare("abc", "abcd"), Ordering::Less);
        assert_eq!(natural_compare("abcd", "abc"), Ordering::Greater);
    }

    #[test]
    fn merge_tool_natural_compare_treats_hyphen_like_underscore_for_file_names() {
        assert_eq!(natural_compare("a-b", "a_b"), Ordering::Less);
        assert_eq!(natural_compare("a-b", "a.b"), Ordering::Greater);
        assert_eq!(natural_compare("a-b", "aab"), Ordering::Less);
        assert_eq!(natural_compare("a-b", "abb"), Ordering::Less);
        assert_eq!(natural_compare("a_b", "a.b"), Ordering::Greater);
    }

    #[test]
    fn merge_tool_natural_compare_treats_spaces_inside_numbers_like_leading_zeros() {
        assert_eq!(natural_compare("a 5", "a5"), Ordering::Greater);
        assert_eq!(natural_compare("a  5", "a 5"), Ordering::Greater);
        assert_eq!(natural_compare("a 5", "a6"), Ordering::Less);
    }

    #[test]
    fn merge_tool_hierarchical_compare_puts_directories_before_files() {
        assert_eq!(
            hierarchical_compare(&path("z/a.rs"), &path("b.rs")),
            Ordering::Less
        );
        assert_eq!(
            hierarchical_compare(&path("b.rs"), &path("z/a.rs")),
            Ordering::Greater
        );
        assert_eq!(
            hierarchical_compare(&path("a/z.rs"), &path("b/a.rs")),
            Ordering::Less
        );
    }

    #[test]
    fn merge_tool_hierarchical_compare_orders_siblings_by_natural_name() {
        assert_eq!(
            hierarchical_compare(&path("src/file2.rs"), &path("src/file10.rs")),
            Ordering::Less
        );
        assert_eq!(
            hierarchical_compare(&path("src/B/x.rs"), &path("src/a/x.rs")),
            Ordering::Greater
        );
    }

    #[test]
    fn merge_tool_hierarchical_compare_ignores_case_when_matching_the_common_prefix() {
        assert_eq!(
            hierarchical_compare(&path("Src/a.rs"), &path("src/a.rs")),
            Ordering::Equal
        );
        assert_eq!(
            hierarchical_compare(&path("Src/a.rs"), &path("src/b.rs")),
            Ordering::Less
        );
    }

    #[test]
    fn merge_tool_hierarchical_compare_puts_a_file_after_a_directory_of_the_same_name() {
        assert_eq!(
            hierarchical_compare(&path("a"), &path("a/b")),
            Ordering::Greater
        );
        assert_eq!(
            hierarchical_compare(&path("a/b"), &path("a")),
            Ordering::Less
        );
    }

    #[test]
    fn merge_tool_flat_order_sorts_by_name_then_path() {
        let files = vec![path("z/a.rs"), path("a/b.rs"), path("m/a.rs"), path("c.rs")];
        assert_eq!(
            sorted_files(&files, true),
            vec![path("m/a.rs"), path("z/a.rs"), path("a/b.rs"), path("c.rs")]
        );
    }

    #[test]
    fn merge_tool_tree_order_lists_directories_before_files() {
        let files = vec![path("z/a.rs"), path("a/b.rs"), path("m/a.rs"), path("a.rs")];
        assert_eq!(
            sorted_files(&files, false),
            vec![path("a/b.rs"), path("m/a.rs"), path("z/a.rs"), path("a.rs")]
        );
    }

    #[test]
    fn merge_tool_stable_sort_keeps_insertion_order_of_equal_keys() {
        let mut items = vec![(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd'), (3, 'e'), (2, 'f')];
        stable_sort_by(
            &mut items,
            &mut |left: &(i32, char), right: &(i32, char)| left.0.cmp(&right.0),
        );
        assert_eq!(
            items,
            vec![(1, 'b'), (1, 'd'), (2, 'a'), (2, 'c'), (2, 'f'), (3, 'e')]
        );
    }
}
