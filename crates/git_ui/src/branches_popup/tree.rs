use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

use collections::{HashMap, HashSet};
use git::repository::{Branch, Tag, Upstream};
use gpui::SharedString;

use crate::branch_refs::RefTarget;
use crate::ref_matcher::QueryMatcher;

const MAX_RECENT_BRANCHES: usize = 5;
const CLEAR_FILTER_QUERY: &str = "/";
const DEFAULT_FAVORITE_LOCAL_NAMES: [&str; 2] = ["main", "master"];
const DEFAULT_FAVORITE_REMOTE_NAMES: [&str; 2] = ["origin/main", "origin/master"];
const REMOTE_HEAD_NAME: &str = "HEAD";

const MATCHED_CHARACTER_SCORE: i64 = 10;
const WORD_START_BONUS: i64 = 15;
const EXACT_CASE_BONUS: i64 = 1;
const CONSECUTIVE_BONUS: i64 = 12;
const FIRST_MATCH_BASE_SCORE: i64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Recent,
    Local,
    Remote,
    Tags,
}

impl SectionKind {
    pub const ALL: [SectionKind; 4] = [
        SectionKind::Recent,
        SectionKind::Local,
        SectionKind::Remote,
        SectionKind::Tags,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SectionKind::Recent => "Recent",
            SectionKind::Local => "Local",
            SectionKind::Remote => "Remote",
            SectionKind::Tags => "Tags",
        }
    }

    /// The section header: with several repositories IntelliJ names the repository the popup
    /// shows (`GitRefType.getInRepoText`).
    pub fn title(self, repository_name: Option<&str>) -> String {
        let Some(repository_name) = repository_name else {
            return self.label().to_string();
        };
        match self {
            SectionKind::Recent => format!("Recent Branches in {repository_name}"),
            SectionKind::Local => format!("Local Branches in {repository_name}"),
            SectionKind::Remote => format!("Remote Branches in {repository_name}"),
            SectionKind::Tags => format!("Tags in {repository_name}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeOptions {
    pub group_by_directory: bool,
    pub show_recent: bool,
    pub show_tags: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            group_by_directory: true,
            show_recent: true,
            show_tags: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefRow {
    pub target: RefTarget,
    pub label: SharedString,
    pub depth: usize,
    pub section: SectionKind,
    pub is_current: bool,
    pub is_favorite: bool,
    pub tracked: Option<SharedString>,
    pub incoming: u32,
    pub outgoing: u32,
    pub highlight_positions: Vec<usize>,
    pub score: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRow {
    pub path: SharedString,
    pub label: SharedString,
    pub depth: usize,
    pub section: SectionKind,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionRow {
    pub kind: SectionKind,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeRow {
    Section(SectionRow),
    Folder(FolderRow),
    Ref(RefRow),
}

#[derive(Clone)]
pub struct TreeInput<'a> {
    pub local: &'a [Branch],
    pub remote: &'a [Branch],
    pub tags: &'a [Tag],
    pub recent: &'a [SharedString],
    pub current_branch: Option<&'a str>,
    pub head_tag: Option<&'a str>,
    pub favorites: &'a HashSet<RefTarget>,
    pub options: TreeOptions,
    pub expanded: &'a HashSet<SharedString>,
    /// Expands every section and folder, for finding rows wherever they are.
    pub expand_all: bool,
    pub query: &'a str,
}

pub fn node_id_for_section(kind: SectionKind) -> SharedString {
    SharedString::from(format!("section:{}", kind.label()))
}

pub fn node_id_for_folder(section: SectionKind, path: &str) -> SharedString {
    SharedString::from(format!("folder:{}:{}", section.label(), path))
}

pub fn default_expanded() -> HashSet<SharedString> {
    let mut expanded = HashSet::default();
    expanded.insert(node_id_for_section(SectionKind::Recent));
    expanded
}

pub fn default_favorites(local: &[Branch], remote: &[Branch]) -> HashSet<RefTarget> {
    let mut favorites = HashSet::default();
    for branch in local {
        if DEFAULT_FAVORITE_LOCAL_NAMES.contains(&branch.name()) {
            favorites.insert(RefTarget::local(branch.name().to_string()));
        }
    }
    for branch in remote {
        if DEFAULT_FAVORITE_REMOTE_NAMES.contains(&branch.name()) {
            favorites.insert(RefTarget::remote(branch.name().to_string()));
        }
    }
    favorites
}

pub fn ancestor_node_ids(
    section: SectionKind,
    name: &str,
    group_by_directory: bool,
) -> Vec<SharedString> {
    let mut ids = vec![node_id_for_section(section)];
    if !group_by_directory {
        return ids;
    }
    let mut segments: Vec<&str> = name.split('/').collect();
    segments.pop();
    let mut path = String::new();
    for segment in segments {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(segment);
        ids.push(node_id_for_folder(section, &path));
    }
    ids
}

/// How many nodes sit directly under a section, expanded or not, like `TreeModel.getChildCount`.
pub fn section_child_count(input: &TreeInput<'_>, kind: SectionKind) -> usize {
    let mut expanded = HashSet::default();
    expanded.insert(node_id_for_section(kind));
    let rows = build_rows(&TreeInput {
        expanded: &expanded,
        expand_all: false,
        query: "",
        ..input.clone()
    });
    let mut in_section = false;
    let mut count = 0;
    for row in &rows {
        match row {
            TreeRow::Section(section) => in_section = section.kind == kind,
            TreeRow::Folder(FolderRow { depth: 1, .. }) | TreeRow::Ref(RefRow { depth: 1, .. })
                if in_section =>
            {
                count += 1
            }
            TreeRow::Folder(_) | TreeRow::Ref(_) => {}
        }
    }
    count
}

pub fn build_rows(input: &TreeInput<'_>) -> Vec<TreeRow> {
    let matcher = active_query(input.query).map(QueryMatcher::new);
    let query = matcher.as_ref();
    let recent_names = if input.options.show_recent {
        recent_branch_names(input)
    } else {
        Vec::new()
    };
    let group_by_directory = input.options.group_by_directory;
    let mut rows = Vec::new();
    for kind in SectionKind::ALL {
        let mut entries = section_entries(kind, input, query, &recent_names);
        if entries.is_empty() {
            continue;
        }
        let expanded = input.expand_all || input.expanded.contains(&node_id_for_section(kind));
        rows.push(TreeRow::Section(SectionRow { kind, expanded }));
        if !expanded {
            continue;
        }
        // Like IntelliJ's `LazyRefsSubtreeHolder`, the refs are sorted before the tree is built and
        // every folder takes the place of its first ref, so a folder holding the current branch or
        // a favorite moves up with it. Recent keeps the reflog order (`emptyBranchComparator`).
        if kind != SectionKind::Recent {
            sort_entries(&mut entries, group_by_directory);
        }
        let mut root = FolderNode::default();
        for entry in entries {
            root.insert(entry, group_by_directory);
        }
        let context = EmitContext {
            section: kind,
            expanded: input.expanded,
            force_expanded: input.expand_all,
        };
        emit_node(&root, "", 1, &context, &mut rows);
    }
    rows
}

/// Matches `text` against the popup query like IntelliJ's branches popup, returning the match
/// degree and the matched character positions.
pub fn match_popup_query(query: &str, text: &str) -> Option<(i64, Vec<usize>)> {
    match_name(&QueryMatcher::new(query), text)
}

fn match_name(matcher: &QueryMatcher, text: &str) -> Option<(i64, Vec<usize>)> {
    let found = matcher.match_name(text)?;
    let positions = found.fragments.into_iter().flatten().collect();
    Some((i64::from(found.degree), positions))
}

pub fn match_query(query: &str, text: &str) -> Option<(i64, Vec<usize>)> {
    let query_chars: Vec<char> = query.trim().chars().collect();
    if query_chars.is_empty() {
        return Some((0, Vec::new()));
    }
    let text_chars: Vec<char> = text.chars().collect();
    if query_chars.len() > text_chars.len() {
        return None;
    }
    let query_folded: Vec<char> = query_chars.iter().copied().map(fold_char).collect();
    let text_folded: Vec<char> = text_chars.iter().copied().map(fold_char).collect();
    let query_length = query_chars.len();
    let width = text_chars.len();
    let mut scores: Vec<Option<i64>> = vec![None; query_length * width];
    let mut previous: Vec<usize> = vec![0; query_length * width];

    for row in 0..query_length {
        let mut best_before: Option<(i64, usize)> = None;
        for column in 0..width {
            if row > 0 && column >= 2 {
                let earlier_column = column - 2;
                if let Some(score) = scores[(row - 1) * width + earlier_column] {
                    match best_before {
                        Some((best, _)) if best >= score => {}
                        _ => best_before = Some((score, earlier_column)),
                    }
                }
            }
            if query_folded[row] != text_folded[column] {
                continue;
            }
            let gain = character_gain(&text_chars, column, query_chars[row]);
            let candidate = if row == 0 {
                Some((gain, 0))
            } else {
                let separated = best_before.map(|(score, from)| (score + gain, from));
                let adjacent = column.checked_sub(1).and_then(|from| {
                    scores[(row - 1) * width + from]
                        .map(|score| (score + gain + CONSECUTIVE_BONUS, from))
                });
                match (adjacent, separated) {
                    (Some(adjacent), Some(separated)) => Some(if adjacent.0 >= separated.0 {
                        adjacent
                    } else {
                        separated
                    }),
                    (adjacent, separated) => adjacent.or(separated),
                }
            };
            if let Some((score, from)) = candidate {
                scores[row * width + column] = Some(score);
                previous[row * width + column] = from;
            }
        }
    }

    let last_row = query_length - 1;
    let mut best: Option<(i64, usize)> = None;
    for column in 0..width {
        if let Some(score) = scores[last_row * width + column] {
            match best {
                Some((best_score, _)) if best_score >= score => {}
                _ => best = Some((score, column)),
            }
        }
    }
    let (degree, mut column) = best?;
    let mut positions = vec![0; query_length];
    for row in (0..query_length).rev() {
        positions[row] = column;
        if row > 0 {
            column = previous[row * width + column];
        }
    }
    let first_match_offset = positions[0] as i64;
    Some((
        degree + FIRST_MATCH_BASE_SCORE - first_match_offset,
        positions,
    ))
}

fn character_gain(text: &[char], column: usize, query_char: char) -> i64 {
    let mut gain = MATCHED_CHARACTER_SCORE;
    let starts_word = match column.checked_sub(1).map(|index| text[index]) {
        None => true,
        Some(before) => {
            !before.is_alphanumeric() || (before.is_lowercase() && text[column].is_uppercase())
        }
    };
    if starts_word {
        gain += WORD_START_BONUS;
    }
    if text[column] == query_char {
        gain += EXACT_CASE_BONUS;
    }
    gain
}

fn fold_char(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

fn natural_compare(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return left.cmp(right),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char))
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() =>
            {
                let left_digits = take_digits(&mut left_chars);
                let right_digits = take_digits(&mut right_chars);
                let ordering = compare_digit_runs(&left_digits, &right_digits);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(left_char), Some(right_char)) => {
                let ordering = fold_char(left_char).cmp(&fold_char(right_char));
                if ordering != Ordering::Equal {
                    return ordering;
                }
                left_chars.next();
                right_chars.next();
            }
        }
    }
}

fn take_digits(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(&character) = chars.peek() {
        if !character.is_ascii_digit() {
            break;
        }
        digits.push(character);
        chars.next();
    }
    digits
}

fn compare_digit_runs(left: &str, right: &str) -> Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn active_query(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == CLEAR_FILTER_QUERY {
        None
    } else {
        Some(trimmed)
    }
}

fn match_entry(query: Option<&QueryMatcher>, name: &str) -> Option<(i64, Vec<usize>)> {
    match query {
        None => Some((0, Vec::new())),
        Some(matcher) => match_name(matcher, name),
    }
}

fn is_remote_head(name: &str) -> bool {
    name == REMOTE_HEAD_NAME
        || name
            .strip_suffix(REMOTE_HEAD_NAME)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn tracked_name(upstream: &Upstream) -> SharedString {
    let name = upstream
        .stripped_ref_name()
        .or_else(|| upstream.ref_name.strip_prefix("refs/heads/"))
        .unwrap_or(&upstream.ref_name);
    SharedString::from(name.to_string())
}

fn recent_branch_names<'a>(input: &TreeInput<'a>) -> Vec<&'a str> {
    let mut names: Vec<&'a str> = Vec::new();
    for name in input.recent {
        if names.len() >= MAX_RECENT_BRANCHES {
            break;
        }
        let name: &'a str = name;
        if names.contains(&name) {
            continue;
        }
        if input.local.iter().any(|branch| branch.name() == name) {
            names.push(name);
        }
    }
    names
}

struct Entry {
    target: RefTarget,
    is_current: bool,
    is_favorite: bool,
    tracked: Option<SharedString>,
    incoming: u32,
    outgoing: u32,
    match_positions: Vec<usize>,
    score: i64,
}

fn local_entry(
    branch: &Branch,
    input: &TreeInput<'_>,
    query: Option<&QueryMatcher>,
) -> Option<Entry> {
    let name = branch.name();
    let (score, match_positions) = match_entry(query, name)?;
    let target = RefTarget::local(name.to_string());
    let status = branch.tracking_status();
    Some(Entry {
        is_current: input.current_branch == Some(name),
        is_favorite: input.favorites.contains(&target),
        tracked: branch.upstream.as_ref().map(tracked_name),
        incoming: status.map_or(0, |status| status.behind),
        outgoing: status.map_or(0, |status| status.ahead),
        match_positions,
        score,
        target,
    })
}

fn remote_entry(
    branch: &Branch,
    input: &TreeInput<'_>,
    query: Option<&QueryMatcher>,
) -> Option<Entry> {
    let name = branch.name();
    if is_remote_head(name) {
        return None;
    }
    let (score, match_positions) = match_entry(query, name)?;
    let target = RefTarget::remote(name.to_string());
    Some(Entry {
        is_current: false,
        is_favorite: input.favorites.contains(&target),
        tracked: None,
        incoming: 0,
        outgoing: 0,
        match_positions,
        score,
        target,
    })
}

fn tag_entry(tag: &Tag, input: &TreeInput<'_>, query: Option<&QueryMatcher>) -> Option<Entry> {
    let (score, match_positions) = match_entry(query, &tag.name)?;
    let target = RefTarget::tag(tag.name.clone());
    Some(Entry {
        is_current: input.head_tag == Some(&*tag.name),
        is_favorite: input.favorites.contains(&target),
        tracked: None,
        incoming: 0,
        outgoing: 0,
        match_positions,
        score,
        target,
    })
}

fn section_entries(
    kind: SectionKind,
    input: &TreeInput<'_>,
    query: Option<&QueryMatcher>,
    recent_names: &[&str],
) -> Vec<Entry> {
    match kind {
        SectionKind::Recent => recent_names
            .iter()
            .filter_map(|name| {
                input
                    .local
                    .iter()
                    .find(|branch| branch.name() == *name)
                    .and_then(|branch| local_entry(branch, input, query))
            })
            .collect(),
        // IntelliJ's except-filter only takes recent branches out of Local while a search pattern
        // is set (`match` returns every node when the matcher is null).
        SectionKind::Local => input
            .local
            .iter()
            .filter(|branch| query.is_none() || !recent_names.contains(&branch.name()))
            .filter_map(|branch| local_entry(branch, input, query))
            .collect(),
        SectionKind::Remote => input
            .remote
            .iter()
            .filter_map(|branch| remote_entry(branch, input, query))
            .collect(),
        SectionKind::Tags => {
            if input.options.show_tags {
                input
                    .tags
                    .iter()
                    .filter_map(|tag| tag_entry(tag, input, query))
                    .collect()
            } else {
                Vec::new()
            }
        }
    }
}

fn sort_entries(entries: &mut [Entry], group_by_directory: bool) {
    entries.sort_by(|left, right| {
        entry_order_key(left, group_by_directory)
            .cmp(&entry_order_key(right, group_by_directory))
            .then_with(|| natural_compare(&left.target.name, &right.target.name))
    });
}

fn entry_order_key(entry: &Entry, group_by_directory: bool) -> (bool, bool, bool) {
    (
        !entry.is_current,
        !entry.is_favorite,
        !(group_by_directory && entry.target.name.contains('/')),
    )
}

struct Leaf {
    label: String,
    entry: Entry,
}

enum FolderChild {
    Leaf(Leaf),
    Folder { label: String, node: FolderNode },
}

#[derive(Default)]
struct FolderNode {
    children: Vec<FolderChild>,
    folder_positions: HashMap<String, usize>,
}

impl FolderNode {
    fn insert(&mut self, entry: Entry, group_by_directory: bool) {
        let name = entry.target.name.to_string();
        if !group_by_directory {
            self.children
                .push(FolderChild::Leaf(Leaf { label: name, entry }));
            return;
        }
        let mut segments: Vec<&str> = name.split('/').collect();
        let label = segments.pop().unwrap_or_default().to_string();
        self.insert_below(&segments, Leaf { label, entry });
    }

    fn insert_below(&mut self, folders: &[&str], leaf: Leaf) {
        let Some((folder, rest)) = folders.split_first() else {
            self.children.push(FolderChild::Leaf(leaf));
            return;
        };
        let next_position = self.children.len();
        let position = *self
            .folder_positions
            .entry(folder.to_string())
            .or_insert(next_position);
        if position == next_position {
            self.children.push(FolderChild::Folder {
                label: folder.to_string(),
                node: FolderNode::default(),
            });
        }
        if let Some(FolderChild::Folder { node, .. }) = self.children.get_mut(position) {
            node.insert_below(rest, leaf);
        }
    }
}

fn subtree_group(child: &FolderChild) -> u8 {
    match child {
        FolderChild::Leaf(leaf) if leaf.entry.is_current || leaf.entry.is_favorite => 0,
        FolderChild::Folder { .. } => 1,
        FolderChild::Leaf(_) => 2,
    }
}

struct EmitContext<'a> {
    section: SectionKind,
    expanded: &'a HashSet<SharedString>,
    force_expanded: bool,
}

fn emit_node(
    node: &FolderNode,
    folder_path: &str,
    depth: usize,
    context: &EmitContext<'_>,
    rows: &mut Vec<TreeRow>,
) {
    let mut children: Vec<&FolderChild> = node.children.iter().collect();
    // The section level keeps the insertion order; inside a folder IntelliJ stably re-sorts the
    // children into current and favorite refs, then folders, then the rest (`getSubTreeComparator`).
    if depth > 1 {
        children.sort_by_key(|child| subtree_group(child));
    }
    for child in children {
        let (label, child_node) = match child {
            FolderChild::Leaf(leaf) => {
                rows.push(ref_row(leaf, depth, context.section));
                continue;
            }
            FolderChild::Folder { label, node } => (label, node),
        };
        let path = if folder_path.is_empty() {
            label.clone()
        } else {
            format!("{folder_path}/{label}")
        };
        let id = node_id_for_folder(context.section, &path);
        let expanded = context.force_expanded || context.expanded.contains(&id);
        rows.push(TreeRow::Folder(FolderRow {
            path: SharedString::from(path.as_str()),
            label: SharedString::from(label.as_str()),
            depth,
            section: context.section,
            expanded,
        }));
        if expanded {
            emit_node(child_node, &path, depth + 1, context, rows);
        }
    }
}

fn ref_row(leaf: &Leaf, depth: usize, section: SectionKind) -> TreeRow {
    let entry = &leaf.entry;
    let name_length = entry.target.name.chars().count();
    let label_length = leaf.label.chars().count();
    let label_offset = name_length.saturating_sub(label_length);
    let highlight_positions = entry
        .match_positions
        .iter()
        .filter_map(|position| position.checked_sub(label_offset))
        .collect();
    TreeRow::Ref(RefRow {
        target: entry.target.clone(),
        label: SharedString::from(leaf.label.as_str()),
        depth,
        section,
        is_current: entry.is_current,
        is_favorite: entry.is_favorite,
        tracked: entry.tracked.clone(),
        incoming: entry.incoming,
        outgoing: entry.outgoing,
        highlight_positions,
        score: entry.score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use git::repository::{UpstreamTracking, UpstreamTrackingStatus};

    fn local_branch(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/heads/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn remote_branch(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/remotes/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn tracking_branch(name: &str, upstream: &str, tracking: UpstreamTracking) -> Branch {
        Branch {
            upstream: Some(Upstream {
                ref_name: SharedString::from(upstream.to_string()),
                tracking,
            }),
            ..local_branch(name)
        }
    }

    fn tag(name: &str) -> Tag {
        Tag {
            name: SharedString::from(name.to_string()),
            commit_sha: SharedString::from("0123456789abcdef".to_string()),
        }
    }

    fn tags(names: &[&str]) -> Vec<Tag> {
        names.iter().map(|name| tag(name)).collect()
    }

    fn locals(names: &[&str]) -> Vec<Branch> {
        names.iter().map(|name| local_branch(name)).collect()
    }

    fn remotes(names: &[&str]) -> Vec<Branch> {
        names.iter().map(|name| remote_branch(name)).collect()
    }

    fn shared_strings(names: &[&str]) -> Vec<SharedString> {
        names
            .iter()
            .map(|name| SharedString::from(name.to_string()))
            .collect()
    }

    #[derive(Default)]
    struct Fixture {
        local: Vec<Branch>,
        remote: Vec<Branch>,
        tags: Vec<Tag>,
        recent: Vec<SharedString>,
        current: Option<String>,
        head_tag: Option<String>,
        favorites: HashSet<RefTarget>,
        options: TreeOptions,
        expanded: HashSet<SharedString>,
        expand_all: bool,
        query: String,
    }

    impl Fixture {
        fn rows(&self) -> Vec<TreeRow> {
            build_rows(&TreeInput {
                local: &self.local,
                remote: &self.remote,
                tags: &self.tags,
                recent: &self.recent,
                current_branch: self.current.as_deref(),
                head_tag: self.head_tag.as_deref(),
                favorites: &self.favorites,
                options: self.options,
                expanded: &self.expanded,
                expand_all: self.expand_all,
                query: &self.query,
            })
        }

        fn expand_sections(&mut self, kinds: &[SectionKind]) {
            for kind in kinds {
                self.expanded.insert(node_id_for_section(*kind));
            }
        }

        fn expand_folder(&mut self, section: SectionKind, path: &str) {
            self.expanded.insert(node_id_for_folder(section, path));
        }
    }

    fn describe(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                TreeRow::Section(section) => format!("S:{}", section.kind.label()),
                TreeRow::Folder(folder) => {
                    format!("{}D:{}", "  ".repeat(folder.depth), folder.path)
                }
                TreeRow::Ref(row) => format!("{}R:{}", "  ".repeat(row.depth), row.label),
            })
            .collect()
    }

    fn ref_rows(rows: &[TreeRow]) -> Vec<&RefRow> {
        rows.iter()
            .filter_map(|row| match row {
                TreeRow::Ref(row) => Some(row),
                _ => None,
            })
            .collect()
    }

    fn section_expanded(rows: &[TreeRow], kind: SectionKind) -> Option<bool> {
        rows.iter().find_map(|row| match row {
            TreeRow::Section(section) if section.kind == kind => Some(section.expanded),
            _ => None,
        })
    }

    #[test]
    fn sections_follow_fixed_order_and_empty_sections_are_hidden() {
        let mut fixture = Fixture {
            local: locals(&["main", "dev"]),
            remote: remotes(&["origin/main"]),
            recent: shared_strings(&["dev"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&SectionKind::ALL);
        fixture.expand_folder(SectionKind::Remote, "origin");
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Recent",
                "  R:dev",
                "S:Local",
                "  R:dev",
                "  R:main",
                "S:Remote",
                "  D:origin",
                "    R:main",
            ]
        );

        fixture.tags = vec![tag("v1")];
        fixture.expand_sections(&SectionKind::ALL);
        assert_eq!(
            describe(&fixture.rows()).last().map(String::as_str),
            Some("  R:v1")
        );

        let empty = Fixture::default();
        assert!(empty.rows().is_empty());
    }

    #[test]
    fn collapsed_sections_show_only_their_header_rows() {
        let fixture = Fixture {
            local: locals(&["main"]),
            remote: remotes(&["origin/main"]),
            tags: vec![tag("v1")],
            recent: shared_strings(&["main"]),
            expanded: default_expanded(),
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            ["S:Recent", "  R:main", "S:Local", "S:Remote", "S:Tags"]
        );
        assert_eq!(section_expanded(&rows, SectionKind::Recent), Some(true));
        assert_eq!(section_expanded(&rows, SectionKind::Local), Some(false));
        assert_eq!(section_expanded(&rows, SectionKind::Remote), Some(false));
        assert_eq!(section_expanded(&rows, SectionKind::Tags), Some(false));
    }

    #[test]
    fn recent_is_capped_at_five_deduplicated_and_limited_to_existing_local_branches() {
        let mut fixture = Fixture {
            local: locals(&["main", "a", "b", "c", "d", "e", "f", "g"]),
            recent: shared_strings(&["gone", "c", "main", "c", "a", "b", "d", "e"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Recent, SectionKind::Local]);
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Recent", "  R:c", "  R:main", "  R:a", "  R:b", "  R:d", "S:Local", "  R:a",
                "  R:b", "  R:c", "  R:d", "  R:e", "  R:f", "  R:g", "  R:main",
            ]
        );
    }

    #[test]
    fn recent_rows_keep_reflog_order_and_are_grouped_by_directory() {
        let mut fixture = Fixture {
            local: locals(&["feature/x", "main", "zzz"]),
            recent: shared_strings(&["zzz", "feature/x", "main"]),
            current: Some("main".to_string()),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("feature/x"));
        fixture.expand_sections(&[SectionKind::Recent]);
        // Without a query Local still lists the recent branches, so its collapsed header follows.
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Recent", "  R:zzz", "  D:feature", "  R:main", "S:Local"]
        );

        fixture.expand_folder(SectionKind::Recent, "feature");
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Recent",
                "  R:zzz",
                "  D:feature",
                "    R:x",
                "  R:main",
                "S:Local"
            ]
        );
        let refs = ref_rows(&rows);
        assert_eq!(refs[1].target, RefTarget::local("feature/x"));
        assert!(refs[1].is_favorite);
        assert!(refs[2].is_current);
        assert!(refs.iter().all(|row| row.section == SectionKind::Recent));

        fixture.options.group_by_directory = false;
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Recent",
                "  R:zzz",
                "  R:feature/x",
                "  R:main",
                "S:Local"
            ]
        );
    }

    #[test]
    fn recent_folders_keep_reflog_order_but_lead_with_favorites_inside() {
        let mut fixture = Fixture {
            local: locals(&["feature/b", "feature/a", "feature/c", "main"]),
            recent: shared_strings(&["feature/b", "main", "feature/a", "feature/c"]),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("feature/c"));
        fixture.expand_sections(&[SectionKind::Recent]);
        fixture.expand_folder(SectionKind::Recent, "feature");
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Recent",
                "  D:feature",
                "    R:c",
                "    R:b",
                "    R:a",
                "  R:main",
                "S:Local"
            ]
        );
    }

    #[test]
    fn recent_branches_are_not_removed_from_local_when_recent_is_hidden() {
        let mut fixture = Fixture {
            local: locals(&["main", "dev"]),
            recent: shared_strings(&["dev"]),
            options: TreeOptions {
                show_recent: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&SectionKind::ALL);
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  R:dev", "  R:main"]
        );
    }

    #[test]
    fn recent_branches_are_repeated_in_local_without_a_query() {
        let mut fixture = Fixture {
            local: locals(&["main"]),
            recent: shared_strings(&["main"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&SectionKind::ALL);
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Recent", "  R:main", "S:Local", "  R:main"]
        );
    }

    #[test]
    fn a_section_emptied_by_recent_extraction_while_searching_disappears() {
        let fixture = Fixture {
            local: locals(&["main"]),
            recent: shared_strings(&["main"]),
            query: "main".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert_eq!(describe(&fixture.rows()), ["S:Recent", "  R:main"]);
    }

    #[test]
    fn tags_are_hidden_when_show_tags_is_off() {
        let mut fixture = Fixture {
            tags: vec![tag("v1")],
            options: TreeOptions {
                show_tags: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&SectionKind::ALL);
        assert!(fixture.rows().is_empty());

        fixture.options.show_tags = true;
        assert_eq!(describe(&fixture.rows()), ["S:Tags", "  R:v1"]);
        let rows = fixture.rows();
        assert_eq!(ref_rows(&rows)[0].target, RefTarget::tag("v1"));
    }

    #[test]
    fn tags_use_natural_order_and_directory_grouping() {
        let mut fixture = Fixture {
            tags: vec![tag("v1.10"), tag("v1.9"), tag("release/v2")],
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Tags]);
        fixture.expand_folder(SectionKind::Tags, "release");
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Tags", "  D:release", "    R:v2", "  R:v1.9", "  R:v1.10"]
        );
    }

    #[test]
    fn remote_head_symrefs_are_filtered_out() {
        let mut fixture = Fixture {
            remote: remotes(&["origin/HEAD", "origin/main", "upstream/HEAD", "HEAD"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Remote]);
        assert_eq!(describe(&fixture.rows()), ["S:Remote", "  R:origin/main"]);

        fixture.remote = remotes(&["origin/HEAD"]);
        assert!(fixture.rows().is_empty());
    }

    #[test]
    fn remote_branch_with_head_letters_in_its_name_is_kept_but_slash_head_refs_are_dropped() {
        let mut fixture = Fixture {
            remote: remotes(&["origin/AHEAD", "origin/feature/HEAD"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Remote]);
        assert_eq!(describe(&fixture.rows()), ["S:Remote", "  R:origin/AHEAD"]);
    }

    #[test]
    fn local_refs_sort_current_then_favorites_then_natural_case_insensitive_order() {
        let mut fixture = Fixture {
            local: locals(&["zeta", "Alpha", "beta10", "beta2", "main"]),
            current: Some("zeta".to_string()),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("main"));
        fixture.expand_sections(&[SectionKind::Local]);
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Local",
                "  R:zeta",
                "  R:main",
                "  R:Alpha",
                "  R:beta2",
                "  R:beta10"
            ]
        );
        let refs = ref_rows(&rows);
        assert!(refs[0].is_current && !refs[0].is_favorite);
        assert!(!refs[1].is_current && refs[1].is_favorite);
        assert!(!refs[2].is_current && !refs[2].is_favorite);
    }

    #[test]
    fn current_branch_that_is_also_favorite_is_flagged_for_both() {
        let mut fixture = Fixture {
            local: locals(&["main", "dev"]),
            current: Some("main".to_string()),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("main"));
        fixture.expand_sections(&[SectionKind::Local]);
        let rows = fixture.rows();
        let refs = ref_rows(&rows);
        assert!(refs[0].is_current && refs[0].is_favorite);
    }

    #[test]
    fn grouping_puts_folders_before_plain_refs_and_labels_leaves_with_their_last_segment() {
        let mut fixture = Fixture {
            local: locals(&[
                "feature/login",
                "feature/signup",
                "fix/typo",
                "main",
                "bugfix",
            ]),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local]);
        fixture.expand_folder(SectionKind::Local, "feature");
        fixture.expand_folder(SectionKind::Local, "fix");
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Local",
                "  D:feature",
                "    R:login",
                "    R:signup",
                "  D:fix",
                "    R:typo",
                "  R:bugfix",
                "  R:main",
            ]
        );
        let login = ref_rows(&rows)[0];
        assert_eq!(login.target, RefTarget::local("feature/login"));
        assert_eq!(login.depth, 2);
    }

    #[test]
    fn collapsed_folders_hide_their_children_and_report_their_state() {
        let mut fixture = Fixture {
            local: locals(&["feature/login", "fix/typo", "main"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local]);
        fixture.expand_folder(SectionKind::Local, "fix");
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Local",
                "  D:feature",
                "  D:fix",
                "    R:typo",
                "  R:main"
            ]
        );
        let folder_states: Vec<(String, bool)> = rows
            .iter()
            .filter_map(|row| match row {
                TreeRow::Folder(folder) => Some((folder.path.to_string(), folder.expanded)),
                _ => None,
            })
            .collect();
        assert_eq!(
            folder_states,
            [("feature".to_string(), false), ("fix".to_string(), true)]
        );
    }

    #[test]
    fn folder_expansion_is_tracked_per_section() {
        let mut fixture = Fixture {
            local: locals(&["origin/shared"]),
            remote: remotes(&["origin/shared"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local, SectionKind::Remote]);
        fixture.expand_folder(SectionKind::Local, "origin");
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Local",
                "  D:origin",
                "    R:shared",
                "S:Remote",
                "  D:origin"
            ]
        );
    }

    #[test]
    fn without_grouping_refs_stay_flat_with_full_names() {
        let mut fixture = Fixture {
            local: locals(&["main", "feature/login"]),
            remote: remotes(&["origin/main"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local, SectionKind::Remote]);
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Local",
                "  R:feature/login",
                "  R:main",
                "S:Remote",
                "  R:origin/main"
            ]
        );
    }

    #[test]
    fn remote_refs_group_under_their_remote_folder_and_favorites_lead_inside_it() {
        let mut fixture = Fixture {
            remote: remotes(&[
                "origin/main",
                "origin/feature/x",
                "origin/HEAD",
                "upstream/dev",
                "origin/alpha",
            ]),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::remote("origin/main"));
        fixture.expand_sections(&[SectionKind::Remote]);
        fixture.expand_folder(SectionKind::Remote, "origin");
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Remote",
                "  D:origin",
                "    R:main",
                "    D:origin/feature",
                "    R:alpha",
                "  D:upstream",
            ]
        );
        let main = ref_rows(&rows)[0];
        assert_eq!(main.target, RefTarget::remote("origin/main"));
        assert!(main.is_favorite);
        assert!(!main.is_current);
        assert_eq!(main.section, SectionKind::Remote);
    }

    #[test]
    fn current_and_favorite_leaves_precede_folders_which_precede_plain_refs() {
        let mut fixture = Fixture {
            local: locals(&["aaa", "feature/a", "yyy", "zzz"]),
            current: Some("zzz".to_string()),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("yyy"));
        fixture.expand_sections(&[SectionKind::Local]);
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  R:zzz", "  R:yyy", "  D:feature", "  R:aaa"]
        );
    }

    #[test]
    fn a_folder_holding_the_current_branch_comes_before_favorite_leaves() {
        let mut fixture = Fixture {
            local: locals(&["main", "feature/x", "aaa"]),
            current: Some("feature/x".to_string()),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("main"));
        fixture.expand_sections(&[SectionKind::Local]);
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  D:feature", "  R:main", "  R:aaa"]
        );
    }

    #[test]
    fn folders_holding_favorites_come_before_other_folders_and_favorite_leaves() {
        let mut fixture = Fixture {
            local: locals(&["aaa/b", "zzz/y", "main", "dev"]),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("zzz/y"));
        fixture.expand_sections(&[SectionKind::Local]);
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  D:zzz", "  D:aaa", "  R:dev", "  R:main"]
        );

        fixture.favorites.insert(RefTarget::local("main"));
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  D:zzz", "  R:main", "  D:aaa", "  R:dev"]
        );
    }

    #[test]
    fn nested_folders_holding_favorites_come_first_among_folders() {
        let mut fixture = Fixture {
            local: locals(&["team/alpha/one", "team/zeta/two", "team/plain"]),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("team/zeta/two"));
        fixture.expand_sections(&[SectionKind::Local]);
        fixture.expand_folder(SectionKind::Local, "team");
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Local",
                "  D:team",
                "    D:team/zeta",
                "    D:team/alpha",
                "    R:plain"
            ]
        );
    }

    #[test]
    fn favorites_lead_inside_a_nested_folder() {
        let mut fixture = Fixture {
            local: locals(&["feature/a", "feature/b", "feature/c"]),
            ..Fixture::default()
        };
        fixture.favorites.insert(RefTarget::local("feature/c"));
        fixture.expand_sections(&[SectionKind::Local]);
        fixture.expand_folder(SectionKind::Local, "feature");
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  D:feature", "    R:c", "    R:a", "    R:b"]
        );
    }

    #[test]
    fn in_a_detached_head_the_tag_at_head_is_marked_current_and_sorted_first() {
        let mut fixture = Fixture {
            tags: tags(&["v1", "v2", "v3"]),
            head_tag: Some("v2".to_string()),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Tags]);
        let rows = fixture.rows();
        assert_eq!(describe(&rows), ["S:Tags", "  R:v2", "  R:v1", "  R:v3"]);
        let refs = ref_rows(&rows);
        assert!(refs[0].is_current);
        assert!(refs[1..].iter().all(|row| !row.is_current));

        fixture.head_tag = None;
        let rows = fixture.rows();
        assert_eq!(describe(&rows), ["S:Tags", "  R:v1", "  R:v2", "  R:v3"]);
        assert!(ref_rows(&rows).iter().all(|row| !row.is_current));
    }

    #[test]
    fn tracked_incoming_and_outgoing_are_reported_for_local_branches_only() {
        let tracked = tracking_branch(
            "main",
            "refs/remotes/origin/main",
            UpstreamTracking::Tracked(UpstreamTrackingStatus {
                ahead: 2,
                behind: 3,
            }),
        );
        let gone = tracking_branch("gone", "refs/remotes/origin/gone", UpstreamTracking::Gone);
        let mut fixture = Fixture {
            local: vec![tracked, gone, local_branch("plain")],
            remote: remotes(&["origin/main"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local, SectionKind::Remote]);
        let rows = fixture.rows();
        let refs = ref_rows(&rows);
        assert_eq!(refs.len(), 4);

        let gone_row = refs[0];
        assert_eq!(gone_row.label, "gone");
        assert_eq!(gone_row.tracked.as_deref(), Some("origin/gone"));
        assert_eq!((gone_row.incoming, gone_row.outgoing), (0, 0));

        let main_row = refs[1];
        assert_eq!(main_row.label, "main");
        assert_eq!(main_row.tracked.as_deref(), Some("origin/main"));
        assert_eq!((main_row.incoming, main_row.outgoing), (3, 2));

        let plain_row = refs[2];
        assert_eq!(plain_row.tracked, None);
        assert_eq!((plain_row.incoming, plain_row.outgoing), (0, 0));

        let remote_row = refs[3];
        assert_eq!(remote_row.target, RefTarget::remote("origin/main"));
        assert_eq!(remote_row.tracked, None);
        assert_eq!((remote_row.incoming, remote_row.outgoing), (0, 0));
    }

    #[test]
    fn local_upstream_without_remote_prefix_is_reported_by_its_branch_name() {
        let upstream_on_local = tracking_branch(
            "topic",
            "refs/heads/main",
            UpstreamTracking::Tracked(UpstreamTrackingStatus {
                ahead: 0,
                behind: 0,
            }),
        );
        let mut fixture = Fixture {
            local: vec![upstream_on_local],
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local]);
        let rows = fixture.rows();
        assert_eq!(ref_rows(&rows)[0].tracked.as_deref(), Some("main"));
    }

    #[test]
    fn while_searching_only_the_expanded_sections_and_folders_are_open() {
        let mut fixture = Fixture {
            local: locals(&["main", "feature/login"]),
            remote: remotes(&["origin/feature/login"]),
            query: "login".to_string(),
            ..Fixture::default()
        };
        assert_eq!(describe(&fixture.rows()), ["S:Local", "S:Remote"]);

        fixture.expand_sections(&[SectionKind::Local]);
        fixture.expand_folder(SectionKind::Local, "feature");
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  D:feature", "    R:login", "S:Remote"]
        );
    }

    #[test]
    fn query_filters_refs_and_forces_ancestors_open_regardless_of_expanded_state() {
        let fixture = Fixture {
            local: locals(&["main", "feature/login", "feature/signup", "dev"]),
            remote: remotes(&["origin/main", "origin/feature/login"]),
            tags: vec![tag("v1")],
            query: "login".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(
            describe(&rows),
            [
                "S:Local",
                "  D:feature",
                "    R:login",
                "S:Remote",
                "  D:origin",
                "    D:origin/feature",
                "      R:login",
            ]
        );
        assert_eq!(section_expanded(&rows, SectionKind::Local), Some(true));
        assert!(rows.iter().all(|row| match row {
            TreeRow::Folder(folder) => folder.expanded,
            _ => true,
        }));
    }

    #[test]
    fn query_matches_the_full_name_and_maps_highlights_onto_the_leaf_label() {
        let fixture = Fixture {
            remote: remotes(&["origin/main", "origin/dev"]),
            query: "origin/ma".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(describe(&rows), ["S:Remote", "  D:origin", "    R:main"]);
        assert_eq!(ref_rows(&rows)[0].highlight_positions, [0, 1]);
    }

    #[test]
    fn highlight_positions_are_relative_to_the_label_and_skip_folder_characters() {
        let fixture = Fixture {
            local: locals(&["feature/login"]),
            query: "login".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(ref_rows(&rows)[0].highlight_positions, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn highlight_positions_use_the_full_name_when_grouping_is_off() {
        let fixture = Fixture {
            local: locals(&["feature/login"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            query: "login".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(ref_rows(&rows)[0].highlight_positions, [8, 9, 10, 11, 12]);
    }

    #[test]
    fn query_without_matches_yields_no_rows_and_unfiltered_rows_have_no_highlights() {
        let mut fixture = Fixture {
            local: locals(&["main"]),
            query: "zzz".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert!(fixture.rows().is_empty());

        fixture.query.clear();
        fixture.expand_sections(&[SectionKind::Local]);
        let rows = fixture.rows();
        assert!(ref_rows(&rows)[0].highlight_positions.is_empty());
    }

    #[test]
    fn query_is_trimmed_and_a_lone_slash_clears_the_filter() {
        let mut fixture = Fixture {
            local: locals(&["main", "dev"]),
            query: "  dev ".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert_eq!(describe(&fixture.rows()), ["S:Local", "  R:dev"]);

        fixture.expand_all = false;
        fixture.query = "/".to_string();
        assert_eq!(describe(&fixture.rows()), ["S:Local"]);

        fixture.query = "   ".to_string();
        assert_eq!(describe(&fixture.rows()), ["S:Local"]);
    }

    #[test]
    fn query_filters_recent_and_extracted_branches_stay_out_of_local() {
        let fixture = Fixture {
            local: locals(&["main", "dev", "devops"]),
            recent: shared_strings(&["main", "dev"]),
            query: "dev".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Recent", "  R:dev", "S:Local", "  R:devops"]
        );
    }

    #[test]
    fn recent_cap_applies_before_the_query_so_overflowing_recents_stay_in_local() {
        let fixture = Fixture {
            local: locals(&["a", "b", "c", "d", "e", "needle"]),
            recent: shared_strings(&["a", "b", "c", "d", "e", "needle"]),
            query: "needle".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert_eq!(describe(&fixture.rows()), ["S:Local", "  R:needle"]);
    }

    #[test]
    fn query_keeps_the_regular_sort_order() {
        let mut fixture = Fixture {
            local: locals(&["dev-b", "dev-a", "dev-main"]),
            current: Some("dev-main".to_string()),
            query: "dev".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  R:dev-main", "  R:dev-a", "  R:dev-b"]
        );
        fixture.favorites.insert(RefTarget::local("dev-b"));
        assert_eq!(
            describe(&fixture.rows()),
            ["S:Local", "  R:dev-main", "  R:dev-b", "  R:dev-a"]
        );
    }

    #[test]
    fn remote_head_is_filtered_even_when_it_matches_the_query() {
        let fixture = Fixture {
            remote: remotes(&["origin/HEAD"]),
            query: "head".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert!(fixture.rows().is_empty());
    }

    #[test]
    fn default_expanded_contains_only_the_recent_section() {
        let expanded = default_expanded();
        assert_eq!(expanded.len(), 1);
        assert!(expanded.contains(&node_id_for_section(SectionKind::Recent)));
    }

    #[test]
    fn default_favorites_pick_main_and_master_on_local_and_origin_when_present() {
        let local = locals(&["main", "dev", "master"]);
        let remote = remotes(&["origin/master", "origin/HEAD", "other/main", "origin/dev"]);
        let favorites = default_favorites(&local, &remote);
        let expected: HashSet<RefTarget> = [
            RefTarget::local("main"),
            RefTarget::local("master"),
            RefTarget::remote("origin/master"),
        ]
        .into_iter()
        .collect();
        assert_eq!(favorites, expected);
        assert!(default_favorites(&[], &[]).is_empty());
    }

    #[test]
    fn node_ids_are_stable_strings() {
        assert_eq!(node_id_for_section(SectionKind::Local), "section:Local");
        assert_eq!(node_id_for_section(SectionKind::Tags), "section:Tags");
        assert_eq!(
            node_id_for_folder(SectionKind::Remote, "origin/feature"),
            "folder:Remote:origin/feature"
        );
    }

    #[test]
    fn ancestor_node_ids_list_the_section_and_every_folder_prefix() {
        assert_eq!(
            ancestor_node_ids(SectionKind::Remote, "origin/feature/x", true),
            [
                "section:Remote",
                "folder:Remote:origin",
                "folder:Remote:origin/feature"
            ]
        );
        assert_eq!(
            ancestor_node_ids(SectionKind::Remote, "origin/feature/x", false),
            ["section:Remote"]
        );
        assert_eq!(
            ancestor_node_ids(SectionKind::Recent, "feature/x", true),
            ["section:Recent", "folder:Recent:feature"]
        );
        assert_eq!(
            ancestor_node_ids(SectionKind::Recent, "feature/x", false),
            ["section:Recent"]
        );
        assert_eq!(
            ancestor_node_ids(SectionKind::Local, "main", true),
            ["section:Local"]
        );
    }

    #[test]
    fn ancestor_ids_expand_the_path_to_a_nested_ref() {
        let mut fixture = Fixture {
            local: locals(&["feature/deep/leaf", "main"]),
            ..Fixture::default()
        };
        for id in ancestor_node_ids(SectionKind::Local, "feature/deep/leaf", true) {
            fixture.expanded.insert(id);
        }
        assert_eq!(
            describe(&fixture.rows()),
            [
                "S:Local",
                "  D:feature",
                "    D:feature/deep",
                "      R:leaf",
                "  R:main"
            ]
        );
    }

    #[test]
    fn natural_compare_orders_digit_runs_numerically() {
        assert_eq!(natural_compare("file2", "file10"), Ordering::Less);
        assert_eq!(natural_compare("file10", "file2"), Ordering::Greater);
        assert_eq!(natural_compare("v1.9", "v1.10"), Ordering::Less);
        assert_eq!(natural_compare("a007b", "a7c"), Ordering::Less);
        assert_eq!(natural_compare("a2", "a02"), Ordering::Greater);
    }

    #[test]
    fn natural_compare_is_case_insensitive_with_a_deterministic_tie_break() {
        assert_eq!(natural_compare("Alpha", "beta"), Ordering::Less);
        assert_eq!(natural_compare("alpha", "Beta"), Ordering::Less);
        assert_eq!(natural_compare("B", "a"), Ordering::Greater);
        assert_ne!(natural_compare("main", "Main"), Ordering::Equal);
        assert_eq!(natural_compare("main", "main"), Ordering::Equal);
    }

    #[test]
    fn natural_compare_puts_prefixes_first() {
        assert_eq!(natural_compare("", "a"), Ordering::Less);
        assert_eq!(natural_compare("fix", "fix-1"), Ordering::Less);
        assert_eq!(natural_compare("fix-1", "fix"), Ordering::Greater);
    }

    #[test]
    fn match_query_with_an_empty_query_matches_everything_with_no_positions() {
        assert_eq!(match_query("", "main"), Some((0, Vec::new())));
        assert_eq!(match_query("   ", "main"), Some((0, Vec::new())));
    }

    #[test]
    fn match_query_requires_every_query_character_in_order() {
        assert_eq!(
            match_query("mn", "main").map(|result| result.1),
            Some(vec![0, 3])
        );
        assert_eq!(match_query("nm", "main"), None);
        assert_eq!(match_query("x", "main"), None);
        assert_eq!(match_query("mainz", "main"), None);
        assert_eq!(match_query("mm", "main"), None);
        assert_eq!(match_query("a", ""), None);
    }

    #[test]
    fn match_query_is_case_insensitive_and_reports_character_positions() {
        assert_eq!(
            match_query("MAIN", "main").map(|result| result.1),
            Some(vec![0, 1, 2, 3])
        );
        assert_eq!(
            match_query("lg", "Login").map(|result| result.1),
            Some(vec![0, 2])
        );
    }

    #[test]
    fn match_query_positions_count_characters_not_bytes() {
        assert_eq!(
            match_query("b", "ключ/b").map(|result| result.1),
            Some(vec![5])
        );
    }

    #[test]
    fn match_query_ignores_surrounding_whitespace_but_not_inner_whitespace() {
        assert!(match_query("  main  ", "main").is_some());
        assert_eq!(match_query("a b", "ab"), None);
    }

    #[test]
    fn match_query_scores_an_earlier_first_match_higher() {
        let at_start = match_query("main", "main");
        let after_prefix = match_query("main", "x-main");
        let (start_score, _) = at_start.expect("matches at the start");
        let (later_score, later_positions) = after_prefix.expect("matches after the prefix");
        assert_eq!(later_positions, [2, 3, 4, 5]);
        assert_eq!(start_score - later_score, 2);
    }

    #[test]
    fn match_query_prefers_word_starts_over_mid_word_hits() {
        assert_eq!(
            match_query("m", "summary/main").map(|result| result.1),
            Some(vec![8])
        );
        assert_eq!(
            match_query("gb", "GitBranch").map(|result| result.1),
            Some(vec![0, 3])
        );
    }

    #[test]
    fn match_query_scores_consecutive_runs_above_scattered_matches() {
        let scattered = match_query("ab", "azb").expect("scattered matches");
        let consecutive = match_query("ab", "abz").expect("consecutive matches");
        assert!(consecutive.0 > scattered.0);
    }

    #[test]
    fn match_query_prefers_the_exact_case_when_everything_else_is_equal() {
        let exact = match_query("Main", "Main").expect("exact case matches");
        let folded = match_query("main", "Main").expect("folded case matches");
        assert!(exact.0 > folded.0);
    }

    #[test]
    fn ref_rows_carry_the_match_score_of_the_full_name_and_zero_without_a_query() {
        let mut fixture = Fixture {
            local: locals(&["main", "a-main"]),
            ..Fixture::default()
        };
        fixture.expand_sections(&[SectionKind::Local]);
        let rows = fixture.rows();
        assert!(ref_rows(&rows).iter().all(|row| row.score == 0));

        fixture.query = "main".to_string();
        let rows = fixture.rows();
        let refs = ref_rows(&rows);
        let exact = refs
            .iter()
            .find(|row| &*row.target.name == "main")
            .expect("main row");
        let later = refs
            .iter()
            .find(|row| &*row.target.name == "a-main")
            .expect("a-main row");
        assert_eq!(
            exact.score,
            match_popup_query("main", "main").expect("matches").0
        );
        assert!(exact.score > later.score);
    }

    #[test]
    fn filtering_uses_intellij_word_starts_instead_of_any_subsequence() {
        let mut fixture = Fixture {
            local: locals(&["reformatCode", "feature/login", "main"]),
            query: "fc".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        assert!(fixture.rows().is_empty());

        fixture.query = "fl".to_string();
        let rows = fixture.rows();
        assert_eq!(describe(&rows), ["S:Local", "  D:feature", "    R:login"]);
        assert_eq!(ref_rows(&rows)[0].highlight_positions, [0]);

        fixture.query = "mn".to_string();
        assert!(fixture.rows().is_empty());
    }

    #[test]
    fn a_space_in_the_query_is_a_wildcard_inside_one_pattern() {
        let fixture = Fixture {
            local: locals(&["feature/login", "login/feature", "main"]),
            options: TreeOptions {
                group_by_directory: false,
                ..TreeOptions::default()
            },
            query: "fea log".to_string(),
            expand_all: true,
            ..Fixture::default()
        };
        let rows = fixture.rows();
        assert_eq!(describe(&rows), ["S:Local", "  R:feature/login"]);
        assert_eq!(ref_rows(&rows)[0].highlight_positions, [0, 1, 2, 8, 9, 10]);
    }

    #[test]
    fn section_child_count_counts_direct_children_even_when_collapsed() {
        let fixture = Fixture {
            local: locals(&["main", "feature/a", "feature/b", "fix/c", "dev"]),
            recent: shared_strings(&["feature/a", "main"]),
            ..Fixture::default()
        };
        let input = TreeInput {
            local: &fixture.local,
            remote: &fixture.remote,
            tags: &fixture.tags,
            recent: &fixture.recent,
            current_branch: None,
            head_tag: None,
            favorites: &fixture.favorites,
            options: fixture.options,
            expanded: &fixture.expanded,
            expand_all: false,
            query: "fea",
        };
        assert_eq!(section_child_count(&input, SectionKind::Recent), 2);
        assert_eq!(section_child_count(&input, SectionKind::Local), 4);
        assert_eq!(section_child_count(&input, SectionKind::Remote), 0);
    }

    #[test]
    fn section_titles_name_the_repository_only_when_one_is_given() {
        assert_eq!(SectionKind::Local.title(None), "Local");
        assert_eq!(SectionKind::Recent.title(None), "Recent");
        assert_eq!(
            SectionKind::Recent.title(Some("api")),
            "Recent Branches in api"
        );
        assert_eq!(
            SectionKind::Local.title(Some("api")),
            "Local Branches in api"
        );
        assert_eq!(
            SectionKind::Remote.title(Some("api")),
            "Remote Branches in api"
        );
        assert_eq!(SectionKind::Tags.title(Some("api")), "Tags in api");
    }
}
