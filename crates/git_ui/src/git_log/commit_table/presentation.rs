use std::{collections::VecDeque, ops::Range, sync::Arc};

use collections::{HashMap, HashSet};
use git::{
    Oid,
    repository::{CommitData, GitUser, InitialGraphCommitData},
};
use gpui::{Pixels, SharedString};
use project::git_store::RepositorySnapshot;

use crate::git_log::{
    log_dates,
    properties::{HEAD_REVISION, USER_FILTER_ME},
    ref_labels::RefGroupingContext,
};

pub(super) const SHORT_HASH_LENGTH: usize = 8;
pub(super) const MAX_SELECTED_COMMITS: usize = 1000;
const HASH_TRIM_CHARACTERS: &str = "()'\"`";
const MINIMUM_HASH_PREFIX_LENGTH: usize = 4;
const MAXIMUM_HASH_PREFIX_LENGTH: usize = 64;
const SYMBOLIC_HEAD_PREFIX: &str = "HEAD -> ";
const ARROW_SUBJECT_MAX_LENGTH: usize = 50;
const ACTION_SUBJECT_MAX_LENGTH: usize = 40;
const ELLIPSIS: &str = "...";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectedRowType {
    Single,
    Top,
    Middle,
    Bottom,
}

impl SelectedRowType {
    pub(super) fn rounds_top(self) -> bool {
        matches!(self, Self::Single | Self::Top)
    }

    pub(super) fn rounds_bottom(self) -> bool {
        matches!(self, Self::Single | Self::Bottom)
    }
}

pub(super) fn selected_row_type(
    single_selected_row: bool,
    previous_selected: bool,
    next_selected: bool,
) -> SelectedRowType {
    if single_selected_row {
        return SelectedRowType::Single;
    }
    match (previous_selected, next_selected) {
        (true, true) => SelectedRowType::Middle,
        (true, false) => SelectedRowType::Bottom,
        (false, true) => SelectedRowType::Top,
        (false, false) => SelectedRowType::Single,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LabelsTooltip {
    References,
    DetachedHead,
}

pub(super) fn labels_tooltip_areas(
    left_aligned: bool,
    graph_width: Pixels,
    column_width: Pixels,
    content_width: Pixels,
    labels_width: Pixels,
    warning_range: Option<Range<Pixels>>,
) -> Vec<(Range<Pixels>, LabelsTooltip)> {
    if labels_width <= Pixels::ZERO {
        return Vec::new();
    }
    let origin = if left_aligned {
        graph_width
    } else {
        column_width - labels_width
    };
    let area_start = origin.max(graph_width).max(Pixels::ZERO);
    let area_end = if left_aligned {
        graph_width + labels_width
    } else {
        content_width
    }
    .min(content_width);
    let mut areas = Vec::new();
    let mut push = |start: Pixels, end: Pixels, tooltip: LabelsTooltip| {
        let start = start.max(area_start);
        let end = end.min(area_end);
        if start < end {
            areas.push((start..end, tooltip));
        }
    };
    match warning_range {
        Some(warning) => {
            let warning_start = origin + warning.start;
            let warning_end = origin + warning.end;
            push(area_start, warning_start, LabelsTooltip::References);
            push(warning_start, warning_end, LabelsTooltip::DetachedHead);
            push(warning_end, area_end, LabelsTooltip::References);
        }
        None => push(area_start, area_end, LabelsTooltip::References),
    }
    areas
}

pub(super) fn short_hash(sha: &Oid) -> SharedString {
    short_text(&sha.to_string(), SHORT_HASH_LENGTH).into()
}

fn short_text(text: &str, length: usize) -> String {
    text.chars().take(length).collect()
}

pub(super) fn shorten_with_ellipsis(text: &str, max_length: usize) -> String {
    if text.chars().count() <= max_length {
        return text.to_string();
    }
    let prefix_length = max_length.saturating_sub(ELLIPSIS.len());
    format!("{}{ELLIPSIS}", short_text(text, prefix_length))
}

fn name_from_email(email: &str) -> Option<&str> {
    email.find('@').filter(|at| *at > 0).map(|at| &email[..at])
}

pub(super) fn user_name(name: &str, email: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    name_from_email(email).unwrap_or(email).to_string()
}

fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn is_name_separator(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(character, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

fn first_and_last_name(name: &str) -> Option<(&str, &str)> {
    name.char_indices().rev().find_map(|(index, character)| {
        let first = &name[..index];
        let last = &name[index + character.len_utf8()..];
        (is_name_separator(character)
            && !first.is_empty()
            && !last.is_empty()
            && first.chars().all(is_word_character)
            && last.chars().all(is_word_character))
        .then_some((first, last))
    })
}

fn name_to_lower_case(name: &str) -> String {
    if name
        .chars()
        .all(|character| (' '..='~').contains(&character))
    {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

pub(super) fn name_in_standard_form(name: &str) -> String {
    match first_and_last_name(name) {
        Some((first, last)) => format!(
            "{} {}",
            first.to_ascii_lowercase(),
            last.to_ascii_lowercase()
        ),
        None => name_to_lower_case(name),
    }
}

pub(super) fn is_same_person(
    first_name: &str,
    first_email: &str,
    second_name: &str,
    second_email: &str,
) -> bool {
    name_in_standard_form(&user_name(first_name, first_email))
        == name_in_standard_form(&user_name(second_name, second_email))
}

pub(super) fn author_presentation(data: &CommitData) -> SharedString {
    let author = user_name(&data.author_name, &data.author_email);
    if is_same_person(
        &data.author_name,
        &data.author_email,
        &data.committer_name,
        &data.committer_email,
    ) {
        author.into()
    } else {
        format!("{author}*").into()
    }
}

fn resolve_user_name<'a>(name: &str, all_users: &'a [GitUser]) -> Vec<&'a GitUser> {
    let standard_name = name_in_standard_form(name);
    all_users
        .iter()
        .filter(|user| {
            (!user.name.is_empty() && name_in_standard_form(&user.name) == standard_name)
                || name_from_email(&user.email)
                    .is_some_and(|email_name| name_in_standard_form(email_name) == standard_name)
        })
        .collect()
}

fn user_identity(name: &str, email: &str) -> (String, String) {
    (name.to_string(), email.to_lowercase())
}

pub(super) fn resolve_current_users(
    current_user: &GitUser,
    all_users: &[GitUser],
) -> HashSet<(String, String)> {
    let users_by_name = resolve_user_name(&current_user.name, all_users);
    let mut result: HashSet<(String, String)> = users_by_name
        .iter()
        .map(|user| user_identity(&user.name, &user.email))
        .collect();
    if let Some(email_name) = name_from_email(&current_user.email) {
        let emails: HashSet<String> = users_by_name
            .iter()
            .map(|user| user.email.to_lowercase())
            .collect();
        for user in resolve_user_name(email_name, all_users) {
            if emails.contains(&user.email.to_lowercase()) {
                result.insert(user_identity(&user.name, &user.email));
            }
        }
    }
    result
}

pub(super) fn is_single_user(current_user: &GitUser, all_users: &[GitUser]) -> bool {
    all_users.iter().all(|user| {
        is_same_person(
            &user.name,
            &user.email,
            &current_user.name,
            &current_user.email,
        )
    })
}

pub(super) fn is_my_commit(data: &CommitData, identities: &HashSet<(String, String)>) -> bool {
    identities.contains(&user_identity(&data.author_name, &data.author_email))
}

pub(super) fn is_filtered_by_me(users: &[String]) -> bool {
    !users.is_empty() && users.iter().all(|user| user == USER_FILTER_ME)
}

pub(super) fn single_filtered_branch(
    revisions: &[SharedString],
    branch_names: &[SharedString],
) -> Option<SharedString> {
    match revisions {
        [revision] if branch_names.iter().any(|name| name == revision) => Some(revision.clone()),
        _ => None,
    }
}

pub(super) fn branch_names(snapshot: &RepositorySnapshot) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = snapshot
        .branch_list
        .iter()
        .map(|branch| SharedString::from(branch.name().to_string()))
        .collect();
    names.push(SharedString::new_static(HEAD_REVISION));
    names
}

pub(super) fn ref_grouping_context(snapshot: &RepositorySnapshot) -> RefGroupingContext {
    let mut upstreams = HashMap::default();
    let mut remote_names: Vec<SharedString> = Vec::new();
    for branch in snapshot.branch_list.iter() {
        if branch.is_remote() {
            if let Some(remote_name) = branch.remote_name()
                && !remote_names.iter().any(|name| name.as_ref() == remote_name)
            {
                remote_names.push(SharedString::from(remote_name.to_string()));
            }
        } else if let Some(upstream) = branch
            .upstream
            .as_ref()
            .and_then(|upstream| upstream.stripped_ref_name())
        {
            upstreams.insert(
                SharedString::from(branch.name().to_string()),
                SharedString::from(upstream.to_string()),
            );
        }
    }
    RefGroupingContext {
        current_branch: snapshot
            .branch
            .as_ref()
            .map(|branch| SharedString::from(branch.name().to_string())),
        upstreams,
        remote_names,
    }
}

fn is_head_decoration(decoration: &str) -> bool {
    decoration == HEAD_REVISION || decoration.starts_with(SYMBOLIC_HEAD_PREFIX)
}

/// Marks the commits reachable from HEAD by their position in `commits`; `index_of` finds the
/// position of a sha.
pub(super) fn commits_reachable_from_head(
    commits: &[Arc<InitialGraphCommitData>],
    index_of: impl Fn(&Oid) -> Option<usize>,
) -> Vec<bool> {
    let mut reachable = vec![false; commits.len()];
    let Some(head) = commits.iter().position(|commit| {
        commit
            .ref_names
            .iter()
            .any(|decoration| is_head_decoration(decoration))
    }) else {
        return reachable;
    };
    let mut queue = VecDeque::from([head]);
    while let Some(index) = queue.pop_front() {
        let Some(seen) = reachable.get_mut(index) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        if let Some(commit) = commits.get(index) {
            queue.extend(commit.parents.iter().filter_map(|parent| index_of(parent)));
        }
    }
    reachable
}

pub(super) fn trim_hash_quotes(text: &str) -> &str {
    text.trim_matches(|character: char| HASH_TRIM_CHARACTERS.contains(character))
}

pub(super) fn is_hash_prefix(text: &str) -> bool {
    (MINIMUM_HASH_PREFIX_LENGTH..=MAXIMUM_HASH_PREFIX_LENGTH).contains(&text.len())
        && text.chars().all(|character| character.is_ascii_hexdigit())
}

pub(super) fn commit_presentation(sha: &Oid) -> String {
    format!("Commit '{}'", short_hash(sha))
}

pub(super) fn reference_presentation(text: &str) -> String {
    if is_hash_prefix(text) {
        format!(
            "Commit or reference '{}'",
            short_text(text, SHORT_HASH_LENGTH)
        )
    } else {
        format!("Commit or reference '{text}'")
    }
}

pub(super) fn not_found_message(presentation: &str, exists: bool) -> String {
    if exists {
        format!("{presentation} doesn't match the filters")
    } else {
        format!("{presentation} not found")
    }
}

pub(super) fn not_a_hash_message(text: &str) -> String {
    format!("'{text}' does not look like a commit hash")
}

pub(super) fn arrow_tooltip_text(sha: &Oid, data: Option<&CommitData>) -> String {
    match data {
        Some(data) => format!(
            "Jump to \"{}\" by {} on {} at {}",
            shorten_with_ellipsis(&data.subject, ARROW_SUBJECT_MAX_LENGTH),
            author_presentation(data),
            log_dates::format_date(data.author_timestamp),
            log_dates::format_time(data.author_timestamp)
        ),
        None => format!("Jump to commit {}", short_hash(sha)),
    }
}

pub(super) fn go_to_action_text(sha: &Oid, data: Option<&CommitData>) -> String {
    match data {
        Some(data) => format!(
            "{} \"{}\" by {} on {} at {}",
            short_hash(sha),
            shorten_with_ellipsis(&data.subject, ACTION_SUBJECT_MAX_LENGTH),
            author_presentation(data),
            log_dates::format_date(data.author_timestamp),
            log_dates::format_time(data.author_timestamp)
        ),
        None => short_hash(sha).to_string(),
    }
}

pub(super) fn subject_text(subject: &str) -> SharedString {
    subject.replace('\t', " ").trim().to_string().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use smallvec::smallvec;

    #[test]
    fn right_aligned_labels_tooltip_starts_at_column_width_minus_labels_width() {
        assert_eq!(
            labels_tooltip_areas(false, px(50.), px(500.), px(492.), px(100.), None),
            vec![(px(400.)..px(492.), LabelsTooltip::References)]
        );
        assert_eq!(
            labels_tooltip_areas(
                false,
                px(50.),
                px(500.),
                px(492.),
                px(100.),
                Some(px(4.)..px(30.))
            ),
            vec![
                (px(400.)..px(404.), LabelsTooltip::References),
                (px(404.)..px(430.), LabelsTooltip::DetachedHead),
                (px(430.)..px(492.), LabelsTooltip::References),
            ]
        );
    }

    #[test]
    fn left_aligned_labels_tooltip_follows_the_graph_and_never_covers_it() {
        assert_eq!(
            labels_tooltip_areas(true, px(50.), px(500.), px(492.), px(100.), None),
            vec![(px(50.)..px(150.), LabelsTooltip::References)]
        );
        assert_eq!(
            labels_tooltip_areas(false, px(50.), px(120.), px(112.), px(100.), None),
            vec![(px(50.)..px(112.), LabelsTooltip::References)]
        );
        assert!(
            labels_tooltip_areas(false, px(50.), px(500.), px(492.), Pixels::ZERO, None).is_empty()
        );
    }

    fn user(name: &str, email: &str) -> GitUser {
        GitUser {
            name: name.to_string().into(),
            email: email.to_string().into(),
        }
    }

    fn commit_data(author: (&str, &str), committer: (&str, &str), parents: usize) -> CommitData {
        CommitData {
            sha: Oid::from_bytes(&[1; 20]).unwrap(),
            parents: (0..parents)
                .map(|index| Oid::from_bytes(&[index as u8 + 2; 20]).unwrap())
                .collect(),
            author_name: author.0.to_string().into(),
            author_email: author.1.to_string().into(),
            commit_timestamp: 0,
            author_timestamp: 0,
            committer_name: committer.0.to_string().into(),
            committer_email: committer.1.to_string().into(),
            committer_timestamp: 0,
            subject: "Subject".into(),
            message: "Subject".into(),
        }
    }

    #[test]
    fn author_presentation_marks_a_different_committer_with_an_asterisk() {
        let same = commit_data(
            ("Ivan Petrov", "ivan@x.com"),
            ("ivan.petrov", "other@y.com"),
            1,
        );
        assert_eq!(author_presentation(&same).as_ref(), "Ivan Petrov");
        let different = commit_data(("Ivan Petrov", "ivan@x.com"), ("Bot", "bot@y.com"), 1);
        assert_eq!(author_presentation(&different).as_ref(), "Ivan Petrov*");
        let email_only = commit_data(("", "ivan@x.com"), ("", "ivan@x.com"), 1);
        assert_eq!(author_presentation(&email_only).as_ref(), "ivan");
    }

    #[test]
    fn names_in_standard_form_follow_vcs_user_util() {
        assert_eq!(name_in_standard_form("Ivan Petrov"), "ivan petrov");
        assert_eq!(name_in_standard_form("Ivan.Petrov"), "ivan petrov");
        assert_eq!(name_in_standard_form("a_b_c"), "a_b c");
        assert_eq!(
            name_in_standard_form("Ivan Ivanovich Petrov"),
            "ivan ivanovich petrov"
        );
        assert_eq!(name_in_standard_form("Иван Петров"), "Иван Петров");
    }

    #[test]
    fn current_users_are_resolved_by_name_and_by_email_name_part() {
        let current = user("Ivan Petrov", "ivan@company.com");
        let all_users = [
            user("Ivan Petrov", "ivan@company.com"),
            user("ivan.petrov", "IVAN@home.org"),
            user("Ivan", "ivan@company.com"),
            user("Ivan", "ivan@other.com"),
            user("Somebody", "somebody@company.com"),
        ];
        let resolved = resolve_current_users(&current, &all_users);
        assert!(resolved.contains(&("Ivan Petrov".to_string(), "ivan@company.com".to_string())));
        assert!(resolved.contains(&("ivan.petrov".to_string(), "ivan@home.org".to_string())));
        assert!(resolved.contains(&("Ivan".to_string(), "ivan@company.com".to_string())));
        assert!(!resolved.contains(&("Ivan".to_string(), "ivan@other.com".to_string())));
        assert!(!is_single_user(&current, &all_users));
        assert!(is_single_user(&current, &all_users[..2]));
    }

    #[test]
    fn filtering_by_me_disables_my_commits_highlighting() {
        assert!(is_filtered_by_me(&[USER_FILTER_ME.to_string()]));
        assert!(!is_filtered_by_me(&[]));
        assert!(!is_filtered_by_me(&[
            USER_FILTER_ME.to_string(),
            "Bob".to_string()
        ]));
    }

    #[test]
    fn single_filtered_branch_requires_exactly_one_existing_branch() {
        let names: Vec<SharedString> = vec!["main".into(), "origin/main".into(), "HEAD".into()];
        assert_eq!(
            single_filtered_branch(&["HEAD".into()], &names),
            Some(SharedString::from("HEAD"))
        );
        assert_eq!(single_filtered_branch(&["v1.0".into()], &names), None);
        assert_eq!(
            single_filtered_branch(&["main".into(), "origin/main".into()], &names),
            None
        );
        assert_eq!(single_filtered_branch(&[], &names), None);
    }

    #[test]
    fn selected_row_types_round_only_the_outer_corners() {
        assert_eq!(
            selected_row_type(true, false, false),
            SelectedRowType::Single
        );
        assert_eq!(selected_row_type(false, false, true), SelectedRowType::Top);
        assert_eq!(
            selected_row_type(false, true, true),
            SelectedRowType::Middle
        );
        assert_eq!(
            selected_row_type(false, true, false),
            SelectedRowType::Bottom
        );
        assert!(SelectedRowType::Top.rounds_top() && !SelectedRowType::Top.rounds_bottom());
        assert!(!SelectedRowType::Middle.rounds_top() && !SelectedRowType::Middle.rounds_bottom());
    }

    #[test]
    fn hash_prefixes_are_four_to_sixty_four_hex_digits_without_quotes() {
        assert_eq!(trim_hash_quotes("('abc123')"), "abc123");
        assert!(is_hash_prefix("abcd"));
        assert!(!is_hash_prefix("abc"));
        assert!(!is_hash_prefix("abcg12"));
        assert_eq!(
            reference_presentation("abcdef0123"),
            "Commit or reference 'abcdef01'"
        );
        assert_eq!(reference_presentation("main"), "Commit or reference 'main'");
        assert_eq!(
            not_found_message("Commit 'abcdef01'", true),
            "Commit 'abcdef01' doesn't match the filters"
        );
        assert_eq!(
            not_found_message("Commit 'abcdef01'", false),
            "Commit 'abcdef01' not found"
        );
    }

    #[test]
    fn reachability_from_head_follows_parents_of_the_head_commit_only() {
        let sha = |byte: u8| Oid::from_bytes(&[byte; 20]).unwrap();
        let commit = |own: u8, parents: &[u8], refs: Vec<SharedString>| {
            Arc::new(InitialGraphCommitData {
                sha: sha(own),
                parents: parents.iter().map(|parent| sha(*parent)).collect(),
                ref_names: refs,
                committer_timestamp: 0,
                dotted_parents: smallvec![],
            })
        };
        let commits = vec![
            commit(1, &[3], vec!["feature".into()]),
            commit(2, &[3], vec!["HEAD -> main".into()]),
            commit(3, &[4], vec![]),
            commit(4, &[], vec![]),
        ];
        let reachable = commits_reachable_from_head(&commits, |parent| {
            commits.iter().position(|commit| commit.sha == *parent)
        });
        assert_eq!(reachable, [false, true, true, true]);
    }

    #[test]
    fn subjects_are_shortened_with_three_dots() {
        assert_eq!(shorten_with_ellipsis("short", 50), "short");
        let long = "x".repeat(60);
        assert_eq!(
            shorten_with_ellipsis(&long, 50),
            format!("{}...", "x".repeat(47))
        );
        assert_eq!(subject_text("\tFix\tbug  ").as_ref(), "Fix bug");
    }
}
