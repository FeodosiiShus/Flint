use std::{cmp::Ordering, ops::Range};

use collections::{HashMap, HashSet};

use super::{
    EdgeDirection, EdgePrintElement, LineStyle, NodeType, PrintElement, PrintElementKind,
    linear_graph::{EdgeFilter, GraphEdge, GraphEdgeType, LinearGraph, NormalEdge, stable_sort_by},
};

pub(crate) const LONG_EDGE_SIZE: u32 = 30;
const VERY_LONG_EDGE_SIZE: u32 = 1000;
const LONG_EDGE_PART_SIZE: u32 = 1;
const VERY_LONG_EDGE_PART_SIZE: u32 = 250;
const RECOMMENDED_WIDTH_SAMPLE_SIZE: usize = 20_000;
const RECOMMENDED_WIDTH_WEIGHT_RATIO: f64 = 0.1;
const EDGES_IN_ROW_CHECKPOINT_INTERVAL: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GraphElement {
    Node(u32),
    Edge(GraphEdge),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LongEdgeSizes {
    long_edge_size: u32,
    visible_part_size: u32,
    edge_with_arrow_size: u32,
}

impl LongEdgeSizes {
    pub(crate) fn new(show_long_edges: bool) -> Self {
        if show_long_edges {
            Self {
                long_edge_size: VERY_LONG_EDGE_SIZE,
                visible_part_size: VERY_LONG_EDGE_PART_SIZE,
                edge_with_arrow_size: LONG_EDGE_SIZE,
            }
        } else {
            Self {
                long_edge_size: LONG_EDGE_SIZE,
                visible_part_size: LONG_EDGE_PART_SIZE,
                edge_with_arrow_size: u32::MAX,
            }
        }
    }

    fn is_visible_in_row(&self, edge: NormalEdge, row: u32) -> bool {
        let attachment_distance =
            (i64::from(row) - i64::from(edge.up)).min(i64::from(edge.down) - i64::from(row));
        edge.down.saturating_sub(edge.up) < self.long_edge_size
            || attachment_distance <= i64::from(self.visible_part_size)
    }

    fn normal_arrow(&self, edge: NormalEdge, row: u32) -> Option<EdgeDirection> {
        let edge_size = edge.down.saturating_sub(edge.up);
        let up_offset = i64::from(row) - i64::from(edge.up);
        let down_offset = i64::from(edge.down) - i64::from(row);
        if edge_size >= self.long_edge_size {
            if up_offset == i64::from(self.visible_part_size) {
                return Some(EdgeDirection::Down);
            }
            if down_offset == i64::from(self.visible_part_size) {
                return Some(EdgeDirection::Up);
            }
        }
        if edge_size >= self.edge_with_arrow_size {
            if up_offset == 1 {
                return Some(EdgeDirection::Down);
            }
            if down_offset == 1 {
                return Some(EdgeDirection::Up);
            }
        }
        None
    }

    fn arrow(&self, edge: &GraphEdge, row: u32) -> Option<EdgeDirection> {
        match edge.as_normal() {
            Some(normal) => self.normal_arrow(normal, row),
            None => (edge.edge_type == GraphEdgeType::NotLoadCommit
                && edge.up.is_some_and(|up| up + 1 == row))
            .then_some(EdgeDirection::Down),
        }
    }
}

pub(crate) struct EdgesInRowIndex {
    checkpoints: Vec<Vec<GraphEdge>>,
}

struct EdgesInRowWalker<'a> {
    graph: &'a LinearGraph,
    row: usize,
    edges: HashSet<GraphEdge>,
}

impl EdgesInRowWalker<'_> {
    fn advance_to(&mut self, row: usize) {
        while self.row < row {
            let current = self.row as u32;
            self.edges
                .extend(self.graph.adjacent_edges(current, EdgeFilter::NormalDown));
            for edge in self.graph.adjacent_edges(current + 1, EdgeFilter::NormalUp) {
                self.edges.remove(&edge);
            }
            self.row += 1;
        }
    }
}

impl EdgesInRowIndex {
    pub(crate) fn build(graph: &LinearGraph) -> Self {
        let mut walker = EdgesInRowWalker {
            graph,
            row: 0,
            edges: HashSet::default(),
        };
        let checkpoints = (0..graph.nodes_count())
            .step_by(EDGES_IN_ROW_CHECKPOINT_INTERVAL)
            .map(|row| {
                walker.advance_to(row);
                let mut edges: Vec<GraphEdge> = walker.edges.iter().copied().collect();
                edges.sort_unstable();
                edges
            })
            .collect();
        Self { checkpoints }
    }

    fn walker<'a>(&self, graph: &'a LinearGraph, row: usize) -> EdgesInRowWalker<'a> {
        let checkpoint =
            (row / EDGES_IN_ROW_CHECKPOINT_INTERVAL).min(self.checkpoints.len().saturating_sub(1));
        let mut walker = EdgesInRowWalker {
            graph,
            row: checkpoint * EDGES_IN_ROW_CHECKPOINT_INTERVAL,
            edges: self
                .checkpoints
                .get(checkpoint)
                .map(|edges| edges.iter().copied().collect())
                .unwrap_or_default(),
        };
        walker.advance_to(row);
        walker
    }
}

pub(crate) struct PrintElementGenerator<'a> {
    graph: &'a LinearGraph,
    edges_in_row: &'a EdgesInRowIndex,
    sizes: LongEdgeSizes,
    layout_index: &'a dyn Fn(u32) -> i32,
}

impl<'a> PrintElementGenerator<'a> {
    pub(crate) fn new(
        graph: &'a LinearGraph,
        edges_in_row: &'a EdgesInRowIndex,
        sizes: LongEdgeSizes,
        layout_index: &'a dyn Fn(u32) -> i32,
    ) -> Self {
        Self {
            graph,
            edges_in_row,
            sizes,
            layout_index,
        }
    }

    pub(crate) fn print_elements(&self, rows: Range<usize>) -> Vec<Vec<PrintElement>> {
        let row_count = self.graph.nodes_count();
        let rows = rows.start.min(row_count)..rows.end.min(row_count);
        if rows.is_empty() {
            return Vec::new();
        }
        let first_sorted_row = rows.start.saturating_sub(1);
        let sorted_rows_end = (rows.end + 1).min(row_count);
        let mut walker = self.edges_in_row.walker(self.graph, first_sorted_row);
        let mut sorted_rows = Vec::with_capacity(sorted_rows_end - first_sorted_row);
        for row in first_sorted_row..sorted_rows_end {
            walker.advance_to(row);
            sorted_rows.push(self.sorted_visible_elements(row as u32, &walker.edges));
        }
        let sorted_row = |row: Option<usize>| {
            row.and_then(|row| row.checked_sub(first_sorted_row))
                .and_then(|index| sorted_rows.get(index))
                .map(Vec::as_slice)
        };
        rows.map(|row| {
            self.row_print_elements(
                row as u32,
                sorted_row(Some(row)).unwrap_or_default(),
                sorted_row(row.checked_sub(1)),
                sorted_row(Some(row + 1)),
            )
        })
        .collect()
    }

    fn sorted_visible_elements(
        &self,
        row: u32,
        edges_in_row: &HashSet<GraphEdge>,
    ) -> Vec<GraphElement> {
        let mut visible_edges: Vec<GraphEdge> = edges_in_row
            .iter()
            .filter(|edge| {
                edge.as_normal()
                    .is_some_and(|normal| self.sizes.is_visible_in_row(normal, row))
            })
            .copied()
            .collect();
        visible_edges.sort_unstable();

        let mut elements = Vec::with_capacity(visible_edges.len() + 1);
        elements.push(GraphElement::Node(row));
        elements.extend(visible_edges.into_iter().map(GraphElement::Edge));
        if let Some(previous) = row.checked_sub(1) {
            elements.extend(
                self.graph
                    .adjacent_edges(previous, EdgeFilter::Special)
                    .filter(|edge| edge.is_down_edge_of(previous))
                    .map(GraphElement::Edge),
            );
        }
        stable_sort_by(&mut elements, |left, right| {
            self.compare_elements(left, right)
        });
        elements
    }

    fn row_print_elements(
        &self,
        row: u32,
        current: &[GraphElement],
        above: Option<&[GraphElement]>,
        below: Option<&[GraphElement]>,
    ) -> Vec<PrintElement> {
        let above_positions = element_positions(above);
        let below_positions = element_positions(below);
        let up_position = |edge: &GraphEdge| end_position(&above_positions, edge, edge.up);
        let down_position = |edge: &GraphEdge| end_position(&below_positions, edge, edge.down);
        let edge_element = |position: usize,
                            other_position: usize,
                            direction: EdgeDirection,
                            edge: &GraphEdge,
                            has_arrow: bool,
                            is_terminal: bool| PrintElement {
            row: row as usize,
            position,
            kind: PrintElementKind::Edge(EdgePrintElement {
                other_position,
                direction,
                line_style: line_style(edge.edge_type),
                has_arrow,
                is_terminal,
            }),
            color_id: 0,
            is_selected: false,
            graph_element: GraphElement::Edge(*edge),
        };

        let mut elements = Vec::new();
        let mut nodes = Vec::new();
        for (position, element) in current.iter().enumerate() {
            match element {
                GraphElement::Node(node) => {
                    nodes.push(PrintElement {
                        row: row as usize,
                        position,
                        kind: PrintElementKind::Node(NodeType::Fill),
                        color_id: 0,
                        is_selected: false,
                        graph_element: *element,
                    });
                    for edge in self.graph.edges(*node) {
                        let arrow = self.sizes.arrow(edge, row);
                        if let Some(down) = down_position(edge) {
                            elements.push(edge_element(
                                position,
                                down,
                                EdgeDirection::Down,
                                edge,
                                arrow == Some(EdgeDirection::Down),
                                false,
                            ));
                        }
                        if let Some(up) = up_position(edge) {
                            elements.push(edge_element(
                                position,
                                up,
                                EdgeDirection::Up,
                                edge,
                                arrow == Some(EdgeDirection::Up),
                                false,
                            ));
                        }
                    }
                }
                GraphElement::Edge(edge) => {
                    let arrow = self.sizes.arrow(edge, row);
                    for direction in [EdgeDirection::Down, EdgeDirection::Up] {
                        let other_position = match direction {
                            EdgeDirection::Down => down_position(edge),
                            EdgeDirection::Up => up_position(edge),
                        };
                        let has_arrow = arrow == Some(direction);
                        match other_position {
                            Some(other_position) => elements.push(edge_element(
                                position,
                                other_position,
                                direction,
                                edge,
                                has_arrow,
                                false,
                            )),
                            None if has_arrow => elements.push(edge_element(
                                position, position, direction, edge, true, true,
                            )),
                            None => {}
                        }
                    }
                }
            }
        }
        elements.extend(nodes);
        elements
    }

    fn layout_index(&self, node: u32) -> i64 {
        i64::from((self.layout_index)(node))
    }

    fn compare_edge_with_node(&self, edge: &GraphEdge, node: u32) -> i64 {
        let node_layout = self.layout_index(node);
        match edge.as_normal() {
            None => self.layout_index(edge.not_null_node()) - node_layout,
            Some(normal) => {
                let edge_layout = self
                    .layout_index(normal.up)
                    .max(self.layout_index(normal.down));
                if edge_layout != node_layout {
                    edge_layout - node_layout
                } else {
                    i64::from(normal.up) - i64::from(node)
                }
            }
        }
    }

    fn compare_elements(&self, left: &GraphElement, right: &GraphElement) -> Ordering {
        let difference = match (left, right) {
            (GraphElement::Edge(left), GraphElement::Edge(right)) => {
                match (left.as_normal(), right.as_normal()) {
                    (None, _) => -self.compare_edge_with_node(right, left.not_null_node()),
                    (_, None) => self.compare_edge_with_node(left, right.not_null_node()),
                    (Some(left_normal), Some(right_normal)) => {
                        match left_normal.up.cmp(&right_normal.up) {
                            Ordering::Equal if left_normal.down < right_normal.down => {
                                -self.compare_edge_with_node(right, left_normal.down)
                            }
                            Ordering::Equal => self.compare_edge_with_node(left, right_normal.down),
                            Ordering::Less => self.compare_edge_with_node(left, right_normal.up),
                            Ordering::Greater => {
                                -self.compare_edge_with_node(right, left_normal.up)
                            }
                        }
                    }
                }
            }
            (GraphElement::Edge(edge), GraphElement::Node(node)) => {
                self.compare_edge_with_node(edge, *node)
            }
            (GraphElement::Node(node), GraphElement::Edge(edge)) => {
                -self.compare_edge_with_node(edge, *node)
            }
            (GraphElement::Node(_), GraphElement::Node(_)) => 0,
        };
        difference.cmp(&0)
    }
}

fn element_positions(elements: Option<&[GraphElement]>) -> HashMap<GraphElement, usize> {
    elements
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(position, element)| (*element, position))
        .collect()
}

fn end_position(
    positions: &HashMap<GraphElement, usize>,
    edge: &GraphEdge,
    node: Option<u32>,
) -> Option<usize> {
    positions
        .get(&GraphElement::Edge(*edge))
        .or_else(|| node.and_then(|node| positions.get(&GraphElement::Node(node))))
        .copied()
}

fn line_style(edge_type: GraphEdgeType) -> LineStyle {
    match edge_type {
        GraphEdgeType::Usual | GraphEdgeType::NotLoadCommit => LineStyle::Solid,
        GraphEdgeType::Dotted => LineStyle::Dashed,
    }
}

pub(crate) fn recommended_width(graph: &LinearGraph, sizes: LongEdgeSizes) -> usize {
    let node_count = graph.nodes_count();
    match node_count {
        0 => return 0,
        1 => return 1,
        _ => {}
    }
    let sample_size = node_count.min(RECOMMENDED_WIDTH_SAMPLE_SIZE);
    let mut sum = 0.0;
    let mut sum_of_squares = 0.0;
    let mut edges_count = 0usize;
    let mut current_edges: HashSet<NormalEdge> = HashSet::default();
    for row in 0..sample_size as u32 {
        let mut up_arrows = 0usize;
        let mut down_arrows = 0usize;
        for edge in graph.edges(row) {
            match edge.as_normal() {
                Some(normal) if edge.is_up_edge_of(row) => {
                    current_edges.remove(&normal);
                }
                Some(normal) => {
                    current_edges.insert(normal);
                }
                None => down_arrows += 1,
            }
        }
        let mut new_edges_count = 0usize;
        for edge in &current_edges {
            if sizes.is_visible_in_row(*edge, row) {
                new_edges_count += 1;
            } else {
                match sizes.normal_arrow(*edge, row) {
                    Some(EdgeDirection::Down) => down_arrows += 1,
                    Some(EdgeDirection::Up) => up_arrows += 1,
                    None => {}
                }
            }
        }
        let width = (edges_count + up_arrows).max(new_edges_count + down_arrows) as f64;
        let weight = 2.0 / (sample_size as f64 * (RECOMMENDED_WIDTH_WEIGHT_RATIO + 1.0))
            * (1.0
                + (RECOMMENDED_WIDTH_WEIGHT_RATIO - 1.0) * f64::from(row)
                    / (sample_size - 1) as f64);
        sum += width * weight;
        sum_of_squares += width * width * weight;
        edges_count = new_edges_count;
    }
    let average = sum;
    let deviation = (sum_of_squares - average * average).max(0.0).sqrt();
    (average + deviation + 0.5).floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    impl LongEdgeSizes {
        fn custom(long_edge_size: u32, visible_part_size: u32, edge_with_arrow_size: u32) -> Self {
            Self {
                long_edge_size,
                visible_part_size,
                edge_with_arrow_size,
            }
        }
    }

    fn parse_linear_graph(input: &str) -> LinearGraph {
        let lines: Vec<&str> = input
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let node_ids: Vec<i32> = lines
            .iter()
            .map(|line| {
                let (node, _) = line.split_once("|-").unwrap();
                node[..node.len() - 2].parse().unwrap()
            })
            .collect();
        let index_of = |id: i32| node_ids.iter().position(|node| *node == id).unwrap() as u32;
        let mut up_edges: Vec<Vec<GraphEdge>> = vec![Vec::new(); lines.len()];
        let mut down_edges: Vec<Vec<GraphEdge>> = vec![Vec::new(); lines.len()];
        for (node, line) in lines.iter().enumerate() {
            let (_, edges) = line.split_once("|-").unwrap();
            for edge in edges.split_whitespace() {
                let (target, edge_type) = edge.split_at(edge.len() - 2);
                let target: i32 = target.parse().unwrap();
                let edge = match &edge_type[1..] {
                    "U" => GraphEdge::normal(node as u32, index_of(target), GraphEdgeType::Usual),
                    "D" => GraphEdge::normal(node as u32, index_of(target), GraphEdgeType::Dotted),
                    "N" => GraphEdge::not_loaded(node as u32, target),
                    other => panic!("unsupported edge type {other}"),
                };
                if let Some(up) = edge.up {
                    down_edges[up as usize].push(edge);
                }
                if let Some(down) = edge.down {
                    up_edges[down as usize].push(edge);
                }
            }
        }
        let mut graph = LinearGraph::with_capacity(lines.len(), 0);
        for (node, (up, down)) in up_edges.into_iter().zip(down_edges).enumerate() {
            graph.push_node(node as u32, up.into_iter().chain(down));
        }
        graph
    }

    fn layout_indexes(graph: &LinearGraph) -> Vec<i32> {
        let layout =
            crate::git_log::graph::linear_graph::GraphLayout::build(graph, &[], |left, right| {
                left.cmp(&right)
            });
        (0..graph.nodes_count() as u32)
            .map(|node| layout.layout_index(node))
            .collect()
    }

    fn test_color(element: &GraphElement) -> i64 {
        match element {
            GraphElement::Node(node) => i64::from(*node),
            GraphElement::Edge(edge) => match edge.as_normal() {
                Some(normal) => i64::from(normal.up) + i64::from(normal.down),
                None => i64::from(edge.not_null_node()),
            },
        }
    }

    fn element_string(element: &PrintElement) -> String {
        let graph_element = match element.graph_element {
            GraphElement::Node(node) => format!("{node}_U"),
            GraphElement::Edge(edge) => {
                crate::git_log::graph::linear_graph::tests::edge_string(&edge)
            }
        };
        let color = test_color(&element.graph_element);
        match element.kind {
            PrintElementKind::Node(_) => format!(
                "Node|-{}:{}|-{color}:Unselect({graph_element})",
                element.row, element.position
            ),
            PrintElementKind::Edge(edge) => {
                let direction = match edge.direction {
                    EdgeDirection::Up => "UP",
                    EdgeDirection::Down => "DOWN",
                };
                let arrow = if edge.has_arrow { "_ARROW" } else { "" };
                let style = match edge.line_style {
                    LineStyle::Solid => "SOLID",
                    LineStyle::Dashed => "DASHED",
                };
                format!(
                    "Edge:{direction}{arrow}:{style}|-{}:{}:{}|-{color}:Unselect({graph_element})",
                    element.row, element.position, edge.other_position
                )
            }
        }
    }

    fn print_elements_string(input: &str, sizes: LongEdgeSizes) -> String {
        let graph = parse_linear_graph(input);
        let layout = layout_indexes(&graph);
        let layout_index = |node: u32| layout[node as usize];
        let edges_in_row = EdgesInRowIndex::build(&graph);
        let generator = PrintElementGenerator::new(&graph, &edges_in_row, sizes, &layout_index);
        generator
            .print_elements(0..graph.nodes_count())
            .into_iter()
            .map(|mut elements| {
                let sort_key = |element: &PrintElement| match element.kind {
                    PrintElementKind::Node(_) => 1024 * element.position,
                    PrintElementKind::Edge(edge) => {
                        let direction = match edge.direction {
                            EdgeDirection::Up => 1,
                            EdgeDirection::Down => 2,
                        };
                        1024 * element.position + direction * 64 + edge.other_position
                    }
                };
                elements.sort_by_key(sort_key);
                elements
                    .iter()
                    .map(element_string)
                    .collect::<Vec<_>>()
                    .join("\n  ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn edges_in_row_string(input: &str) -> String {
        let graph = parse_linear_graph(input);
        let index = EdgesInRowIndex::build(&graph);
        let layout_index = |_: u32| 0;
        let generator =
            PrintElementGenerator::new(&graph, &index, LongEdgeSizes::new(false), &layout_index);
        (0..graph.nodes_count())
            .map(|row| {
                let walker = index.walker(&graph, row);
                let mut edges: Vec<GraphElement> = walker
                    .edges
                    .iter()
                    .copied()
                    .map(GraphElement::Edge)
                    .collect();
                edges.sort_unstable_by_key(|element| match element {
                    GraphElement::Edge(edge) => (edge.up, edge.down),
                    GraphElement::Node(node) => (Some(*node), None),
                });
                stable_sort_by(&mut edges, |left, right| {
                    generator.compare_elements(left, right)
                });
                if edges.is_empty() {
                    return "none".to_string();
                }
                edges
                    .iter()
                    .filter_map(|element| match element {
                        GraphElement::Edge(edge) => {
                            let type_char = if edge.edge_type == GraphEdgeType::Dotted {
                                'D'
                            } else {
                                'U'
                            };
                            Some(format!(
                                "{}_{}_{type_char}",
                                edge.up.unwrap_or_default(),
                                edge.down.unwrap_or_default()
                            ))
                        }
                        GraphElement::Node(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn edges_in_row_tracks_edges_passing_each_row() {
        assert_eq!(
            edges_in_row_string("0_U|-1_U 3_D\n1_U|-2_U\n2_U|-3_U\n3_U|-"),
            "none\n0_3_D\n0_3_D\nnone"
        );
        assert_eq!(
            edges_in_row_string(
                "0_U|-1_U 6_U 3_U\n1_U|-6_D\n2_U|-3_U\n3_U|-\n4_U|-5_U\n5_U|-\n6_U|-7_U\n7_U|-"
            ),
            "none\n0_6_U 0_3_U\n0_6_U 0_3_U 1_6_D\n0_6_U 1_6_D\n0_6_U 1_6_D\n0_6_U 1_6_D\nnone\nnone"
        );
        assert_eq!(
            edges_in_row_string("0_U|-6_N 2_U\n1_U|-5_N\n2_U|-"),
            "none\n0_2_U\nnone"
        );
    }

    #[test]
    fn edges_in_row_crosses_checkpoints() {
        let mut input = String::from("0_U|-1_U 2500_U\n");
        for node in 1..2500 {
            input.push_str(&format!("{node}_U|-{}_U\n", node + 1));
        }
        input.push_str("2500_U|-\n");
        let graph = parse_linear_graph(&input);
        let index = EdgesInRowIndex::build(&graph);
        for row in [1, 999, 1000, 1001, 2000, 2499] {
            let walker = index.walker(&graph, row);
            let edges: Vec<GraphEdge> = walker.edges.iter().copied().collect();
            assert_eq!(
                edges,
                vec![GraphEdge::normal(0, 2500, GraphEdgeType::Usual)],
                "row {row}"
            );
        }
    }

    #[test]
    fn print_elements_match_intellij_many_nodes() {
        let expected = "\
Node|-0:0|-0:Unselect(0_U)
  Edge:DOWN:SOLID|-0:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-0:0:1|-1:Unselect(0:1:n_U)
  Edge:DOWN:SOLID|-0:0:2|-3:Unselect(0:3:n_U)
Edge:UP:SOLID|-1:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-1:0:0|-6:Unselect(0:6:n_U)
  Node|-1:1|-1:Unselect(1_U)
  Edge:UP:SOLID|-1:1:0|-1:Unselect(0:1:n_U)
  Edge:DOWN:DASHED|-1:1:1|-7:Unselect(1:6:n_D)
  Edge:UP:SOLID|-1:2:0|-3:Unselect(0:3:n_U)
  Edge:DOWN:SOLID|-1:2:2|-3:Unselect(0:3:n_U)
Edge:UP:SOLID|-2:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-2:0:0|-6:Unselect(0:6:n_U)
  Edge:UP:DASHED|-2:1:1|-7:Unselect(1:6:n_D)
  Edge:DOWN:DASHED|-2:1:1|-7:Unselect(1:6:n_D)
  Edge:UP:SOLID|-2:2:2|-3:Unselect(0:3:n_U)
  Edge:DOWN:SOLID|-2:2:2|-3:Unselect(0:3:n_U)
  Node|-2:3|-2:Unselect(2_U)
  Edge:DOWN:SOLID|-2:3:2|-5:Unselect(2:3:n_U)
Edge:UP:SOLID|-3:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-3:0:0|-6:Unselect(0:6:n_U)
  Edge:UP:DASHED|-3:1:1|-7:Unselect(1:6:n_D)
  Edge:DOWN:DASHED|-3:1:1|-7:Unselect(1:6:n_D)
  Node|-3:2|-3:Unselect(3_U)
  Edge:UP:SOLID|-3:2:2|-3:Unselect(0:3:n_U)
  Edge:UP:SOLID|-3:2:3|-5:Unselect(2:3:n_U)
Edge:UP:SOLID|-4:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-4:0:0|-6:Unselect(0:6:n_U)
  Edge:UP:DASHED|-4:1:1|-7:Unselect(1:6:n_D)
  Edge:DOWN:DASHED|-4:1:1|-7:Unselect(1:6:n_D)
  Node|-4:2|-4:Unselect(4_U)
  Edge:DOWN:SOLID|-4:2:2|-9:Unselect(4:5:n_U)
Edge:UP:SOLID|-5:0:0|-6:Unselect(0:6:n_U)
  Edge:DOWN:SOLID|-5:0:0|-6:Unselect(0:6:n_U)
  Edge:UP:DASHED|-5:1:1|-7:Unselect(1:6:n_D)
  Edge:DOWN:DASHED|-5:1:0|-7:Unselect(1:6:n_D)
  Node|-5:2|-5:Unselect(5_U)
  Edge:UP:SOLID|-5:2:2|-9:Unselect(4:5:n_U)
Node|-6:0|-6:Unselect(6_U)
  Edge:UP:SOLID|-6:0:0|-6:Unselect(0:6:n_U)
  Edge:UP:DASHED|-6:0:1|-7:Unselect(1:6:n_D)
  Edge:DOWN:SOLID|-6:0:0|-13:Unselect(6:7:n_U)
Node|-7:0|-7:Unselect(7_U)
  Edge:UP:SOLID|-7:0:0|-13:Unselect(6:7:n_U)";
        assert_eq!(
            print_elements_string(
                "0_U|-1_U 6_U 3_U\n1_U|-6_D\n2_U|-3_U\n3_U|-\n4_U|-5_U\n5_U|-\n6_U|-7_U\n7_U|-",
                LongEdgeSizes::custom(7, 2, 10),
            ),
            expected
        );
    }

    #[test]
    fn print_elements_hide_long_edges_behind_arrows() {
        let expected = "\
Node|-0:0|-0:Unselect(0_U)
  Edge:DOWN:SOLID|-0:0:0|-1:Unselect(0:1:n_U)
  Edge:DOWN:SOLID|-0:0:1|-7:Unselect(0:7:n_U)
Node|-1:0|-1:Unselect(1_U)
  Edge:UP:SOLID|-1:0:0|-1:Unselect(0:1:n_U)
  Edge:DOWN:SOLID|-1:0:0|-3:Unselect(1:2:n_U)
  Edge:UP:SOLID|-1:1:0|-7:Unselect(0:7:n_U)
  Edge:DOWN_ARROW:SOLID|-1:1:1|-7:Unselect(0:7:n_U)
Node|-2:0|-2:Unselect(2_U)
  Edge:UP:SOLID|-2:0:0|-3:Unselect(1:2:n_U)
  Edge:DOWN:SOLID|-2:0:0|-5:Unselect(2:3:n_U)
Node|-3:0|-3:Unselect(3_U)
  Edge:UP:SOLID|-3:0:0|-5:Unselect(2:3:n_U)
  Edge:DOWN:SOLID|-3:0:0|-15:Unselect(3:12:n_U)
  Edge:DOWN:SOLID|-3:0:1|-7:Unselect(3:4:n_U)
Edge:UP:SOLID|-4:0:0|-15:Unselect(3:12:n_U)
  Edge:DOWN_ARROW:SOLID|-4:0:0|-15:Unselect(3:12:n_U)
  Node|-4:1|-4:Unselect(4_U)
  Edge:UP:SOLID|-4:1:0|-7:Unselect(3:4:n_U)
  Edge:DOWN:SOLID|-4:1:0|-9:Unselect(4:5:n_U)
  Edge:DOWN:SOLID|-4:1:1|-10:Unselect(4:6:n_U)
Node|-5:0|-5:Unselect(5_U)
  Edge:UP:SOLID|-5:0:1|-9:Unselect(4:5:n_U)
  Edge:DOWN:SOLID|-5:0:0|-13:Unselect(5:8:n_U)
  Edge:UP:SOLID|-5:1:1|-10:Unselect(4:6:n_U)
  Edge:DOWN:SOLID|-5:1:1|-10:Unselect(4:6:n_U)
Edge:UP:SOLID|-6:0:0|-13:Unselect(5:8:n_U)
  Edge:DOWN:SOLID|-6:0:0|-13:Unselect(5:8:n_U)
  Node|-6:1|-6:Unselect(6_U)
  Edge:UP:SOLID|-6:1:1|-10:Unselect(4:6:n_U)
  Edge:DOWN:SOLID|-6:1:1|-14:Unselect(6:8:n_U)
  Edge:UP_ARROW:SOLID|-6:2:2|-7:Unselect(0:7:n_U)
  Edge:DOWN:SOLID|-6:2:2|-7:Unselect(0:7:n_U)
Edge:UP:SOLID|-7:0:0|-13:Unselect(5:8:n_U)
  Edge:DOWN:SOLID|-7:0:0|-13:Unselect(5:8:n_U)
  Edge:UP:SOLID|-7:1:1|-14:Unselect(6:8:n_U)
  Edge:DOWN:SOLID|-7:1:0|-14:Unselect(6:8:n_U)
  Node|-7:2|-7:Unselect(7_U)
  Edge:UP:SOLID|-7:2:2|-7:Unselect(0:7:n_U)
  Edge:DOWN:SOLID|-7:2:1|-22:Unselect(7:15:n_U)
Node|-8:0|-8:Unselect(8_U)
  Edge:UP:SOLID|-8:0:0|-13:Unselect(5:8:n_U)
  Edge:UP:SOLID|-8:0:1|-14:Unselect(6:8:n_U)
  Edge:DOWN:SOLID|-8:0:0|-17:Unselect(8:9:n_U)
  Edge:DOWN:SOLID|-8:0:1|-18:Unselect(8:10:n_U)
  Edge:UP:SOLID|-8:1:2|-22:Unselect(7:15:n_U)
  Edge:DOWN_ARROW:SOLID|-8:1:1|-22:Unselect(7:15:n_U)
Node|-9:0|-9:Unselect(9_U)
  Edge:UP:SOLID|-9:0:0|-17:Unselect(8:9:n_U)
  Edge:DOWN:SOLID|-9:0:0|-20:Unselect(9:11:n_U)
  Edge:UP:SOLID|-9:1:0|-18:Unselect(8:10:n_U)
  Edge:DOWN:SOLID|-9:1:1|-18:Unselect(8:10:n_U)
Edge:UP:SOLID|-10:0:0|-20:Unselect(9:11:n_U)
  Edge:DOWN:SOLID|-10:0:1|-20:Unselect(9:11:n_U)
  Node|-10:1|-10:Unselect(10_U)
  Edge:UP:SOLID|-10:1:1|-18:Unselect(8:10:n_U)
  Edge:DOWN:SOLID|-10:1:1|-21:Unselect(10:11:n_U)
Edge:UP_ARROW:SOLID|-11:0:0|-15:Unselect(3:12:n_U)
  Edge:DOWN:SOLID|-11:0:0|-15:Unselect(3:12:n_U)
  Node|-11:1|-11:Unselect(11_U)
  Edge:UP:SOLID|-11:1:0|-20:Unselect(9:11:n_U)
  Edge:UP:SOLID|-11:1:1|-21:Unselect(10:11:n_U)
  Edge:DOWN:SOLID|-11:1:0|-23:Unselect(11:12:n_U)
Node|-12:0|-12:Unselect(12_U)
  Edge:UP:SOLID|-12:0:0|-15:Unselect(3:12:n_U)
  Edge:UP:SOLID|-12:0:1|-23:Unselect(11:12:n_U)
  Edge:DOWN:SOLID|-12:0:0|-25:Unselect(12:13:n_U)
Node|-13:0|-13:Unselect(13_U)
  Edge:UP:SOLID|-13:0:0|-25:Unselect(12:13:n_U)
  Edge:DOWN:SOLID|-13:0:0|-27:Unselect(13:14:n_U)
Node|-14:0|-14:Unselect(14_U)
  Edge:UP:SOLID|-14:0:0|-27:Unselect(13:14:n_U)
  Edge:DOWN:SOLID|-14:0:0|-29:Unselect(14:15:n_U)
  Edge:UP_ARROW:SOLID|-14:1:1|-22:Unselect(7:15:n_U)
  Edge:DOWN:SOLID|-14:1:0|-22:Unselect(7:15:n_U)
Node|-15:0|-15:Unselect(15_U)
  Edge:UP:SOLID|-15:0:0|-29:Unselect(14:15:n_U)
  Edge:UP:SOLID|-15:0:1|-22:Unselect(7:15:n_U)";
        assert_eq!(
            print_elements_string(
                "0_U|-1_U 7_U\n1_U|-2_U\n2_U|-3_U\n3_U|-4_U 12_U\n4_U|-5_U 6_U\n5_U|-8_U\n\
                 6_U|-8_U\n7_U|-15_U\n8_U|-9_U 10_U\n9_U|-11_U\n10_U|-11_U\n11_U|-12_U\n\
                 12_U|-13_U\n13_U|-14_U\n14_U|-15_U\n15_U|-",
                LongEdgeSizes::custom(7, 1, 10),
            ),
            expected
        );
    }

    #[test]
    fn print_elements_end_not_loaded_parents_with_a_terminal_arrow() {
        let graph = parse_linear_graph("0_U|-5_N\n1_U|-");
        let layout_index = |_: u32| 0;
        let edges_in_row = EdgesInRowIndex::build(&graph);
        let generator = PrintElementGenerator::new(
            &graph,
            &edges_in_row,
            LongEdgeSizes::new(false),
            &layout_index,
        );
        let rows = generator.print_elements(0..2);
        let terminal: Vec<&PrintElement> = rows[1]
            .iter()
            .filter(
                |element| matches!(element.kind, PrintElementKind::Edge(edge) if edge.is_terminal),
            )
            .collect();
        assert_eq!(terminal.len(), 1);
        let PrintElementKind::Edge(edge) = terminal[0].kind else {
            unreachable!()
        };
        assert_eq!(edge.direction, EdgeDirection::Down);
        assert!(edge.has_arrow);
        assert_eq!(edge.other_position, terminal[0].position);
        assert!(rows[0].iter().any(|element| matches!(
            element.kind,
            PrintElementKind::Edge(edge) if edge.direction == EdgeDirection::Down && !edge.has_arrow
        )));
    }

    #[test]
    fn long_dotted_edges_collapse_into_dashed_terminal_arrows() {
        let mut input = String::from("0_U|-1_U 40_D\n");
        for node in 1..40 {
            input.push_str(&format!("{node}_U|-{}_U\n", node + 1));
        }
        input.push_str("40_U|-\n");
        let graph = parse_linear_graph(&input);
        let layout_index = |_: u32| 0;
        let edges_in_row = EdgesInRowIndex::build(&graph);
        let generator = PrintElementGenerator::new(
            &graph,
            &edges_in_row,
            LongEdgeSizes::new(false),
            &layout_index,
        );
        let rows = generator.print_elements(0..graph.nodes_count());
        let dashed_edges = |row: usize| -> Vec<EdgePrintElement> {
            rows[row]
                .iter()
                .filter_map(|element| match element.kind {
                    PrintElementKind::Edge(edge) if edge.line_style == LineStyle::Dashed => {
                        Some(edge)
                    }
                    _ => None,
                })
                .collect()
        };
        let dashed = |direction: EdgeDirection, has_arrow: bool| EdgePrintElement {
            other_position: 0,
            direction,
            line_style: LineStyle::Dashed,
            has_arrow,
            is_terminal: has_arrow,
        };

        assert_eq!(dashed_edges(0), vec![dashed(EdgeDirection::Down, false)]);
        assert_eq!(
            dashed_edges(1),
            vec![
                dashed(EdgeDirection::Down, true),
                dashed(EdgeDirection::Up, false),
            ]
        );
        assert!((2..39).all(|row| dashed_edges(row).is_empty()));
        assert_eq!(
            dashed_edges(39),
            vec![
                dashed(EdgeDirection::Down, false),
                dashed(EdgeDirection::Up, true),
            ]
        );
        assert_eq!(dashed_edges(40), vec![dashed(EdgeDirection::Up, false)]);
    }

    #[test]
    fn recommended_width_grows_with_parallel_branches() {
        let single = parse_linear_graph("0_U|-1_U\n1_U|-2_U\n2_U|-");
        assert_eq!(recommended_width(&single, LongEdgeSizes::new(false)), 1);
        let empty = LinearGraph::default();
        assert_eq!(recommended_width(&empty, LongEdgeSizes::new(false)), 0);
        let parallel = parse_linear_graph(
            "0_U|-4_U\n1_U|-5_U\n2_U|-6_U\n3_U|-7_U\n4_U|-8_U\n5_U|-8_U\n6_U|-8_U\n7_U|-8_U\n8_U|-",
        );
        assert!(recommended_width(&parallel, LongEdgeSizes::new(false)) >= 4);
    }
}
