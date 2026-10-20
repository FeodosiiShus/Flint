use std::cmp::Ordering;

use collections::HashSet;
use gpui::SharedString;

use crate::branch_refs::{RefKind, RefTarget};
use crate::git_log::properties::HEAD_REVISION;
use crate::ref_matcher::{BranchesTreeMatcher, Matching, classify};

pub(crate) const HEAD_NODE_ID: &str = "HEAD";
pub(crate) const MANY_NODES_THRESHOLD: usize = 30_000;
const HEAD_DISPLAY_NAME: &str = "HEAD (Current Branch)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TopLevelKind {
    Local,
    Remote,
    Tags,
}

impl TopLevelKind {
    const ALL: [TopLevelKind; 3] = [
        TopLevelKind::Local,
        TopLevelKind::Remote,
        TopLevelKind::Tags,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            TopLevelKind::Local => "Local",
            TopLevelKind::Remote => "Remote",
            TopLevelKind::Tags => "Tags",
        }
    }

    fn node_id(self) -> &'static str {
        match self {
            TopLevelKind::Local => "LOCAL",
            TopLevelKind::Remote => "REMOTE",
            TopLevelKind::Tags => "TAG",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefInfo {
    pub target: RefTarget,
    pub remote: Option<SharedString>,
    pub is_current: bool,
    pub is_favorite: bool,
    pub incoming: u32,
    pub outgoing: u32,
    pub tracked: Option<SharedString>,
}

impl RefInfo {
    pub(crate) fn new(target: RefTarget) -> Self {
        Self {
            target,
            remote: None,
            is_current: false,
            is_favorite: false,
            incoming: 0,
            outgoing: 0,
            tracked: None,
        }
    }

    pub(crate) fn is_branch(&self) -> bool {
        self.target.kind != RefKind::Tag
    }

    fn path_name(&self) -> &str {
        let name: &str = &self.target.name;
        self.remote
            .as_ref()
            .and_then(|remote| name.strip_prefix(remote.as_ref()))
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Head,
    TopLevel(TopLevelKind),
    RemoteGroup,
    Group { has_favorites: bool },
    Ref(RefInfo),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    pub id: SharedString,
    pub display_name: SharedString,
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

impl Node {
    pub(crate) fn match_text(&self) -> Option<SharedString> {
        match &self.kind {
            NodeKind::Ref(info) => Some(info.target.name.clone()),
            NodeKind::RemoteGroup | NodeKind::Group { .. } => Some(self.display_name.clone()),
            NodeKind::Head | NodeKind::TopLevel(_) => None,
        }
    }

    fn with_children(&self, children: Vec<Node>) -> Node {
        Node {
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            kind: self.kind.clone(),
            children,
        }
    }

    fn order_weight(&self) -> u8 {
        match &self.kind {
            NodeKind::Ref(info) if info.is_current => 0,
            NodeKind::Ref(info) if info.is_favorite => 1,
            NodeKind::Group {
                has_favorites: true,
            } => 2,
            NodeKind::Group { .. } => 3,
            _ => 4,
        }
    }
}

pub(crate) struct ModelInput<'a> {
    pub local: &'a [RefInfo],
    pub remote: &'a [RefInfo],
    pub tags: &'a [RefInfo],
    pub group_by_directory: bool,
}

pub(crate) fn build_nodes(input: &ModelInput<'_>) -> Vec<Node> {
    let mut nodes = vec![Node {
        id: HEAD_NODE_ID.into(),
        display_name: HEAD_DISPLAY_NAME.into(),
        kind: NodeKind::Head,
        children: Vec::new(),
    }];
    for kind in TopLevelKind::ALL {
        let refs = match kind {
            TopLevelKind::Local => input.local,
            TopLevelKind::Remote => input.remote,
            TopLevelKind::Tags => input.tags,
        };
        let children = group_by_prefix_and_remote(kind.node_id(), refs, input.group_by_directory);
        if !children.is_empty() {
            nodes.push(Node {
                id: kind.node_id().into(),
                display_name: kind.label().into(),
                kind: NodeKind::TopLevel(kind),
                children,
            });
        }
    }
    nodes
}

fn group_by_prefix_and_remote(
    parent_id: &str,
    refs: &[RefInfo],
    group_by_directory: bool,
) -> Vec<Node> {
    let mut without_remote: Vec<&RefInfo> = Vec::new();
    let mut by_remote: Vec<(SharedString, Vec<&RefInfo>)> = Vec::new();
    for info in refs {
        match info.remote.as_ref().filter(|_| group_by_directory) {
            Some(remote) => match by_remote.iter_mut().find(|(name, _)| name == remote) {
                Some((_, branches)) => branches.push(info),
                None => by_remote.push((remote.clone(), vec![info])),
            },
            None => without_remote.push(info),
        }
    }
    let mut nodes = group_by_prefix_if_applicable(parent_id, &without_remote, group_by_directory);
    for (remote, branches) in by_remote {
        let id = format!("{parent_id}/REMOTE:{remote}");
        let children = group_by_prefix_if_applicable(&id, &branches, group_by_directory);
        nodes.push(Node {
            id: id.into(),
            display_name: remote,
            kind: NodeKind::RemoteGroup,
            children,
        });
    }
    sort_nodes(&mut nodes);
    nodes
}

fn group_by_prefix_if_applicable(
    parent_id: &str,
    refs: &[&RefInfo],
    group_by_directory: bool,
) -> Vec<Node> {
    if group_by_directory {
        let paths = refs
            .iter()
            .map(|info| (info.path_name().split('/').collect::<Vec<_>>(), *info))
            .collect();
        return group_by_prefix(parent_id, paths, 0);
    }
    let mut nodes: Vec<Node> = refs
        .iter()
        .map(|info| ref_node(parent_id, info, info.target.name.clone()))
        .collect();
    sort_nodes(&mut nodes);
    nodes
}

fn group_by_prefix(parent_id: &str, paths: Vec<(Vec<&str>, &RefInfo)>, offset: usize) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut groups: Vec<(String, Vec<(Vec<&str>, &RefInfo)>)> = Vec::new();
    for (segments, info) in paths {
        let Some(segment) = segments.get(offset).map(|segment| segment.to_string()) else {
            continue;
        };
        if offset + 1 == segments.len() {
            nodes.push(ref_node(parent_id, info, segment.into()));
            continue;
        }
        match groups.iter_mut().find(|(name, _)| *name == segment) {
            Some((_, entries)) => entries.push((segments, info)),
            None => groups.push((segment, vec![(segments, info)])),
        }
    }
    for (name, entries) in groups {
        let id = format!("{parent_id}/GROUP:{name}");
        let children = group_by_prefix(&id, entries, offset + 1);
        let has_favorites = children.iter().any(|child| match &child.kind {
            NodeKind::Ref(info) => info.is_favorite,
            NodeKind::Group { has_favorites } => *has_favorites,
            _ => false,
        });
        nodes.push(Node {
            id: id.into(),
            display_name: name.into(),
            kind: NodeKind::Group { has_favorites },
            children,
        });
    }
    sort_nodes(&mut nodes);
    nodes
}

fn ref_node(parent_id: &str, info: &RefInfo, display_name: SharedString) -> Node {
    Node {
        id: format!("{parent_id}/REF:{}", info.target.full_ref_name()).into(),
        display_name,
        kind: NodeKind::Ref(info.clone()),
        children: Vec::new(),
    }
}

fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by(|left, right| {
        left.order_weight()
            .cmp(&right.order_weight())
            .then_with(|| natural_compare(&left.display_name, &right.display_name))
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub id: SharedString,
    pub depth: usize,
    pub display_name: SharedString,
    pub kind: NodeKind,
    pub has_children: bool,
    pub expanded: bool,
    pub match_text: Option<SharedString>,
}

pub(crate) fn flatten(nodes: &[Node], expanded: &HashSet<SharedString>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    flatten_into(nodes, 0, expanded, &mut rows);
    rows
}

fn flatten_into(
    nodes: &[Node],
    depth: usize,
    expanded: &HashSet<SharedString>,
    rows: &mut Vec<TreeRow>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        let is_expanded = has_children && expanded.contains(&node.id);
        rows.push(TreeRow {
            id: node.id.clone(),
            depth,
            display_name: node.display_name.clone(),
            kind: node.kind.clone(),
            has_children,
            expanded: is_expanded,
            match_text: node.match_text(),
        });
        if is_expanded {
            flatten_into(&node.children, depth + 1, expanded, rows);
        }
    }
}

pub(crate) fn container_ids(nodes: &[Node], ids: &mut Vec<SharedString>) {
    for node in nodes {
        if !node.children.is_empty() {
            ids.push(node.id.clone());
            container_ids(&node.children, ids);
        }
    }
}

pub(crate) fn preorder_nodes<'a>(nodes: &'a [Node], ordered: &mut Vec<&'a Node>) {
    for node in nodes {
        ordered.push(node);
        preorder_nodes(&node.children, ordered);
    }
}

pub(crate) fn count_nodes(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_nodes(&node.children))
        .sum()
}

pub(crate) fn find_node<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|node| {
        if node.id.as_ref() == id {
            Some(node)
        } else {
            find_node(&node.children, id)
        }
    })
}

pub(crate) struct FilterOutcome {
    pub nodes: Vec<Node>,
    pub best_match: Option<(i32, SharedString)>,
    pub no_hits: bool,
}

pub(crate) fn filter_nodes(nodes: &[Node], matcher: &BranchesTreeMatcher) -> FilterOutcome {
    let mut best_match = None;
    let filtered: Vec<Node> = nodes
        .iter()
        .filter_map(|node| {
            if node.match_text().is_none() {
                let children = filter_children(&node.children, matcher, &mut best_match);
                Some(node.with_children(children))
            } else {
                filter_node(node, matcher, &mut best_match)
            }
        })
        .collect();
    let no_hits = filtered.iter().all(|node| node.children.is_empty());
    FilterOutcome {
        nodes: filtered,
        best_match,
        no_hits,
    }
}

fn filter_children(
    children: &[Node],
    matcher: &BranchesTreeMatcher,
    best_match: &mut Option<(i32, SharedString)>,
) -> Vec<Node> {
    children
        .iter()
        .filter_map(|child| filter_node(child, matcher, best_match))
        .collect()
}

fn filter_node(
    node: &Node,
    matcher: &BranchesTreeMatcher,
    best_match: &mut Option<(i32, SharedString)>,
) -> Option<Node> {
    let children = filter_children(&node.children, matcher, best_match);
    if !children.is_empty() {
        return Some(node.with_children(children));
    }
    let text = node.match_text()?;
    let fragments = matcher.match_fragments(&text)?;
    if let [fragment] = fragments.as_slice() {
        let degree = matcher.matching_degree(&text, Some(std::slice::from_ref(fragment)));
        let best_degree = best_match.as_ref().map_or(0, |(degree, _)| *degree);
        if degree > best_degree {
            *best_match = Some((degree, node.id.clone()));
        }
    }
    Some(node.with_children(Vec::new()))
}

pub(crate) fn node_matching(
    kind: &NodeKind,
    text: Option<&str>,
    matcher: &BranchesTreeMatcher,
) -> Matching {
    if matches!(kind, NodeKind::Group { .. }) {
        return Matching::None;
    }
    match text {
        Some(text) => classify(matcher.match_fragments(text).as_deref()),
        None => Matching::None,
    }
}

pub(crate) fn branch_filters<'a>(
    kinds: impl IntoIterator<Item = &'a NodeKind>,
) -> Vec<SharedString> {
    kinds
        .into_iter()
        .filter_map(|kind| match kind {
            NodeKind::Head => Some(SharedString::new_static(HEAD_REVISION)),
            NodeKind::Ref(info) if info.is_branch() => Some(info.target.name.clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn navigatable_revision<'a>(
    kinds: impl IntoIterator<Item = &'a NodeKind>,
) -> Option<SharedString> {
    kinds.into_iter().find_map(|kind| match kind {
        NodeKind::Head => Some(SharedString::new_static(HEAD_REVISION)),
        NodeKind::Ref(info) => Some(info.target.name.clone()),
        _ => None,
    })
}

pub(crate) fn natural_compare(left: &str, right: &str) -> Ordering {
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
    let left_upper = upper(left);
    let right_upper = upper(right);
    if left_upper == right_upper {
        return Ordering::Equal;
    }
    lower(left_upper).cmp(&lower(right_upper))
}

fn upper(character: char) -> char {
    let mut mapping = character.to_uppercase();
    match (mapping.next(), mapping.next()) {
        (Some(mapped), None) => mapped,
        _ => character,
    }
}

fn lower(character: char) -> char {
    let mut mapping = character.to_lowercase();
    match (mapping.next(), mapping.next()) {
        (Some(mapped), None) => mapped,
        _ => character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(name: &str) -> RefInfo {
        RefInfo::new(RefTarget::local(name.to_string()))
    }

    fn remote(name: &str) -> RefInfo {
        let remote_name = name.split('/').next().unwrap_or_default().to_string();
        RefInfo {
            remote: Some(remote_name.into()),
            ..RefInfo::new(RefTarget::remote(name.to_string()))
        }
    }

    fn tag(name: &str) -> RefInfo {
        RefInfo::new(RefTarget::tag(name.to_string()))
    }

    fn describe(nodes: &[Node]) -> Vec<String> {
        let all: HashSet<SharedString> = {
            let mut ids = Vec::new();
            container_ids(nodes, &mut ids);
            ids.into_iter().collect()
        };
        flatten(nodes, &all)
            .into_iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.display_name))
            .collect()
    }

    fn build(local: &[RefInfo], remote: &[RefInfo], tags: &[RefInfo], group: bool) -> Vec<Node> {
        build_nodes(&ModelInput {
            local,
            remote,
            tags,
            group_by_directory: group,
        })
    }

    #[test]
    fn head_comes_first_and_empty_top_level_groups_are_skipped() {
        let nodes = build(&[local("main")], &[], &[], true);
        assert_eq!(
            describe(&nodes),
            vec!["HEAD (Current Branch)", "Local", "  main"]
        );
        let nodes = build(
            &[local("main")],
            &[remote("origin/main")],
            &[tag("v1")],
            true,
        );
        assert_eq!(
            describe(&nodes),
            vec![
                "HEAD (Current Branch)",
                "Local",
                "  main",
                "Remote",
                "  origin",
                "    main",
                "Tags",
                "  v1"
            ]
        );
    }

    #[test]
    fn nodes_sort_current_then_favorites_then_groups_with_favorites_then_groups_then_others() {
        let current = RefInfo {
            is_current: true,
            ..local("zeta")
        };
        let favorite = RefInfo {
            is_favorite: true,
            ..local("yankee")
        };
        let favorite_in_group = RefInfo {
            is_favorite: true,
            ..local("fix/b")
        };
        let nodes = build(
            &[
                local("alpha10"),
                local("alpha9"),
                local("feature/a"),
                favorite_in_group,
                favorite,
                current,
                local("Beta"),
            ],
            &[],
            &[],
            true,
        );
        assert_eq!(
            describe(&nodes),
            vec![
                "HEAD (Current Branch)",
                "Local",
                "  zeta",
                "  yankee",
                "  fix",
                "    b",
                "  feature",
                "    a",
                "  alpha9",
                "  alpha10",
                "  Beta",
            ]
        );
    }

    #[test]
    fn without_grouping_refs_keep_full_names_and_remotes_are_not_grouped() {
        let nodes = build(
            &[local("feature/a")],
            &[remote("origin/feature/a"), remote("upstream/main")],
            &[],
            false,
        );
        assert_eq!(
            describe(&nodes),
            vec![
                "HEAD (Current Branch)",
                "Local",
                "  feature/a",
                "Remote",
                "  origin/feature/a",
                "  upstream/main",
            ]
        );
    }

    #[test]
    fn natural_order_compares_numbers_by_value_and_ignores_case_first() {
        assert_eq!(natural_compare("v1.9", "v1.10"), Ordering::Less);
        assert_eq!(natural_compare("alpha", "Beta"), Ordering::Less);
        assert_eq!(natural_compare("B", "b"), Ordering::Less);
        assert_eq!(natural_compare("fix", "fix-1"), Ordering::Less);
        assert_eq!(natural_compare("a01", "a1"), Ordering::Greater);
    }

    #[test]
    fn filtering_keeps_head_and_top_level_groups_and_matching_refs_with_their_ancestors() {
        let nodes = build(
            &[local("feature/login"), local("main")],
            &[remote("origin/main")],
            &[],
            true,
        );
        let outcome = filter_nodes(&nodes, &BranchesTreeMatcher::new("login"));
        assert!(!outcome.no_hits);
        assert_eq!(
            describe(&outcome.nodes),
            vec![
                "HEAD (Current Branch)",
                "Local",
                "  feature",
                "    login",
                "Remote"
            ]
        );
        let best = outcome.best_match.map(|(_, id)| id);
        assert_eq!(
            best.as_deref(),
            Some("LOCAL/GROUP:feature/REF:refs/heads/feature/login")
        );

        let outcome = filter_nodes(&nodes, &BranchesTreeMatcher::new("zzz"));
        assert!(outcome.no_hits);
        assert_eq!(
            describe(&outcome.nodes),
            vec!["HEAD (Current Branch)", "Local", "Remote"]
        );
    }

    #[test]
    fn the_best_match_prefers_the_name_that_starts_with_the_pattern() {
        let nodes = build(
            &[local("a-topic"), local("topic"), local("topic-b")],
            &[],
            &[],
            true,
        );
        let outcome = filter_nodes(&nodes, &BranchesTreeMatcher::new("topic"));
        let best = outcome.best_match.map(|(_, id)| id);
        assert_eq!(best.as_deref(), Some("LOCAL/REF:refs/heads/topic"));
    }

    #[test]
    fn head_filters_by_head_and_tags_are_not_branch_filters() {
        let kinds = [
            NodeKind::Head,
            NodeKind::Ref(local("main")),
            NodeKind::Ref(remote("origin/main")),
            NodeKind::Ref(tag("v1")),
            NodeKind::Group {
                has_favorites: false,
            },
        ];
        assert_eq!(
            branch_filters(&kinds),
            vec![
                SharedString::from("HEAD"),
                SharedString::from("main"),
                SharedString::from("origin/main")
            ]
        );
        assert_eq!(navigatable_revision(&kinds[3..]).as_deref(), Some("v1"));
        assert!(navigatable_revision(&kinds[4..]).is_none());
    }

    #[test]
    fn directory_groups_never_match_for_selection() {
        let matcher = BranchesTreeMatcher::new("feat");
        assert_eq!(
            node_matching(
                &NodeKind::Group {
                    has_favorites: false
                },
                Some("feature"),
                &matcher
            ),
            Matching::None
        );
        assert_eq!(
            node_matching(&NodeKind::Ref(local("feature")), Some("feature"), &matcher),
            Matching::Full
        );
    }
}
