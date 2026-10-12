use std::{borrow::Cow, cmp::Ordering};

use collections::{HashMap, HashSet};
use git::Oid;
use smallvec::SmallVec;

use super::GraphCommitSource;

const UNDERDONE_EDGE: i32 = -1;
const FIRST_NOT_LOADED_ID: i32 = -2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum GraphEdgeType {
    Usual,
    Dotted,
    NotLoadCommit,
}

impl GraphEdgeType {
    pub(crate) fn is_normal(self) -> bool {
        matches!(self, Self::Usual | Self::Dotted)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GraphEdge {
    pub(crate) up: Option<u32>,
    pub(crate) down: Option<u32>,
    pub(crate) target: Option<i32>,
    pub(crate) edge_type: GraphEdgeType,
}

impl GraphEdge {
    pub(crate) fn new(up: u32, down: u32, edge_type: GraphEdgeType) -> Self {
        Self {
            up: Some(up),
            down: Some(down),
            target: None,
            edge_type,
        }
    }

    pub(crate) fn normal(first: u32, second: u32, edge_type: GraphEdgeType) -> Self {
        Self::new(first.min(second), first.max(second), edge_type)
    }

    pub(crate) fn not_loaded(up: u32, target: i32) -> Self {
        Self {
            up: Some(up),
            down: None,
            target: Some(target),
            edge_type: GraphEdgeType::NotLoadCommit,
        }
    }

    pub(crate) fn as_normal(&self) -> Option<NormalEdge> {
        if !self.edge_type.is_normal() {
            return None;
        }
        Some(NormalEdge {
            up: self.up?,
            down: self.down?,
        })
    }

    pub(crate) fn not_null_node(&self) -> u32 {
        self.up.or(self.down).unwrap_or_default()
    }

    pub(crate) fn is_up_edge_of(&self, node: u32) -> bool {
        self.down == Some(node)
    }

    pub(crate) fn is_down_edge_of(&self, node: u32) -> bool {
        self.up == Some(node)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NormalEdge {
    pub(crate) up: u32,
    pub(crate) down: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeFilter {
    All,
    NormalDown,
    NormalUp,
    Special,
}

impl EdgeFilter {
    fn up_normal(self) -> bool {
        matches!(self, Self::All | Self::NormalUp)
    }

    fn down_normal(self) -> bool {
        matches!(self, Self::All | Self::NormalDown)
    }

    fn special(self) -> bool {
        matches!(self, Self::All | Self::Special)
    }

    pub(crate) fn matches(self, edge: &GraphEdge, node: u32) -> bool {
        if edge.edge_type.is_normal() {
            (edge.is_up_edge_of(node) && self.up_normal())
                || (edge.is_down_edge_of(node) && self.down_normal())
        } else {
            self.special()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LinearGraph {
    node_ids: Vec<u32>,
    edge_starts: Vec<u32>,
    edges: Vec<GraphEdge>,
}

impl LinearGraph {
    pub(crate) fn with_capacity(node_count: usize, edge_count: usize) -> Self {
        let mut edge_starts = Vec::with_capacity(node_count + 1);
        edge_starts.push(0);
        Self {
            node_ids: Vec::with_capacity(node_count),
            edge_starts,
            edges: Vec::with_capacity(edge_count),
        }
    }

    pub(crate) fn push_node(&mut self, node_id: u32, edges: impl IntoIterator<Item = GraphEdge>) {
        if self.edge_starts.is_empty() {
            self.edge_starts.push(0);
        }
        self.node_ids.push(node_id);
        self.edges.extend(edges);
        self.edge_starts.push(self.edges.len() as u32);
    }

    pub(crate) fn nodes_count(&self) -> usize {
        self.node_ids.len()
    }

    pub(crate) fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub(crate) fn node_id(&self, node: u32) -> Option<u32> {
        self.node_ids.get(node as usize).copied()
    }

    pub(crate) fn node_ids(&self) -> &[u32] {
        &self.node_ids
    }

    pub(crate) fn edges(&self, node: u32) -> &[GraphEdge] {
        let node = node as usize;
        match (self.edge_starts.get(node), self.edge_starts.get(node + 1)) {
            (Some(&start), Some(&end)) => self
                .edges
                .get(start as usize..end as usize)
                .unwrap_or_default(),
            _ => &[],
        }
    }

    pub(crate) fn adjacent_edges(
        &self,
        node: u32,
        filter: EdgeFilter,
    ) -> impl Iterator<Item = GraphEdge> + '_ {
        self.edges(node)
            .iter()
            .filter(move |edge| filter.matches(edge, node))
            .copied()
    }

    pub(crate) fn down_nodes(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        self.adjacent_edges(node, EdgeFilter::NormalDown)
            .filter_map(|edge| edge.down)
    }

    pub(crate) fn up_nodes(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        self.adjacent_edges(node, EdgeFilter::NormalUp)
            .filter_map(|edge| edge.up)
    }

    pub(crate) fn edge_between(&self, up: u32, down: u32) -> Option<GraphEdge> {
        self.adjacent_edges(up, EdgeFilter::NormalDown)
            .find(|edge| edge.down == Some(down))
    }

    pub(crate) fn heads(&self) -> Vec<u32> {
        (0..self.nodes_count() as u32)
            .filter(|&node| self.up_nodes(node).next().is_none())
            .collect()
    }
}

pub(crate) struct PermanentGraph {
    pub(crate) graph: LinearGraph,
    pub(crate) not_loaded_commits: Vec<Oid>,
}

struct UnresolvedParent {
    first_seen: usize,
    up_nodes: SmallVec<[(u32, GraphEdgeType); 2]>,
}

fn parent_edge_type<C: GraphCommitSource>(commit: &C, parent: &Oid) -> GraphEdgeType {
    if commit.dotted_parents().contains(parent) {
        GraphEdgeType::Dotted
    } else {
        GraphEdgeType::Usual
    }
}

pub(crate) fn fix_duplicate_parents(parents: &[Oid]) -> Cow<'_, [Oid]> {
    match parents {
        [] | [_] => Cow::Borrowed(parents),
        [first, second] if first != second => Cow::Borrowed(parents),
        [first, _] => Cow::Owned(vec![*first]),
        _ => {
            let mut seen = HashSet::default();
            if parents.iter().all(|parent| seen.insert(*parent)) {
                return Cow::Borrowed(parents);
            }
            let mut seen = HashSet::default();
            Cow::Owned(
                parents
                    .iter()
                    .filter(|parent| seen.insert(**parent))
                    .copied()
                    .collect(),
            )
        }
    }
}

pub(crate) fn build_permanent_graph<C: GraphCommitSource>(commits: &[C]) -> PermanentGraph {
    let node_count = commits.len();
    let simple_nodes: Vec<bool> = commits
        .iter()
        .enumerate()
        .map(
            |(index, commit)| match &*fix_duplicate_parents(commit.parents()) {
                [parent] => {
                    parent_edge_type(commit, parent) == GraphEdgeType::Usual
                        && commits
                            .get(index + 1)
                            .is_some_and(|next| next.sha() == *parent)
                }
                _ => false,
            },
        )
        .collect();
    let long_edge_count: usize = commits
        .iter()
        .zip(&simple_nodes)
        .filter(|(_, simple)| !**simple)
        .map(|(commit, _)| fix_duplicate_parents(commit.parents()).len())
        .sum();

    let mut node_to_edge_index = vec![0u32; node_count + 1];
    let mut long_edges = vec![(UNDERDONE_EDGE, GraphEdgeType::Usual); 2 * long_edge_count];
    let mut unresolved: HashMap<Oid, UnresolvedParent> = HashMap::default();
    let mut registration_count = 0;

    for (node, commit) in commits.iter().enumerate() {
        let up_nodes = unresolved
            .remove(&commit.sha())
            .map(|parent| parent.up_nodes)
            .unwrap_or_default();
        let mut edge_index = node_to_edge_index[node] as usize;
        for (up_node, edge_type) in up_nodes {
            fix_underdone_edge(
                commits,
                &node_to_edge_index,
                &mut long_edges,
                up_node,
                node as u32,
                commit.sha(),
            );
            if let Some(slot) = long_edges.get_mut(edge_index) {
                *slot = (up_node as i32, edge_type);
            }
            edge_index += 1;
        }
        if !simple_nodes[node] {
            for parent in fix_duplicate_parents(commit.parents()).iter() {
                let edge_type = parent_edge_type(commit, parent);
                unresolved
                    .entry(*parent)
                    .or_insert_with(|| {
                        registration_count += 1;
                        UnresolvedParent {
                            first_seen: registration_count,
                            up_nodes: SmallVec::new(),
                        }
                    })
                    .up_nodes
                    .push((node as u32, edge_type));
                if let Some(slot) = long_edges.get_mut(edge_index) {
                    *slot = (UNDERDONE_EDGE, edge_type);
                }
                edge_index += 1;
            }
        }
        node_to_edge_index[node + 1] = edge_index as u32;
    }

    let mut not_loaded: Vec<(Oid, UnresolvedParent)> = unresolved.into_iter().collect();
    not_loaded.sort_by_key(|(_, parent)| {
        (
            parent
                .up_nodes
                .iter()
                .map(|(up_node, _)| *up_node)
                .min()
                .unwrap_or(u32::MAX),
            parent.first_seen,
        )
    });
    let mut not_loaded_commits = Vec::with_capacity(not_loaded.len());
    for (order, (sha, parent)) in not_loaded.into_iter().enumerate() {
        let not_loaded_id = FIRST_NOT_LOADED_ID - order as i32;
        not_loaded_commits.push(sha);
        for (up_node, _) in parent.up_nodes {
            let start = node_to_edge_index[up_node as usize] as usize;
            let end = node_to_edge_index[up_node as usize + 1] as usize;
            if let Some(slot) = long_edges
                .get_mut(start..end)
                .and_then(|slots| slots.iter_mut().find(|slot| slot.0 == UNDERDONE_EDGE))
            {
                slot.0 = not_loaded_id;
            }
        }
    }

    let simple_edge_count = simple_nodes.iter().filter(|simple| **simple).count();
    let mut graph = LinearGraph::with_capacity(
        node_count,
        node_to_edge_index[node_count] as usize + 2 * simple_edge_count,
    );
    let mut previous_is_simple = false;
    for (node, &is_simple) in simple_nodes.iter().enumerate() {
        let node_index = node as u32;
        let mut edges: SmallVec<[GraphEdge; 4]> = SmallVec::new();
        if previous_is_simple {
            edges.push(GraphEdge::new(
                node_index - 1,
                node_index,
                GraphEdgeType::Usual,
            ));
        }
        let start = node_to_edge_index[node] as usize;
        let end = node_to_edge_index[node + 1] as usize;
        for &(adjacent, edge_type) in long_edges.get(start..end).unwrap_or_default() {
            if adjacent < 0 {
                if adjacent != UNDERDONE_EDGE {
                    edges.push(GraphEdge::not_loaded(node_index, adjacent));
                }
                continue;
            }
            let adjacent = adjacent as u32;
            match adjacent.cmp(&node_index) {
                Ordering::Less => edges.push(GraphEdge::new(adjacent, node_index, edge_type)),
                Ordering::Greater => edges.push(GraphEdge::new(node_index, adjacent, edge_type)),
                Ordering::Equal => {}
            }
        }
        if is_simple {
            edges.push(GraphEdge::new(
                node_index,
                node_index + 1,
                GraphEdgeType::Usual,
            ));
        }
        graph.push_node(node_index, edges);
        previous_is_simple = is_simple;
    }

    PermanentGraph {
        graph,
        not_loaded_commits,
    }
}

fn fix_underdone_edge<C: GraphCommitSource>(
    commits: &[C],
    node_to_edge_index: &[u32],
    long_edges: &mut [(i32, GraphEdgeType)],
    up_node: u32,
    down_node: u32,
    down_sha: Oid,
) {
    let Some(up_commit) = commits.get(up_node as usize) else {
        return;
    };
    let Some(&end) = node_to_edge_index.get(up_node as usize + 1) else {
        return;
    };
    let parents = fix_duplicate_parents(up_commit.parents());
    let Some(parent_index) = parents.iter().position(|parent| *parent == down_sha) else {
        log::error!("no underdone edge for node {up_node} and adjacent down node {down_node}");
        return;
    };
    let Some(edge_index) = (end as usize).checked_sub(parents.len() - parent_index) else {
        return;
    };
    match long_edges.get_mut(edge_index) {
        Some(slot) if slot.0 == UNDERDONE_EDGE => slot.0 = down_node as i32,
        _ => log::error!("edge was set early, up node {up_node}, down node {down_node}"),
    }
}

pub(crate) enum WalkStep {
    Next(u32),
    NotFound,
}

pub(crate) fn walk(start: u32, mut next_node: impl FnMut(u32) -> WalkStep) {
    let mut stack = vec![start];
    while let Some(&current) = stack.last() {
        match next_node(current) {
            WalkStep::Next(node) => stack.push(node),
            WalkStep::NotFound => {
                stack.pop();
            }
        }
    }
}

pub(crate) fn stable_sort_by<T: Copy>(
    items: &mut [T],
    mut compare: impl FnMut(&T, &T) -> Ordering,
) {
    let length = items.len();
    if length < 2 {
        return;
    }
    let mut buffer = items.to_vec();
    let mut width = 1;
    while width < length {
        let mut start = 0;
        while start < length {
            let middle = (start + width).min(length);
            let end = (start + 2 * width).min(length);
            let mut left = start;
            let mut right = middle;
            let mut output = start;
            while left < middle && right < end {
                if compare(&items[right], &items[left]) == Ordering::Less {
                    buffer[output] = items[right];
                    right += 1;
                } else {
                    buffer[output] = items[left];
                    left += 1;
                }
                output += 1;
            }
            let left_rest = middle - left;
            buffer[output..output + left_rest].copy_from_slice(&items[left..middle]);
            output += left_rest;
            buffer[output..output + (end - right)].copy_from_slice(&items[right..end]);
            start = end;
        }
        items.copy_from_slice(&buffer);
        width *= 2;
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GraphLayout {
    layout_indexes: Vec<i32>,
    head_nodes: Vec<u32>,
    head_layout_indexes: Vec<i32>,
}

impl GraphLayout {
    pub(crate) fn build(
        graph: &LinearGraph,
        branch_nodes: &[u32],
        mut compare_heads: impl FnMut(u32, u32) -> Ordering,
    ) -> Self {
        let node_count = graph.nodes_count();
        let mut is_head = vec![false; node_count];
        let mut heads = Vec::new();
        for node in branch_nodes.iter().copied().chain(graph.heads()) {
            if let Some(seen) = is_head.get_mut(node as usize)
                && !*seen
            {
                *seen = true;
                heads.push(node);
            }
        }
        stable_sort_by(&mut heads, |left, right| compare_heads(*left, *right));

        let mut layout_indexes = vec![0i32; node_count];
        let mut head_nodes = Vec::new();
        let mut current_layout_index = 1;
        for head in heads {
            if layout_indexes[head as usize] != 0 {
                continue;
            }
            head_nodes.push(head);
            walk(head, |current| {
                let first_visit = layout_indexes[current as usize] == 0;
                if first_visit {
                    layout_indexes[current as usize] = current_layout_index;
                }
                match graph
                    .down_nodes(current)
                    .find(|child| layout_indexes[*child as usize] == 0)
                {
                    Some(child) => WalkStep::Next(child),
                    None => {
                        if first_visit {
                            current_layout_index += 1;
                        }
                        WalkStep::NotFound
                    }
                }
            });
        }

        let head_layout_indexes = head_nodes
            .iter()
            .map(|head| layout_indexes[*head as usize])
            .collect();
        Self {
            layout_indexes,
            head_nodes,
            head_layout_indexes,
        }
    }

    pub(crate) fn layout_index(&self, node: u32) -> i32 {
        self.layout_indexes
            .get(node as usize)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn one_of_head_node_index(&self, node: u32) -> u32 {
        let order = match self
            .head_layout_indexes
            .binary_search(&self.layout_index(node))
        {
            Ok(order) => order,
            Err(insertion_point) => insertion_point.saturating_sub(1),
        };
        self.head_nodes.get(order).copied().unwrap_or_default()
    }

    pub(crate) fn head_nodes(&self) -> &[u32] {
        &self.head_nodes
    }
}

pub(crate) fn first_parent_graph(permanent: &LinearGraph, start_nodes: &[u32]) -> LinearGraph {
    let node_count = permanent.nodes_count();
    let mut visible = vec![false; node_count];
    let mut visited = vec![false; node_count];
    let mut hidden_edges = HashSet::default();
    for &start in start_nodes {
        let mut current = Some(start);
        while let Some(node) = current {
            match visited.get_mut(node as usize) {
                Some(seen) if !*seen => *seen = true,
                _ => break,
            }
            visible[node as usize] = true;
            let mut down_edges = permanent.adjacent_edges(node, EdgeFilter::NormalDown);
            current = down_edges.next().and_then(|edge| edge.down);
            hidden_edges.extend(down_edges);
        }
    }

    let mut rows_by_node = vec![None; node_count];
    let mut visible_nodes = Vec::new();
    for (node, is_visible) in visible.iter().enumerate() {
        if *is_visible {
            rows_by_node[node] = Some(visible_nodes.len() as u32);
            visible_nodes.push(node as u32);
        }
    }
    let compiled_index = |node: Option<u32>| -> Option<Option<u32>> {
        match node {
            None => Some(None),
            Some(node) => rows_by_node.get(node as usize).copied().flatten().map(Some),
        }
    };

    let mut graph = LinearGraph::with_capacity(visible_nodes.len(), visible_nodes.len() * 2);
    for &node in &visible_nodes {
        let edges = permanent
            .edges(node)
            .iter()
            .filter(|edge| !hidden_edges.contains(*edge))
            .filter_map(|edge| {
                Some(GraphEdge {
                    up: compiled_index(edge.up)?,
                    down: compiled_index(edge.down)?,
                    target: edge.target,
                    edge_type: edge.edge_type,
                })
            })
            .collect::<SmallVec<[GraphEdge; 4]>>();
        graph.push_node(node, edges);
    }
    graph
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::SharedString;
    use pretty_assertions::assert_eq;

    pub(crate) struct TestCommit {
        pub(crate) sha: Oid,
        pub(crate) parents: Vec<Oid>,
        pub(crate) dotted_parents: Vec<Oid>,
        pub(crate) timestamp: i64,
        pub(crate) ref_names: Vec<SharedString>,
    }

    impl GraphCommitSource for TestCommit {
        fn sha(&self) -> Oid {
            self.sha
        }

        fn parents(&self) -> &[Oid] {
            &self.parents
        }

        fn dotted_parents(&self) -> &[Oid] {
            &self.dotted_parents
        }

        fn timestamp(&self) -> i64 {
            self.timestamp
        }

        fn ref_names(&self) -> &[SharedString] {
            &self.ref_names
        }
    }

    pub(crate) fn test_oid(name: &str) -> Oid {
        let mut bytes = [0u8; 20];
        for (index, byte) in name.bytes().enumerate().take(19) {
            bytes[index + 1] = byte;
        }
        bytes[0] = name.len() as u8;
        Oid::from_bytes(&bytes).unwrap()
    }

    pub(crate) fn parse_commits(input: &str) -> Vec<TestCommit> {
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (sha, parents) = line.trim().split_once("|-").unwrap();
                TestCommit {
                    sha: test_oid(sha),
                    parents: parents.split_whitespace().map(test_oid).collect(),
                    dotted_parents: Vec::new(),
                    timestamp: 0,
                    ref_names: Vec::new(),
                }
            })
            .collect()
    }

    pub(crate) fn numbered_commits(parents: &[&[u32]]) -> Vec<TestCommit> {
        parents
            .iter()
            .enumerate()
            .map(|(index, parents)| TestCommit {
                sha: test_oid(&index.to_string()),
                parents: parents
                    .iter()
                    .map(|parent| test_oid(&parent.to_string()))
                    .collect(),
                dotted_parents: Vec::new(),
                timestamp: 0,
                ref_names: Vec::new(),
            })
            .collect()
    }

    pub(crate) fn filtered_commits(parents: &[(&[u32], &[u32])]) -> Vec<TestCommit> {
        let numbered_oid = |parent: &u32| test_oid(&parent.to_string());
        parents
            .iter()
            .enumerate()
            .map(|(index, (direct, dotted))| TestCommit {
                sha: test_oid(&index.to_string()),
                parents: direct
                    .iter()
                    .chain(dotted.iter())
                    .map(numbered_oid)
                    .collect(),
                dotted_parents: dotted.iter().map(numbered_oid).collect(),
                timestamp: 0,
                ref_names: Vec::new(),
            })
            .collect()
    }

    pub(crate) fn edge_string(edge: &GraphEdge) -> String {
        let value = |value: Option<u32>| value.map_or("n".to_string(), |value| value.to_string());
        let type_char = match edge.edge_type {
            GraphEdgeType::Usual => 'U',
            GraphEdgeType::Dotted => 'D',
            GraphEdgeType::NotLoadCommit => 'N',
        };
        format!(
            "{}:{}:{}_{type_char}",
            value(edge.up),
            value(edge.down),
            edge.target
                .map_or("n".to_string(), |target| target.to_string())
        )
    }

    pub(crate) fn graph_string(graph: &LinearGraph) -> String {
        (0..graph.nodes_count() as u32)
            .map(|node| {
                let edges = graph
                    .edges(node)
                    .iter()
                    .map(edge_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{node}_U|-{edges}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn test_graph_string(graph: &LinearGraph) -> String {
        let mut result = String::new();
        for node in 0..graph.nodes_count() as u32 {
            let node_id = |node: u32| graph.node_id(node).unwrap();
            let down_edges = graph
                .edges(node)
                .iter()
                .filter(|edge| edge.up == Some(node))
                .map(|edge| {
                    let target = if edge.edge_type.is_normal() {
                        node_id(edge.down.unwrap()).to_string()
                    } else {
                        edge.target
                            .map_or("null".to_string(), |target| target.to_string())
                    };
                    match edge.edge_type {
                        GraphEdgeType::Usual => target,
                        GraphEdgeType::Dotted => format!("{target}.dot"),
                        GraphEdgeType::NotLoadCommit => format!("{target}.not_load"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            result.push_str(&format!("{}({down_edges})\n", node_id(node)));
        }
        result
    }

    pub(crate) fn expected_graph(nodes: &[(u32, &[&str])]) -> String {
        let mut result = String::new();
        for (node, edges) in nodes {
            result.push_str(&format!("{node}({})\n", edges.join(", ")));
        }
        result
    }

    pub(crate) fn layout_by_node_index(graph: &LinearGraph) -> GraphLayout {
        GraphLayout::build(graph, &[], |left, right| left.cmp(&right))
    }

    fn layout_string(layout: &GraphLayout, node_count: usize) -> String {
        (0..node_count as u32)
            .map(|node| {
                format!(
                    "{}|-{}",
                    layout.layout_index(node),
                    layout.one_of_head_node_index(node)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_graph_builder(input: &str, expected: &str) {
        let commits = parse_commits(input);
        let permanent = build_permanent_graph(&commits);
        assert_eq!(graph_string(&permanent.graph), expected.trim());
    }

    #[test]
    fn graph_builder_links_single_parent_chains_as_simple_edges() {
        assert_graph_builder(
            "a0|-a1\na1|-a2\na2|-a3\na3|-",
            "0_U|-0:1:n_U\n1_U|-0:1:n_U 1:2:n_U\n2_U|-1:2:n_U 2:3:n_U\n3_U|-2:3:n_U",
        );
    }

    #[test]
    fn graph_builder_keeps_parent_order_for_long_edges() {
        assert_graph_builder(
            "a0|-a1 a6 a3\na1|-a6\na2|-a3\na3|-\na4|-a5\na5|-\na6|-a7\na7|-",
            "0_U|-0:1:n_U 0:6:n_U 0:3:n_U\n\
             1_U|-0:1:n_U 1:6:n_U\n\
             2_U|-2:3:n_U\n\
             3_U|-2:3:n_U 0:3:n_U\n\
             4_U|-4:5:n_U\n\
             5_U|-4:5:n_U\n\
             6_U|-0:6:n_U 1:6:n_U 6:7:n_U\n\
             7_U|-6:7:n_U",
        );
        assert_graph_builder(
            "a66|-ffff a aa\naa|-f ffff ff\nffff|-ff\nff|-\na|-\nf|-",
            "0_U|-0:2:n_U 0:4:n_U 0:1:n_U\n\
             1_U|-0:1:n_U 1:5:n_U 1:2:n_U 1:3:n_U\n\
             2_U|-0:2:n_U 1:2:n_U 2:3:n_U\n\
             3_U|-2:3:n_U 1:3:n_U\n\
             4_U|-0:4:n_U\n\
             5_U|-1:5:n_U",
        );
    }

    #[test]
    fn graph_builder_lists_many_up_nodes_with_the_simple_edge_first() {
        assert_graph_builder(
            "a0|-a4\na1|-a4\na2|-a4\na3|-a4\na4|-",
            "0_U|-0:4:n_U\n1_U|-1:4:n_U\n2_U|-2:4:n_U\n3_U|-3:4:n_U\n4_U|-3:4:n_U 0:4:n_U 1:4:n_U 2:4:n_U",
        );
    }

    #[test]
    fn graph_builder_removes_duplicate_parents() {
        assert_graph_builder(
            "0|-1 1 1 3 1 1\n1|-6\n2|-3 4 4 3\n3|-4 4\n4|-5 6\n5|-\n6|-",
            "0_U|-0:1:n_U 0:3:n_U\n\
             1_U|-0:1:n_U 1:6:n_U\n\
             2_U|-2:3:n_U 2:4:n_U\n\
             3_U|-0:3:n_U 2:3:n_U 3:4:n_U\n\
             4_U|-3:4:n_U 2:4:n_U 4:5:n_U 4:6:n_U\n\
             5_U|-4:5:n_U\n\
             6_U|-1:6:n_U 4:6:n_U",
        );
    }

    #[test]
    fn graph_builder_numbers_not_loaded_parents_by_first_child() {
        let commits = parse_commits("a0|-a1 a6 a3\na1|-a6\na2|-a3\na3|-\na4|-a5");
        let permanent = build_permanent_graph(&commits);
        assert_eq!(
            graph_string(&permanent.graph),
            "0_U|-0:1:n_U 0:n:-2_N 0:3:n_U\n\
             1_U|-0:1:n_U 1:n:-2_N\n\
             2_U|-2:3:n_U\n\
             3_U|-2:3:n_U 0:3:n_U\n\
             4_U|-4:n:-3_N"
        );
        assert_eq!(
            permanent.not_loaded_commits,
            vec![test_oid("a6"), test_oid("a5")]
        );
    }

    #[test]
    fn graph_builder_treats_parents_listed_above_their_child_as_not_loaded() {
        let commits = numbered_commits(&[&[], &[0]]);
        let permanent = build_permanent_graph(&commits);
        assert_eq!(graph_string(&permanent.graph), "0_U|-\n1_U|-1:n:-2_N");
    }

    #[test]
    fn graph_builder_draws_rewritten_parents_as_dotted_edges() {
        let commits = filtered_commits(&[(&[], &[1]), (&[], &[])]);
        assert_eq!(
            graph_string(&build_permanent_graph(&commits).graph),
            "0_U|-0:1:n_D\n1_U|-0:1:n_D"
        );

        let commits = filtered_commits(&[(&[1], &[2]), (&[2], &[]), (&[], &[])]);
        assert_eq!(
            graph_string(&build_permanent_graph(&commits).graph),
            "0_U|-0:1:n_U 0:2:n_D\n1_U|-0:1:n_U 1:2:n_U\n2_U|-1:2:n_U 0:2:n_D"
        );
    }

    #[test]
    fn graph_builder_reproduces_intellij_filtered_graphs_from_rewritten_parents() {
        let filtered_graph = |parents: &[(&[u32], &[u32])]| {
            test_graph_string(&build_permanent_graph(&filtered_commits(parents)).graph)
        };
        assert_eq!(
            filtered_graph(&[(&[1, 2], &[]), (&[], &[3]), (&[], &[3]), (&[], &[])]),
            expected_graph(&[(0, &["1", "2"]), (1, &["3.dot"]), (2, &["3.dot"]), (3, &[]),])
        );
        assert_eq!(
            filtered_graph(&[(&[], &[1, 2]), (&[3], &[]), (&[3], &[]), (&[], &[])]),
            expected_graph(&[(0, &["1.dot", "2.dot"]), (1, &["3"]), (2, &["3"]), (3, &[]),])
        );
        assert_eq!(
            filtered_graph(&[(&[], &[1, 2, 3]), (&[], &[]), (&[], &[]), (&[], &[])]),
            expected_graph(&[
                (0, &["1.dot", "2.dot", "3.dot"]),
                (1, &[]),
                (2, &[]),
                (3, &[]),
            ])
        );
        assert_eq!(
            filtered_graph(&[(&[2], &[]), (&[], &[3]), (&[], &[3]), (&[], &[])]),
            expected_graph(&[(0, &["2"]), (1, &["3.dot"]), (2, &["3.dot"]), (3, &[])])
        );
    }

    #[test]
    fn graph_builder_keeps_a_rewritten_parent_that_is_not_loaded_as_a_not_loaded_edge() {
        let commits = filtered_commits(&[(&[], &[7])]);
        let permanent = build_permanent_graph(&commits);
        assert_eq!(graph_string(&permanent.graph), "0_U|-0:n:-2_N");
        assert_eq!(permanent.not_loaded_commits, vec![test_oid("7")]);
    }

    fn assert_layout(input: &str, expected: &str) {
        let commits = parse_commits(input);
        let permanent = build_permanent_graph(&commits);
        let names: Vec<String> = input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().split_once("|-").unwrap().0.to_string())
            .collect();
        let layout = GraphLayout::build(&permanent.graph, &[], |left, right| {
            names[left as usize].cmp(&names[right as usize])
        });
        assert_eq!(
            layout_string(&layout, permanent.graph.nodes_count()),
            expected.trim()
        );
    }

    #[test]
    fn layout_assigns_fragments_in_depth_first_order() {
        assert_layout(
            "a0|-a1 a6 a3\na1|-a7\na2|-a3\na3|-\na4|-a5\na5|-\na6|-a7\na7|-a8\na8|-",
            "1|-0\n1|-0\n4|-2\n3|-0\n5|-4\n5|-4\n2|-0\n1|-0\n1|-0",
        );
        assert_layout(
            "a0|-a1 a6 a3\na1|-a6\na2|-a3\na3|-\na4|-a5",
            "1|-0\n1|-0\n3|-2\n2|-0\n4|-4",
        );
    }

    #[test]
    fn layout_starts_with_the_first_head_in_comparator_order() {
        assert_layout(
            "ff|-a1 a3\na1|-a4\na2|-a3\na3|-a4\na4|-",
            "2|-0\n2|-0\n1|-2\n1|-2\n1|-2",
        );
    }

    fn heads_for_commits(graph: &LinearGraph, branch_heads: &[u32], order: &[u32]) -> Vec<u32> {
        let position = |node: u32| order.iter().position(|head| *head == node);
        let layout = GraphLayout::build(graph, branch_heads, |left, right| {
            position(left).cmp(&position(right))
        });
        (0..graph.nodes_count() as u32)
            .map(|node| layout.one_of_head_node_index(node))
            .collect()
    }

    #[test]
    fn layout_branch_heads_take_over_the_rest_of_a_linear_history() {
        let commits = numbered_commits(&[&[1], &[2], &[3], &[]]);
        let graph = build_permanent_graph(&commits).graph;
        assert_eq!(
            heads_for_commits(&graph, &[0, 2], &[0, 2]),
            vec![0, 0, 0, 0]
        );
        assert_eq!(
            heads_for_commits(&graph, &[0, 2], &[2, 0]),
            vec![0, 0, 2, 2]
        );
    }

    #[test]
    fn layout_head_order_decides_which_branch_owns_shared_history() {
        let commits = numbered_commits(&[&[2], &[3], &[3], &[4], &[]]);
        let graph = build_permanent_graph(&commits).graph;
        let cases: [(&[u32], [u32; 5]); 6] = [
            (&[0, 1, 2], [0, 1, 0, 0, 0]),
            (&[0, 2, 1], [0, 1, 0, 0, 0]),
            (&[1, 0, 2], [0, 1, 0, 1, 1]),
            (&[1, 2, 0], [0, 1, 2, 1, 1]),
            (&[2, 0, 1], [0, 1, 2, 2, 2]),
            (&[2, 1, 0], [0, 1, 2, 2, 2]),
        ];
        for (order, expected) in cases {
            assert_eq!(heads_for_commits(&graph, &[0, 2], order), expected.to_vec());
        }
    }

    fn first_parent(parents: &[&[u32]], start_nodes: &[u32]) -> String {
        let commits = numbered_commits(parents);
        let graph = build_permanent_graph(&commits).graph;
        test_graph_string(&first_parent_graph(&graph, start_nodes))
    }

    #[test]
    fn first_parent_hides_merged_branches() {
        assert_eq!(
            first_parent(&[&[1], &[2, 3], &[4], &[4], &[]], &[0]),
            expected_graph(&[(0, &["1"]), (1, &["2"]), (2, &["4"]), (4, &[])])
        );
    }

    #[test]
    fn first_parent_follows_every_start_node() {
        assert_eq!(
            first_parent(&[&[2], &[5], &[3, 4], &[6], &[5], &[6], &[]], &[0, 1]),
            expected_graph(&[
                (0, &["2"]),
                (1, &["5"]),
                (2, &["3"]),
                (3, &["6"]),
                (5, &["6"]),
                (6, &[]),
            ])
        );
        assert_eq!(
            first_parent(
                &[&[2], &[3], &[5, 3], &[4], &[6, 5], &[7], &[7], &[]],
                &[0, 1]
            ),
            expected_graph(&[
                (0, &["2"]),
                (1, &["3"]),
                (2, &["5"]),
                (3, &["4"]),
                (4, &["6"]),
                (5, &["7"]),
                (6, &["7"]),
                (7, &[]),
            ])
        );
    }

    #[test]
    fn first_parent_starting_below_a_merge_ignores_nodes_above() {
        assert_eq!(
            first_parent(&[&[1], &[2, 3], &[4], &[4], &[]], &[3]),
            expected_graph(&[(3, &["4"]), (4, &[])])
        );
    }

    #[test]
    fn stable_sort_keeps_equal_elements_in_input_order() {
        let mut items = [(3, 'a'), (1, 'b'), (3, 'c'), (2, 'd'), (1, 'e')];
        stable_sort_by(&mut items, |left, right| left.0.cmp(&right.0));
        assert_eq!(items, [(1, 'b'), (1, 'e'), (2, 'd'), (3, 'a'), (3, 'c')]);
    }
}
