use collections::HashSet;
use git::repository::{GitUser, LogFilter, LogTextFilter, RepoPath};
use gpui::SharedString;
use regex::Regex;
use util::ResultExt as _;

use crate::git_log::filter_toolbar::users::{resolve_current_user, resolve_user_name};
use crate::git_log::log_dates::format_date;
use crate::git_log::properties::{
    GraphOptions, HEAD_REVISION, LogFilterState, LogTabProperties, USER_FILTER_ME,
    USER_FILTER_ME_LABEL,
};

pub(super) const MATCHES_NOTHING_REVISION: &str = "";

const LOG_TAB_NAME: &str = "Log";
const TAB_NAME_SEPARATOR: &str = ": ";
const TITLE_MAX_LENGTH: usize = 150;
const TITLE_SUFFIX_LENGTH: usize = 20;
const SHORTENED_TEXT_SYMBOL: &str = "...";
const FIRST_PARENT_PRESENTATION: &str = "--first-parent";
const LINEARIZED_PRESENTATION: &str = "linearized";
const SHORT_HASH_LENGTH: usize = 8;
const SHA1_HEX_LENGTH: usize = 40;
const SHA256_HEX_LENGTH: usize = 64;
const REGEX_SPECIAL_CHARACTERS: &str = "()[]{}.*?+^$\\|";
const RANGE_SEPARATOR: &str = "..";
const EXCLUDED_BRANCH_PREFIX: &str = "-";
const VALUE_SEPARATOR: &str = ", ";
const PRESENTATION_SEPARATOR: &str = " ";

pub(super) fn log_filter(
    properties: &LogTabProperties,
    branch_names: &[SharedString],
    authors: Vec<SharedString>,
) -> LogFilter {
    let filters = &properties.filters;
    LogFilter {
        revisions: branch_revisions(&filters.branches, branch_names),
        authors,
        after: filters.date_after,
        before: filters.date_before,
        paths: filters
            .paths
            .iter()
            .filter_map(|path| structure_path(path))
            .collect(),
        text: text_filter(properties),
    }
}

pub(super) fn author_identities(
    users: &[String],
    current_user: Option<&GitUser>,
    all_users: &[GitUser],
) -> Vec<SharedString> {
    let mut identities: Vec<SharedString> = Vec::new();
    for user in users {
        let resolved = if user == USER_FILTER_ME {
            current_user
                .map(|current_user| resolve_current_user(current_user, all_users))
                .unwrap_or_default()
        } else {
            resolve_user_name(user, all_users)
        };
        for resolved_user in resolved {
            let identity = SharedString::from(resolved_user.identity());
            if !identities.contains(&identity) {
                identities.push(identity);
            }
        }
    }
    identities
}

pub(super) fn filter_by_branches(
    filters: &LogFilterState,
    branches: &[SharedString],
) -> LogFilterState {
    LogFilterState {
        branches: branches.iter().map(|branch| branch.to_string()).collect(),
        ..filters.clone()
    }
}

pub(super) fn title(properties: &LogTabProperties, branch_names: &[SharedString]) -> SharedString {
    let short_name = short_title(properties, branch_names);
    if short_name.is_empty() {
        SharedString::new_static(LOG_TAB_NAME)
    } else {
        format!("{LOG_TAB_NAME}{TAB_NAME_SEPARATOR}{short_name}").into()
    }
}

fn short_title(properties: &LogTabProperties, branch_names: &[SharedString]) -> String {
    let options_presentation = graph_options_presentation(properties.graph_options);
    let filters_presentation =
        filters_presentation(properties, branch_names, !options_presentation.is_empty());
    let presentation = [options_presentation, filters_presentation.as_str()]
        .into_iter()
        .filter(|presentation| !presentation.is_empty())
        .collect::<Vec<_>>()
        .join(PRESENTATION_SEPARATOR);
    shorten_text_with_ellipsis(&presentation, TITLE_MAX_LENGTH, TITLE_SUFFIX_LENGTH)
}

fn graph_options_presentation(options: GraphOptions) -> &'static str {
    match options {
        GraphOptions::FirstParent => FIRST_PARENT_PRESENTATION,
        GraphOptions::LinearBek => LINEARIZED_PRESENTATION,
        GraphOptions::Base(_) => "",
    }
}

struct FilterPresentation {
    display: String,
    prefixed: String,
}

impl FilterPresentation {
    fn with_prefix(prefix: &str, display: String) -> Self {
        Self {
            prefixed: format!("{prefix} {display}"),
            display,
        }
    }

    fn without_prefix(display: String) -> Self {
        Self {
            prefixed: display.clone(),
            display,
        }
    }
}

fn filters_presentation(
    properties: &LogTabProperties,
    branch_names: &[SharedString],
    with_prefix: bool,
) -> String {
    let filters = &properties.filters;
    let text_filter = text_filter(properties);
    if let Some(text_filter) = &text_filter {
        let hashes = text_filter.hash_prefixes();
        if !hashes.is_empty() {
            return join_values(hashes.into_iter().map(short_hash));
        }
    }

    let branch_like = BranchLikeFilters::new(&filters.branches, branch_names);
    let mut presentations = Vec::new();
    if let Some(branch_filter) = &branch_like.branch_filter {
        presentations.push(FilterPresentation::with_prefix(
            "on",
            branch_filter.text_presentation().join(VALUE_SEPARATOR),
        ));
    }
    if let Some(date) = date_presentation(filters.date_after, filters.date_before) {
        presentations.push(date);
    }
    if !branch_like.ranges.is_empty() {
        presentations.push(FilterPresentation::without_prefix(join_values(
            branch_like.ranges.iter().map(|(exclusive, inclusive)| {
                format!(
                    "{}{RANGE_SEPARATOR}{}",
                    short_hash(exclusive),
                    short_hash(inclusive)
                )
            }),
        )));
    }
    if !branch_like.revisions.is_empty() {
        presentations.push(FilterPresentation::without_prefix(join_values(
            branch_like
                .revisions
                .iter()
                .map(|revision| short_hash(revision)),
        )));
    }
    if !filters.paths.is_empty() {
        presentations.push(FilterPresentation::with_prefix(
            "for",
            join_values(filters.paths.iter().map(|path| file_name(path).to_string())),
        ));
    }
    if let Some(text_filter) = &text_filter {
        presentations.push(FilterPresentation::with_prefix(
            "containing",
            format!("'{}'", text_filter.pattern),
        ));
    }
    if !filters.users.is_empty() {
        presentations.push(FilterPresentation::with_prefix(
            "by",
            join_values(filters.users.iter().map(|user| {
                if user == USER_FILTER_ME {
                    USER_FILTER_ME_LABEL.to_string()
                } else {
                    user.clone()
                }
            })),
        ));
    }

    let prefixed = presentations.len() != 1 || with_prefix;
    presentations
        .into_iter()
        .map(|presentation| {
            if prefixed {
                presentation.prefixed
            } else {
                presentation.display
            }
        })
        .collect::<Vec<_>>()
        .join(PRESENTATION_SEPARATOR)
}

fn date_presentation(after: Option<i64>, before: Option<i64>) -> Option<FilterPresentation> {
    match (after, before) {
        (Some(after), Some(before)) => {
            let after = format_date(after);
            let before = format_date(before);
            Some(FilterPresentation {
                display: format!("between {after} and {before}"),
                prefixed: format!("made between {after} and {before}"),
            })
        }
        (Some(after), None) => {
            let after = format_date(after);
            Some(FilterPresentation {
                display: format!("since {after}"),
                prefixed: format!("made since {after}"),
            })
        }
        (None, Some(before)) => {
            let before = format_date(before);
            Some(FilterPresentation {
                display: format!("until {before}"),
                prefixed: format!("made until {before}"),
            })
        }
        (None, None) => None,
    }
}

fn join_values(values: impl Iterator<Item = String>) -> String {
    values.collect::<Vec<_>>().join(VALUE_SEPARATOR)
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(SHORT_HASH_LENGTH).collect()
}

fn file_name(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path)
}

fn shorten_text_with_ellipsis(text: &str, max_length: usize, suffix_length: usize) -> String {
    let length = text.chars().count();
    if length <= max_length {
        return text.to_string();
    }
    let prefix_length = max_length.saturating_sub(suffix_length + SHORTENED_TEXT_SYMBOL.len());
    let prefix_end = text
        .char_indices()
        .nth(prefix_length)
        .map_or(text.len(), |(index, _)| index);
    let suffix_start = text
        .char_indices()
        .nth(length - suffix_length)
        .map_or(text.len(), |(index, _)| index);
    format!(
        "{}{SHORTENED_TEXT_SYMBOL}{}",
        &text[..prefix_end],
        &text[suffix_start..]
    )
}

fn text_filter(properties: &LogTabProperties) -> Option<LogTextFilter> {
    let text = &properties.filters.text;
    (!text.trim().is_empty()).then(|| LogTextFilter {
        pattern: text.clone().into(),
        regex: properties.text_filter_regex,
        match_case: properties.text_filter_match_case,
    })
}

fn structure_path(path: &str) -> Option<RepoPath> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    RepoPath::new(path).log_err()
}

fn branch_revisions(values: &[String], branch_names: &[SharedString]) -> Vec<SharedString> {
    if values.is_empty() {
        return Vec::new();
    }
    let revisions = BranchLikeFilters::new(values, branch_names).revisions(branch_names);
    if revisions.is_empty() {
        vec![SharedString::new_static(MATCHES_NOTHING_REVISION)]
    } else {
        revisions
    }
}

struct BranchLikeFilters<'a> {
    branch_filter: Option<BranchFilter>,
    revisions: Vec<&'a str>,
    ranges: Vec<(&'a str, &'a str)>,
}

impl<'a> BranchLikeFilters<'a> {
    fn new(values: &'a [String], branch_names: &[SharedString]) -> Self {
        let mut branch_values = Vec::new();
        let mut revisions = Vec::new();
        let mut ranges = Vec::new();
        for value in values {
            if let Some(range) = parse_range(value) {
                ranges.push(range);
            } else if is_full_hash(value) {
                revisions.push(value.as_str());
            } else {
                branch_values.push(value.as_str());
            }
        }
        let branch_filter = (!branch_values.is_empty()).then(|| {
            let existing_branches = branch_names
                .iter()
                .map(SharedString::as_str)
                .chain([HEAD_REVISION])
                .collect::<HashSet<_>>();
            BranchFilter::from_patterns(&branch_values, &existing_branches)
        });
        Self {
            branch_filter,
            revisions,
            ranges,
        }
    }

    fn revisions(&self, branch_names: &[SharedString]) -> Vec<SharedString> {
        let mut revisions = Vec::new();
        if let Some(branch_filter) = &self.branch_filter {
            revisions.extend(
                branch_names
                    .iter()
                    .map(SharedString::as_str)
                    .chain([HEAD_REVISION])
                    .filter(|name| branch_filter.matches(name))
                    .map(SharedString::from),
            );
        }
        revisions.extend(
            self.revisions
                .iter()
                .map(|revision| SharedString::from(*revision)),
        );
        revisions.extend(self.ranges.iter().map(|(exclusive, inclusive)| {
            SharedString::from(format!("{exclusive}{RANGE_SEPARATOR}{inclusive}"))
        }));
        revisions
    }
}

fn parse_range(value: &str) -> Option<(&str, &str)> {
    let first = value.find(RANGE_SEPARATOR)?;
    (first > 0 && value.rfind(RANGE_SEPARATOR) == Some(first))
        .then(|| (&value[..first], &value[first + RANGE_SEPARATOR.len()..]))
}

fn is_full_hash(value: &str) -> bool {
    matches!(value.len(), SHA1_HEX_LENGTH | SHA256_HEX_LENGTH)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_regexp(text: &str) -> bool {
    text.contains(|character: char| REGEX_SPECIAL_CHARACTERS.contains(character))
}

struct BranchPattern {
    text: String,
    regex: Regex,
}

impl BranchPattern {
    fn new(text: &str) -> Option<Self> {
        let regex = Regex::new(&format!("^(?:{text})$")).warn_on_err()?;
        Some(Self {
            text: text.to_string(),
            regex,
        })
    }
}

#[derive(Default)]
struct BranchFilter {
    branches: Vec<String>,
    patterns: Vec<BranchPattern>,
    excluded_branches: Vec<String>,
    excluded_patterns: Vec<BranchPattern>,
}

impl BranchFilter {
    fn from_patterns(patterns: &[&str], existing_branches: &HashSet<&str>) -> Self {
        let mut filter = Self::default();
        for pattern in patterns {
            let (excluded, name) = match pattern.strip_prefix(EXCLUDED_BRANCH_PREFIX) {
                Some(name) => (true, name),
                None => (false, *pattern),
            };
            let regex = if !existing_branches.contains(name) && is_regexp(name) {
                BranchPattern::new(name)
            } else {
                None
            };
            match (regex, excluded) {
                (Some(regex), false) => filter.patterns.push(regex),
                (Some(regex), true) => filter.excluded_patterns.push(regex),
                (None, false) => filter.branches.push(name.to_string()),
                (None, true) => filter.excluded_branches.push(name.to_string()),
            }
        }
        filter
    }

    fn text_presentation(&self) -> Vec<String> {
        self.branches
            .iter()
            .cloned()
            .chain(self.patterns.iter().map(|pattern| pattern.text.clone()))
            .chain(
                self.excluded_branches
                    .iter()
                    .map(|branch| format!("{EXCLUDED_BRANCH_PREFIX}{branch}")),
            )
            .chain(
                self.excluded_patterns
                    .iter()
                    .map(|pattern| format!("{EXCLUDED_BRANCH_PREFIX}{}", pattern.text)),
            )
            .collect()
    }

    fn matches(&self, name: &str) -> bool {
        self.is_included(name) && !self.is_excluded(name)
    }

    fn is_included(&self, name: &str) -> bool {
        (self.branches.is_empty() && self.patterns.is_empty())
            || is_matched(name, &self.branches, &self.patterns)
    }

    fn is_excluded(&self, name: &str) -> bool {
        is_matched(name, &self.excluded_branches, &self.excluded_patterns)
    }
}

fn is_matched(name: &str, branches: &[String], patterns: &[BranchPattern]) -> bool {
    branches.iter().any(|branch| branch == name)
        || patterns.iter().any(|pattern| pattern.regex.is_match(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_log::properties::GraphSortType;

    fn branch_names() -> Vec<SharedString> {
        [
            "main",
            "old",
            "release/1.0",
            "release/2.0",
            "origin/main",
            "origin/feature",
        ]
        .into_iter()
        .map(SharedString::from)
        .collect()
    }

    fn with_filters(filters: LogFilterState) -> LogTabProperties {
        LogTabProperties {
            filters,
            ..LogTabProperties::new(true)
        }
    }

    fn with_branches(branches: &[&str]) -> LogTabProperties {
        with_filters(LogFilterState {
            branches: branches.iter().map(|branch| branch.to_string()).collect(),
            ..LogFilterState::default()
        })
    }

    fn revisions(branches: &[&str]) -> Vec<SharedString> {
        log_filter(&with_branches(branches), &branch_names(), Vec::new()).revisions
    }

    fn user(name: &str, email: &str) -> GitUser {
        GitUser {
            name: name.to_string().into(),
            email: email.to_string().into(),
        }
    }

    #[test]
    fn no_branch_values_keep_the_whole_log() {
        assert!(revisions(&[]).is_empty());
    }

    #[test]
    fn branch_values_expand_to_matching_branch_names_and_head() {
        assert_eq!(revisions(&["HEAD"]), vec![SharedString::from("HEAD")]);
        assert_eq!(
            revisions(&["origin/main", "main"]),
            vec![
                SharedString::from("main"),
                SharedString::from("origin/main")
            ]
        );
        assert_eq!(
            revisions(&["release/.*"]),
            vec![
                SharedString::from("release/1.0"),
                SharedString::from("release/2.0")
            ]
        );
    }

    #[test]
    fn excluded_branches_keep_every_other_branch_and_head() {
        assert_eq!(
            revisions(&["-old", "-origin/.*"]),
            vec![
                SharedString::from("main"),
                SharedString::from("release/1.0"),
                SharedString::from("release/2.0"),
                SharedString::from("HEAD")
            ]
        );
    }

    #[test]
    fn a_branch_filter_without_matching_branches_matches_nothing() {
        assert_eq!(
            revisions(&["missing"]),
            vec![SharedString::from(MATCHES_NOTHING_REVISION)]
        );
    }

    #[test]
    fn hashes_and_ranges_pass_through_after_branches() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            revisions(&["v1.0..main", hash, "main"]),
            vec![
                SharedString::from("main"),
                SharedString::from(hash),
                SharedString::from("v1.0..main")
            ]
        );
    }

    #[test]
    fn dates_paths_and_text_convert_to_the_backend_filter() {
        let properties = LogTabProperties {
            text_filter_regex: true,
            text_filter_match_case: true,
            ..with_filters(LogFilterState {
                date_after: Some(100),
                date_before: Some(200),
                paths: vec![
                    "src/".to_string(),
                    "crates/git_ui/src/git_log.rs".to_string(),
                    String::new(),
                ],
                text: "fix.*crash".to_string(),
                ..LogFilterState::default()
            })
        };
        let filter = log_filter(&properties, &branch_names(), Vec::new());
        assert_eq!(filter.after, Some(100));
        assert_eq!(filter.before, Some(200));
        assert_eq!(
            filter.paths,
            vec![
                RepoPath::new("src").expect("valid path"),
                RepoPath::new("crates/git_ui/src/git_log.rs").expect("valid path")
            ]
        );
        assert_eq!(
            filter.text,
            Some(LogTextFilter {
                pattern: "fix.*crash".into(),
                regex: true,
                match_case: true,
            })
        );
    }

    #[test]
    fn blank_text_is_no_text_filter() {
        let properties = with_filters(LogFilterState {
            text: "   ".to_string(),
            ..LogFilterState::default()
        });
        assert_eq!(
            log_filter(&properties, &branch_names(), Vec::new()).text,
            None
        );
    }

    #[test]
    fn users_resolve_to_exact_identities_with_me_as_the_current_user() {
        let ann = user("Ann", "ann@example.com");
        let bob = user("Bob", "bob@example.com");
        let all_users = vec![ann.clone(), bob.clone()];
        assert_eq!(
            author_identities(&[USER_FILTER_ME.to_string()], Some(&bob), &all_users),
            vec![SharedString::from(bob.identity())]
        );
        assert_eq!(
            author_identities(&["Ann".to_string()], Some(&bob), &all_users),
            vec![SharedString::from(ann.identity())]
        );
        assert_eq!(
            author_identities(
                &["Ann".to_string(), USER_FILTER_ME.to_string()],
                Some(&ann),
                &all_users
            ),
            vec![SharedString::from(ann.identity())]
        );
        assert!(author_identities(&[USER_FILTER_ME.to_string()], None, &all_users).is_empty());
    }

    #[test]
    fn branch_filter_replacement_keeps_the_other_filters() {
        let filters = LogFilterState {
            branches: vec!["main".to_string(), "origin/feature".to_string()],
            users: vec![USER_FILTER_ME.to_string()],
            date_after: Some(5),
            date_before: None,
            paths: vec!["src/".to_string()],
            text: "fix".to_string(),
        };
        let replaced = filter_by_branches(&filters, &[SharedString::from("HEAD")]);
        assert_eq!(
            replaced,
            LogFilterState {
                branches: vec!["HEAD".to_string()],
                ..filters.clone()
            }
        );
        assert!(filter_by_branches(&filters, &[]).branches.is_empty());
    }

    #[test]
    fn title_without_filters_is_the_tab_name() {
        assert_eq!(title(&LogTabProperties::new(true), &branch_names()), "Log");
        let blank_text = with_filters(LogFilterState {
            text: "  ".to_string(),
            ..LogFilterState::default()
        });
        assert_eq!(title(&blank_text, &branch_names()), "Log");
    }

    #[test]
    fn a_single_filter_uses_its_bare_display_text() {
        assert_eq!(
            title(&with_branches(&["HEAD"]), &branch_names()),
            "Log: HEAD"
        );
        let me = with_filters(LogFilterState {
            users: vec![USER_FILTER_ME.to_string(), "Ann".to_string()],
            ..LogFilterState::default()
        });
        assert_eq!(title(&me, &branch_names()), "Log: me, Ann");
        let paths = with_filters(LogFilterState {
            paths: vec![
                "src/".to_string(),
                "crates/git_ui/src/git_log.rs".to_string(),
            ],
            ..LogFilterState::default()
        });
        assert_eq!(title(&paths, &branch_names()), "Log: src, git_log.rs");
        let after = with_filters(LogFilterState {
            date_after: Some(1_700_000_000),
            ..LogFilterState::default()
        });
        assert_eq!(
            title(&after, &branch_names()),
            format!("Log: since {}", format_date(1_700_000_000))
        );
    }

    #[test]
    fn several_filters_use_prefixes_in_filter_key_order() {
        let properties = with_filters(LogFilterState {
            branches: vec!["main".to_string()],
            users: vec![USER_FILTER_ME.to_string()],
            date_after: Some(10),
            date_before: Some(20),
            paths: vec!["src/".to_string()],
            text: "fix".to_string(),
        });
        assert_eq!(
            title(&properties, &branch_names()),
            format!(
                "Log: on main made between {} and {} for src containing 'fix' by me",
                format_date(10),
                format_date(20)
            )
        );
    }

    #[test]
    fn graph_options_come_first_and_force_prefixes() {
        let first_parent = LogTabProperties {
            graph_options: GraphOptions::FirstParent,
            ..with_branches(&["HEAD"])
        };
        assert_eq!(
            title(&first_parent, &branch_names()),
            "Log: --first-parent on HEAD"
        );
        let linearized = LogTabProperties {
            graph_options: GraphOptions::LinearBek,
            ..LogTabProperties::new(true)
        };
        assert_eq!(title(&linearized, &branch_names()), "Log: linearized");
        let intelli_sort = LogTabProperties {
            graph_options: GraphOptions::Base(GraphSortType::Bek),
            ..with_branches(&["HEAD"])
        };
        assert_eq!(title(&intelli_sort, &branch_names()), "Log: HEAD");
    }

    #[test]
    fn hash_text_replaces_every_other_filter_in_the_title() {
        let properties = with_filters(LogFilterState {
            branches: vec!["main".to_string()],
            text: "abc1234, 0123456789abcdef".to_string(),
            ..LogFilterState::default()
        });
        assert_eq!(
            title(&properties, &branch_names()),
            "Log: abc1234, 01234567"
        );
    }

    #[test]
    fn branch_like_values_are_presented_per_filter_kind() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            title(
                &with_branches(&["-old", "release/.*", "main", "v1.0..feature/login", hash]),
                &branch_names()
            ),
            "Log: on main, release/.*, -old v1.0..feature/ 01234567"
        );
    }

    #[test]
    fn long_titles_are_shortened_keeping_the_end() {
        let branches = (0..40)
            .map(|index| format!("branch-{index}"))
            .collect::<Vec<_>>();
        let properties = with_filters(LogFilterState {
            branches,
            ..LogFilterState::default()
        });
        let short_name = short_title(&properties, &branch_names());
        assert_eq!(short_name.chars().count(), TITLE_MAX_LENGTH);
        assert!(short_name.starts_with("branch-0, branch-1, "));
        assert!(short_name.ends_with("branch-38, branch-39"));
        assert_eq!(
            short_name
                .chars()
                .nth(TITLE_MAX_LENGTH - TITLE_SUFFIX_LENGTH - 3),
            Some('.')
        );
    }
}
