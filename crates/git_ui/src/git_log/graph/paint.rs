use gpui::{Bounds, ContentMask, Hsla, PathBuilder, Pixels, Point, Window, point, px};
use smallvec::SmallVec;
use util::ResultExt;

use super::{
    EdgeDirection, EdgePrintElement, LineStyle, NodeType, PrintElement, PrintElementKind,
    color::GraphPalette,
};

const WIDTH_NODE: f64 = 16.0;
const CIRCLE_RADIUS: f64 = 4.0;
const THICK_LINE: f64 = 1.5;
const SELECT_THICK_LINE: f64 = 2.5;
const GRAPH_TEXT_GAP: f64 = 2.0;
const REFERENCE_ROW_HEIGHT: f64 = 22.0;
const HEAD_NODE_RADIUS_DELTA: f64 = 2.0;
const ARROW_ANGLE_COS2: f64 = 0.7;
const ARROW_LENGTH: f64 = 0.3;
const MAX_GRAPH_WIDTH: f64 = 6.0;
const TERMINAL_ARROW_GAP_PADDING: f64 = 1.0;
const DASH_SPACE_REDUCTION: f64 = 2.0;

fn scale_with_row_height(value: f64, row_height: f64) -> f64 {
    value * row_height / REFERENCE_ROW_HEIGHT
}

fn align_floor(value: f64, scale: f64) -> f64 {
    if scale <= 0.0 {
        return 0.0;
    }
    (value * scale).floor() / scale
}

fn align_floor_odd(value: f64, scale: f64) -> f64 {
    if scale <= 0.0 {
        return 0.0;
    }
    let mut device_value = (value * scale).floor() as i64;
    if device_value % 2 == 0 {
        device_value -= 1;
    }
    device_value as f64 / scale
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GraphGeometry {
    row_height: f64,
    row_center: f64,
    element_width: f64,
    element_center: f64,
    line_thickness: f64,
    selected_line_thickness: f64,
    circle_diameter: f64,
    circle_radius: f64,
    selected_circle_diameter: f64,
    selected_circle_radius: f64,
    head_outer_diameter: f64,
    head_outer_radius: f64,
    head_selected_outer_diameter: f64,
    head_selected_outer_radius: f64,
    head_ring_delta: f64,
}

impl GraphGeometry {
    fn new(row_height: f64, scale: f64) -> Self {
        let pixel = if scale > 0.0 { 1.0 / scale } else { 0.0 };
        let raw_element_width = scale_with_row_height(WIDTH_NODE, row_height);
        let line_thickness =
            align_floor_odd(scale_with_row_height(THICK_LINE, row_height), scale).max(pixel);
        let selected_line_thickness =
            align_floor_odd(scale_with_row_height(SELECT_THICK_LINE, row_height), scale)
                .max(line_thickness + 2.0 * pixel);
        let selected_extra = selected_line_thickness - line_thickness;
        let circle_diameter = align_floor_odd(
            2.0 * scale_with_row_height(CIRCLE_RADIUS, row_height),
            scale,
        );
        let selected_circle_diameter = circle_diameter + selected_extra;
        let head_outer_diameter = align_floor_odd(
            2.0 * scale_with_row_height(CIRCLE_RADIUS + HEAD_NODE_RADIUS_DELTA, row_height),
            scale,
        );
        let head_selected_outer_diameter = head_outer_diameter + selected_extra;
        Self {
            row_height,
            row_center: align_floor_odd(row_height / 2.0, scale),
            element_width: align_floor_odd(raw_element_width, scale),
            element_center: align_floor(raw_element_width / 2.0, scale),
            line_thickness,
            selected_line_thickness,
            circle_diameter,
            circle_radius: align_floor(circle_diameter / 2.0, scale),
            selected_circle_diameter,
            selected_circle_radius: align_floor(selected_circle_diameter / 2.0, scale),
            head_outer_diameter,
            head_outer_radius: align_floor(head_outer_diameter / 2.0, scale),
            head_selected_outer_diameter,
            head_selected_outer_radius: align_floor(head_selected_outer_diameter / 2.0, scale),
            head_ring_delta: align_floor(
                scale_with_row_height(HEAD_NODE_RADIUS_DELTA, row_height),
                scale,
            ),
        }
    }

    fn element_x(&self, position: usize) -> f64 {
        self.element_width * position as f64 + self.element_center
    }

    fn edge_end_y(&self, direction: EdgeDirection) -> f64 {
        match direction {
            EdgeDirection::Down => self.row_height + self.row_center,
            EdgeDirection::Up => self.row_center - self.row_height,
        }
    }

    fn is_over_node(&self, position: usize, x: f64, y: f64) -> bool {
        (self.element_x(position) - x).hypot(self.row_center - y) <= self.circle_radius
    }

    fn is_over_edge(&self, position: usize, edge: &EdgePrintElement, x: f64, y: f64) -> bool {
        let x1 = self.element_x(position);
        let y1 = self.row_center;
        let x2 = self.element_x(edge.other_position);
        let y2 = self.edge_end_y(edge.direction);
        (x1 - x).hypot(y1 - y) + (x2 - x).hypot(y2 - y)
            < (x1 - x2).hypot(y1 - y2) + self.line_thickness
    }
}

pub(crate) fn graph_width(
    elements: &[PrintElement],
    recommended_width: usize,
    row_height: Pixels,
) -> Pixels {
    if elements.is_empty() {
        return Pixels::ZERO;
    }
    let mut max_index = 0.0f64;
    for element in elements {
        max_index = max_index.max(element.position as f64);
        if let PrintElementKind::Edge(edge) = element.kind {
            max_index = max_index.max((element.position + edge.other_position) as f64 / 2.0);
        }
    }
    max_index += 1.0;
    max_index = max_index.max(MAX_GRAPH_WIDTH.min(recommended_width as f64));
    let row_height = row_height.to_f64();
    let width = (max_index * scale_with_row_height(WIDTH_NODE, row_height)).trunc()
        + scale_with_row_height(GRAPH_TEXT_GAP, row_height).trunc();
    px(width as f32)
}

pub(crate) fn element_at(
    elements: &[PrintElement],
    position_in_cell: Point<Pixels>,
    row_height: Pixels,
    scale_factor: f32,
) -> Option<&PrintElement> {
    let geometry = GraphGeometry::new(row_height.to_f64(), f64::from(scale_factor));
    let x = position_in_cell.x.to_f64();
    let y = position_in_cell.y.to_f64();
    elements
        .iter()
        .find(|element| {
            matches!(element.kind, PrintElementKind::Node(_))
                && geometry.is_over_node(element.position, x, y)
        })
        .or_else(|| {
            elements.iter().find(|element| match &element.kind {
                PrintElementKind::Edge(edge) => geometry.is_over_edge(element.position, edge, x, y),
                PrintElementKind::Node(_) => false,
            })
        })
}

pub(crate) fn paint_graph_row(
    elements: &[PrintElement],
    bounds: Bounds<Pixels>,
    row_background: Hsla,
    palette: &GraphPalette,
    window: &mut Window,
) {
    let geometry = GraphGeometry::new(
        bounds.size.height.to_f64(),
        f64::from(window.scale_factor()),
    );
    let shapes = row_shapes(elements, geometry, row_background, palette);
    window.with_content_mask(
        Some(ContentMask {
            bounds,
            ..Default::default()
        }),
        |window| {
            for shape in &shapes {
                let color = shape.color();
                if let Some(path) = shape.path(bounds.origin).build().log_err() {
                    window.paint_layer(bounds, |window| window.paint_path(path, color));
                }
            }
        },
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GraphShape {
    Segment {
        from: (f64, f64),
        to: (f64, f64),
        thickness: f64,
        color: Hsla,
    },
    Circle {
        x: f64,
        y: f64,
        diameter: f64,
        color: Hsla,
    },
}

impl GraphShape {
    fn color(&self) -> Hsla {
        match self {
            Self::Segment { color, .. } | Self::Circle { color, .. } => *color,
        }
    }

    fn path(&self, origin: Point<Pixels>) -> PathBuilder {
        let canvas_point =
            |(x, y): (f64, f64)| point(origin.x + px(x as f32), origin.y + px(y as f32));
        match *self {
            Self::Circle { x, y, diameter, .. } => {
                let radius = diameter / 2.0;
                circle_path((x + radius, y + radius), radius, &canvas_point)
            }
            Self::Segment {
                from,
                to,
                thickness,
                ..
            } => {
                let half_thickness = thickness / 2.0;
                let length = (to.0 - from.0).hypot(to.1 - from.1);
                if length <= f64::EPSILON {
                    return circle_path(from, half_thickness, &canvas_point);
                }
                let normal_x = -(to.1 - from.1) / length * half_thickness;
                let normal_y = (to.0 - from.0) / length * half_thickness;
                let radii = point(px(half_thickness as f32), px(half_thickness as f32));
                let mut builder = PathBuilder::fill();
                builder.move_to(canvas_point((from.0 + normal_x, from.1 + normal_y)));
                builder.line_to(canvas_point((to.0 + normal_x, to.1 + normal_y)));
                builder.arc_to(
                    radii,
                    px(0.),
                    false,
                    false,
                    canvas_point((to.0 - normal_x, to.1 - normal_y)),
                );
                builder.line_to(canvas_point((from.0 - normal_x, from.1 - normal_y)));
                builder.arc_to(
                    radii,
                    px(0.),
                    false,
                    false,
                    canvas_point((from.0 + normal_x, from.1 + normal_y)),
                );
                builder.close();
                builder
            }
        }
    }
}

fn circle_path(
    center: (f64, f64),
    radius: f64,
    canvas_point: &impl Fn((f64, f64)) -> Point<Pixels>,
) -> PathBuilder {
    let radii = point(px(radius as f32), px(radius as f32));
    let mut builder = PathBuilder::fill();
    builder.move_to(canvas_point((center.0 + radius, center.1)));
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        canvas_point((center.0 - radius, center.1)),
    );
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        canvas_point((center.0 + radius, center.1)),
    );
    builder.close();
    builder
}

fn row_shapes(
    elements: &[PrintElement],
    geometry: GraphGeometry,
    row_background: Hsla,
    palette: &GraphPalette,
) -> Vec<GraphShape> {
    let mut collected_shapes = RowShapes {
        geometry,
        row_background,
        shapes: Vec::new(),
    };
    for element in elements.iter().filter(|element| !element.is_selected) {
        collected_shapes.add_element(element, palette.color(element.color_id), false);
    }
    for element in elements.iter().filter(|element| element.is_selected) {
        collected_shapes.add_element(element, palette.mark_color(), true);
    }
    for element in elements.iter().filter(|element| element.is_selected) {
        collected_shapes.add_element(element, palette.color(element.color_id), false);
    }
    collected_shapes.shapes
}

#[derive(Clone, Copy)]
struct GraphLine {
    from: (f64, f64),
    to: (f64, f64),
    arrow_start: (f64, f64),
}

struct RowShapes {
    geometry: GraphGeometry,
    row_background: Hsla,
    shapes: Vec<GraphShape>,
}

impl RowShapes {
    fn add_element(&mut self, element: &PrintElement, color: Hsla, is_selected: bool) {
        match element.kind {
            PrintElementKind::Edge(edge) => {
                self.add_edge(element.position, edge, color, is_selected)
            }
            PrintElementKind::Node(node_type) => {
                self.add_node(element.position, node_type, color, is_selected)
            }
        }
    }

    fn add_edge(
        &mut self,
        position: usize,
        edge: EdgePrintElement,
        color: Hsla,
        is_selected: bool,
    ) {
        let geometry = self.geometry;
        let x1 = geometry.element_x(position);
        let y1 = geometry.row_center;
        let line = if position == edge.other_position {
            let arrow_gap = if edge.is_terminal {
                geometry.circle_radius / 2.0 + TERMINAL_ARROW_GAP_PADDING
            } else {
                0.0
            };
            let y2 = match edge.direction {
                EdgeDirection::Down => geometry.row_height - arrow_gap,
                EdgeDirection::Up => arrow_gap,
            };
            GraphLine {
                from: (x1, y1),
                to: (x1, y2),
                arrow_start: (x1, y2),
            }
        } else {
            let x2 = geometry.element_x(edge.other_position);
            let y2 = geometry.edge_end_y(edge.direction);
            GraphLine {
                from: (x1, y1),
                to: (x2, y2),
                arrow_start: ((x1 + x2) / 2.0, (y1 + y2) / 2.0),
            }
        };
        let thickness = if is_selected {
            geometry.selected_line_thickness
        } else {
            geometry.line_thickness
        };
        self.add_line(
            line,
            edge.has_arrow,
            edge.line_style == LineStyle::Solid,
            thickness,
            color,
        );
    }

    fn add_line(
        &mut self,
        line: GraphLine,
        has_arrow: bool,
        is_solid: bool,
        thickness: f64,
        color: Hsla,
    ) {
        let GraphLine {
            from,
            to,
            arrow_start,
        } = line;
        if is_solid || has_arrow {
            self.add_segment(from, to, thickness, color);
        } else {
            let edge_length = if from.0 == to.0 {
                self.geometry.row_height
            } else {
                (from.0 - to.0).hypot(from.1 - to.1)
            };
            let (dash, space) = dash_pattern(edge_length, self.geometry.row_height);
            let line_length = (to.0 - from.0).hypot(to.1 - from.1);
            for (start, end) in dash_segments(line_length, dash, space) {
                self.add_segment(
                    point_along(from, to, line_length, start),
                    point_along(from, to, line_length, end),
                    thickness,
                    color,
                );
            }
        }
        if has_arrow {
            let cosine = ARROW_ANGLE_COS2.sqrt();
            let sine = (1.0 - ARROW_ANGLE_COS2).sqrt();
            let length = ARROW_LENGTH * self.geometry.row_height;
            for arm_sine in [sine, -sine] {
                if let Some(arm_end) = rotate(from, arrow_start, cosine, arm_sine, length) {
                    self.add_segment(arrow_start, arm_end, thickness, color);
                }
            }
        }
    }

    fn add_node(&mut self, position: usize, node_type: NodeType, color: Hsla, is_selected: bool) {
        let geometry = self.geometry;
        let x0 = geometry.element_x(position);
        let y0 = geometry.row_center;
        match node_type {
            NodeType::Fill => {
                let (radius, diameter) = if is_selected {
                    (
                        geometry.selected_circle_radius,
                        geometry.selected_circle_diameter,
                    )
                } else {
                    (geometry.circle_radius, geometry.circle_diameter)
                };
                self.add_circle(x0 - radius, y0 - radius, diameter, color);
            }
            NodeType::OutlineAndFill => self.add_head_node(x0, y0, is_selected, color),
        }
    }

    fn add_head_node(&mut self, x0: f64, y0: f64, is_selected: bool, color: Hsla) {
        let geometry = self.geometry;
        if is_selected {
            let radius = geometry.head_selected_outer_radius;
            self.add_circle(
                x0 - radius,
                y0 - radius,
                geometry.head_selected_outer_diameter,
                color,
            );
            return;
        }
        let delta = geometry.head_ring_delta;
        let mut x = x0 - geometry.head_outer_radius;
        let mut y = y0 - geometry.head_outer_radius;
        let mut diameter = geometry.head_outer_diameter;
        self.add_circle(x, y, diameter, color);
        for ring_color in [self.row_background, color] {
            diameter -= delta * 2.0;
            x += delta;
            y += delta;
            self.add_circle(x, y, diameter, ring_color);
        }
    }

    fn add_circle(&mut self, x: f64, y: f64, diameter: f64, color: Hsla) {
        if diameter <= 0.0 {
            return;
        }
        self.shapes.push(GraphShape::Circle {
            x,
            y,
            diameter,
            color,
        });
    }

    fn add_segment(&mut self, from: (f64, f64), to: (f64, f64), thickness: f64, color: Hsla) {
        self.shapes.push(GraphShape::Segment {
            from,
            to,
            thickness,
            color,
        });
    }
}

fn dash_pattern(edge_length: f64, row_height: f64) -> (f64, f64) {
    let dash_count = (edge_length / row_height).floor().max(1.0);
    let space = row_height / 2.0 - DASH_SPACE_REDUCTION;
    (edge_length / dash_count - space, space)
}

fn dash_segments(length: f64, dash: f64, space: f64) -> SmallVec<[(f64, f64); 4]> {
    let mut segments = SmallVec::new();
    if dash <= 0.0 || space < 0.0 {
        segments.push((0.0, length));
        return segments;
    }
    let mut start = 0.0;
    let mut dash_remainder = dash / 2.0;
    while start < length {
        let end = (start + dash_remainder).min(length);
        segments.push((start, end));
        start = end + space;
        dash_remainder = dash;
    }
    segments
}

fn point_along(from: (f64, f64), to: (f64, f64), length: f64, distance: f64) -> (f64, f64) {
    if length <= 0.0 {
        return from;
    }
    (
        from.0 + (to.0 - from.0) * distance / length,
        from.1 + (to.1 - from.1) * distance / length,
    )
}

fn rotate(
    target: (f64, f64),
    center: (f64, f64),
    cosine: f64,
    sine: f64,
    length: f64,
) -> Option<(f64, f64)> {
    let translate_x = target.0 - center.0;
    let translate_y = target.1 - center.1;
    let distance = translate_x.hypot(translate_y);
    if distance <= 0.0 {
        return None;
    }
    let scale_x = length * translate_x / distance;
    let scale_y = length * translate_y / distance;
    Some((
        scale_x * cosine - scale_y * sine + center.0,
        scale_x * sine + scale_y * cosine + center.1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_log::graph::{
        GraphElement,
        linear_graph::{GraphEdge, GraphEdgeType},
    };
    use gpui::rgb;
    use pretty_assertions::assert_eq;
    use theme::Appearance;

    fn node(position: usize) -> PrintElement {
        PrintElement {
            row: 0,
            position,
            kind: PrintElementKind::Node(NodeType::Fill),
            color_id: 1,
            is_selected: false,
            graph_element: GraphElement::Node(0),
        }
    }

    fn edge(
        position: usize,
        other_position: usize,
        direction: EdgeDirection,
        line_style: LineStyle,
        has_arrow: bool,
        is_terminal: bool,
    ) -> PrintElement {
        PrintElement {
            row: 0,
            position,
            kind: PrintElementKind::Edge(EdgePrintElement {
                other_position,
                direction,
                line_style,
                has_arrow,
                is_terminal,
            }),
            color_id: 1,
            is_selected: false,
            graph_element: GraphElement::Edge(GraphEdge::new(0, 1, GraphEdgeType::Usual)),
        }
    }

    fn solid_edge(
        position: usize,
        other_position: usize,
        direction: EdgeDirection,
    ) -> PrintElement {
        edge(
            position,
            other_position,
            direction,
            LineStyle::Solid,
            false,
            false,
        )
    }

    fn palette() -> GraphPalette {
        GraphPalette::new(Appearance::Dark, Hsla::white())
    }

    fn shapes(elements: &[PrintElement], row_background: Hsla) -> Vec<GraphShape> {
        row_shapes(
            elements,
            GraphGeometry::new(26.0, 1.0),
            row_background,
            &palette(),
        )
    }

    fn segment_length(shape: &GraphShape) -> f64 {
        match shape {
            GraphShape::Segment { from, to, .. } => (to.0 - from.0).hypot(to.1 - from.1),
            GraphShape::Circle { .. } => 0.0,
        }
    }

    #[test]
    fn geometry_aligns_metrics_to_odd_device_pixels() {
        let geometry = GraphGeometry::new(26.0, 1.0);
        assert_eq!(geometry.row_center, 13.0);
        assert_eq!(geometry.element_width, 17.0);
        assert_eq!(geometry.element_center, 9.0);
        assert_eq!(geometry.line_thickness, 1.0);
        assert_eq!(geometry.selected_line_thickness, 3.0);
        assert_eq!(geometry.circle_diameter, 9.0);
        assert_eq!(geometry.circle_radius, 4.0);
        assert_eq!(geometry.selected_circle_diameter, 11.0);
        assert_eq!(geometry.selected_circle_radius, 5.0);
        assert_eq!(geometry.head_outer_diameter, 13.0);
        assert_eq!(geometry.head_outer_radius, 6.0);
        assert_eq!(geometry.head_selected_outer_diameter, 15.0);
        assert_eq!(geometry.head_selected_outer_radius, 7.0);
        assert_eq!(geometry.head_ring_delta, 2.0);

        let retina = GraphGeometry::new(26.0, 2.0);
        assert_eq!(retina.row_center, 12.5);
        assert_eq!(retina.element_width, 18.5);
        assert_eq!(retina.element_center, 9.0);
        assert_eq!(retina.line_thickness, 1.5);
        assert_eq!(retina.selected_line_thickness, 2.5);
        assert_eq!(retina.circle_diameter, 8.5);
        assert_eq!(retina.circle_radius, 4.0);
        assert_eq!(retina.selected_circle_diameter, 9.5);
        assert_eq!(retina.selected_circle_radius, 4.5);
        assert_eq!(retina.head_outer_diameter, 13.5);
        assert_eq!(retina.head_outer_radius, 6.5);
        assert_eq!(retina.head_selected_outer_diameter, 14.5);
        assert_eq!(retina.head_selected_outer_radius, 7.0);
        assert_eq!(retina.head_ring_delta, 2.0);
    }

    #[test]
    fn graph_width_uses_recommended_width_up_to_six_lanes() {
        let elements = [node(1), solid_edge(1, 2, EdgeDirection::Down)];
        assert_eq!(graph_width(&elements, 3, px(26.)), px(58.));
        assert_eq!(graph_width(&elements, 10, px(26.)), px(115.));
        assert_eq!(graph_width(&[], 10, px(26.)), Pixels::ZERO);
    }

    #[test]
    fn element_at_prefers_nodes_over_edges() {
        let elements = [solid_edge(0, 0, EdgeDirection::Down), node(0)];
        let found = element_at(&elements, point(px(9.), px(13.)), px(26.), 1.0);
        assert!(matches!(
            found.map(|element| element.kind),
            Some(PrintElementKind::Node(_))
        ));
        let found = element_at(&elements, point(px(9.), px(22.)), px(26.), 1.0);
        assert!(matches!(
            found.map(|element| element.kind),
            Some(PrintElementKind::Edge(_))
        ));
        assert!(element_at(&elements, point(px(40.), px(13.)), px(26.), 1.0).is_none());
    }

    #[test]
    fn dashes_start_with_half_a_dash_and_keep_the_space() {
        let (dash, space) = dash_pattern(26.0, 26.0);
        assert_eq!((dash, space), (15.0, 11.0));
        let segments = dash_segments(13.0, dash, space);
        assert_eq!(segments.into_vec(), vec![(0.0, 7.5)]);
        let segments = dash_segments(40.0, dash, space);
        assert_eq!(segments.into_vec(), vec![(0.0, 7.5), (18.5, 33.5)]);
    }

    #[test]
    fn arrow_arms_open_back_towards_the_line_start() {
        let cosine = ARROW_ANGLE_COS2.sqrt();
        let sine = (1.0 - ARROW_ANGLE_COS2).sqrt();
        let (x, y) = rotate((0.0, 0.0), (0.0, 10.0), cosine, sine, 5.0).unwrap();
        assert!(y < 10.0);
        assert!((x.hypot(y - 10.0) - 5.0).abs() < 1e-9);
        assert!(rotate((1.0, 1.0), (1.0, 1.0), cosine, sine, 5.0).is_none());
    }

    #[test]
    fn dashed_edges_are_painted_as_dashes_and_solid_edges_as_one_stroke() {
        let color = palette().color(1);
        assert_eq!(
            shapes(&[solid_edge(0, 0, EdgeDirection::Down)], Hsla::black()),
            vec![GraphShape::Segment {
                from: (9.0, 13.0),
                to: (9.0, 26.0),
                thickness: 1.0,
                color,
            }]
        );
        assert_eq!(
            shapes(
                &[edge(
                    0,
                    0,
                    EdgeDirection::Down,
                    LineStyle::Dashed,
                    false,
                    false
                )],
                Hsla::black()
            ),
            vec![GraphShape::Segment {
                from: (9.0, 13.0),
                to: (9.0, 20.5),
                thickness: 1.0,
                color,
            }]
        );

        let dashes = shapes(
            &[edge(
                0,
                1,
                EdgeDirection::Down,
                LineStyle::Dashed,
                false,
                false,
            )],
            Hsla::black(),
        );
        let line_length = 17.0f64.hypot(26.0);
        let space = 11.0;
        let dash = line_length - space;
        assert_eq!(dashes.len(), 2);
        assert!((segment_length(&dashes[0]) - dash / 2.0).abs() < 1e-9);
        assert!((segment_length(&dashes[1]) - (line_length - dash / 2.0 - space)).abs() < 1e-9);
    }

    #[test]
    fn edges_with_arrows_are_painted_solid_even_when_dashed() {
        let painted = shapes(
            &[edge(
                0,
                0,
                EdgeDirection::Down,
                LineStyle::Dashed,
                true,
                false,
            )],
            Hsla::black(),
        );
        assert_eq!(painted.len(), 3);
        assert_eq!(
            painted[0],
            GraphShape::Segment {
                from: (9.0, 13.0),
                to: (9.0, 26.0),
                thickness: 1.0,
                color: palette().color(1),
            }
        );
        for arm in &painted[1..] {
            let GraphShape::Segment { from, to, .. } = *arm else {
                panic!("arrow arms are segments");
            };
            assert_eq!(from, (9.0, 26.0));
            assert!(to.1 < 26.0);
            assert!((segment_length(arm) - ARROW_LENGTH * 26.0).abs() < 1e-9);
        }
    }

    #[test]
    fn terminal_arrows_stop_short_of_the_row_border() {
        let painted = shapes(
            &[
                edge(0, 0, EdgeDirection::Down, LineStyle::Dashed, true, true),
                edge(1, 1, EdgeDirection::Up, LineStyle::Dashed, true, true),
            ],
            Hsla::black(),
        );
        let color = palette().color(1);
        assert_eq!(painted.len(), 6);
        assert_eq!(
            painted[0],
            GraphShape::Segment {
                from: (9.0, 13.0),
                to: (9.0, 23.0),
                thickness: 1.0,
                color,
            }
        );
        assert_eq!(
            painted[3],
            GraphShape::Segment {
                from: (26.0, 13.0),
                to: (26.0, 3.0),
                thickness: 1.0,
                color,
            }
        );
    }

    #[test]
    fn selected_elements_get_a_mark_pass_below_their_normal_pass() {
        let palette = palette();
        let mut selected_node = node(0);
        selected_node.is_selected = true;
        assert_eq!(
            shapes(
                &[selected_node, solid_edge(1, 1, EdgeDirection::Down)],
                Hsla::black()
            ),
            vec![
                GraphShape::Segment {
                    from: (26.0, 13.0),
                    to: (26.0, 26.0),
                    thickness: 1.0,
                    color: palette.color(1),
                },
                GraphShape::Circle {
                    x: 4.0,
                    y: 8.0,
                    diameter: 11.0,
                    color: palette.mark_color(),
                },
                GraphShape::Circle {
                    x: 5.0,
                    y: 9.0,
                    diameter: 9.0,
                    color: palette.color(1),
                },
            ]
        );
    }

    #[test]
    fn head_nodes_are_rings_filled_with_the_row_background() {
        let palette = palette();
        let color = palette.color(1);
        let background = Hsla::from(rgb(0x191A1C));
        let mut head = node(0);
        head.kind = PrintElementKind::Node(NodeType::OutlineAndFill);
        let ring = [
            GraphShape::Circle {
                x: 3.0,
                y: 7.0,
                diameter: 13.0,
                color,
            },
            GraphShape::Circle {
                x: 5.0,
                y: 9.0,
                diameter: 9.0,
                color: background,
            },
            GraphShape::Circle {
                x: 7.0,
                y: 11.0,
                diameter: 5.0,
                color,
            },
        ];
        assert_eq!(shapes(&[head], background), ring.to_vec());

        head.is_selected = true;
        let mut selected = vec![GraphShape::Circle {
            x: 2.0,
            y: 6.0,
            diameter: 15.0,
            color: palette.mark_color(),
        }];
        selected.extend(ring);
        assert_eq!(shapes(&[head], background), selected);
    }
}
