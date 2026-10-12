use std::cmp::Ordering;

use collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use smallvec::SmallVec;

use super::{
    linear_graph::{
        EdgeFilter, GraphEdge, GraphEdgeType, GraphLayout, LinearGraph, WalkStep, walk,
    },
    print::LONG_EDGE_SIZE,
};

const BEK_MAX_BLOCK_SIZE: usize = 20;
const BEK_MAX_DELTA_TIME: u64 = 60 * 60 * 24 * 3 * 1000;
const BEK_SMALL_DELTA_TIME: u64 = 60 * 60 * 4 * 1000;
const LINEAR_BEK_MAX_BLOCK_SIZE: u32 = 200;
const LINEAR_BEK_MAGIC_SET_SIZE: u32 = LONG_EDGE_SIZE;

pub(crate) struct SortIndexMap {
    usual_by_sorted: Vec<u32>,
    sorted_by_usual: Vec<u32>,
}

impl SortIndexMap {
    fn new(usual_by_sorted: Vec<u32>) -> Option<Self> {
        let mut sorted_by_usual = vec![u32::MAX; usual_by_sorted.len()];
        for (sorted, &usual) in usual_by_sorted.iter().enumerate() {
            let slot = sorted_by_usual.get_mut(usual as usize)?;
            if *slot != u32::MAX {
                return None;
            }
            *slot = sorted as u32;
        }
        Some(Self {
            usual_by_sorted,
            sorted_by_usual,
        })
    }

    fn identity(node_count: usize) -> Self {
        let indexes: Vec<u32> = (0..node_count as u32).collect();
        Self {
            usual_by_sorted: indexes.clone(),
            sorted_by_usual: indexes,
        }
    }

    fn sorted_index(&self, usual: u32) -> Option<u32> {
        self.sorted_by_usual.get(usual as usize).copied()
    }
}

#[derive(Default)]
struct BekEdgeRestrictions {
    up_to_down: HashMap<u32, SmallVec<[u32; 2]>>,
    down_to_up: HashMap<u32, SmallVec<[u32; 2]>>,
}

impl BekEdgeRestrictions {
    fn add_restriction(&mut self, up: u32, down: u32) {
        put_value(&mut self.up_to_down, up, down);
        put_value(&mut self.down_to_up, down, up);
    }

    fn remove_restriction(&mut self, down: u32) {
        if let Some(up_nodes) = self.down_to_up.get(&down) {
            for &up in up_nodes {
                remove_value(&mut self.up_to_down, up, down);
            }
        }
    }

    fn has_restriction(&self, up: u32) -> bool {
        self.up_to_down.contains_key(&up)
    }
}

fn put_value(map: &mut HashMap<u32, SmallVec<[u32; 2]>>, key: u32, value: u32) {
    let values = map.entry(key).or_default();
    if !values.contains(&value) {
        values.push(value);
    }
}

fn remove_value(map: &mut HashMap<u32, SmallVec<[u32; 2]>>, key: u32, value: u32) {
    let Some(values) = map.get_mut(&key) else {
        return;
    };
    if let Some(position) = values.iter().position(|existing| *existing == value) {
        values.remove(position);
    }
    if values.is_empty() {
        map.remove(&key);
    }
}

struct BekBranch {
    nodes: Vec<u32>,
    no_insert_size: usize,
    prepared_start: Option<usize>,
}

impl BekBranch {
    fn new(nodes: Vec<u32>) -> Self {
        Self {
            no_insert_size: nodes.len(),
            nodes,
            prepared_start: None,
        }
    }

    fn is_done(&self) -> bool {
        self.no_insert_size == 0
    }

    fn prepared_part(&self) -> Option<&[u32]> {
        self.nodes.get(self.prepared_start?..self.no_insert_size)
    }

    fn update_prepared_part(
        &mut self,
        graph: &LinearGraph,
        timestamps: &[i64],
        restrictions: &BekEdgeRestrictions,
    ) {
        let Some(last_index) = self.no_insert_size.checked_sub(1) else {
            return;
        };
        let Some(&current) = self.nodes.get(last_index) else {
            return;
        };
        if restrictions.has_restriction(current) {
            return;
        }
        let timestamp = |node: u32| timestamps.get(node as usize).copied().unwrap_or_default();

        let mut previous = last_index;
        while previous > 0 {
            let up = self.nodes[previous - 1];
            let down = self.nodes[previous];
            if restrictions.has_restriction(up) {
                break;
            }
            let down_nodes: SmallVec<[u32; 4]> = graph.down_nodes(up).collect();
            if down_nodes.len() > 1 && down_nodes.contains(&down) {
                previous -= 1;
                continue;
            }
            if !down_nodes.contains(&down) {
                break;
            }
            let delta = timestamp(up).abs_diff(timestamp(down));
            if delta > BEK_MAX_DELTA_TIME {
                break;
            }
            if previous + BEK_MAX_BLOCK_SIZE < self.no_insert_size && delta > BEK_SMALL_DELTA_TIME {
                break;
            }
            previous -= 1;
        }
        self.prepared_start = Some(previous);
    }

    fn done_insert_prepared_part(&mut self) {
        if let Some(start) = self.prepared_start.take() {
            self.no_insert_size = start;
        }
    }
}

pub(crate) fn bek_sort_map(
    graph: &LinearGraph,
    layout: &GraphLayout,
    timestamps: &[i64],
) -> SortIndexMap {
    let node_count = graph.nodes_count();
    let mut restrictions = BekEdgeRestrictions::default();
    let mut branches = create_bek_branches(graph, layout, &mut restrictions);
    merge_bek_branches(graph, &mut branches, &mut restrictions, timestamps)
        .filter(|order| order.len() == node_count)
        .and_then(SortIndexMap::new)
        .unwrap_or_else(|| {
            log::error!("IntelliSort could not order {node_count} commits topologically");
            SortIndexMap::identity(node_count)
        })
}

fn create_bek_branches(
    graph: &LinearGraph,
    layout: &GraphLayout,
    restrictions: &mut BekEdgeRestrictions,
) -> Vec<BekBranch> {
    let mut done = vec![false; graph.nodes_count()];
    let mut branches = Vec::new();
    for &head in layout.head_nodes() {
        if done.get(head as usize) != Some(&false) {
            continue;
        }
        let nodes = create_next_bek_branch(graph, layout, &mut done, restrictions, head);
        branches.push(BekBranch::new(nodes));
    }
    branches
}

fn create_next_bek_branch(
    graph: &LinearGraph,
    layout: &GraphLayout,
    done: &mut [bool],
    restrictions: &mut BekEdgeRestrictions,
    head: u32,
) -> Vec<u32> {
    done[head as usize] = true;
    let mut nodes = vec![head];
    let start_layout = layout.layout_index(head);
    walk(head, |current| {
        let current_layout = layout.layout_index(current);
        let down_nodes: SmallVec<[u32; 4]> = graph.down_nodes(current).collect();
        for &down in down_nodes.iter().rev() {
            let down_layout = layout.layout_index(down);
            if done[down as usize] {
                if down_layout < start_layout {
                    restrictions.add_restriction(current, down);
                }
            } else if current_layout <= down_layout {
                let has_undone_up_nodes = graph
                    .up_nodes(down)
                    .any(|up| !done[up as usize] && layout.layout_index(up) <= down_layout);
                if !has_undone_up_nodes {
                    done[down as usize] = true;
                    nodes.push(down);
                    return WalkStep::Next(down);
                }
            }
        }
        WalkStep::NotFound
    });
    nodes
}

fn merge_bek_branches(
    graph: &LinearGraph,
    branches: &mut [BekBranch],
    restrictions: &mut BekEdgeRestrictions,
    timestamps: &[i64],
) -> Option<Vec<u32>> {
    let part_timestamp = |branch: &BekBranch| {
        branch
            .prepared_part()
            .and_then(|part| part.first())
            .map_or(i64::MAX, |node| {
                timestamps.get(*node as usize).copied().unwrap_or_default()
            })
    };
    let mut inverse_result: Vec<u32> = Vec::with_capacity(graph.nodes_count());
    loop {
        let mut has_undone_branches = false;
        for branch in branches.iter_mut() {
            if !branch.is_done() {
                has_undone_branches = true;
                if branch.prepared_start.is_none() {
                    branch.update_prepared_part(graph, timestamps, restrictions);
                }
            }
        }
        if !has_undone_branches {
            break;
        }

        let mut selected = 0;
        for (index, branch) in branches.iter().enumerate() {
            if part_timestamp(&branches[selected]) > part_timestamp(branch) {
                selected = index;
            }
        }
        let branch = branches.get_mut(selected)?;
        let part = branch.prepared_part()?.to_vec();
        for node in &part {
            restrictions.remove_restriction(*node);
        }
        inverse_result.extend(part.iter().rev());
        branch.done_insert_prepared_part();
    }
    inverse_result.reverse();
    Some(inverse_result)
}

pub(crate) fn sorted_graph(permanent: &LinearGraph, map: &SortIndexMap) -> LinearGraph {
    let mut graph = LinearGraph::with_capacity(permanent.nodes_count(), permanent.edge_count());
    for &usual in &map.usual_by_sorted {
        let edges = permanent
            .edges(usual)
            .iter()
            .map(|edge| GraphEdge {
                up: edge.up.and_then(|up| map.sorted_index(up)),
                down: edge.down.and_then(|down| map.sorted_index(down)),
                target: edge.target,
                edge_type: edge.edge_type,
            })
            .collect::<SmallVec<[GraphEdge; 4]>>();
        graph.push_node(usual, edges);
    }
    graph
}

#[derive(Default)]
struct EdgeStorage {
    adjacency: HashMap<u32, SmallVec<[(u32, GraphEdgeType); 2]>>,
}

impl EdgeStorage {
    fn create_edge(&mut self, edge: GraphEdge) {
        let (Some(up), Some(down)) = (edge.up, edge.down) else {
            return;
        };
        self.put(up, (down, edge.edge_type));
        self.put(down, (up, edge.edge_type));
    }

    fn remove_edge(&mut self, edge: GraphEdge) {
        let (Some(up), Some(down)) = (edge.up, edge.down) else {
            return;
        };
        self.remove(up, (down, edge.edge_type));
        self.remove(down, (up, edge.edge_type));
    }

    fn put(&mut self, node: u32, entry: (u32, GraphEdgeType)) {
        let entries = self.adjacency.entry(node).or_default();
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }

    fn remove(&mut self, node: u32, entry: (u32, GraphEdgeType)) {
        let Some(entries) = self.adjacency.get_mut(&node) else {
            return;
        };
        if let Some(position) = entries.iter().position(|existing| *existing == entry) {
            entries.remove(position);
        }
        if entries.is_empty() {
            self.adjacency.remove(&node);
        }
    }

    fn has_edge(&self, from: u32, to: u32) -> bool {
        self.adjacency.get(&from).is_some_and(|entries| {
            entries
                .iter()
                .any(|(other, edge_type)| edge_type.is_normal() && *other == to)
        })
    }

    fn adjacent_edges(&self, node: u32, filter: EdgeFilter) -> SmallVec<[GraphEdge; 4]> {
        self.adjacency
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&(other, edge_type)| GraphEdge::normal(node, other, edge_type))
            .filter(|edge| filter.matches(edge, node))
            .collect()
    }
}

struct LinearBekGraph {
    base: LinearGraph,
    dotted_edges: EdgeStorage,
    hidden_edges: EdgeStorage,
}

impl LinearBekGraph {
    fn adjacent_edges(&self, node: u32, filter: EdgeFilter) -> SmallVec<[GraphEdge; 4]> {
        let hidden = self.hidden_edges.adjacent_edges(node, filter);
        let mut edges = self.dotted_edges.adjacent_edges(node, filter);
        edges.extend(self.base.adjacent_edges(node, filter));
        edges.retain(|edge| !hidden.contains(edge));
        edges
    }

    fn down_nodes(&self, node: u32) -> SmallVec<[u32; 4]> {
        self.adjacent_edges(node, EdgeFilter::NormalDown)
            .iter()
            .filter_map(|edge| edge.down)
            .collect()
    }

    fn remove_edge(&mut self, up: u32, down: u32) {
        if self.dotted_edges.has_edge(up, down) {
            let edge = GraphEdge::new(up, down, GraphEdgeType::Dotted);
            self.dotted_edges.remove_edge(edge);
            self.hidden_edges.create_edge(edge);
        } else if let Some(edge) = self.base.edge_between(up, down) {
            self.hidden_edges.create_edge(edge);
        } else {
            log::error!("linear IntelliSort found no edge between {up} and {down}");
        }
    }

    fn replace_edge(&mut self, up: u32, down: u32) {
        if self.dotted_edges.has_edge(up, down) {
            return;
        }
        if let Some(edge) = self.base.edge_between(up, down) {
            self.hidden_edges.create_edge(edge);
        }
        self.dotted_edges
            .create_edge(GraphEdge::new(up, down, GraphEdgeType::Dotted));
    }

    fn compile(&self) -> LinearGraph {
        let node_count = self.base.nodes_count();
        let mut graph = LinearGraph::with_capacity(node_count, self.base.edge_count());
        for node in 0..node_count as u32 {
            graph.push_node(
                self.base.node_id(node).unwrap_or(node),
                self.adjacent_edges(node, EdgeFilter::All),
            );
        }
        graph
    }
}

struct MergeFragment {
    parent: u32,
    left_child: u32,
    merge_with_old_commit: bool,
    tail_edges: BTreeMap<u32, SmallVec<[u32; 2]>>,
    body: HashSet<u32>,
    tails: BTreeSet<u32>,
}

impl MergeFragment {
    fn new(parent: u32, left_child: u32) -> Self {
        Self {
            parent,
            left_child,
            merge_with_old_commit: false,
            tail_edges: BTreeMap::new(),
            body: HashSet::default(),
            tails: BTreeSet::new(),
        }
    }

    fn add_tail(&mut self, tail: u32) {
        if !self.body.contains(&tail) {
            self.tails.insert(tail);
        }
    }

    fn add_tail_edge(&mut self, up: u32, down: u32) {
        if self.body.contains(&up) {
            return;
        }
        self.tails.insert(up);
        let downs = self.tail_edges.entry(up).or_default();
        if !downs.contains(&down) {
            downs.push(down);
        }
    }

    fn add_body(&mut self, body: u32) {
        self.body.insert(body);
    }

    fn has_tail_edge(&self, node: u32) -> bool {
        self.tail_edges
            .get(&node)
            .is_some_and(|downs| !downs.is_empty())
    }

    fn collapse(&self, graph: &mut LinearBekGraph) {
        for (&up, downs) in &self.tail_edges {
            for &down in downs {
                graph.remove_edge(up, down);
            }
        }
        for &tail in &self.tails {
            if graph.down_nodes(tail).contains(&self.left_child) {
                graph.replace_edge(tail, self.left_child);
            } else {
                graph.dotted_edges.create_edge(GraphEdge::new(
                    tail,
                    self.left_child,
                    GraphEdgeType::Dotted,
                ));
            }
        }
        graph.remove_edge(self.parent, self.left_child);
    }
}

struct JavaPriorityQueue<T, F> {
    queue: Vec<T>,
    compare: F,
}

impl<T: Copy, F: Fn(&T, &T) -> Ordering> JavaPriorityQueue<T, F> {
    fn new(compare: F) -> Self {
        Self {
            queue: Vec::new(),
            compare,
        }
    }

    fn add_all(&mut self, elements: impl IntoIterator<Item = T>) {
        for element in elements {
            self.offer(element);
        }
    }

    fn offer(&mut self, element: T) {
        let mut index = self.queue.len();
        self.queue.push(element);
        while index > 0 {
            let parent = (index - 1) / 2;
            if (self.compare)(&element, &self.queue[parent]) != Ordering::Less {
                break;
            }
            self.queue[index] = self.queue[parent];
            index = parent;
        }
        self.queue[index] = element;
    }

    fn poll(&mut self) -> Option<T> {
        let last = self.queue.pop()?;
        let Some(&result) = self.queue.first() else {
            return Some(last);
        };
        let length = self.queue.len();
        let mut index = 0;
        while index < length / 2 {
            let mut child = 2 * index + 1;
            let right = child + 1;
            if right < length
                && (self.compare)(&self.queue[child], &self.queue[right]) == Ordering::Greater
            {
                child = right;
            }
            if (self.compare)(&last, &self.queue[child]) != Ordering::Greater {
                break;
            }
            self.queue[index] = self.queue[child];
            index = child;
        }
        self.queue[index] = last;
        Some(result)
    }
}

fn compare_by_down_node(left: &GraphEdge, right: &GraphEdge) -> Ordering {
    match (left.down, right.down) {
        (Some(left), Some(right)) => left.cmp(&right),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

pub(crate) fn linear_bek_graph(base: LinearGraph, layout: &GraphLayout) -> LinearGraph {
    let node_layouts: Vec<i32> = base
        .node_ids()
        .iter()
        .map(|node_id| layout.layout_index(*node_id))
        .collect();
    let layout_index = |node: u32| node_layouts.get(node as usize).copied().unwrap_or_default();
    let mut graph = LinearBekGraph {
        base,
        dotted_edges: EdgeStorage::default(),
        hidden_edges: EdgeStorage::default(),
    };
    for node in (0..graph.base.nodes_count() as u32).rev() {
        if let Some(fragment) = merge_fragment(&graph, node, &layout_index) {
            fragment.collapse(&mut graph);
        }
    }
    graph.compile()
}

fn merge_fragment(
    graph: &LinearBekGraph,
    merge: u32,
    layout_index: &impl Fn(u32) -> i32,
) -> Option<MergeFragment> {
    let mut down_nodes = graph.down_nodes(merge);
    down_nodes.sort_unstable();
    let [right_child, left_child] = down_nodes[..] else {
        return None;
    };

    let mut fragment = MergeFragment::new(merge, left_child);
    let left_layout = layout_index(left_child);
    let right_layout = layout_index(right_child);
    let mut rows_count: u32 = 1;
    let mut block_size: u32 = 1;
    let mut queue = JavaPriorityQueue::new(compare_by_down_node);
    queue.add_all(graph.adjacent_edges(right_child, EdgeFilter::NormalDown));
    let mut magic_set: Option<HashSet<u32>> = None;

    while let Some(edge) = queue.poll() {
        let Some(up) = edge.up else {
            continue;
        };
        let Some(next) = edge.down else {
            fragment.add_tail(up);
            continue;
        };
        if next == left_child {
            fragment.add_tail(up);
            fragment.merge_with_old_commit = true;
        } else if next == right_child + rows_count {
            rows_count += 1;
            block_size += 1;
            queue.add_all(graph.adjacent_edges(next, EdgeFilter::NormalDown));
            fragment.add_body(up);
        } else if next > right_child + rows_count && next < left_child {
            rows_count = next - right_child + 1;
            block_size += 1;
            queue.add_all(graph.adjacent_edges(next, EdgeFilter::NormalDown));
            fragment.add_body(up);
        } else if next > left_child {
            let next_layout = layout_index(next);
            if left_layout > right_layout && !fragment.merge_with_old_commit {
                if next > left_child + LINEAR_BEK_MAGIC_SET_SIZE {
                    return None;
                }
                let magic_set =
                    magic_set.get_or_insert_with(|| calculate_magic_set(graph, left_child));
                if !magic_set.contains(&next) {
                    return None;
                }
                fragment.add_tail_edge(up, next);
            } else if (next_layout > left_layout && next_layout < right_layout)
                || next_layout == left_layout
            {
                fragment.add_tail_edge(up, next);
            } else if next_layout >= right_layout {
                return None;
            } else if next > left_child + LINEAR_BEK_MAGIC_SET_SIZE {
                if !fragment.has_tail_edge(up) && !fragment.body.contains(&up) {
                    return None;
                }
            } else {
                let magic_set =
                    magic_set.get_or_insert_with(|| calculate_magic_set(graph, left_child));
                if !magic_set.contains(&next) {
                    return None;
                }
                fragment.add_tail_edge(up, next);
            }
        }

        if block_size >= LINEAR_BEK_MAX_BLOCK_SIZE {
            return None;
        }
    }

    if fragment.tails.is_empty() {
        return None;
    }
    Some(fragment)
}

fn calculate_magic_set(graph: &LinearBekGraph, node: u32) -> HashSet<u32> {
    let mut magic_set = HashSet::default();
    let mut queue: BTreeSet<u32> = graph.down_nodes(node).into_iter().collect();
    while let Some(next) = queue.pop_first() {
        if next > node + LINEAR_BEK_MAGIC_SET_SIZE {
            break;
        }
        if magic_set.insert(next) {
            queue.extend(graph.down_nodes(next));
        }
    }
    magic_set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_log::graph::linear_graph::{
        build_permanent_graph,
        tests::{expected_graph, layout_by_node_index, numbered_commits, test_graph_string},
    };
    use pretty_assertions::assert_eq;

    fn bek(parents: &[&[u32]]) -> String {
        let commits = numbered_commits(parents);
        let graph = build_permanent_graph(&commits).graph;
        let layout = layout_by_node_index(&graph);
        let timestamps = vec![0; graph.nodes_count()];
        let map = bek_sort_map(&graph, &layout, &timestamps);
        let sorted = sorted_graph(&graph, &map);
        for node in 0..sorted.nodes_count() as u32 {
            assert!(
                sorted.down_nodes(node).all(|down| down > node),
                "IntelliSort produced a reversed edge at row {node}"
            );
        }
        test_graph_string(&sorted)
    }

    fn linear_bek(parents: &[&[u32]]) -> String {
        let commits = numbered_commits(parents);
        let graph = build_permanent_graph(&commits).graph;
        let layout = layout_by_node_index(&graph);
        test_graph_string(&linear_bek_graph(graph, &layout))
    }

    #[test]
    fn bek_keeps_already_sorted_history() {
        let parents: &[&[u32]] = &[&[1, 3], &[4, 2], &[4, 3], &[], &[]];
        assert_eq!(
            bek(parents),
            expected_graph(&[
                (0, &["1", "3"]),
                (1, &["4", "2"]),
                (2, &["4", "3"]),
                (3, &[]),
                (4, &[]),
            ])
        );
    }

    #[test]
    fn bek_shows_merged_commits_directly_below_the_merge() {
        assert_eq!(
            bek(&[&[1, 2], &[3], &[4], &[5], &[5], &[]]),
            expected_graph(&[
                (0, &["1", "2"]),
                (2, &["4"]),
                (4, &["5"]),
                (1, &["3"]),
                (3, &["5"]),
                (5, &[]),
            ])
        );
    }

    #[test]
    fn bek_orders_independent_heads_by_their_layout() {
        assert_eq!(
            bek(&[&[2], &[2], &[]]),
            expected_graph(&[(1, &["2"]), (0, &["2"]), (2, &[])])
        );
        assert_eq!(
            bek(&[&[2, 3], &[4], &[4], &[5], &[6], &[6], &[]]),
            expected_graph(&[
                (1, &["4"]),
                (0, &["2", "3"]),
                (3, &["5"]),
                (5, &["6"]),
                (2, &["4"]),
                (4, &["6"]),
                (6, &[]),
            ])
        );
    }

    #[test]
    fn linear_bek_collapses_a_merged_branch_into_a_dotted_edge() {
        assert_eq!(
            linear_bek(&[&[4, 1], &[2], &[3], &[7], &[5], &[6], &[7], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["3"]),
                (3, &["4.dot"]),
                (4, &["5"]),
                (5, &["6"]),
                (6, &["7"]),
                (7, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[1, 3], &[2], &[3], &[]]),
            expected_graph(&[(0, &["1"]), (1, &["2"]), (2, &["3.dot"]), (3, &[])])
        );
        assert_eq!(
            linear_bek(&[&[2, 1], &[2], &[]]),
            expected_graph(&[(0, &["1"]), (1, &["2.dot"]), (2, &[])])
        );
    }

    #[test]
    fn linear_bek_collapses_consecutive_and_nested_merges() {
        assert_eq!(
            linear_bek(&[&[2, 1], &[3], &[3], &[5, 4], &[6], &[6], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2.dot"]),
                (2, &["3"]),
                (3, &["4"]),
                (4, &["5.dot"]),
                (5, &["6"]),
                (6, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[5, 1], &[3, 2], &[4], &[4], &[6], &[6], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["3.dot"]),
                (3, &["4"]),
                (4, &["5.dot"]),
                (5, &["6"]),
                (6, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[
                &[5, 1],
                &[2, 3],
                &[4, 3],
                &[4],
                &[9],
                &[6, 7],
                &[8, 7],
                &[8],
                &[9],
                &[],
            ]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["3.dot"]),
                (3, &["4.dot"]),
                (4, &["5.dot"]),
                (5, &["6"]),
                (6, &["7.dot"]),
                (7, &["8.dot"]),
                (8, &["9"]),
                (9, &[]),
            ])
        );
    }

    #[test]
    fn linear_bek_handles_diagonal_merges() {
        assert_eq!(
            linear_bek(&[&[2, 1], &[3], &[4, 3], &[5], &[5], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2.dot"]),
                (2, &["3"]),
                (3, &["4.dot"]),
                (4, &["5"]),
                (5, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[2, 1], &[2, 3], &[3], &[]]),
            expected_graph(&[(0, &["1"]), (1, &["2.dot"]), (2, &["3.dot"]), (3, &[])])
        );
    }

    #[test]
    fn linear_bek_keeps_tails_that_cannot_be_rebased() {
        assert_eq!(
            linear_bek(&[
                &[8, 1],
                &[2],
                &[6, 3],
                &[4],
                &[9, 5],
                &[11],
                &[7],
                &[10],
                &[9],
                &[10],
                &[11],
                &[],
            ]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["6", "3"]),
                (3, &["4"]),
                (4, &["5"]),
                (5, &["8.dot"]),
                (6, &["7"]),
                (7, &["8.dot"]),
                (8, &["9"]),
                (9, &["10"]),
                (10, &["11"]),
                (11, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[4, 1], &[3, 2], &[4], &[5], &[5], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["3", "2"]),
                (2, &["4.dot"]),
                (3, &["4.dot"]),
                (4, &["5"]),
                (5, &[]),
            ])
        );
    }

    #[test]
    fn linear_bek_respects_heads_and_initial_imports() {
        assert_eq!(
            linear_bek(&[&[4, 2], &[3], &[3], &[5], &[5], &[]]),
            expected_graph(&[
                (0, &["2"]),
                (1, &["3"]),
                (2, &["3"]),
                (3, &["4.dot"]),
                (4, &["5"]),
                (5, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[2], &[4], &[5, 3], &[4], &[5], &[]]),
            expected_graph(&[
                (0, &["2"]),
                (1, &["4"]),
                (2, &["3"]),
                (3, &["4"]),
                (4, &["5.dot"]),
                (5, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[3, 1], &[2], &[], &[]]),
            expected_graph(&[(0, &["3", "1"]), (1, &["2"]), (2, &[]), (3, &[])])
        );
    }

    #[test]
    fn linear_bek_merges_with_old_commits() {
        assert_eq!(
            linear_bek(&[&[5, 1], &[5, 2], &[3], &[4], &[5], &[6], &[7], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["3"]),
                (3, &["4"]),
                (4, &["5.dot"]),
                (5, &["6"]),
                (6, &["7"]),
                (7, &[]),
            ])
        );
        assert_eq!(
            linear_bek(&[&[1, 4], &[2], &[3, 4], &[5, 6], &[5], &[6], &[]]),
            expected_graph(&[
                (0, &["1"]),
                (1, &["2"]),
                (2, &["3"]),
                (3, &["4.dot"]),
                (4, &["5"]),
                (5, &["6.dot"]),
                (6, &[]),
            ])
        );
    }

    #[test]
    fn java_priority_queue_polls_equal_keys_in_heap_order() {
        let mut queue =
            JavaPriorityQueue::new(|left: &(u32, char), right: &(u32, char)| left.0.cmp(&right.0));
        queue.add_all([(5, 'a'), (3, 'b'), (3, 'c'), (1, 'd'), (3, 'e')]);
        let polled: Vec<(u32, char)> = std::iter::from_fn(|| queue.poll()).collect();
        assert_eq!(
            polled,
            vec![(1, 'd'), (3, 'e'), (3, 'b'), (3, 'c'), (5, 'a')]
        );
    }
}
