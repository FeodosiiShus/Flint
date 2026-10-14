use std::ops::Range;

use collections::{HashMap, HashSet};
use gpui::SharedString;
use language::HighlightId;
use serde::{Deserialize, Serialize};

pub const USAGES_PAGE_SIZE: usize = 100;
pub const MAX_LINE_LENGTH_TO_SHOW: usize = 200;

const CHARS_BEFORE_FIRST_MATCH: usize = 40;
const ELLIPSIS: &str = "…";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UsageScope {
    #[default]
    ProjectFiles,
    AllPlaces,
    OpenFiles,
    CurrentFile,
}

impl UsageScope {
    pub const ALL: [UsageScope; 4] = [
        UsageScope::ProjectFiles,
        UsageScope::AllPlaces,
        UsageScope::OpenFiles,
        UsageScope::CurrentFile,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            UsageScope::ProjectFiles => "Project Files",
            UsageScope::AllPlaces => "All Places",
            UsageScope::OpenFiles => "Open Files",
            UsageScope::CurrentFile => "Current File",
        }
    }

    pub fn maximal() -> UsageScope {
        UsageScope::AllPlaces
    }

    fn contains<L>(self, row: &UsageRow<L>) -> bool {
        match self {
            UsageScope::ProjectFiles => row.in_project,
            UsageScope::AllPlaces => true,
            UsageScope::OpenFiles => row.in_open_file,
            UsageScope::CurrentFile => row.in_current_file,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageAccess {
    #[default]
    Unknown,
    Read,
    Write,
    ReadWrite,
}

impl UsageAccess {
    pub fn merge(self, other: UsageAccess) -> UsageAccess {
        match (self, other) {
            (UsageAccess::Unknown, access) | (access, UsageAccess::Unknown) => access,
            (left, right) if left == right => left,
            _ => UsageAccess::ReadWrite,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageFilters {
    pub show_read_access: bool,
    pub show_write_access: bool,
    pub show_imports: bool,
    pub show_comments: bool,
}

impl Default for UsageFilters {
    fn default() -> Self {
        Self {
            show_read_access: true,
            show_write_access: true,
            show_imports: true,
            show_comments: true,
        }
    }
}

impl UsageFilters {
    #[cfg(test)]
    pub fn all_shown(&self) -> bool {
        self.show_read_access && self.show_write_access && self.show_imports && self.show_comments
    }

    pub fn is_visible(&self, access: UsageAccess, is_import: bool, is_comment: bool) -> bool {
        if !self.show_read_access && access == UsageAccess::Read {
            return false;
        }
        if !self.show_write_access && access == UsageAccess::Write {
            return false;
        }
        if !self.show_imports && is_import {
            return false;
        }
        if !self.show_comments && is_comment {
            return false;
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShowUsagesState {
    pub filters: UsageFilters,
    pub scope: UsageScope,
    pub preview: bool,
    pub short_file_path: bool,
    pub file_structure: bool,
    pub popup_size: Option<(f32, f32)>,
    pub preview_ratio: f32,
}

impl Default for ShowUsagesState {
    fn default() -> Self {
        Self {
            filters: UsageFilters::default(),
            scope: UsageScope::default(),
            preview: false,
            short_file_path: false,
            file_structure: false,
            popup_size: None,
            preview_ratio: 0.6,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawUsage<L> {
    pub location: L,
    pub worktree_order: usize,
    pub file_path: SharedString,
    pub file_name: SharedString,
    pub row: u32,
    pub column: u32,
    pub offset: usize,
    pub line_text: SharedString,
    pub match_range: Range<usize>,
    pub syntax_highlights: Vec<(Range<usize>, HighlightId)>,
    pub structure: Vec<SharedString>,
    pub access: UsageAccess,
    pub is_import: bool,
    pub is_comment: bool,
    pub is_origin: bool,
    pub in_project: bool,
    pub in_open_file: bool,
    pub in_current_file: bool,
}

#[derive(Clone, Debug)]
pub struct UsageRow<L> {
    pub locations: Vec<L>,
    pub raw_indices: Vec<usize>,
    pub file_path: SharedString,
    pub file_name: SharedString,
    pub structure: Vec<SharedString>,
    pub row: u32,
    pub column: u32,
    pub display_text: SharedString,
    pub match_ranges: Vec<Range<usize>>,
    pub syntax_highlights: Vec<(Range<usize>, HighlightId)>,
    pub access: UsageAccess,
    pub is_import: bool,
    pub is_comment: bool,
    pub is_origin: bool,
    pub in_project: bool,
    pub in_open_file: bool,
    pub in_current_file: bool,
}

impl<L> UsageRow<L> {
    pub fn line_column_label(&self) -> SharedString {
        SharedString::from(format!(
            "({}: {})",
            u64::from(self.row) + 1,
            u64::from(self.column) + 1
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayRow {
    Usage(usize),
    MoreUsages,
    OutOfScope(usize),
    FilteredOut(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeedSearchMatch {
    pub display_index: usize,
    pub file_label_ranges: Vec<Range<usize>>,
    pub text_ranges: Vec<Range<usize>>,
}

pub struct UsagesModel<L> {
    raw_usages: Vec<RawUsage<L>>,
    rows: Vec<UsageRow<L>>,
    row_of_raw_usage: Vec<Option<usize>>,
    short_file_labels: HashMap<SharedString, SharedString>,
    scope: UsageScope,
    filters: UsageFilters,
    row_limit: usize,
    display_rows: Vec<DisplayRow>,
    in_scope_count: usize,
    visible_count: usize,
    visible_usage_count: usize,
    filtered_out_count: usize,
    out_of_scope_count: usize,
}

impl<L: Clone> UsagesModel<L> {
    pub fn new(raw_usages: Vec<RawUsage<L>>, scope: UsageScope, filters: UsageFilters) -> Self {
        let mut row_of_raw_usage = vec![None; raw_usages.len()];
        let mut rows = Vec::new();
        for group in group_usages(&raw_usages) {
            if let Some(row) = build_row(&raw_usages, group) {
                let row_index = rows.len();
                for raw_index in &row.raw_indices {
                    if let Some(slot) = row_of_raw_usage.get_mut(*raw_index) {
                        *slot = Some(row_index);
                    }
                }
                rows.push(row);
            }
        }
        let short_file_labels = short_file_labels(&rows);
        let mut model = Self {
            raw_usages,
            rows,
            row_of_raw_usage,
            short_file_labels,
            scope,
            filters,
            row_limit: USAGES_PAGE_SIZE,
            display_rows: Vec::new(),
            in_scope_count: 0,
            visible_count: 0,
            visible_usage_count: 0,
            filtered_out_count: 0,
            out_of_scope_count: 0,
        };
        model.rebuild_display_rows();
        model
    }

    pub fn scope(&self) -> UsageScope {
        self.scope
    }

    pub fn filters(&self) -> UsageFilters {
        self.filters
    }

    pub fn set_filters(&mut self, filters: UsageFilters) {
        self.filters = filters;
        self.rebuild_display_rows();
    }

    #[cfg(test)]
    pub fn set_access(&mut self, raw_index: usize, access: UsageAccess) {
        self.set_accesses([(raw_index, access)]);
    }

    pub fn set_accesses(&mut self, accesses: impl IntoIterator<Item = (usize, UsageAccess)>) {
        let mut changed_rows = Vec::new();
        for (raw_index, access) in accesses {
            let Some(raw_usage) = self.raw_usages.get_mut(raw_index) else {
                continue;
            };
            raw_usage.access = access;
            if let Some(row_index) = self.row_of_raw_usage.get(raw_index).copied().flatten() {
                changed_rows.push(row_index);
            }
        }
        changed_rows.sort_unstable();
        changed_rows.dedup();
        for row_index in changed_rows {
            if let Some(row) = self.rows.get_mut(row_index) {
                row.access = row
                    .raw_indices
                    .iter()
                    .filter_map(|index| self.raw_usages.get(*index))
                    .fold(UsageAccess::Unknown, |merged, usage| {
                        merged.merge(usage.access)
                    });
            }
        }
        self.rebuild_display_rows();
    }

    #[cfg(test)]
    pub fn raw_usages(&self) -> &[RawUsage<L>] {
        &self.raw_usages
    }

    pub fn show_more(&mut self) {
        self.row_limit = self.row_limit.saturating_add(USAGES_PAGE_SIZE);
        self.rebuild_display_rows();
    }

    pub fn display_rows(&self) -> &[DisplayRow] {
        &self.display_rows
    }

    pub fn row(&self, row_index: usize) -> Option<&UsageRow<L>> {
        self.rows.get(row_index)
    }

    pub fn usage_row_at(&self, display_index: usize) -> Option<&UsageRow<L>> {
        match self.display_rows.get(display_index)? {
            DisplayRow::Usage(row_index) => self.rows.get(*row_index),
            DisplayRow::MoreUsages | DisplayRow::OutOfScope(_) | DisplayRow::FilteredOut(_) => None,
        }
    }

    pub fn in_scope_count(&self) -> usize {
        self.in_scope_count
    }

    pub fn visible_count(&self) -> usize {
        self.visible_count
    }

    pub fn visible_usage_count(&self) -> usize {
        self.visible_usage_count
    }

    pub fn shown_count(&self) -> usize {
        self.visible_count.min(self.row_limit)
    }

    pub fn filtered_out_count(&self) -> usize {
        self.filtered_out_count
    }

    pub fn out_of_scope_count(&self) -> usize {
        self.out_of_scope_count
    }

    pub fn has_more(&self) -> bool {
        self.visible_count > self.row_limit || self.out_of_scope_count > 0
    }

    pub fn status_text(&self, searching: bool) -> SharedString {
        if searching || self.has_more() {
            match self.shown_count() {
                0 => SharedString::new_static("0 usages"),
                1 => SharedString::new_static("1 usage"),
                count => SharedString::from(format!("{count}+ usages")),
            }
        } else if self.visible_count != self.in_scope_count {
            SharedString::from(format!(
                "{} of {} usages",
                self.visible_count, self.in_scope_count
            ))
        } else {
            match self.visible_count {
                0 => SharedString::new_static("0 usages"),
                1 => SharedString::new_static("1 usage"),
                count => SharedString::from(format!("{count} usages")),
            }
        }
    }

    pub fn initial_selection(&self) -> Option<usize> {
        let first_row = self.display_rows.first()?;
        let first_is_origin = match first_row {
            DisplayRow::Usage(row_index) => {
                self.rows.get(*row_index).is_some_and(|row| row.is_origin)
            }
            DisplayRow::MoreUsages | DisplayRow::OutOfScope(_) | DisplayRow::FilteredOut(_) => {
                false
            }
        };
        if first_is_origin && self.display_rows.len() > 1 {
            Some(1)
        } else {
            Some(0)
        }
    }

    pub fn file_label(&self, row: &UsageRow<L>, short_file_path: bool) -> SharedString {
        if short_file_path {
            self.short_file_labels
                .get(&row.file_path)
                .cloned()
                .unwrap_or_else(|| row.file_path.clone())
        } else {
            row.file_name.clone()
        }
    }

    pub fn speed_search(&self, query: &str, short_file_path: bool) -> Vec<SpeedSearchMatch> {
        let query_characters: Vec<char> = query.chars().collect();
        self.display_rows
            .iter()
            .enumerate()
            .filter_map(|(display_index, display_row)| {
                let DisplayRow::Usage(row_index) = display_row else {
                    return None;
                };
                let row = self.rows.get(*row_index)?;
                let label = self.file_label(row, short_file_path);
                let element_text = format!("{label} {}", row.display_text);
                let ranges = speed_search_ranges(&element_text, &query_characters)?;
                let label_end = label.len();
                let text_start = label_end + 1;
                let mut file_label_ranges = Vec::new();
                let mut text_ranges = Vec::new();
                for range in ranges {
                    if range.start < label_end {
                        file_label_ranges.push(range.start..range.end.min(label_end));
                    }
                    if range.end > text_start {
                        text_ranges
                            .push(range.start.max(text_start) - text_start..range.end - text_start);
                    }
                }
                Some(SpeedSearchMatch {
                    display_index,
                    file_label_ranges,
                    text_ranges,
                })
            })
            .collect()
    }

    pub fn visible_locations(&self) -> Vec<L> {
        self.rows
            .iter()
            .filter(|row| self.is_row_visible(row))
            .flat_map(|row| row.locations.iter().cloned())
            .collect()
    }

    fn is_row_visible(&self, row: &UsageRow<L>) -> bool {
        self.scope.contains(row)
            && self
                .filters
                .is_visible(row.access, row.is_import, row.is_comment)
    }

    fn rebuild_display_rows(&mut self) {
        let mut display_rows = Vec::new();
        let mut in_scope_count = 0;
        let mut visible_count = 0;
        let mut visible_usage_count = 0;
        let mut filtered_out_count = 0;
        let mut out_of_scope_count = 0;
        for (row_index, row) in self.rows.iter().enumerate() {
            if !self.scope.contains(row) {
                out_of_scope_count += 1;
                continue;
            }
            in_scope_count += 1;
            if !self
                .filters
                .is_visible(row.access, row.is_import, row.is_comment)
            {
                filtered_out_count += 1;
                continue;
            }
            visible_count += 1;
            visible_usage_count += row.raw_indices.len();
            if display_rows.len() < self.row_limit {
                display_rows.push(DisplayRow::Usage(row_index));
            }
        }
        if visible_count > self.row_limit {
            display_rows.push(DisplayRow::MoreUsages);
        } else if out_of_scope_count > 0 {
            display_rows.push(DisplayRow::OutOfScope(out_of_scope_count));
        }
        if filtered_out_count > 0 {
            display_rows.push(DisplayRow::FilteredOut(filtered_out_count));
        }
        self.display_rows = display_rows;
        self.in_scope_count = in_scope_count;
        self.visible_count = visible_count;
        self.visible_usage_count = visible_usage_count;
        self.filtered_out_count = filtered_out_count;
        self.out_of_scope_count = out_of_scope_count;
    }
}

pub fn no_usages_message(scope: UsageScope) -> String {
    format!("No usages found in {}", scope.display_name())
}

pub fn only_usage_message(scope: UsageScope) -> String {
    format!("It's the only usage in {}", scope.display_name())
}

pub fn all_usages_in_line_message(count: usize, scope: UsageScope) -> String {
    format!(
        "All {count} usages in {} are in this line",
        scope.display_name()
    )
}

pub fn out_of_scope_message(count: usize, scope: UsageScope) -> String {
    let subject = match count {
        0 => "0 usages are".to_string(),
        1 => "One usage is".to_string(),
        count => format!("{count} usages are"),
    };
    format!("{subject} out of scope '{}'", scope.display_name())
}

pub fn filtered_out_message(count: usize) -> String {
    if count == 1 {
        format!("{count} usage was filtered out")
    } else {
        format!("{count} usages were filtered out")
    }
}

pub fn second_invocation_hint(shortcut: &str, scope: UsageScope) -> Option<String> {
    let maximal_scope = UsageScope::maximal();
    if scope == maximal_scope {
        None
    } else {
        Some(format!(
            "Press {shortcut} again to search in '{}'",
            maximal_scope.display_name()
        ))
    }
}

fn group_usages<L>(raw_usages: &[RawUsage<L>]) -> Vec<Vec<usize>> {
    let mut ordered: Vec<(usize, &RawUsage<L>)> = raw_usages.iter().enumerate().collect();
    ordered.sort_by(|(_, left), (_, right)| {
        (
            left.worktree_order,
            left.file_path.as_str(),
            left.row,
            left.column,
        )
            .cmp(&(
                right.worktree_order,
                right.file_path.as_str(),
                right.row,
                right.column,
            ))
    });
    let mut seen_positions: HashSet<(usize, &str, usize)> = HashSet::default();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut previous_line: Option<(usize, &str, u32)> = None;
    for (raw_index, usage) in ordered {
        if !seen_positions.insert((usage.worktree_order, usage.file_path.as_str(), usage.offset)) {
            continue;
        }
        let line = (usage.worktree_order, usage.file_path.as_str(), usage.row);
        match groups.last_mut() {
            Some(group) if previous_line == Some(line) => group.push(raw_index),
            _ => groups.push(vec![raw_index]),
        }
        previous_line = Some(line);
    }
    groups
}

fn build_row<L: Clone>(raw_usages: &[RawUsage<L>], group: Vec<usize>) -> Option<UsageRow<L>> {
    let members: Vec<&RawUsage<L>> = group
        .iter()
        .filter_map(|raw_index| raw_usages.get(*raw_index))
        .collect();
    let first = *members.first()?;
    let excerpt = LineExcerpt::new(first.line_text.as_str(), first.match_range.start);
    let mut match_ranges: Vec<Range<usize>> = members
        .iter()
        .filter_map(|usage| excerpt.map_range(&usage.match_range))
        .collect();
    match_ranges.sort_by_key(|range| (range.start, range.end));
    match_ranges.dedup();
    let syntax_highlights = first
        .syntax_highlights
        .iter()
        .filter_map(|(range, highlight_id)| Some((excerpt.map_range(range)?, *highlight_id)))
        .collect();
    let access = members.iter().fold(UsageAccess::Unknown, |merged, usage| {
        merged.merge(usage.access)
    });
    Some(UsageRow {
        locations: members.iter().map(|usage| usage.location.clone()).collect(),
        file_path: first.file_path.clone(),
        file_name: first.file_name.clone(),
        structure: first.structure.clone(),
        row: first.row,
        column: first.column,
        display_text: excerpt.text,
        match_ranges,
        syntax_highlights,
        access,
        is_import: members.iter().all(|usage| usage.is_import),
        is_comment: members.iter().all(|usage| usage.is_comment),
        is_origin: members.iter().any(|usage| usage.is_origin),
        in_project: first.in_project,
        in_open_file: first.in_open_file,
        in_current_file: first.in_current_file,
        raw_indices: group,
    })
}

struct LineExcerpt {
    text: SharedString,
    window_start: usize,
    window_end: usize,
    prefix_length: usize,
}

impl LineExcerpt {
    fn new(line_text: &str, first_match_start: usize) -> Self {
        let line = line_text.trim_end_matches('\r');
        let content = line.trim_start();
        let content_start = line.len() - content.len();
        let boundaries: Vec<usize> = content
            .char_indices()
            .map(|(index, _)| content_start + index)
            .chain(std::iter::once(line.len()))
            .collect();
        let character_count = boundaries.len().saturating_sub(1);
        if character_count <= MAX_LINE_LENGTH_TO_SHOW {
            return Self {
                text: SharedString::from(content),
                window_start: content_start,
                window_end: line.len(),
                prefix_length: 0,
            };
        }
        let first_match_character =
            boundaries.partition_point(|boundary| *boundary < first_match_start);
        let start_character = first_match_character
            .saturating_sub(CHARS_BEFORE_FIRST_MATCH)
            .min(character_count - MAX_LINE_LENGTH_TO_SHOW);
        let end_character = start_character + MAX_LINE_LENGTH_TO_SHOW;
        let window_start = boundaries
            .get(start_character)
            .copied()
            .unwrap_or(content_start);
        let window_end = boundaries.get(end_character).copied().unwrap_or(line.len());
        let clipped_start = start_character > 0;
        let clipped_end = end_character < character_count;
        let window_text = line.get(window_start..window_end).unwrap_or_default();
        let mut text = String::with_capacity(window_text.len() + 2 * ELLIPSIS.len());
        if clipped_start {
            text.push_str(ELLIPSIS);
        }
        text.push_str(window_text);
        if clipped_end {
            text.push_str(ELLIPSIS);
        }
        Self {
            text: SharedString::from(text),
            window_start,
            window_end,
            prefix_length: if clipped_start { ELLIPSIS.len() } else { 0 },
        }
    }

    fn map_range(&self, range: &Range<usize>) -> Option<Range<usize>> {
        let start = range.start.max(self.window_start);
        let end = range.end.min(self.window_end);
        if start >= end {
            return None;
        }
        let text = self.text.as_str();
        let mut display_start = self.prefix_length + (start - self.window_start);
        let mut display_end = self.prefix_length + (end - self.window_start);
        while display_start > 0 && !text.is_char_boundary(display_start) {
            display_start -= 1;
        }
        while display_end < text.len() && !text.is_char_boundary(display_end) {
            display_end += 1;
        }
        if display_start < display_end && display_end <= text.len() {
            Some(display_start..display_end)
        } else {
            None
        }
    }
}

fn short_file_labels<L>(rows: &[UsageRow<L>]) -> HashMap<SharedString, SharedString> {
    let mut paths: Vec<&SharedString> = rows.iter().map(|row| &row.file_path).collect();
    paths.sort();
    paths.dedup();
    let mut suffix_counts: HashMap<&str, usize> = HashMap::default();
    for path in &paths {
        for suffix in path_suffixes(path.as_str()) {
            *suffix_counts.entry(suffix).or_default() += 1;
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let label = path_suffixes(path.as_str())
                .find(|suffix| suffix_counts.get(suffix).copied() == Some(1))
                .unwrap_or(path.as_str());
            (path.clone(), SharedString::from(label))
        })
        .collect()
}

fn path_suffixes(path: &str) -> impl Iterator<Item = &str> {
    path.rmatch_indices('/')
        .filter_map(move |(separator_index, _)| path.get(separator_index + 1..))
        .chain(std::iter::once(path))
        .filter(|suffix| !suffix.is_empty())
}

fn speed_search_ranges(text: &str, query: &[char]) -> Option<Vec<Range<usize>>> {
    if query.is_empty() {
        return Some(Vec::new());
    }
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    let positions = substring_positions(&characters, query)
        .or_else(|| subsequence_positions(&characters, query))?;
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for position in positions {
        let Some((start, character)) = characters.get(position).copied() else {
            continue;
        };
        let end = start + character.len_utf8();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    Some(ranges)
}

fn substring_positions(characters: &[(usize, char)], query: &[char]) -> Option<Vec<usize>> {
    let start = characters.windows(query.len()).position(|window| {
        window
            .iter()
            .zip(query)
            .all(|((_, character), expected)| characters_match(*character, *expected))
    })?;
    Some((start..start + query.len()).collect())
}

fn subsequence_positions(characters: &[(usize, char)], query: &[char]) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(query.len());
    for (position, (_, character)) in characters.iter().enumerate() {
        let Some(expected) = query.get(positions.len()) else {
            break;
        };
        if characters_match(*character, *expected) {
            positions.push(position);
        }
    }
    (positions.len() == query.len()).then_some(positions)
}

fn characters_match(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn usage(
        location: usize,
        file_path: &str,
        row: u32,
        line_text: &str,
        needle: &str,
    ) -> RawUsage<usize> {
        usage_at(location, file_path, row, line_text, needle, 0)
    }

    fn usage_at(
        location: usize,
        file_path: &str,
        row: u32,
        line_text: &str,
        needle: &str,
        occurrence: usize,
    ) -> RawUsage<usize> {
        let (start, _) = line_text.match_indices(needle).nth(occurrence).unwrap();
        let column = line_text[..start].chars().count() as u32;
        let file_name = file_path.rsplit('/').next().unwrap();
        RawUsage {
            location,
            worktree_order: 0,
            file_path: SharedString::from(file_path),
            file_name: SharedString::from(file_name),
            row,
            column,
            offset: row as usize * 100_000 + start,
            line_text: SharedString::from(line_text),
            match_range: start..start + needle.len(),
            syntax_highlights: Vec::new(),
            structure: Vec::new(),
            access: UsageAccess::Unknown,
            is_import: false,
            is_comment: false,
            is_origin: false,
            in_project: true,
            in_open_file: false,
            in_current_file: false,
        }
    }

    fn model(raw_usages: Vec<RawUsage<usize>>) -> UsagesModel<usize> {
        UsagesModel::new(
            raw_usages,
            UsageScope::ProjectFiles,
            UsageFilters::default(),
        )
    }

    fn only_row(model: &UsagesModel<usize>) -> &UsageRow<usize> {
        assert_eq!(model.display_rows(), &[DisplayRow::Usage(0)]);
        model.usage_row_at(0).unwrap()
    }

    fn matched_texts(row: &UsageRow<usize>) -> Vec<&str> {
        row.match_ranges
            .iter()
            .map(|range| &row.display_text[range.clone()])
            .collect()
    }

    fn file_labels(model: &UsagesModel<usize>, short_file_path: bool) -> Vec<String> {
        (0..model.display_rows().len())
            .filter_map(|index| model.usage_row_at(index))
            .map(|row| model.file_label(row, short_file_path).to_string())
            .collect()
    }

    #[test]
    fn rows_are_sorted_by_worktree_then_path_then_line() {
        let model = model(vec![
            RawUsage {
                worktree_order: 1,
                ..usage(0, "a.ts", 0, "foo", "foo")
            },
            usage(1, "z.ts", 5, "foo", "foo"),
            usage(2, "b.ts", 3, "foo", "foo"),
            usage(3, "z.ts", 2, "foo", "foo"),
            usage(4, "b.ts", 1, "foo", "foo"),
        ]);

        assert_eq!(model.visible_locations(), vec![4, 2, 3, 1, 0]);
        assert_eq!(model.display_rows().len(), 5);
        assert_eq!(model.usage_row_at(0).unwrap().line_column_label(), "(2: 1)");
    }

    #[test]
    fn line_column_label_is_one_based() {
        let model = model(vec![RawUsage {
            row: 45,
            column: 11,
            ..usage(0, "a.ts", 45, "foo", "foo")
        }]);

        assert_eq!(only_row(&model).line_column_label(), "(46: 12)");
    }

    #[test]
    fn usages_on_one_line_merge_into_one_row_with_every_match() {
        let line = "foo(foo)";
        let model = model(vec![
            usage_at(7, "a.ts", 0, line, "foo", 1),
            usage_at(3, "a.ts", 0, line, "foo", 0),
        ]);

        let row = only_row(&model);
        assert_eq!(row.locations, vec![3, 7]);
        assert_eq!(row.raw_indices, vec![1, 0]);
        assert_eq!(row.column, 0);
        assert_eq!(row.match_ranges, vec![0..3, 4..7]);
        assert_eq!(model.visible_count(), 1);
        assert_eq!(model.visible_usage_count(), 2);
    }

    #[test]
    fn merged_row_combines_flags_and_access() {
        let line = "import { foo, foo }";
        let model = model(vec![
            RawUsage {
                access: UsageAccess::Read,
                is_import: true,
                is_comment: true,
                ..usage_at(0, "a.ts", 0, line, "foo", 0)
            },
            RawUsage {
                access: UsageAccess::Write,
                is_import: true,
                is_origin: true,
                ..usage_at(1, "a.ts", 0, line, "foo", 1)
            },
        ]);

        let row = only_row(&model);
        assert_eq!(row.access, UsageAccess::ReadWrite);
        assert!(row.is_import);
        assert!(!row.is_comment);
        assert!(row.is_origin);
    }

    #[test]
    fn duplicate_usages_keep_the_first() {
        let model = model(vec![
            usage(5, "a.ts", 0, "foo", "foo"),
            usage(9, "a.ts", 0, "foo", "foo"),
        ]);

        let row = only_row(&model);
        assert_eq!(row.locations, vec![5]);
        assert_eq!(row.match_ranges, vec![0..3]);
        assert_eq!(model.visible_usage_count(), 1);
    }

    #[test]
    fn leading_whitespace_is_trimmed_and_ranges_rebased() {
        let line = "    let foo = 1;\r";
        let highlight = HighlightId::new(1);
        let model = model(vec![RawUsage {
            syntax_highlights: vec![(4..7, highlight), (8..11, highlight)],
            ..usage(0, "a.ts", 0, line, "foo")
        }]);

        let row = only_row(&model);
        assert_eq!(row.display_text, "let foo = 1;");
        assert_eq!(row.match_ranges, vec![4..7]);
        assert_eq!(
            row.syntax_highlights,
            vec![(0..3, highlight), (4..7, highlight)]
        );
    }

    #[test]
    fn long_line_is_clipped_around_the_first_match() {
        let line = format!("{}foo{}", "x".repeat(300), "y".repeat(300));
        let highlight = HighlightId::new(2);
        let model = model(vec![RawUsage {
            syntax_highlights: vec![
                (0..10, highlight),
                (300..303, highlight),
                (455..470, highlight),
            ],
            ..usage(0, "a.ts", 0, &line, "foo")
        }]);

        let row = only_row(&model);
        let prefix_length = ELLIPSIS.len();
        assert!(row.display_text.starts_with(ELLIPSIS));
        assert!(row.display_text.ends_with(ELLIPSIS));
        assert_eq!(
            row.display_text.chars().count(),
            MAX_LINE_LENGTH_TO_SHOW + 2
        );
        assert_eq!(matched_texts(row), vec!["foo"]);
        assert_eq!(row.match_ranges[0].start, prefix_length + 40);
        assert_eq!(
            row.syntax_highlights,
            vec![
                (prefix_length + 40..prefix_length + 43, highlight),
                (prefix_length + 195..prefix_length + 200, highlight),
            ]
        );
    }

    #[test]
    fn long_line_with_match_at_start_is_clipped_only_at_the_end() {
        let line = format!("    foo{}", "y".repeat(300));
        let model = model(vec![usage(0, "a.ts", 0, &line, "foo")]);

        let row = only_row(&model);
        assert!(row.display_text.starts_with("foo"));
        assert!(row.display_text.ends_with(ELLIPSIS));
        assert_eq!(row.match_ranges, vec![0..3]);
    }

    #[test]
    fn long_multibyte_line_with_match_at_end_is_clipped_only_at_the_start() {
        let line = format!("{}foo", "ж".repeat(300));
        let model = model(vec![usage(0, "a.ts", 0, &line, "foo")]);

        let row = only_row(&model);
        assert!(row.display_text.starts_with(ELLIPSIS));
        assert!(row.display_text.ends_with("foo"));
        assert_eq!(
            row.display_text.chars().count(),
            MAX_LINE_LENGTH_TO_SHOW + 1
        );
        assert_eq!(matched_texts(row), vec!["foo"]);
    }

    #[test]
    fn read_filter_hides_only_pure_reads() {
        let filters = UsageFilters {
            show_read_access: false,
            ..UsageFilters::default()
        };

        assert!(!filters.all_shown());
        assert!(!filters.is_visible(UsageAccess::Read, false, false));
        assert!(filters.is_visible(UsageAccess::ReadWrite, false, false));
        assert!(filters.is_visible(UsageAccess::Write, false, false));
        assert!(filters.is_visible(UsageAccess::Unknown, false, false));
    }

    #[test]
    fn write_filter_hides_only_pure_writes() {
        let filters = UsageFilters {
            show_write_access: false,
            ..UsageFilters::default()
        };

        assert!(!filters.is_visible(UsageAccess::Write, false, false));
        assert!(filters.is_visible(UsageAccess::ReadWrite, false, false));
        assert!(filters.is_visible(UsageAccess::Read, false, false));
        assert!(filters.is_visible(UsageAccess::Unknown, false, false));
    }

    #[test]
    fn import_and_comment_filters_hide_their_usages() {
        let hide_imports = UsageFilters {
            show_imports: false,
            ..UsageFilters::default()
        };
        let hide_comments = UsageFilters {
            show_comments: false,
            ..UsageFilters::default()
        };

        assert!(!hide_imports.is_visible(UsageAccess::Unknown, true, false));
        assert!(hide_imports.is_visible(UsageAccess::Unknown, false, true));
        assert!(!hide_comments.is_visible(UsageAccess::Unknown, false, true));
        assert!(hide_comments.is_visible(UsageAccess::Unknown, true, false));
    }

    #[test]
    fn filtered_rows_are_counted_after_out_of_scope_row() {
        let mut model = model(vec![
            RawUsage {
                access: UsageAccess::Read,
                ..usage(0, "a.ts", 0, "foo", "foo")
            },
            RawUsage {
                access: UsageAccess::Write,
                ..usage(1, "a.ts", 1, "foo = 1", "foo")
            },
            RawUsage {
                is_import: true,
                ..usage(2, "a.ts", 2, "import foo", "foo")
            },
            RawUsage {
                in_project: false,
                ..usage(3, "lib.ts", 0, "foo", "foo")
            },
        ]);

        model.set_filters(UsageFilters {
            show_read_access: false,
            show_imports: false,
            ..UsageFilters::default()
        });

        assert_eq!(
            model.display_rows(),
            &[
                DisplayRow::Usage(1),
                DisplayRow::OutOfScope(1),
                DisplayRow::FilteredOut(2),
            ]
        );
        assert_eq!(model.filtered_out_count(), 2);
        assert_eq!(model.visible_count(), 1);
        assert_eq!(model.in_scope_count(), 3);
        assert_eq!(model.visible_locations(), vec![1]);
    }

    #[test]
    fn scope_decides_which_rows_are_in_scope() {
        let raw_usages = vec![
            RawUsage {
                in_open_file: true,
                in_current_file: true,
                ..usage(0, "a.ts", 0, "foo", "foo")
            },
            RawUsage {
                in_open_file: true,
                ..usage(1, "b.ts", 0, "foo", "foo")
            },
            usage(2, "c.ts", 0, "foo", "foo"),
            RawUsage {
                in_project: false,
                ..usage(3, "lib.ts", 0, "foo", "foo")
            },
        ];
        let visible_for = |scope| {
            let model = UsagesModel::new(raw_usages.clone(), scope, UsageFilters::default());
            (
                model.visible_locations(),
                model.out_of_scope_count(),
                model.display_rows().last().copied(),
            )
        };

        assert_eq!(
            visible_for(UsageScope::ProjectFiles),
            (vec![0, 1, 2], 1, Some(DisplayRow::OutOfScope(1)))
        );
        assert_eq!(
            visible_for(UsageScope::AllPlaces),
            (vec![0, 1, 2, 3], 0, Some(DisplayRow::Usage(3)))
        );
        assert_eq!(
            visible_for(UsageScope::OpenFiles),
            (vec![0, 1], 2, Some(DisplayRow::OutOfScope(2)))
        );
        assert_eq!(
            visible_for(UsageScope::CurrentFile),
            (vec![0], 3, Some(DisplayRow::OutOfScope(3)))
        );
    }

    #[test]
    fn rows_are_paged_and_more_usages_replaces_out_of_scope() {
        let mut raw_usages: Vec<RawUsage<usize>> = (0..101)
            .map(|index| usage(index, "a.ts", index as u32, "foo", "foo"))
            .collect();
        raw_usages.push(RawUsage {
            in_project: false,
            ..usage(101, "lib.ts", 0, "foo", "foo")
        });
        let mut model = model(raw_usages);

        assert_eq!(model.display_rows().len(), USAGES_PAGE_SIZE + 1);
        assert_eq!(model.display_rows().last(), Some(&DisplayRow::MoreUsages));
        assert_eq!(model.shown_count(), USAGES_PAGE_SIZE);
        assert!(model.has_more());
        assert_eq!(model.status_text(false), "100+ usages");

        model.show_more();

        assert_eq!(model.display_rows().len(), 102);
        assert_eq!(model.display_rows()[100], DisplayRow::Usage(100));
        assert_eq!(model.display_rows()[101], DisplayRow::OutOfScope(1));
        assert!(!model.display_rows().contains(&DisplayRow::MoreUsages));
        assert_eq!(model.shown_count(), 101);
        assert_eq!(model.visible_locations().len(), 101);
    }

    #[test]
    fn status_text_covers_every_branch() {
        let empty = model(Vec::new());
        assert_eq!(empty.status_text(true), "0 usages");
        assert_eq!(empty.status_text(false), "0 usages");

        let single = model(vec![usage(0, "a.ts", 0, "foo", "foo")]);
        assert_eq!(single.status_text(true), "1 usage");
        assert_eq!(single.status_text(false), "1 usage");

        let three = || {
            vec![
                RawUsage {
                    access: UsageAccess::Read,
                    ..usage(0, "a.ts", 0, "foo", "foo")
                },
                usage(1, "a.ts", 1, "foo", "foo"),
                usage(2, "a.ts", 2, "foo", "foo"),
            ]
        };
        let mut complete = model(three());
        assert_eq!(complete.status_text(true), "3+ usages");
        assert_eq!(complete.status_text(false), "3 usages");

        complete.set_filters(UsageFilters {
            show_read_access: false,
            ..UsageFilters::default()
        });
        assert_eq!(complete.status_text(false), "2 of 3 usages");

        let mut with_out_of_scope = three();
        with_out_of_scope.push(RawUsage {
            in_project: false,
            ..usage(3, "lib.ts", 0, "foo", "foo")
        });
        assert_eq!(model(with_out_of_scope).status_text(false), "3+ usages");
    }

    #[test]
    fn initial_selection_skips_a_top_origin_row() {
        let origin_first = model(vec![
            RawUsage {
                is_origin: true,
                ..usage(0, "a.ts", 0, "foo", "foo")
            },
            usage(1, "a.ts", 1, "foo", "foo"),
        ]);
        assert_eq!(origin_first.initial_selection(), Some(1));

        let origin_second = model(vec![
            usage(0, "a.ts", 0, "foo", "foo"),
            RawUsage {
                is_origin: true,
                ..usage(1, "a.ts", 1, "foo", "foo")
            },
        ]);
        assert_eq!(origin_second.initial_selection(), Some(0));

        let origin_only = model(vec![RawUsage {
            is_origin: true,
            ..usage(0, "a.ts", 0, "foo", "foo")
        }]);
        assert_eq!(origin_only.initial_selection(), Some(0));

        assert_eq!(model(Vec::new()).initial_selection(), None);
    }

    #[test]
    fn short_file_path_uses_shortest_unique_suffix() {
        let model = model(vec![
            usage(0, "a/index.ts", 0, "foo", "foo"),
            usage(1, "b/index.ts", 0, "foo", "foo"),
            usage(2, "src/main.ts", 0, "foo", "foo"),
            usage(3, "x/lib/util.ts", 0, "foo", "foo"),
            usage(4, "y/lib/util.ts", 0, "foo", "foo"),
        ]);

        assert_eq!(
            file_labels(&model, true),
            vec![
                "a/index.ts",
                "b/index.ts",
                "main.ts",
                "x/lib/util.ts",
                "y/lib/util.ts"
            ]
        );
        assert_eq!(
            file_labels(&model, false),
            vec!["index.ts", "index.ts", "main.ts", "util.ts", "util.ts"]
        );
    }

    #[test]
    fn speed_search_prefers_substring_and_splits_ranges() {
        let model = model(vec![
            usage(0, "lib/other.ts", 0, "bar()", "bar"),
            usage(1, "src/index.ts", 0, "const value = 1", "value"),
        ]);

        assert_eq!(
            model.speed_search("TS CON", false),
            vec![SpeedSearchMatch {
                display_index: 1,
                file_label_ranges: vec![6..8],
                text_ranges: vec![0..3],
            }]
        );
        assert_eq!(
            model.speed_search("idxVAL", false),
            vec![SpeedSearchMatch {
                display_index: 1,
                file_label_ranges: vec![0..1, 2..3, 4..5],
                text_ranges: vec![6..9],
            }]
        );
        assert_eq!(
            model.speed_search("BAR(", true),
            vec![SpeedSearchMatch {
                display_index: 0,
                file_label_ranges: Vec::new(),
                text_ranges: vec![0..4],
            }]
        );
        assert_eq!(model.speed_search("zzz", false), Vec::new());
    }

    #[test]
    fn set_access_moves_a_row_into_filtered_out() {
        let mut model = UsagesModel::new(
            vec![
                usage(0, "a.ts", 0, "foo", "foo"),
                usage(1, "a.ts", 1, "foo", "foo"),
            ],
            UsageScope::ProjectFiles,
            UsageFilters {
                show_read_access: false,
                ..UsageFilters::default()
            },
        );
        assert_eq!(model.visible_count(), 2);

        model.set_access(0, UsageAccess::Read);

        assert_eq!(model.raw_usages()[0].access, UsageAccess::Read);
        assert_eq!(model.row(0).unwrap().access, UsageAccess::Read);
        assert_eq!(
            model.display_rows(),
            &[DisplayRow::Usage(1), DisplayRow::FilteredOut(1)]
        );
        assert_eq!(model.filtered_out_count(), 1);
    }

    #[test]
    fn messages_use_intellij_wording() {
        assert_eq!(
            no_usages_message(UsageScope::ProjectFiles),
            "No usages found in Project Files"
        );
        assert_eq!(
            only_usage_message(UsageScope::OpenFiles),
            "It's the only usage in Open Files"
        );
        assert_eq!(
            all_usages_in_line_message(3, UsageScope::ProjectFiles),
            "All 3 usages in Project Files are in this line"
        );
        assert_eq!(
            out_of_scope_message(0, UsageScope::ProjectFiles),
            "0 usages are out of scope 'Project Files'"
        );
        assert_eq!(
            out_of_scope_message(1, UsageScope::CurrentFile),
            "One usage is out of scope 'Current File'"
        );
        assert_eq!(
            out_of_scope_message(4, UsageScope::ProjectFiles),
            "4 usages are out of scope 'Project Files'"
        );
        assert_eq!(filtered_out_message(1), "1 usage was filtered out");
        assert_eq!(filtered_out_message(5), "5 usages were filtered out");
        assert_eq!(
            second_invocation_hint("⌥⌘F7", UsageScope::ProjectFiles),
            Some("Press ⌥⌘F7 again to search in 'All Places'".to_string())
        );
        assert_eq!(second_invocation_hint("⌥⌘F7", UsageScope::AllPlaces), None);
    }

    #[test]
    fn state_round_trips_and_defaults_from_empty_object() {
        let state = ShowUsagesState {
            filters: UsageFilters {
                show_comments: false,
                ..UsageFilters::default()
            },
            scope: UsageScope::OpenFiles,
            preview: true,
            short_file_path: true,
            file_structure: true,
            popup_size: Some((640.0, 420.0)),
            preview_ratio: 0.4,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<ShowUsagesState>(&json).unwrap(),
            state
        );

        let defaults = serde_json::from_str::<ShowUsagesState>("{}").unwrap();
        assert_eq!(defaults, ShowUsagesState::default());
        assert!(defaults.filters.all_shown());
        assert_eq!(defaults.scope, UsageScope::ProjectFiles);
        assert_eq!(defaults.preview_ratio, 0.6);
    }
}
