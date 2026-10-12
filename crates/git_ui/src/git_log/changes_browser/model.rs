use std::cmp::Ordering;

use collections::{HashMap, HashSet};
use git::{Oid, repository::RepoPath};
use gpui::SharedString;

pub(crate) const SHOWN_CHANGES_LIMIT: usize = 50_000;
pub(crate) const EXPAND_NODES_THRESHOLD: usize = 30_000;
pub(crate) const SELECT_COMMIT_TEXT: &str = "Select commit to view changes";
pub(crate) const NO_MERGED_CONFLICTS_TEXT: &str = "No merged conflicts.";
pub(crate) const SHOW_CHANGES_TO_PARENTS_LINK: &str = "Show changes to parents";
pub(crate) const NO_AFFECTED_CHANGES_TEXT: &str = "No changes that affect selected paths.";
pub(crate) const SHOW_ALL_PATHS_LINK: &str = "Show changes for all paths";
pub(crate) const SHOW_ANYWAY_LINK: &str = "Show anyway";
pub(crate) const ERROR_LOADING_CHANGES_TEXT: &str = "Error loading changes";
pub(crate) const NO_MERGED_CONFLICTS_NODE: &str = "No merged conflicts";
const SPACE_AND_THIN_SPACE: &str = " \u{2009}";
const PARENT_SUBJECT_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

impl ChangeKind {
    fn has_before(self) -> bool {
        self != ChangeKind::Added
    }

    fn has_after(self) -> bool {
        self != ChangeKind::Deleted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub path: RepoPath,
    pub kind: ChangeKind,
    pub commit: Oid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitChanges {
    pub changes: Vec<Change>,
    pub changes_to_parents: Vec<(Oid, Vec<Change>)>,
}

impl CommitChanges {
    pub(crate) fn size(&self) -> usize {
        if self.changes_to_parents.is_empty() {
            self.changes.len()
        } else {
            self.changes_to_parents
                .iter()
                .map(|(_, changes)| changes.len())
                .sum()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParentChanges {
    pub label: SharedString,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChangesData {
    pub has_commits: bool,
    pub changes: Vec<Change>,
    pub changes_to_parents: Vec<ParentChanges>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangesState {
    Empty,
    Error,
    ManyChanges {
        commit_count: usize,
        max_size: usize,
    },
    Changes(ChangesData),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusAction {
    ShowChangesToParents,
    ShowAllPaths,
    ShowAnyway,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusText {
    pub text: SharedString,
    pub link: Option<(SharedString, StatusAction)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AffectedPath {
    pub path: String,
    pub is_directory: bool,
}

pub(crate) fn affected_paths(filter_paths: &[String]) -> Option<Vec<AffectedPath>> {
    if filter_paths.is_empty() {
        return None;
    }
    Some(
        filter_paths
            .iter()
            .map(|path| match path.strip_suffix('/') {
                Some(directory) => AffectedPath {
                    path: directory.to_string(),
                    is_directory: true,
                },
                None => AffectedPath {
                    path: path.clone(),
                    is_directory: false,
                },
            })
            .collect(),
    )
}

fn affects(change: &Change, affected: &AffectedPath) -> bool {
    let path = change.path.as_unix_str();
    if affected.is_directory {
        affected.path.is_empty()
            || path == affected.path
            || path
                .strip_prefix(affected.path.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    } else {
        path == affected.path
    }
}

pub(crate) fn collect_affected_changes(
    changes: &[Change],
    affected_paths: Option<&[AffectedPath]>,
    show_only_affected: bool,
) -> Vec<Change> {
    match affected_paths {
        Some(affected_paths) if show_only_affected => changes
            .iter()
            .filter(|change| {
                affected_paths
                    .iter()
                    .any(|affected| affects(change, affected))
            })
            .cloned()
            .collect(),
        _ => changes.to_vec(),
    }
}

pub(crate) fn zip_changes(changes: impl IntoIterator<Item = Change>) -> Vec<Change> {
    let mut keys: Vec<Option<RepoPath>> = Vec::new();
    let mut buckets: HashMap<Option<RepoPath>, Vec<Change>> = HashMap::default();
    let put = |keys: &mut Vec<Option<RepoPath>>,
               buckets: &mut HashMap<Option<RepoPath>, Vec<Change>>,
               key: Option<RepoPath>,
               change: Change| {
        if !buckets.contains_key(&key) {
            keys.push(key.clone());
        }
        buckets.entry(key).or_default().push(change);
    };

    for change in changes {
        let after_key = change.kind.has_after().then(|| change.path.clone());
        if !change.kind.has_before() {
            put(&mut keys, &mut buckets, after_key, change);
            continue;
        }
        let previous = buckets
            .get_mut(&Some(change.path.clone()))
            .filter(|bucket| !bucket.is_empty())
            .map(|bucket| bucket.remove(0));
        match previous {
            None => put(&mut keys, &mut buckets, after_key, change),
            Some(previous) => {
                let kind = match (previous.kind.has_before(), change.kind.has_after()) {
                    (true, true) => Some(ChangeKind::Modified),
                    (true, false) => Some(ChangeKind::Deleted),
                    (false, true) => Some(ChangeKind::Added),
                    (false, false) => None,
                };
                if let Some(kind) = kind {
                    put(
                        &mut keys,
                        &mut buckets,
                        after_key,
                        Change {
                            path: change.path,
                            kind,
                            commit: change.commit,
                        },
                    );
                }
            }
        }
    }

    if let Some(deleted) = buckets.remove(&None) {
        keys.retain(Option::is_some);
        for change in deleted {
            let key = Some(change.path.clone());
            put(&mut keys, &mut buckets, key, change);
        }
    }

    keys.into_iter()
        .flat_map(|key| buckets.remove(&key).unwrap_or_default())
        .collect()
}

pub(crate) fn merge_parent_changes(parents: &[Vec<Change>]) -> Vec<Change> {
    let Some((first, _)) = parents.split_first() else {
        return Vec::new();
    };
    if parents.len() == 1 {
        return first.clone();
    }
    let lookups: Vec<HashMap<&RepoPath, ChangeKind>> = parents
        .iter()
        .map(|changes| {
            changes
                .iter()
                .map(|change| (&change.path, change.kind))
                .collect()
        })
        .collect();
    first
        .iter()
        .filter_map(|change| {
            let kinds = lookups
                .iter()
                .map(|lookup| lookup.get(&change.path).copied())
                .collect::<Option<Vec<ChangeKind>>>()?;
            let first_kind = *kinds.first()?;
            let kind = if kinds.iter().all(|kind| *kind == first_kind) {
                first_kind
            } else if kinds.contains(&ChangeKind::Deleted) {
                ChangeKind::Deleted
            } else {
                ChangeKind::Modified
            };
            Some(Change {
                path: change.path.clone(),
                kind,
                commit: change.commit,
            })
        })
        .collect()
}

pub(crate) fn changes_state(
    commits: &[CommitChanges],
    parent_labels: &HashMap<Oid, SharedString>,
    show_anyway: bool,
) -> ChangesState {
    let max_size = commits.iter().map(CommitChanges::size).max().unwrap_or(0);
    if max_size > SHOWN_CHANGES_LIMIT && !show_anyway {
        return ChangesState::ManyChanges {
            commit_count: commits.len(),
            max_size,
        };
    }
    match commits {
        [] => ChangesState::Changes(ChangesData::default()),
        [single] => ChangesState::Changes(ChangesData {
            has_commits: true,
            changes: single.changes.clone(),
            changes_to_parents: if single.changes_to_parents.len() > 1 {
                single
                    .changes_to_parents
                    .iter()
                    .map(|(parent, changes)| ParentChanges {
                        label: parent_labels
                            .get(parent)
                            .cloned()
                            .unwrap_or_else(|| parent_label(*parent, None)),
                        changes: changes.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }),
        _ => ChangesState::Changes(ChangesData {
            has_commits: true,
            changes: zip_changes(
                commits
                    .iter()
                    .rev()
                    .flat_map(|commit| commit.changes.iter().cloned()),
            ),
            changes_to_parents: Vec::new(),
        }),
    }
}

pub(crate) fn parent_label(parent: Oid, subject: Option<&str>) -> SharedString {
    let hash = parent.to_string();
    let short_hash = hash.get(..8).unwrap_or(hash.as_str());
    let mut text = format!("Changes to {short_hash}");
    if let Some(subject) = subject {
        text.push(' ');
        text.push_str(&shorten_with_ellipsis(subject, PARENT_SUBJECT_LIMIT));
    }
    text.into()
}

fn shorten_with_ellipsis(text: &str, max_length: usize) -> String {
    if text.chars().count() <= max_length {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(max_length.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

pub(crate) fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub(crate) fn status_text(
    state: &ChangesState,
    show_only_affected: bool,
    has_affected_paths: bool,
) -> Option<StatusText> {
    match state {
        ChangesState::Empty => None,
        ChangesState::Error => Some(StatusText {
            text: ERROR_LOADING_CHANGES_TEXT.into(),
            link: None,
        }),
        ChangesState::ManyChanges {
            commit_count,
            max_size,
        } => {
            let subject = if *commit_count <= 1 {
                "This commit"
            } else {
                "One of the selected commits"
            };
            Some(StatusText {
                text: format!("{subject} has {} changes", group_thousands(*max_size)).into(),
                link: Some((SHOW_ANYWAY_LINK.into(), StatusAction::ShowAnyway)),
            })
        }
        ChangesState::Changes(data) => {
            if !data.has_commits {
                Some(StatusText {
                    text: SELECT_COMMIT_TEXT.into(),
                    link: None,
                })
            } else if !data.changes_to_parents.is_empty() {
                Some(StatusText {
                    text: NO_MERGED_CONFLICTS_TEXT.into(),
                    link: Some((
                        SHOW_CHANGES_TO_PARENTS_LINK.into(),
                        StatusAction::ShowChangesToParents,
                    )),
                })
            } else if show_only_affected && has_affected_paths {
                Some(StatusText {
                    text: NO_AFFECTED_CHANGES_TEXT.into(),
                    link: Some((SHOW_ALL_PATHS_LINK.into(), StatusAction::ShowAllPaths)),
                })
            } else {
                None
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Directory {
        path: String,
        label: SharedString,
    },
    Change {
        change: Change,
        name: SharedString,
        parent_path: Option<SharedString>,
    },
    Tag {
        label: SharedString,
        expand_by_default: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeNode {
    pub id: SharedString,
    pub kind: NodeKind,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    fn file_count(&self) -> usize {
        let own = usize::from(matches!(self.kind, NodeKind::Change { .. }));
        own + self
            .children
            .iter()
            .map(TreeNode::file_count)
            .sum::<usize>()
    }

    fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(TreeNode::node_count)
            .sum::<usize>()
    }

    pub(crate) fn changes(&self, changes: &mut Vec<Change>) {
        if let NodeKind::Change { change, .. } = &self.kind {
            changes.push(change.clone());
        }
        for child in &self.children {
            child.changes(changes);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub id: SharedString,
    pub depth: usize,
    pub kind: NodeKind,
    pub has_children: bool,
    pub expanded: bool,
    pub count_text: Option<SharedString>,
}

pub(crate) fn count_text(file_count: usize) -> Option<SharedString> {
    match file_count {
        0 => None,
        1 => Some(format!("{SPACE_AND_THIN_SPACE}1 file").into()),
        count => Some(format!("{SPACE_AND_THIN_SPACE}{count} files").into()),
    }
}

pub(crate) fn build_tree(
    data: &ChangesData,
    affected_paths: Option<&[AffectedPath]>,
    show_only_affected: bool,
    show_changes_from_parents: bool,
    group_by_directory: bool,
    repository_name: &str,
) -> Vec<TreeNode> {
    let changes = collect_affected_changes(&data.changes, affected_paths, show_only_affected);
    let mut nodes = build_scope(&changes, "", group_by_directory, repository_name);
    if show_changes_from_parents && !data.changes_to_parents.is_empty() {
        if changes.is_empty() {
            nodes.push(TreeNode {
                id: "tag:merged".into(),
                kind: NodeKind::Tag {
                    label: NO_MERGED_CONFLICTS_NODE.into(),
                    expand_by_default: true,
                },
                children: Vec::new(),
            });
        }
        for (index, parent) in data.changes_to_parents.iter().enumerate() {
            let parent_changes =
                collect_affected_changes(&parent.changes, affected_paths, show_only_affected);
            if parent_changes.is_empty() {
                continue;
            }
            let scope = format!("parent{index}:");
            nodes.push(TreeNode {
                id: format!("tag:{scope}").into(),
                kind: NodeKind::Tag {
                    label: parent.label.clone(),
                    expand_by_default: false,
                },
                children: build_scope(&parent_changes, &scope, group_by_directory, repository_name),
            });
        }
    }
    nodes
}

#[derive(Default)]
struct DirectoryBuilder {
    directories: Vec<String>,
    changes: Vec<usize>,
}

fn build_scope(
    changes: &[Change],
    scope: &str,
    group_by_directory: bool,
    repository_name: &str,
) -> Vec<TreeNode> {
    if changes.is_empty() {
        return Vec::new();
    }
    if !group_by_directory {
        let mut nodes: Vec<TreeNode> = changes
            .iter()
            .enumerate()
            .map(|(index, change)| {
                let parent = parent_directory(change.path.as_unix_str());
                let parent_path = if parent.is_empty() {
                    (!repository_name.is_empty())
                        .then(|| SharedString::from(repository_name.to_string()))
                } else {
                    Some(SharedString::from(parent.to_string()))
                };
                change_node(scope, index, change, parent_path)
            })
            .collect();
        nodes.sort_by(compare_nodes);
        return nodes;
    }

    let mut directories: HashMap<String, DirectoryBuilder> = HashMap::default();
    directories.insert(String::new(), DirectoryBuilder::default());
    for (index, change) in changes.iter().enumerate() {
        let path = change.path.as_unix_str();
        let mut child = parent_directory(path).to_string();
        if let Some(directory) = directories.get_mut(&child) {
            directory.changes.push(index);
        } else {
            directories
                .entry(child.clone())
                .or_default()
                .changes
                .push(index);
            loop {
                let parent = parent_directory(&child).to_string();
                let exists = directories.contains_key(&parent);
                directories
                    .entry(parent.clone())
                    .or_default()
                    .directories
                    .push(child);
                if exists || parent.is_empty() {
                    break;
                }
                child = parent;
            }
        }
    }

    let root = build_directory(String::new(), &directories, changes, scope, None);
    vec![collapse(root, None)]
}

fn build_directory(
    path: String,
    directories: &HashMap<String, DirectoryBuilder>,
    changes: &[Change],
    scope: &str,
    parent: Option<&str>,
) -> TreeNode {
    let label = relative_label(&path, parent);
    let mut children = Vec::new();
    if let Some(directory) = directories.get(&path) {
        for child in &directory.directories {
            let node = build_directory(child.clone(), directories, changes, scope, Some(&path));
            children.push(collapse(node, Some(&path)));
        }
        for index in &directory.changes {
            if let Some(change) = changes.get(*index) {
                children.push(change_node(scope, *index, change, None));
            }
        }
    }
    children.sort_by(compare_nodes);
    TreeNode {
        id: format!("dir:{scope}{path}").into(),
        kind: NodeKind::Directory {
            path,
            label: label.into(),
        },
        children,
    }
}

fn collapse(mut node: TreeNode, parent: Option<&str>) -> TreeNode {
    loop {
        let collapsible = node.children.len() == 1
            && node.children.first().is_some_and(|child| {
                matches!(child.kind, NodeKind::Directory { .. }) && !child.children.is_empty()
            });
        if !collapsible {
            break;
        }
        let Some(child) = node.children.pop() else {
            break;
        };
        node = child;
    }
    if let NodeKind::Directory { path, label } = &mut node.kind {
        *label = relative_label(path, parent).into();
    }
    node
}

fn relative_label(path: &str, parent: Option<&str>) -> String {
    match parent {
        Some(parent) if !parent.is_empty() => path
            .strip_prefix(parent)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path)
            .to_string(),
        _ => path.to_string(),
    }
}

fn change_node(
    scope: &str,
    index: usize,
    change: &Change,
    parent_path: Option<SharedString>,
) -> TreeNode {
    let path = change.path.as_unix_str();
    TreeNode {
        id: format!("change:{scope}{index}:{path}").into(),
        kind: NodeKind::Change {
            change: change.clone(),
            name: file_name(path).to_string().into(),
            parent_path,
        },
        children: Vec::new(),
    }
}

fn parent_directory(path: &str) -> &str {
    path.rfind('/').map_or("", |index| &path[..index])
}

fn file_name(path: &str) -> &str {
    path.rfind('/').map_or(path, |index| &path[index + 1..])
}

fn sort_weight(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Directory { .. } => 5,
        NodeKind::Change { .. } => 7,
        NodeKind::Tag { .. } => 10,
    }
}

fn compare_nodes(left: &TreeNode, right: &TreeNode) -> Ordering {
    let weight = sort_weight(&left.kind).cmp(&sort_weight(&right.kind));
    if weight != Ordering::Equal {
        return weight;
    }
    match (&left.kind, &right.kind) {
        (NodeKind::Directory { path: left, .. }, NodeKind::Directory { path: right, .. }) => {
            compare_paths(left, true, right, true)
        }
        (NodeKind::Change { change: left, .. }, NodeKind::Change { change: right, .. }) => {
            compare_paths(
                left.path.as_unix_str(),
                false,
                right.path.as_unix_str(),
                false,
            )
        }
        _ => Ordering::Equal,
    }
}

fn compare_paths(left: &str, left_directory: bool, right: &str, right_directory: bool) -> Ordering {
    let names = natural_compare(file_name(left), file_name(right), true);
    if names != Ordering::Equal {
        return names;
    }
    hierarchical_compare(left, left_directory, right, right_directory)
}

fn hierarchical_compare(
    left: &str,
    left_directory: bool,
    right: &str,
    right_directory: bool,
) -> Ordering {
    let common = left
        .char_indices()
        .zip(right.chars())
        .find(|((_, left_char), right_char)| !left_char.eq_ignore_ascii_case(right_char))
        .map_or(left.len().min(right.len()), |((index, _), _)| index);
    let directory_order = |is_directory: bool| {
        if is_directory {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    };
    if common == left.len() && common == right.len() {
        if left_directory != right_directory {
            return directory_order(left_directory);
        }
        return Ordering::Equal;
    }
    if common == left.len() && right[common..].starts_with('/') {
        return directory_order(left_directory);
    }
    if common == right.len() && left[common..].starts_with('/') {
        return directory_order(!right_directory);
    }
    let start = left[..common].rfind('/').map_or(0, |index| index + 1);
    let left_end = left[start..].find('/').map(|index| start + index);
    let right_end = right[start..].find('/').map(|index| start + index);
    let left_is_directory = left_end.is_some() || left_directory;
    let right_is_directory = right_end.is_some() || right_directory;
    if left_is_directory != right_is_directory {
        return directory_order(left_is_directory);
    }
    natural_compare(
        &left[start..left_end.unwrap_or(left.len())],
        &right[start..right_end.unwrap_or(right.len())],
        true,
    )
}

pub(crate) fn natural_compare(left: &str, right: &str, like_file_names: bool) -> Ordering {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    natural_compare_chars(&left, &right, true, like_file_names)
}

fn natural_compare_chars(
    left: &[char],
    right: &[char],
    ignore_case: bool,
    like_file_names: bool,
) -> Ordering {
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
            let number = compare_range(left, right, left_start, right_start, left_end);
            if number != Ordering::Equal {
                return number;
            }
            let full_length = (left_end - left_index).cmp(&(right_end - right_index));
            if full_length != Ordering::Equal {
                return full_length;
            }
            let leading = compare_range(left, right, left_index, right_index, left_start);
            if leading != Ordering::Equal {
                return leading;
            }
            left_index = left_end.saturating_sub(1);
            right_index = right_end.saturating_sub(1);
        } else if left_char != right_char {
            let difference = if like_file_names && left_char == '-' && right_char != '_' {
                compare_chars('_', right_char, ignore_case)
            } else if like_file_names && right_char == '-' && left_char != '_' {
                compare_chars(left_char, '_', ignore_case)
            } else {
                compare_chars(left_char, right_char, ignore_case)
            };
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
        natural_compare_chars(left, right, false, like_file_names)
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

fn compare_range(
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
    let left_upper = single_mapping(left.to_uppercase(), left);
    let right_upper = single_mapping(right.to_uppercase(), right);
    if left_upper == right_upper {
        return Ordering::Equal;
    }
    single_mapping(left_upper.to_lowercase(), left_upper)
        .cmp(&single_mapping(right_upper.to_lowercase(), right_upper))
}

fn single_mapping(mut mapping: impl Iterator<Item = char>, fallback: char) -> char {
    match (mapping.next(), mapping.next()) {
        (Some(mapped), None) => mapped,
        _ => fallback,
    }
}

pub(crate) fn default_expanded(nodes: &[TreeNode]) -> HashSet<SharedString> {
    let mut expanded = HashSet::default();
    let total: usize = nodes.iter().map(TreeNode::node_count).sum();
    if total >= EXPAND_NODES_THRESHOLD {
        if let [single] = nodes {
            expanded.insert(single.id.clone());
        }
        return expanded;
    }
    fn visit(nodes: &[TreeNode], expanded: &mut HashSet<SharedString>) {
        for node in nodes {
            if let NodeKind::Tag {
                expand_by_default: false,
                ..
            } = node.kind
            {
                continue;
            }
            if !node.children.is_empty() {
                expanded.insert(node.id.clone());
                visit(&node.children, expanded);
            }
        }
    }
    visit(nodes, &mut expanded);
    expanded
}

pub(crate) fn all_container_ids(nodes: &[TreeNode], ids: &mut HashSet<SharedString>) {
    for node in nodes {
        if !node.children.is_empty() {
            ids.insert(node.id.clone());
            all_container_ids(&node.children, ids);
        }
    }
}

pub(crate) fn flatten(nodes: &[TreeNode], expanded: &HashSet<SharedString>) -> Vec<TreeRow> {
    fn visit(
        nodes: &[TreeNode],
        depth: usize,
        expanded: &HashSet<SharedString>,
        rows: &mut Vec<TreeRow>,
    ) {
        for node in nodes {
            let is_expanded = expanded.contains(&node.id);
            let count_text = match node.kind {
                NodeKind::Change { .. } => None,
                _ => count_text(node.file_count()),
            };
            rows.push(TreeRow {
                id: node.id.clone(),
                depth,
                kind: node.kind.clone(),
                has_children: !node.children.is_empty(),
                expanded: is_expanded,
                count_text,
            });
            if is_expanded {
                visit(&node.children, depth + 1, expanded, rows);
            }
        }
    }
    let mut rows = Vec::new();
    visit(nodes, 0, expanded, &mut rows);
    rows
}

pub(crate) fn find_node<'a>(nodes: &'a [TreeNode], id: &str) -> Option<&'a TreeNode> {
    nodes.iter().find_map(|node| {
        if node.id.as_ref() == id {
            Some(node)
        } else {
            find_node(&node.children, id)
        }
    })
}

pub(crate) fn ancestors_of(nodes: &[TreeNode], id: &str) -> Option<Vec<SharedString>> {
    for node in nodes {
        if node.id.as_ref() == id {
            return Some(Vec::new());
        }
        if let Some(mut path) = ancestors_of(&node.children, id) {
            path.insert(0, node.id.clone());
            return Some(path);
        }
    }
    None
}

pub(crate) fn ids_of_changes(nodes: &[TreeNode], changes: &[Change], ids: &mut Vec<SharedString>) {
    for node in nodes {
        if let NodeKind::Change { change, .. } = &node.kind
            && changes.contains(change)
        {
            ids.push(node.id.clone());
        }
        ids_of_changes(&node.children, changes, ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(byte: u8) -> Oid {
        Oid::from_bytes(&[byte; 20]).expect("valid oid")
    }

    fn change(path: &str, kind: ChangeKind, commit: u8) -> Change {
        Change {
            path: RepoPath::new(path).expect("valid repo path"),
            kind,
            commit: oid(commit),
        }
    }

    fn summary(changes: &[Change]) -> Vec<(String, ChangeKind)> {
        changes
            .iter()
            .map(|change| (change.path.as_unix_str().to_string(), change.kind))
            .collect()
    }

    #[test]
    fn zipping_compounds_changes_of_the_same_file_across_commits() {
        let zipped = zip_changes(vec![
            change("a.txt", ChangeKind::Added, 1),
            change("b.txt", ChangeKind::Modified, 1),
            change("a.txt", ChangeKind::Modified, 2),
            change("b.txt", ChangeKind::Deleted, 2),
            change("c.txt", ChangeKind::Added, 2),
            change("c.txt", ChangeKind::Deleted, 3),
        ]);
        assert_eq!(
            summary(&zipped),
            vec![
                ("a.txt".to_string(), ChangeKind::Added),
                ("b.txt".to_string(), ChangeKind::Deleted),
            ],
            "an addition followed by a deletion cancels out, as CommittedChangesTreeBrowser.zipChanges drops it"
        );
        assert_eq!(
            zipped[0].commit,
            oid(2),
            "the newest commit owns the zipped change"
        );
    }

    #[test]
    fn zipping_keeps_a_deletion_and_a_later_re_addition_separately() {
        let zipped = zip_changes(vec![
            change("a.txt", ChangeKind::Deleted, 1),
            change("a.txt", ChangeKind::Added, 2),
        ]);
        assert_eq!(
            summary(&zipped),
            vec![
                ("a.txt".to_string(), ChangeKind::Added),
                ("a.txt".to_string(), ChangeKind::Deleted),
            ],
            "IntelliJ moves deletions to the bucket of their path after the zip loop"
        );
    }

    #[test]
    fn several_commits_are_zipped_oldest_first() {
        let newest = CommitChanges {
            changes: vec![change("a.txt", ChangeKind::Deleted, 2)],
            changes_to_parents: Vec::new(),
        };
        let oldest = CommitChanges {
            changes: vec![change("a.txt", ChangeKind::Added, 1)],
            changes_to_parents: Vec::new(),
        };
        let state = changes_state(&[newest, oldest], &HashMap::default(), false);
        assert_eq!(
            state,
            ChangesState::Changes(ChangesData {
                has_commits: true,
                changes: Vec::new(),
                changes_to_parents: Vec::new(),
            }),
            "the selection arrives newest first and is reversed before zipping"
        );
    }

    #[test]
    fn merged_changes_keep_only_files_changed_against_every_parent() {
        let merged = merge_parent_changes(&[
            vec![
                change("both.txt", ChangeKind::Modified, 1),
                change("first.txt", ChangeKind::Added, 1),
                change("gone.txt", ChangeKind::Deleted, 1),
            ],
            vec![
                change("both.txt", ChangeKind::Modified, 1),
                change("gone.txt", ChangeKind::Modified, 1),
            ],
        ]);
        assert_eq!(
            summary(&merged),
            vec![
                ("both.txt".to_string(), ChangeKind::Modified),
                ("gone.txt".to_string(), ChangeKind::Deleted),
            ],
            "VcsStatusMerger keeps paths present for all parents and prefers DELETED on mixed types"
        );
    }

    #[test]
    fn empty_state_texts_follow_vcs_log_changes_tree_components() {
        let no_selection = changes_state(&[], &HashMap::default(), false);
        assert_eq!(
            status_text(&no_selection, false, false).map(|status| status.text),
            Some(SELECT_COMMIT_TEXT.into())
        );
        let merge = CommitChanges {
            changes: Vec::new(),
            changes_to_parents: vec![
                (oid(1), vec![change("a.txt", ChangeKind::Modified, 3)]),
                (oid(2), vec![change("b.txt", ChangeKind::Modified, 3)]),
            ],
        };
        let merge_state = changes_state(&[merge], &HashMap::default(), false);
        assert_eq!(
            status_text(&merge_state, true, true),
            Some(StatusText {
                text: NO_MERGED_CONFLICTS_TEXT.into(),
                link: Some((
                    SHOW_CHANGES_TO_PARENTS_LINK.into(),
                    StatusAction::ShowChangesToParents
                )),
            }),
            "merge commits take precedence over the affected paths text"
        );
        let plain = CommitChanges {
            changes: vec![change("a.txt", ChangeKind::Modified, 4)],
            changes_to_parents: Vec::new(),
        };
        let plain_state = changes_state(&[plain], &HashMap::default(), false);
        assert_eq!(
            status_text(&plain_state, true, true).map(|status| status.link),
            Some(Some((
                SHOW_ALL_PATHS_LINK.into(),
                StatusAction::ShowAllPaths
            )))
        );
        assert_eq!(status_text(&plain_state, false, true), None);
        assert_eq!(status_text(&ChangesState::Empty, true, true), None);
    }

    #[test]
    fn too_many_changes_reports_the_largest_commit_until_shown_anyway() {
        let huge = CommitChanges {
            changes: (0..=SHOWN_CHANGES_LIMIT)
                .map(|index| change(&format!("file{index}.txt"), ChangeKind::Added, 5))
                .collect(),
            changes_to_parents: Vec::new(),
        };
        let state = changes_state(std::slice::from_ref(&huge), &HashMap::default(), false);
        assert_eq!(
            status_text(&state, false, false).map(|status| status.text),
            Some("This commit has 50,001 changes".into())
        );
        assert!(matches!(
            changes_state(&[huge], &HashMap::default(), true),
            ChangesState::Changes(_)
        ));
    }

    #[test]
    fn directory_grouping_compacts_single_child_chains() {
        let data = ChangesData {
            has_commits: true,
            changes: vec![
                change("crates/git/src/a.rs", ChangeKind::Modified, 1),
                change("crates/git/src/b.rs", ChangeKind::Added, 1),
                change("README.md", ChangeKind::Deleted, 1),
            ],
            changes_to_parents: Vec::new(),
        };
        let nodes = build_tree(&data, None, false, false, true, "flint");
        let rows = flatten(&nodes, &default_expanded(&nodes));
        let labels: Vec<(usize, String)> = rows
            .iter()
            .map(|row| {
                let label = match &row.kind {
                    NodeKind::Directory { label, .. } => label.to_string(),
                    NodeKind::Change { name, .. } => name.to_string(),
                    NodeKind::Tag { label, .. } => label.to_string(),
                };
                (row.depth, label)
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                (0, String::new()),
                (1, "crates/git/src".to_string()),
                (2, "a.rs".to_string()),
                (2, "b.rs".to_string()),
                (1, "README.md".to_string()),
            ],
            "the repository root keeps its empty presentable path and directories precede files"
        );
        assert_eq!(rows[0].count_text, count_text(3));
    }

    #[test]
    fn affected_paths_distinguish_directories_from_files() {
        let paths = affected_paths(&["src/".to_string(), "README.md".to_string()]);
        let changes = [
            change("src/lib.rs", ChangeKind::Modified, 1),
            change("srcx/lib.rs", ChangeKind::Modified, 1),
            change("README.md", ChangeKind::Modified, 1),
        ];
        assert_eq!(
            summary(&collect_affected_changes(&changes, paths.as_deref(), true)),
            vec![
                ("src/lib.rs".to_string(), ChangeKind::Modified),
                ("README.md".to_string(), ChangeKind::Modified),
            ]
        );
        assert_eq!(
            collect_affected_changes(&changes, paths.as_deref(), false).len(),
            3
        );
    }

    #[test]
    fn file_names_compare_naturally_like_intellij() {
        assert_eq!(natural_compare("file2", "file10", true), Ordering::Less);
        assert_eq!(natural_compare("a-c", "a.b", true), Ordering::Greater);
        assert_eq!(natural_compare("a-c", "a.b", false), Ordering::Less);
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}
