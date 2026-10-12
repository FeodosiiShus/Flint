use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use collections::HashSet;

use crate::git_log::properties::{GraphOptions, LogColumn, USER_FILTER_ME, USER_FILTER_ME_LABEL};

pub(super) const BRANCH_FILTER_NAME: &str = "Branch";
pub(super) const USER_FILTER_NAME: &str = "User";
pub(super) const PATHS_FILTER_NAME: &str = "Paths";
pub(super) const LOCAL_GROUP_NAME: &str = "Local";
pub(super) const MAX_FILTER_VALUE_LENGTH: usize = 20;
pub(super) const SEARCH_HISTORY_SIZE: usize = 5;
pub(super) const SECONDS_PER_DAY: i64 = 86_400;
pub(super) const VALUE_SEPARATORS: [char; 2] = ['|', '\n'];
pub(super) const PATH_SEPARATORS: [char; 1] = ['\n'];
pub(super) const DYNAMIC_COLUMNS: [LogColumn; 3] =
    [LogColumn::Author, LogColumn::Hash, LogColumn::Date];
pub(super) const REFERENCE_DATE: (i32, u8, u8) = (2003, 11, 25);

pub(super) const TOOLBAR_VERTICAL_INSET: f32 = 5.;
pub(super) const TOOLBAR_HORIZONTAL_INSET: f32 = 7.;
pub(super) const BUTTON_SIZE: f32 = 22.;
pub(super) const BUTTON_ORTHOGONAL_GAP: f32 = 1.;
pub(super) const BUTTON_DIRECTIONAL_GAP: f32 = 2.;
pub(super) const SEPARATOR_HEIGHT: f32 = 24.;
pub(super) const CHIP_INSET: f32 = 4.;

const FILTER_LABEL_LENGTH: usize = 30;
const RECENTLY_FILTERED_VALUES_LIMIT: usize = 10;
const MAX_TOOLTIP_PATHS: usize = 10;
const ELLIPSIS: &str = "\u{2026}";
const THREE_DOTS: &str = "...";
const REGEXP_CHARACTERS: &str = "()[]{}.*?+^$\\|";
const ALL_TEXT: &str = "All";
const PATHS_TOOLTIP_HEADER: &str = "Paths:";

pub(super) fn toolbar_height(label_height: f32) -> f32 {
    let chip = label_height + 2. * CHIP_INSET;
    let button = BUTTON_SIZE + 2. * BUTTON_ORTHOGONAL_GAP;
    chip.max(button).max(SEPARATOR_HEIGHT) + 2. * TOOLBAR_VERTICAL_INSET
}

pub(super) fn shorten_text_with_ellipsis(
    text: &str,
    max_length: usize,
    suffix_length: usize,
    use_ellipsis_symbol: bool,
) -> String {
    let symbol = if use_ellipsis_symbol {
        ELLIPSIS
    } else {
        THREE_DOTS
    };
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= max_length {
        return text.to_string();
    }
    let prefix_length = max_length.saturating_sub(suffix_length + symbol.chars().count());
    let mut result: String = characters.iter().take(prefix_length).collect();
    result.push_str(symbol);
    result.extend(
        characters
            .iter()
            .skip(characters.len().saturating_sub(suffix_length)),
    );
    result
}

pub(super) fn shorten_path_with_ellipsis(path: &str, max_length: usize) -> String {
    shorten_text_with_ellipsis(path, max_length, max_length * 7 / 10, false)
}

pub(super) fn displayable_text(values: &[String], max_length: usize) -> String {
    let text = match values {
        [single] => single.clone(),
        _ => values.join("|"),
    };
    shorten_text_with_ellipsis(&text, max_length, 0, true)
}

pub(super) fn action_name(values: &[String]) -> String {
    match values {
        [single] => single.clone(),
        _ => displayable_text(values, 2 * MAX_FILTER_VALUE_LENGTH),
    }
}

pub(super) fn values_tooltip(values: &[String]) -> String {
    values.join(", ")
}

pub(super) fn chip_name(display_name: &str, value_selected: bool) -> String {
    if value_selected {
        format!("{display_name}: ")
    } else {
        display_name.to_string()
    }
}

pub(super) fn localized_users(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| {
            if value == USER_FILTER_ME {
                USER_FILTER_ME_LABEL.to_string()
            } else {
                value.clone()
            }
        })
        .collect()
}

pub(super) fn parse_localized_users(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| {
            if value == USER_FILTER_ME_LABEL {
                USER_FILTER_ME.to_string()
            } else {
                value.clone()
            }
        })
        .collect()
}

pub(super) fn sorted_values(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

pub(super) fn is_same_filter_as(field_text: &str, model_text: &str) -> bool {
    if field_text.trim().is_empty() {
        return model_text.trim().is_empty();
    }
    field_text == model_text
}

pub(super) fn text_filter_value(text: &str) -> String {
    if text.trim().is_empty() {
        String::new()
    } else {
        text.to_string()
    }
}

pub(super) fn add_to_search_history(history: &mut Vec<String>, item: &str) -> bool {
    let item = item.trim();
    if item.is_empty() {
        return false;
    }
    let lowercase_item = item.to_lowercase();
    match history
        .iter()
        .position(|entry| entry.to_lowercase() == lowercase_item)
    {
        Some(0) => return false,
        Some(index) => {
            history.remove(index);
        }
        None => {
            if history.len() >= SEARCH_HISTORY_SIZE {
                history.pop();
            }
        }
    }
    history.insert(0, item.to_string());
    true
}

pub(super) fn add_recent_group(groups: &mut Vec<Vec<String>>, values: Vec<String>) {
    groups.retain(|group| !have_equal_elements(group, &values));
    groups.insert(0, values);
    groups.truncate(RECENTLY_FILTERED_VALUES_LIMIT);
}

fn have_equal_elements(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let left: HashSet<&String> = left.iter().collect();
    let right: HashSet<&String> = right.iter().collect();
    left == right
}

pub(super) fn parse_values(text: &str, separators: &[char]) -> Vec<String> {
    text.split(|character: char| separators.contains(&character))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn separators_text(separators: &[char]) -> String {
    let mut text = String::new();
    for separator in separators {
        let separator = if *separator == '\n' {
            "new lines".to_string()
        } else {
            separator.to_string()
        };
        if text.is_empty() {
            text.push_str(&separator);
        } else {
            text.push_str(&format!(" or {separator}"));
        }
    }
    text
}

pub(super) fn completion_token_start(text: &str, separators: &[&str]) -> usize {
    separators
        .iter()
        .filter_map(|separator| text.rfind(separator).map(|index| index + separator.len()))
        .max()
        .unwrap_or(0)
}

pub(super) fn is_regexp(text: &str) -> bool {
    text.chars()
        .any(|character| REGEXP_CHARACTERS.contains(character))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BranchRefKind {
    Head,
    Local,
    Remote(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BranchRef {
    pub name: String,
    pub kind: BranchRefKind,
    pub favorite: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BranchPopupGroups {
    pub recent: Vec<Vec<String>>,
    pub favorites: Vec<String>,
    pub others: Vec<(String, Vec<String>)>,
}

pub(super) fn branch_popup_groups(
    refs: &[BranchRef],
    recent: Vec<Vec<String>>,
) -> BranchPopupGroups {
    let mut favorites = BTreeSet::new();
    let mut others: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for reference in refs {
        let group = match &reference.kind {
            BranchRefKind::Head => {
                favorites.insert(reference.name.clone());
                continue;
            }
            BranchRefKind::Local => LOCAL_GROUP_NAME.to_string(),
            BranchRefKind::Remote(remote) => format!("{remote}/..."),
        };
        if reference.favorite {
            favorites.insert(reference.name.clone());
        }
        others
            .entry(group)
            .or_default()
            .push(reference.name.clone());
    }
    let others = others
        .into_iter()
        .map(|(group, mut names)| {
            names.sort_by(|left, right| natural_compare(left, right));
            names.dedup();
            (group, names)
        })
        .collect();
    BranchPopupGroups {
        recent,
        favorites: favorites.into_iter().collect(),
        others,
    }
}

pub(super) fn process_recent_branch_filters(
    available_branches: &HashSet<String>,
    recent: &[Vec<String>],
) -> Vec<Vec<String>> {
    if available_branches.is_empty() {
        return recent.to_vec();
    }
    recent
        .iter()
        .filter(|group| {
            group.iter().any(|value| {
                let name = value.strip_prefix('-').unwrap_or(value);
                available_branches.contains(name) || is_regexp(name)
            })
        })
        .cloned()
        .collect()
}

pub(super) fn date_presets(now: i64) -> [i64; 2] {
    [now - SECONDS_PER_DAY, now - 7 * SECONDS_PER_DAY]
}

pub(super) fn date_chip_text(
    after: Option<i64>,
    before: Option<i64>,
    format: impl Fn(i64) -> String,
) -> String {
    match (after, before) {
        (Some(after), Some(before)) => format!("{}-{}", format(after), format(before)),
        (Some(after), None) => format!("Since {}", format(after)),
        (None, Some(before)) => format!("Until {}", format(before)),
        (None, None) => String::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateField {
    Year,
    Month,
    Day,
}

const DEFAULT_DATE_ORDER: [DateField; 3] = [DateField::Year, DateField::Month, DateField::Day];

fn digit_groups(text: &str) -> Vec<&str> {
    text.split(|character: char| !character.is_ascii_digit())
        .filter(|group| !group.is_empty())
        .collect()
}

pub(super) fn date_field_order(formatted_reference: &str) -> [DateField; 3] {
    let fields: Vec<DateField> = digit_groups(formatted_reference)
        .into_iter()
        .filter_map(|group| match group {
            "25" => Some(DateField::Day),
            "11" => Some(DateField::Month),
            "03" | "2003" => Some(DateField::Year),
            _ => None,
        })
        .collect();
    match fields.as_slice() {
        [first, second, third] if first != second && second != third && first != third => {
            [*first, *second, *third]
        }
        _ => DEFAULT_DATE_ORDER,
    }
}

pub(super) fn parse_date(
    text: &str,
    order: [DateField; 3],
    current_year: i32,
) -> Option<(i32, u8, u8)> {
    let groups = digit_groups(text);
    if groups.len() != 3 {
        return None;
    }
    let mut year = None;
    let mut month = None;
    let mut day = None;
    for (field, group) in order.iter().zip(groups) {
        match field {
            DateField::Year => {
                let value: i32 = group.parse().ok()?;
                year = Some(if group.len() <= 2 {
                    expand_two_digit_year(value, current_year)
                } else {
                    value
                });
            }
            DateField::Month => month = Some(group.parse::<u8>().ok()?),
            DateField::Day => day = Some(group.parse::<u8>().ok()?),
        }
    }
    let (year, month, day) = (year?, month?, day?);
    let calendar_month = time::Month::try_from(month).ok()?;
    time::Date::from_calendar_date(year, calendar_month, day).ok()?;
    Some((year, month, day))
}

fn expand_two_digit_year(value: i32, current_year: i32) -> i32 {
    let year = current_year / 100 * 100 + value;
    if year > current_year + 20 {
        year - 100
    } else if year <= current_year - 80 {
        year + 100
    } else {
        year
    }
}

fn path_order(left: &str, right: &str) -> Ordering {
    left.trim_end_matches('/').cmp(right.trim_end_matches('/'))
}

pub(super) fn structure_text(paths: &[String]) -> String {
    match paths.iter().min_by(|left, right| path_order(left, right)) {
        None => ALL_TEXT.to_string(),
        Some(first) => {
            let name = shorten_path_with_ellipsis(first, FILTER_LABEL_LENGTH);
            if paths.len() == 1 {
                name
            } else {
                format!("{name} + {}", paths.len() - 1)
            }
        }
    }
}

pub(super) fn structure_tooltip(paths: &[String]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let mut sorted = paths.to_vec();
    sorted.sort_by(|left, right| path_order(left, right));
    let mut lines = vec![PATHS_TOOLTIP_HEADER.to_string()];
    lines.extend(sorted.into_iter().take(MAX_TOOLTIP_PATHS));
    if paths.len() > MAX_TOOLTIP_PATHS {
        lines.push(THREE_DOTS.to_string());
    }
    Some(lines.join("\n"))
}

pub(super) fn naturally_sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort_by(|left, right| natural_compare(left, right));
    values
}

pub(super) fn updated<T: Clone + PartialEq>(current: &T, update: impl FnOnce(&mut T)) -> Option<T> {
    let mut next = current.clone();
    update(&mut next);
    (next != *current).then_some(next)
}

pub(super) fn toggle_column(order: &[LogColumn], column: LogColumn) -> Vec<LogColumn> {
    if order.contains(&column) {
        order
            .iter()
            .copied()
            .filter(|existing| *existing != column)
            .collect()
    } else {
        let mut next = order.to_vec();
        next.push(column);
        next
    }
}

pub(super) fn first_parent_toggled(options: GraphOptions) -> GraphOptions {
    if options == GraphOptions::FirstParent {
        GraphOptions::default()
    } else {
        GraphOptions::FirstParent
    }
}

pub(super) fn natural_compare(left: &str, right: &str) -> Ordering {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    natural_compare_chars(&left, &right, true)
}

fn natural_compare_chars(left: &[char], right: &[char], ignore_case: bool) -> Ordering {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        let left_char = left[left_index];
        let right_char = right[right_index];
        let left_numeric = left_char.is_ascii_digit() || left_char == ' ';
        let right_numeric = right_char.is_ascii_digit() || right_char == ' ';
        if left_numeric && right_numeric {
            let left_start = skip_char(left, skip_char(left, left_index, ' '), '0');
            let right_start = skip_char(right, skip_char(right, right_index, ' '), '0');
            let left_end = skip_digits(left, left_start);
            let right_end = skip_digits(right, right_start);
            let digits = (left_end - left_start).cmp(&(right_end - right_start));
            if digits != Ordering::Equal {
                return digits;
            }
            let number = compare_char_range(left, right, left_start, right_start, left_end);
            if number != Ordering::Equal {
                return number;
            }
            let full_length = (left_end - left_index).cmp(&(right_end - right_index));
            if full_length != Ordering::Equal {
                return full_length;
            }
            let leading = compare_char_range(left, right, left_index, right_index, left_start);
            if leading != Ordering::Equal {
                return leading;
            }
            left_index = left_end - 1;
            right_index = right_end - 1;
        } else {
            let difference = compare_chars(left_char, right_char, ignore_case);
            if difference != Ordering::Equal {
                return difference;
            }
        }
        left_index += 1;
        right_index += 1;
    }
    if left_index < left.len() {
        return Ordering::Greater;
    }
    if right_index < right.len() {
        return Ordering::Less;
    }
    if left.len() != right.len() {
        return left.len().cmp(&right.len());
    }
    if ignore_case {
        natural_compare_chars(left, right, false)
    } else {
        Ordering::Equal
    }
}

fn skip_char(text: &[char], start: usize, character: char) -> usize {
    let mut index = start;
    while text.get(index) == Some(&character) {
        index += 1;
    }
    index
}

fn skip_digits(text: &[char], start: usize) -> usize {
    let mut index = start;
    while text.get(index).is_some_and(char::is_ascii_digit) {
        index += 1;
    }
    index
}

fn compare_char_range(
    left: &[char],
    right: &[char],
    left_offset: usize,
    right_offset: usize,
    left_end: usize,
) -> Ordering {
    for (left_char, right_char) in left
        .iter()
        .take(left_end)
        .skip(left_offset)
        .zip(right.iter().skip(right_offset))
    {
        let ordering = left_char.cmp(right_char);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn compare_chars(left: char, right: char, ignore_case: bool) -> Ordering {
    if left == ' ' && right > ' ' && right < '0' {
        return Ordering::Greater;
    }
    if right == ' ' && left > ' ' && left < '0' {
        return Ordering::Less;
    }
    if left == right || !ignore_case {
        return left.cmp(&right);
    }
    let left_upper = single_mapping(left, char::to_uppercase);
    let right_upper = single_mapping(right, char::to_uppercase);
    if left_upper == right_upper {
        return Ordering::Equal;
    }
    single_mapping(left_upper, char::to_lowercase)
        .cmp(&single_mapping(right_upper, char::to_lowercase))
}

fn single_mapping<I: Iterator<Item = char>>(character: char, mapping: impl Fn(char) -> I) -> char {
    let mut mapped = mapping(character);
    match (mapped.next(), mapped.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_log::properties::{GraphSortType, LogFilterState, LogTabProperties};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn chip_text_joins_multiple_values_with_a_bar_and_shortens_to_twenty_characters() {
        assert_eq!(
            displayable_text(&strings(&["HEAD"]), MAX_FILTER_VALUE_LENGTH),
            "HEAD"
        );
        assert_eq!(
            displayable_text(&strings(&["main", "origin/main"]), MAX_FILTER_VALUE_LENGTH),
            "main|origin/main"
        );
        assert_eq!(
            displayable_text(
                &strings(&["feature/login", "feature/logout"]),
                MAX_FILTER_VALUE_LENGTH
            ),
            "feature/login|featu\u{2026}"
        );
        assert_eq!(
            displayable_text(
                &strings(&["a-very-long-branch-name-x"]),
                MAX_FILTER_VALUE_LENGTH
            )
            .chars()
            .count(),
            MAX_FILTER_VALUE_LENGTH
        );
        assert_eq!(
            action_name(&strings(&["only-one-value-that-is-long-enough"])),
            "only-one-value-that-is-long-enough"
        );
    }

    #[test]
    fn chip_name_gets_a_colon_only_when_a_value_is_selected() {
        assert_eq!(chip_name("Branch", true), "Branch: ");
        assert_eq!(chip_name("User", false), "User");
    }

    #[test]
    fn me_is_stored_as_star_and_shown_as_me() {
        assert_eq!(
            localized_users(&strings(&["*", "Ivan Ivanov"])),
            strings(&["me", "Ivan Ivanov"])
        );
        assert_eq!(
            parse_localized_users(&strings(&["me", "Ivan Ivanov"])),
            strings(&["*", "Ivan Ivanov"])
        );
    }

    #[test]
    fn date_presets_and_chip_text_follow_date_filter_popup_component() {
        let now = 1_700_000_000;
        assert_eq!(date_presets(now), [now - 86_400, now - 7 * 86_400]);
        let format = |seconds: i64| seconds.to_string();
        assert_eq!(date_chip_text(Some(1), Some(2), format), "1-2");
        assert_eq!(date_chip_text(Some(1), None, format), "Since 1");
        assert_eq!(date_chip_text(None, Some(2), format), "Until 2");
        assert_eq!(date_chip_text(None, None, format), "");
    }

    #[test]
    fn date_fields_follow_the_locale_short_format_and_two_digit_years_expand_like_simple_date_format()
     {
        let us = date_field_order("11/25/03");
        assert_eq!(us, [DateField::Month, DateField::Day, DateField::Year]);
        assert_eq!(parse_date("2/29/24", us, 2026), Some((2024, 2, 29)));
        assert_eq!(parse_date("2/30/24", us, 2026), None);
        assert_eq!(parse_date("1/2/50", us, 2026), Some((1950, 1, 2)));
        let german = date_field_order("25.11.2003");
        assert_eq!(german, [DateField::Day, DateField::Month, DateField::Year]);
        assert_eq!(parse_date("01.12.2025", german, 2026), Some((2025, 12, 1)));
        assert_eq!(date_field_order("Nov 25"), DEFAULT_DATE_ORDER);
    }

    #[test]
    fn search_history_keeps_five_unique_items_most_recent_first() {
        let mut history = Vec::new();
        for item in ["a", "b", "c", "d", "e", "f"] {
            assert!(add_to_search_history(&mut history, item));
        }
        assert_eq!(history, strings(&["f", "e", "d", "c", "b"]));
        assert!(!add_to_search_history(&mut history, "F"));
        assert!(add_to_search_history(&mut history, " C "));
        assert_eq!(history, strings(&["C", "f", "e", "d", "b"]));
        assert!(!add_to_search_history(&mut history, "  "));
    }

    #[test]
    fn recent_groups_move_equal_groups_to_the_top_and_keep_ten() {
        let mut groups = Vec::new();
        for index in 0..12 {
            add_recent_group(&mut groups, vec![index.to_string()]);
        }
        assert_eq!(groups.len(), 10);
        assert_eq!(groups[0], strings(&["11"]));
        add_recent_group(&mut groups, strings(&["a", "b"]));
        add_recent_group(&mut groups, strings(&["5"]));
        add_recent_group(&mut groups, strings(&["b", "a"]));
        assert_eq!(groups[0], strings(&["b", "a"]));
        assert_eq!(groups[1], strings(&["5"]));
        assert_eq!(groups.iter().filter(|group| group.len() == 2).count(), 1);
    }

    #[test]
    fn branch_groups_put_head_and_favorites_first_and_group_remotes_by_remote() {
        let refs = vec![
            BranchRef {
                name: "HEAD".into(),
                kind: BranchRefKind::Head,
                favorite: true,
            },
            BranchRef {
                name: "main".into(),
                kind: BranchRefKind::Local,
                favorite: true,
            },
            BranchRef {
                name: "feature10".into(),
                kind: BranchRefKind::Local,
                favorite: false,
            },
            BranchRef {
                name: "feature9".into(),
                kind: BranchRefKind::Local,
                favorite: false,
            },
            BranchRef {
                name: "origin/main".into(),
                kind: BranchRefKind::Remote("origin".into()),
                favorite: true,
            },
            BranchRef {
                name: "upstream/dev".into(),
                kind: BranchRefKind::Remote("upstream".into()),
                favorite: false,
            },
        ];
        let groups = branch_popup_groups(&refs, Vec::new());
        assert_eq!(groups.favorites, strings(&["HEAD", "main", "origin/main"]));
        assert_eq!(
            groups.others,
            vec![
                (
                    "Local".to_string(),
                    strings(&["feature9", "feature10", "main"])
                ),
                ("origin/...".to_string(), strings(&["origin/main"])),
                ("upstream/...".to_string(), strings(&["upstream/dev"])),
            ]
        );
    }

    #[test]
    fn recent_branch_filters_without_existing_branches_or_patterns_are_dropped() {
        let available: HashSet<String> = ["main".to_string()].into_iter().collect();
        let recent = vec![
            strings(&["gone"]),
            strings(&["-main"]),
            strings(&["feature/.*"]),
            strings(&["gone", "main"]),
        ];
        assert_eq!(
            process_recent_branch_filters(&available, &recent),
            vec![
                strings(&["-main"]),
                strings(&["feature/.*"]),
                strings(&["gone", "main"])
            ]
        );
        assert_eq!(
            process_recent_branch_filters(&HashSet::default(), &recent),
            recent
        );
    }

    #[test]
    fn values_are_split_on_separators_trimmed_and_blank_values_dropped() {
        assert_eq!(
            parse_values(" main |\norigin/x \n\n|", &VALUE_SEPARATORS),
            strings(&["main", "origin/x"])
        );
        assert_eq!(
            parse_values("a|b\nc", &PATH_SEPARATORS),
            strings(&["a|b", "c"])
        );
        assert_eq!(separators_text(&VALUE_SEPARATORS), "| or new lines");
        assert_eq!(separators_text(&PATH_SEPARATORS), "new lines");
        assert_eq!(completion_token_start("main|orig", &["|", "\n", ".."]), 5);
        assert_eq!(completion_token_start("a..b", &["|", "\n", ".."]), 3);
    }

    #[test]
    fn paths_chip_shows_the_first_path_and_the_number_of_others() {
        assert_eq!(structure_text(&[]), "All");
        assert_eq!(structure_text(&strings(&["src/"])), "src/");
        assert_eq!(
            structure_text(&strings(&["src/b.rs", "src/a.rs"])),
            "src/a.rs + 1"
        );
        assert_eq!(
            structure_text(&strings(&[
                "a/very/long/path/that/does/not/fit/in/thirty.rs"
            ])),
            "a/very.../not/fit/in/thirty.rs"
        );
        assert_eq!(structure_tooltip(&[]), None);
        assert_eq!(
            structure_tooltip(&strings(&["b", "a/"])),
            Some("Paths:\na/\nb".to_string())
        );
    }

    #[test]
    fn text_filter_applies_only_when_the_field_differs_from_the_model() {
        assert!(is_same_filter_as("  ", ""));
        assert!(!is_same_filter_as("fix", ""));
        assert!(is_same_filter_as("fix", "fix"));
        assert_eq!(text_filter_value("   "), "");
        assert_eq!(text_filter_value(" fix"), " fix");
    }

    #[test]
    fn filter_transitions_report_new_properties_only_when_something_changed() {
        let tab = LogTabProperties::default();
        let changed = updated(&tab, |tab| tab.filters.branches = strings(&["HEAD"]));
        assert_eq!(
            changed.as_ref().map(|tab| tab.filters.branches.clone()),
            Some(strings(&["HEAD"]))
        );
        let changed = changed.unwrap_or_default();
        assert_eq!(
            updated(&changed, |tab| tab.filters.branches = strings(&["HEAD"])),
            None
        );
        let cleared = updated(&changed, |tab| tab.filters = LogFilterState::default());
        assert!(cleared.is_some_and(|tab| tab.filters.is_empty()));
        assert_eq!(
            first_parent_toggled(GraphOptions::Base(GraphSortType::Bek)),
            GraphOptions::FirstParent
        );
        assert_eq!(
            first_parent_toggled(GraphOptions::FirstParent),
            GraphOptions::default()
        );
    }

    #[test]
    fn columns_are_appended_when_shown_and_removed_when_hidden() {
        let order = LogColumn::DEFAULT_ORDER.to_vec();
        let shown = toggle_column(&order, LogColumn::Hash);
        assert_eq!(shown.last(), Some(&LogColumn::Hash));
        assert_eq!(
            toggle_column(&shown, LogColumn::Author),
            vec![
                LogColumn::Root,
                LogColumn::Commit,
                LogColumn::Date,
                LogColumn::Hash
            ]
        );
    }

    #[test]
    fn toolbar_height_uses_the_tallest_component_plus_toolbar_insets() {
        assert_eq!(toolbar_height(16.), 34.);
        assert_eq!(toolbar_height(10.), 34.);
        assert_eq!(toolbar_height(20.), 38.);
    }

    #[test]
    fn natural_order_sorts_numbers_by_value() {
        assert_eq!(natural_compare("feature9", "feature10"), Ordering::Less);
        assert_eq!(natural_compare("Main", "main"), Ordering::Less);
        assert_eq!(
            naturally_sorted(strings(&["b", "a10", "a9"])),
            strings(&["a9", "a10", "b"])
        );
    }
}
