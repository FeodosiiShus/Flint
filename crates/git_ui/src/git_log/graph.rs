mod color;
mod intelli_sort;
mod linear_graph;
mod paint;
mod print;

use std::{cmp::Ordering, ops::Range, sync::Arc};

use collections::{HashMap, HashSet};
use git::{Oid, repository::InitialGraphCommitData};
use gpui::SharedString;

use crate::git_log::properties::{GraphOptions, GraphSortType};

pub(crate) use color::{GraphPalette, natural_compare};
pub(crate) use paint::{element_at, graph_width, paint_graph_row};

use color::{
    DEFAULT_COLOR_ID, GraphRef, RefType, best_ref_for_layout, compare_for_branch_layout,
    java_string_hash, parse_refs,
};
use intelli_sort::{bek_sort_map, linear_bek_graph, sorted_graph};
use linear_graph::{
    GraphEdgeType, GraphLayout, LinearGraph, build_permanent_graph, first_parent_graph,
};
use print::{
    EdgesInRowIndex, GraphElement, LongEdgeSizes, PrintElementGenerator, recommended_width,
};

const MILLISECONDS_PER_SECOND: i64 = 1000;

pub(crate) trait GraphCommitSource {
    fn sha(&self) -> Oid;
    fn parents(&self) -> &[Oid];
    fn dotted_parents(&self) -> &[Oid];
    fn timestamp(&self) -> i64;
    fn ref_names(&self) -> &[SharedString];
}

impl GraphCommitSource for Arc<InitialGraphCommitData> {
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
        self.committer_timestamp
            .saturating_mul(MILLISECONDS_PER_SECOND)
    }

    fn ref_names(&self) -> &[SharedString] {
        &self.ref_names
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeType {
    Fill,
    OutlineAndFill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineStyle {
    Solid,
    Dashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EdgePrintElement {
    pub(crate) other_position: usize,
    pub(crate) direction: EdgeDirection,
    pub(crate) line_style: LineStyle,
    pub(crate) has_arrow: bool,
    pub(crate) is_terminal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrintElementKind {
    Node(NodeType),
    Edge(EdgePrintElement),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrintElement {
    pub(crate) row: usize,
    pub(crate) position: usize,
    pub(crate) kind: PrintElementKind,
    pub(crate) color_id: i32,
    pub(crate) is_selected: bool,
    graph_element: GraphElement,
}

impl PrintElement {
    pub(crate) fn has_arrow(&self) -> bool {
        matches!(self.kind, PrintElementKind::Edge(edge) if edge.has_arrow)
    }

    pub(crate) fn same_print_element(&self, other: &PrintElement) -> bool {
        if self.row != other.row || self.position != other.position {
            return false;
        }
        match (self.kind, other.kind) {
            (PrintElementKind::Node(_), PrintElementKind::Node(_)) => true,
            (PrintElementKind::Edge(left), PrintElementKind::Edge(right)) => {
                left.other_position == right.other_position
                    && left.direction == right.direction
                    && left.has_arrow == right.has_arrow
            }
            _ => false,
        }
    }
}

pub(crate) struct CommitGraph {
    shas: Vec<Oid>,
    node_by_sha: HashMap<Oid, u32>,
    not_loaded_commits: Vec<Oid>,
    permanent: LinearGraph,
    visible: Option<LinearGraph>,
    rows_by_node: Vec<Option<u32>>,
    layout: GraphLayout,
    best_refs: HashMap<u32, GraphRef>,
    head_nodes: HashSet<u32>,
    edges_in_row: EdgesInRowIndex,
    show_long_edges: bool,
    recommended_width: usize,
}

impl CommitGraph {
    pub(crate) fn new<C: GraphCommitSource>(
        commits: &[C],
        options: GraphOptions,
        show_long_edges: bool,
        remote_names: &[SharedString],
    ) -> Self {
        let permanent = build_permanent_graph(commits);
        let mut node_by_sha = HashMap::default();
        let mut best_refs = HashMap::default();
        let mut head_nodes = HashSet::default();
        let mut branch_nodes = Vec::new();
        for (node, commit) in commits.iter().enumerate() {
            let node = node as u32;
            node_by_sha.entry(commit.sha()).or_insert(node);
            if commit.ref_names().is_empty() {
                continue;
            }
            let refs = parse_refs(commit.ref_names(), remote_names);
            if refs.iter().any(|graph_ref| graph_ref.ref_type.is_branch()) {
                branch_nodes.push(node);
            }
            if refs
                .iter()
                .any(|graph_ref| graph_ref.ref_type == RefType::Head)
            {
                head_nodes.insert(node);
            }
            if let Some(best_ref) = best_ref_for_layout(&refs) {
                best_refs.insert(node, best_ref.clone());
            }
        }

        let layout = GraphLayout::build(&permanent.graph, &branch_nodes, |left, right| {
            compare_heads(&best_refs, left, right)
        });
        let visible = match options {
            GraphOptions::Base(GraphSortType::Normal) => None,
            GraphOptions::Base(GraphSortType::Bek) => {
                let timestamps: Vec<i64> =
                    commits.iter().map(|commit| commit.timestamp()).collect();
                let map = bek_sort_map(&permanent.graph, &layout, &timestamps);
                Some(sorted_graph(&permanent.graph, &map))
            }
            GraphOptions::LinearBek => {
                let timestamps: Vec<i64> =
                    commits.iter().map(|commit| commit.timestamp()).collect();
                let map = bek_sort_map(&permanent.graph, &layout, &timestamps);
                Some(linear_bek_graph(
                    sorted_graph(&permanent.graph, &map),
                    &layout,
                ))
            }
            GraphOptions::FirstParent => {
                let mut start_nodes = branch_nodes;
                start_nodes.extend(permanent.graph.heads());
                Some(first_parent_graph(&permanent.graph, &start_nodes))
            }
        };

        let mut rows_by_node = Vec::new();
        if let Some(visible) = &visible {
            rows_by_node = vec![None; permanent.graph.nodes_count()];
            for (row, node) in visible.node_ids().iter().enumerate() {
                if let Some(slot) = rows_by_node.get_mut(*node as usize) {
                    *slot = Some(row as u32);
                }
            }
        }

        let visible_graph = visible.as_ref().unwrap_or(&permanent.graph);
        let edges_in_row = EdgesInRowIndex::build(visible_graph);
        let recommended_width =
            recommended_width(visible_graph, LongEdgeSizes::new(show_long_edges));

        Self {
            shas: commits.iter().map(|commit| commit.sha()).collect(),
            node_by_sha,
            not_loaded_commits: permanent.not_loaded_commits,
            permanent: permanent.graph,
            visible,
            rows_by_node,
            layout,
            best_refs,
            head_nodes,
            edges_in_row,
            show_long_edges,
            recommended_width,
        }
    }

    fn visible_graph(&self) -> &LinearGraph {
        self.visible.as_ref().unwrap_or(&self.permanent)
    }

    pub(crate) fn row_count(&self) -> usize {
        self.visible_graph().nodes_count()
    }

    pub(crate) fn commit_index_at_row(&self, row: usize) -> Option<usize> {
        let row = u32::try_from(row).ok()?;
        self.visible_graph().node_id(row).map(|node| node as usize)
    }

    pub(crate) fn sha_at_row(&self, row: usize) -> Option<Oid> {
        self.commit_index_at_row(row)
            .and_then(|index| self.shas.get(index).copied())
    }

    pub(crate) fn row_for_commit_index(&self, index: usize) -> Option<usize> {
        if index >= self.shas.len() {
            return None;
        }
        if self.visible.is_none() {
            return Some(index);
        }
        self.rows_by_node
            .get(index)
            .copied()
            .flatten()
            .map(|row| row as usize)
    }

    pub(crate) fn row_for_sha(&self, sha: &Oid) -> Option<usize> {
        self.node_by_sha
            .get(sha)
            .and_then(|node| self.row_for_commit_index(*node as usize))
    }

    pub(crate) fn recommended_width(&self) -> usize {
        self.recommended_width
    }

    pub(crate) fn set_show_long_edges(&mut self, show_long_edges: bool) {
        if self.show_long_edges == show_long_edges {
            return;
        }
        self.show_long_edges = show_long_edges;
        self.recommended_width =
            recommended_width(self.visible_graph(), LongEdgeSizes::new(show_long_edges));
    }

    pub(crate) fn adjacent_rows(&self, row: usize, parents: bool) -> Vec<usize> {
        let Ok(row) = u32::try_from(row) else {
            return Vec::new();
        };
        let graph = self.visible_graph();
        if parents {
            graph.down_nodes(row).map(|node| node as usize).collect()
        } else {
            graph.up_nodes(row).map(|node| node as usize).collect()
        }
    }

    pub(crate) fn print_elements(
        &self,
        rows: Range<usize>,
        selected: Option<&PrintElement>,
    ) -> Vec<Vec<PrintElement>> {
        let graph = self.visible_graph();
        let layout_index = |row: u32| {
            graph
                .node_id(row)
                .map_or(0, |node| self.layout.layout_index(node))
        };
        let generator = PrintElementGenerator::new(
            graph,
            &self.edges_in_row,
            LongEdgeSizes::new(self.show_long_edges),
            &layout_index,
        );
        let mut rows = generator.print_elements(rows);
        for element in rows.iter_mut().flatten() {
            element.color_id = self.color_id(&element.graph_element);
            if let PrintElementKind::Node(node_type) = &mut element.kind
                && self.is_head_row(element.row)
            {
                *node_type = NodeType::OutlineAndFill;
            }
            element.is_selected =
                selected.is_some_and(|selected| selected.same_print_element(element));
        }
        rows
    }

    pub(crate) fn arrow_target(&self, element: &PrintElement) -> Option<Oid> {
        let PrintElementKind::Edge(edge_element) = element.kind else {
            return None;
        };
        let GraphElement::Edge(edge) = element.graph_element else {
            return None;
        };
        if !edge_element.has_arrow {
            return None;
        }
        if edge.edge_type == GraphEdgeType::NotLoadCommit {
            let index = usize::try_from(-i64::from(edge.target?) - 2).ok()?;
            return self.not_loaded_commits.get(index).copied();
        }
        let row = match edge_element.direction {
            EdgeDirection::Down => edge.down?,
            EdgeDirection::Up => edge.up?,
        };
        self.sha_at_row(row as usize)
    }

    fn is_head_row(&self, row: usize) -> bool {
        self.commit_index_at_row(row)
            .is_some_and(|node| self.head_nodes.contains(&(node as u32)))
    }

    fn node_color_id(&self, node: u32) -> i32 {
        let layout_index = self.layout.layout_index(node);
        let head = self.layout.one_of_head_node_index(node);
        if self.layout.layout_index(head) == layout_index {
            self.best_refs
                .get(&head)
                .map_or(DEFAULT_COLOR_ID, |graph_ref| {
                    java_string_hash(&graph_ref.name)
                })
        } else {
            layout_index
        }
    }

    fn color_id(&self, element: &GraphElement) -> i32 {
        let graph = self.visible_graph();
        let row_color = |row: u32| {
            graph
                .node_id(row)
                .map_or(DEFAULT_COLOR_ID, |node| self.node_color_id(node))
        };
        match element {
            GraphElement::Node(row) => row_color(*row),
            GraphElement::Edge(edge) => match edge.as_normal() {
                None => row_color(edge.not_null_node()),
                Some(normal) => match (graph.node_id(normal.up), graph.node_id(normal.down)) {
                    (Some(up), Some(down)) => {
                        if self.layout.layout_index(up) >= self.layout.layout_index(down) {
                            self.node_color_id(up)
                        } else {
                            self.node_color_id(down)
                        }
                    }
                    _ => DEFAULT_COLOR_ID,
                },
            },
        }
    }
}

fn compare_heads(best_refs: &HashMap<u32, GraphRef>, left: u32, right: u32) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    match (best_refs.get(&left), best_refs.get(&right)) {
        (None, None) => left.cmp(&right),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left_ref), Some(right_ref)) => compare_for_branch_layout(left_ref, right_ref),
    }
}

#[cfg(test)]
mod tests {
    use super::linear_graph::tests::{TestCommit, test_oid};
    use super::*;
    use pretty_assertions::assert_eq;

    fn commit(name: &str, parents: &[&str], timestamp: i64, refs: &[&str]) -> TestCommit {
        TestCommit {
            sha: test_oid(name),
            parents: parents.iter().map(|parent| test_oid(parent)).collect(),
            dotted_parents: Vec::new(),
            timestamp,
            ref_names: refs
                .iter()
                .map(|name| SharedString::from(name.to_string()))
                .collect(),
        }
    }

    fn with_dotted_parents(mut commit: TestCommit, dotted_parents: &[&str]) -> TestCommit {
        commit.dotted_parents = dotted_parents
            .iter()
            .map(|parent| test_oid(parent))
            .collect();
        commit
    }

    fn remotes() -> Vec<SharedString> {
        vec![SharedString::from("origin")]
    }

    fn feature_merge_history() -> Vec<TestCommit> {
        vec![
            commit(
                "merge",
                &["main-1", "feature-2"],
                6,
                &["HEAD -> main", "origin/main"],
            ),
            commit("main-1", &["base"], 5, &[]),
            commit("feature-2", &["feature-1"], 4, &[]),
            commit("feature-1", &["base"], 3, &[]),
            commit("base", &["root"], 2, &[]),
            commit("root", &[], 1, &[]),
        ]
    }

    fn row_names(graph: &CommitGraph, names: &[&str]) -> Vec<String> {
        (0..graph.row_count())
            .map(|row| {
                let sha = graph.sha_at_row(row).unwrap();
                names
                    .iter()
                    .find(|name| test_oid(name) == sha)
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn normal_order_keeps_the_git_log_order_and_maps_rows_both_ways() {
        let commits = feature_merge_history();
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        assert_eq!(graph.row_count(), commits.len());
        for (index, commit) in commits.iter().enumerate() {
            assert_eq!(graph.sha_at_row(index), Some(commit.sha));
            assert_eq!(graph.row_for_sha(&commit.sha), Some(index));
            assert_eq!(graph.commit_index_at_row(index), Some(index));
        }
        assert_eq!(graph.sha_at_row(commits.len()), None);
        assert_eq!(graph.row_for_sha(&test_oid("missing")), None);
        assert_eq!(graph.adjacent_rows(0, true), vec![1, 2]);
        assert_eq!(graph.adjacent_rows(4, false), vec![3, 1]);
    }

    #[test]
    fn head_commit_is_drawn_with_the_head_node_painter() {
        let commits = feature_merge_history();
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let rows = graph.print_elements(0..commits.len(), None);
        let node_types: Vec<NodeType> = rows
            .iter()
            .map(|elements| {
                elements
                    .iter()
                    .find_map(|element| match element.kind {
                        PrintElementKind::Node(node_type) => Some(node_type),
                        PrintElementKind::Edge(_) => None,
                    })
                    .unwrap()
            })
            .collect();
        assert_eq!(node_types[0], NodeType::OutlineAndFill);
        assert!(
            node_types[1..]
                .iter()
                .all(|node_type| *node_type == NodeType::Fill)
        );
    }

    #[test]
    fn colors_follow_the_best_ref_of_the_head_fragment() {
        let commits = feature_merge_history();
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let rows = graph.print_elements(0..commits.len(), None);
        let node_color = |row: usize| {
            rows[row]
                .iter()
                .find(|element| matches!(element.kind, PrintElementKind::Node(_)))
                .unwrap()
                .color_id
        };
        let main_color = java_string_hash("origin/main");
        assert_eq!(node_color(0), main_color);
        assert_eq!(node_color(1), main_color);
        assert_eq!(node_color(4), main_color);
        assert_eq!(node_color(5), main_color);
        assert_eq!(node_color(2), 2);
        assert_eq!(node_color(3), 2);
    }

    #[test]
    fn heads_without_refs_use_the_default_color() {
        let commits = [
            commit("tip", &["root"], 2, &[]),
            commit("root", &[], 1, &[]),
        ];
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let rows = graph.print_elements(0..2, None);
        assert!(
            rows.iter()
                .flatten()
                .all(|element| element.color_id == DEFAULT_COLOR_ID)
        );
    }

    #[test]
    fn first_parent_hides_commits_reachable_only_through_merges() {
        let commits = feature_merge_history();
        let names = ["merge", "main-1", "feature-2", "feature-1", "base", "root"];
        let graph = CommitGraph::new(&commits, GraphOptions::FirstParent, false, &remotes());
        assert_eq!(
            row_names(&graph, &names),
            vec!["merge", "main-1", "base", "root"]
        );
        assert_eq!(graph.row_for_sha(&test_oid("feature-2")), None);
        assert_eq!(graph.row_for_sha(&test_oid("base")), Some(2));
        assert_eq!(graph.commit_index_at_row(2), Some(4));
    }

    #[test]
    fn intelli_sort_places_merged_commits_directly_below_the_merge() {
        let commits = feature_merge_history();
        let names = ["merge", "main-1", "feature-2", "feature-1", "base", "root"];
        let graph = CommitGraph::new(
            &commits,
            GraphOptions::Base(GraphSortType::Bek),
            false,
            &remotes(),
        );
        assert_eq!(
            row_names(&graph, &names),
            vec!["merge", "feature-2", "feature-1", "main-1", "base", "root"]
        );
        assert_eq!(graph.row_for_sha(&test_oid("main-1")), Some(3));
    }

    #[test]
    fn linear_intelli_sort_draws_the_merged_branch_as_rebased() {
        let commits = feature_merge_history();
        let names = ["merge", "main-1", "feature-2", "feature-1", "base", "root"];
        let graph = CommitGraph::new(&commits, GraphOptions::LinearBek, false, &remotes());
        assert_eq!(
            row_names(&graph, &names),
            vec!["merge", "feature-2", "feature-1", "main-1", "base", "root"]
        );
        let rows = graph.print_elements(0..graph.row_count(), None);
        assert!(rows.iter().flatten().any(|element| matches!(
            element.kind,
            PrintElementKind::Edge(edge) if edge.line_style == LineStyle::Dashed
        )));
    }

    #[test]
    fn not_loaded_parents_end_in_an_arrow_pointing_at_the_missing_commit() {
        let commits = [
            commit("tip", &["unloaded"], 2, &["main"]),
            commit("other", &[], 1, &["feature"]),
        ];
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let rows = graph.print_elements(0..2, None);
        let arrow = rows[1]
            .iter()
            .find(|element| element.has_arrow())
            .copied()
            .unwrap();
        assert_eq!(graph.arrow_target(&arrow), Some(test_oid("unloaded")));
        let node = rows[1]
            .iter()
            .find(|element| matches!(element.kind, PrintElementKind::Node(_)))
            .unwrap();
        assert_eq!(graph.arrow_target(node), None);
    }

    #[test]
    fn long_edges_toggle_between_arrows_and_full_lines() {
        let mut commits = vec![commit("0", &["1", "40"], 100, &["main"])];
        for index in 1..40 {
            commits.push(commit(
                &index.to_string(),
                &[&(index + 1).to_string()],
                100 - index,
                &[],
            ));
        }
        commits.push(commit("40", &[], 0, &[]));
        let middle_row = 20;
        let edge_count = |graph: &CommitGraph| {
            graph.print_elements(middle_row..middle_row + 1, None)[0]
                .iter()
                .filter(|element| matches!(element.kind, PrintElementKind::Edge(_)))
                .count()
        };

        let mut graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let collapsed_edges = edge_count(&graph);
        let rows = graph.print_elements(0..3, None);
        let arrow = rows[1]
            .iter()
            .find(|element| element.has_arrow())
            .copied()
            .unwrap();
        assert_eq!(graph.arrow_target(&arrow), Some(test_oid("40")));

        graph.set_show_long_edges(true);
        assert!(edge_count(&graph) > collapsed_edges);
    }

    #[test]
    fn selection_marks_only_the_matching_print_element() {
        let commits = feature_merge_history();
        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        let selected = graph.print_elements(1..2, None)[0][0];
        let rows = graph.print_elements(0..commits.len(), Some(&selected));
        let selected_elements: Vec<&PrintElement> = rows
            .iter()
            .flatten()
            .filter(|element| element.is_selected)
            .collect();
        assert_eq!(selected_elements.len(), 1);
        assert!(selected_elements[0].same_print_element(&selected));
    }

    #[test]
    fn rewritten_parents_are_drawn_as_dashed_edges() {
        let commits = [
            with_dotted_parents(commit("tip", &["middle"], 3, &["main"]), &["middle"]),
            commit("middle", &["root"], 2, &[]),
            commit("root", &[], 1, &[]),
        ];
        let edge_styles = |graph: &CommitGraph, row: usize| -> Vec<(EdgeDirection, LineStyle)> {
            graph.print_elements(row..row + 1, None)[0]
                .iter()
                .filter_map(|element| match element.kind {
                    PrintElementKind::Edge(edge) => Some((edge.direction, edge.line_style)),
                    PrintElementKind::Node(_) => None,
                })
                .collect()
        };

        let graph = CommitGraph::new(&commits, GraphOptions::default(), false, &remotes());
        assert_eq!(
            edge_styles(&graph, 0),
            vec![(EdgeDirection::Down, LineStyle::Dashed)]
        );
        assert_eq!(
            edge_styles(&graph, 1),
            vec![
                (EdgeDirection::Up, LineStyle::Dashed),
                (EdgeDirection::Down, LineStyle::Solid),
            ]
        );
        assert_eq!(
            edge_styles(&graph, 2),
            vec![(EdgeDirection::Up, LineStyle::Solid)]
        );
        assert_eq!(graph.adjacent_rows(0, true), vec![1]);
        assert_eq!(graph.adjacent_rows(1, false), vec![0]);

        for options in [
            GraphOptions::Base(GraphSortType::Bek),
            GraphOptions::LinearBek,
            GraphOptions::FirstParent,
        ] {
            let graph = CommitGraph::new(&commits, options, false, &remotes());
            assert_eq!(
                edge_styles(&graph, 0),
                vec![(EdgeDirection::Down, LineStyle::Dashed)],
                "{options:?}"
            );
        }
    }
}
