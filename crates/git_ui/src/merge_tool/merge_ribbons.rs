pub(crate) mod wave;

use std::ops::Range;
use std::rc::Rc;

use editor::Editor;
use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, PathBuilder, Pixels, Point, Style, Window, fill, point, px, size,
};
use util::ResultExt as _;

use self::wave::{PathCommand, Xy, connector_commands, paint_stroke, wave_stroke_width};
use super::merge_gutter::geometry::marker_paint_range;
use super::merge_gutter::pane::PaneGeometry;
use super::palette::{ChangeKind, change_colors, current_palette, editor_background, hex_color};

pub(crate) const DIVIDER_WIDTH: f32 = 24.0;
pub(crate) const CONTROL_PROXIMITY: f32 = 0.3;
pub(crate) const DOTTED_STROKE_WIDTH: f32 = 2.3;
pub(crate) const DOTTED_DASH: f32 = 2.0;
pub(crate) const CENTRE_LINE_WIDTH: f32 = 1.0;
pub(crate) const COLLAPSED_ROW_THRESHOLD: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RibbonRows {
    pub start_left: f32,
    pub end_left: f32,
    pub start_right: f32,
    pub end_right: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cubic {
    pub from: Xy,
    pub control_a: Xy,
    pub control_b: Xy,
    pub to: Xy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RibbonFill {
    pub top: Cubic,
    pub right_edge_to: Xy,
    pub bottom: Cubic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RibbonShape {
    Filled { outline: RibbonFill, centre: Cubic },
    Dotted { top: Cubic, bottom: Cubic },
}

pub(crate) fn side_rows(start: f32, end: f32) -> (f32, f32) {
    if end - start < COLLAPSED_ROW_THRESHOLD {
        (start - 1.0, start)
    } else {
        (start, end - 1.0)
    }
}

pub(crate) fn ribbon_rows(
    start_left: f32,
    end_left: f32,
    start_right: f32,
    end_right: f32,
) -> RibbonRows {
    let (left_start, left_end) = side_rows(start_left, end_left);
    let (right_start, right_end) = side_rows(start_right, end_right);
    RibbonRows {
        start_left: left_start,
        end_left: left_end,
        start_right: right_start,
        end_right: right_end,
    }
}

pub(crate) fn forward_curve(x1: f32, x2: f32, y1: f32, y2: f32) -> Cubic {
    let width = x2 - x1;
    Cubic {
        from: (x1, y1),
        control_a: (x1 + width * CONTROL_PROXIMITY, y1),
        control_b: (x1 + width * (1.0 - CONTROL_PROXIMITY), y2),
        to: (x1 + width, y2),
    }
}

pub(crate) fn backward_curve(x1: f32, x2: f32, y1: f32, y2: f32) -> Cubic {
    let width = x2 - x1;
    Cubic {
        from: (x1 + width, y2),
        control_a: (x1 + width * (1.0 - CONTROL_PROXIMITY), y2),
        control_b: (x1 + width * CONTROL_PROXIMITY, y1),
        to: (x1, y1),
    }
}

pub(crate) fn fill_outline(rows: &RibbonRows, width: f32) -> RibbonFill {
    let top = forward_curve(0.0, width, rows.start_left, rows.start_right);
    let bottom_left = rows.end_left + 1.0;
    let bottom_right = rows.end_right + 1.0;
    RibbonFill {
        top,
        right_edge_to: (width, bottom_right),
        bottom: backward_curve(0.0, width, bottom_left, bottom_right),
    }
}

fn integer_midpoint(first: f32, second: f32) -> f32 {
    ((first + second) / 2.0).trunc()
}

pub(crate) fn centre_line(rows: &RibbonRows, width: f32) -> Cubic {
    forward_curve(
        0.0,
        width,
        integer_midpoint(rows.start_left, rows.end_left),
        integer_midpoint(rows.start_right, rows.end_right),
    )
}

pub(crate) fn ribbon_shape(rows: &RibbonRows, width: f32, resolved: bool) -> RibbonShape {
    if resolved {
        RibbonShape::Dotted {
            top: forward_curve(0.0, width, rows.start_left, rows.start_right),
            bottom: forward_curve(0.0, width, rows.end_left, rows.end_right),
        }
    } else {
        RibbonShape::Filled {
            outline: fill_outline(rows, width),
            centre: centre_line(rows, width),
        }
    }
}

pub(crate) fn displayed_kind(kind: ChangeKind, folded: bool) -> ChangeKind {
    match (folded, kind) {
        (true, ChangeKind::Inserted | ChangeKind::Deleted) => ChangeKind::Modified,
        _ => kind,
    }
}

pub(crate) fn is_hidden(resolved: bool, folded: bool) -> bool {
    resolved && folded
}

pub(crate) fn is_outside_band(left: &Range<f32>, right: &Range<f32>, band_height: f32) -> bool {
    let above = left.end < 0.0 && right.end < 0.0;
    let below = left.start > band_height && right.start > band_height;
    above || below
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RibbonChange {
    pub left_lines: Range<u32>,
    pub right_lines: Range<u32>,
    pub kind: ChangeKind,
    pub resolved: bool,
    pub ai_resolved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FoldConnector {
    pub left_line: u32,
    pub right_line: u32,
    pub hovered: bool,
}

#[derive(Clone)]
pub(crate) struct DividerInteraction {
    pub on_drag: Rc<dyn Fn(Pixels, &mut Window, &mut App)>,
    pub on_double_click: Rc<dyn Fn(&mut Window, &mut App)>,
}

pub(crate) struct MergeRibbons {
    id: ElementId,
    left_editor: Entity<Editor>,
    right_editor: Entity<Editor>,
    changes: Vec<RibbonChange>,
    connectors: Vec<FoldConnector>,
    interaction: Option<DividerInteraction>,
}

impl MergeRibbons {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        left_editor: Entity<Editor>,
        right_editor: Entity<Editor>,
        changes: Vec<RibbonChange>,
    ) -> Self {
        Self {
            id: id.into(),
            left_editor,
            right_editor,
            changes,
            connectors: Vec::new(),
            interaction: None,
        }
    }

    pub(crate) fn connectors(mut self, connectors: Vec<FoldConnector>) -> Self {
        self.connectors = connectors;
        self
    }

    pub(crate) fn interaction(mut self, interaction: DividerInteraction) -> Self {
        self.interaction = Some(interaction);
        self
    }
}

struct PreparedRibbon {
    shape: RibbonShape,
    color: Hsla,
}

struct PreparedConnector {
    commands: Vec<PathCommand>,
    hovered: bool,
}

pub(crate) struct RibbonsPrepaint {
    band: Bounds<Pixels>,
    band_color: Hsla,
    wave_color: Hsla,
    ribbons: Vec<PreparedRibbon>,
    connectors: Vec<PreparedConnector>,
    hitbox: Hitbox,
}

fn place(origin: Point<Pixels>, xy: Xy) -> Point<Pixels> {
    point(origin.x + px(xy.0), origin.y + px(xy.1))
}

fn append_cubic(builder: &mut PathBuilder, origin: Point<Pixels>, curve: &Cubic) {
    builder.cubic_bezier_to(
        place(origin, curve.to),
        place(origin, curve.control_a),
        place(origin, curve.control_b),
    );
}

fn paint_filled(
    window: &mut Window,
    origin: Point<Pixels>,
    outline: &RibbonFill,
    centre: &Cubic,
    color: Hsla,
) {
    let mut builder = PathBuilder::fill();
    builder.move_to(place(origin, outline.top.from));
    append_cubic(&mut builder, origin, &outline.top);
    builder.line_to(place(origin, outline.right_edge_to));
    append_cubic(&mut builder, origin, &outline.bottom);
    builder.close();
    if let Some(path) = builder.build().log_err() {
        window.paint_path(path, color);
    }
    let mut centre_builder = PathBuilder::stroke(px(CENTRE_LINE_WIDTH));
    centre_builder.move_to(place(origin, centre.from));
    append_cubic(&mut centre_builder, origin, centre);
    if let Some(path) = centre_builder.build().log_err() {
        window.paint_path(path, color);
    }
}

fn paint_dotted(window: &mut Window, origin: Point<Pixels>, curve: &Cubic, color: Hsla) {
    let mut builder = PathBuilder::stroke(px(DOTTED_STROKE_WIDTH))
        .dash_array(&[px(DOTTED_DASH), px(DOTTED_DASH)]);
    builder.move_to(place(origin, curve.from));
    append_cubic(&mut builder, origin, curve);
    if let Some(path) = builder.build().log_err() {
        window.paint_path(path, color);
    }
}

fn register_interaction(window: &mut Window, hitbox: &Hitbox, interaction: &DividerInteraction) {
    window.set_cursor_style(CursorStyle::ResizeLeft, hitbox);
    let mouse_down_hitbox = hitbox.clone();
    let on_double_click = interaction.on_double_click.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if !phase.bubble()
            || event.button != MouseButton::Left
            || !mouse_down_hitbox.is_hovered(window)
        {
            return;
        }
        cx.stop_propagation();
        if event.click_count >= 2 {
            on_double_click(window, cx);
        } else {
            window.capture_pointer(mouse_down_hitbox.id);
        }
    });
    let mouse_move_hitbox = hitbox.clone();
    let on_drag = interaction.on_drag.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase.bubble()
            && event.dragging()
            && window.captured_hitbox() == Some(mouse_move_hitbox.id)
        {
            on_drag(event.position.x, window, cx);
        }
    });
}

fn side_paint_range(pane: &PaneGeometry, lines: &Range<u32>) -> (f32, f32) {
    if lines.start >= lines.end && pane.is_line_folded(lines.start) {
        return (
            pane.line_top_y(lines.start),
            pane.line_bottom_y(lines.start),
        );
    }
    marker_paint_range(
        lines,
        |line| pane.line_top_y(line),
        |line| pane.line_bottom_y(line),
    )
}

impl MergeRibbons {
    fn prepare_ribbons(
        &self,
        left: &PaneGeometry,
        right: &PaneGeometry,
        band_top: f32,
        band_height: f32,
        cx: &App,
    ) -> Vec<PreparedRibbon> {
        let local = |y: f32| (y - band_top).round();
        let mut ribbons = Vec::new();
        for change in &self.changes {
            let (left_start, left_end) = side_paint_range(left, &change.left_lines);
            let (right_start, right_end) = side_paint_range(right, &change.right_lines);
            let (left_start, left_end) = (local(left_start), local(left_end));
            let (right_start, right_end) = (local(right_start), local(right_end));
            let folded = left.is_range_folded(&change.left_lines)
                && right.is_range_folded(&change.right_lines);
            if is_hidden(change.resolved, folded) {
                continue;
            }
            if is_outside_band(
                &(left_start..left_end),
                &(right_start..right_end),
                band_height,
            ) {
                continue;
            }
            let rows = ribbon_rows(left_start, left_end, right_start, right_end);
            let kind = displayed_kind(change.kind, folded);
            ribbons.push(PreparedRibbon {
                shape: ribbon_shape(&rows, DIVIDER_WIDTH, change.resolved),
                color: change_colors(cx, kind, change.ai_resolved).solid,
            });
        }
        ribbons
    }

    fn prepare_connectors(
        &self,
        left: &PaneGeometry,
        right: &PaneGeometry,
        band_top: f32,
        band_height: f32,
    ) -> Vec<PreparedConnector> {
        let mut connectors = Vec::new();
        for connector in &self.connectors {
            let left_y = (left.line_top_y(connector.left_line) - band_top).round();
            let right_y = (right.line_top_y(connector.right_line) - band_top).round();
            let outside = left_y + left.line_height < 0.0 && right_y + right.line_height < 0.0
                || left_y > band_height && right_y > band_height;
            if outside {
                continue;
            }
            connectors.push(PreparedConnector {
                commands: connector_commands(0.0, DIVIDER_WIDTH, left_y, right_y, left.line_height),
                hovered: connector.hovered,
            });
        }
        connectors
    }
}

impl IntoElement for MergeRibbons {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MergeRibbons {
    type RequestLayoutState = ();
    type PrepaintState = Option<RibbonsPrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(DIVIDER_WIDTH).into();
        style.flex_shrink = 0.0;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let left = PaneGeometry::measure(&self.left_editor, window, cx);
        let right = PaneGeometry::measure(&self.right_editor, window, cx);
        let (Some(left), Some(right)) = (left, right) else {
            window.request_animation_frame();
            return None;
        };
        let band_top = left.top().max(right.top());
        let band_bottom = (left.top() + left.height()).min(right.top() + right.height());
        if band_bottom <= band_top {
            return None;
        }
        let band_height = band_bottom - band_top;
        let band = Bounds::new(
            point(bounds.origin.x, px(band_top)),
            size(bounds.size.width, px(band_height)),
        );
        let ribbons = self.prepare_ribbons(&left, &right, band_top, band_height, cx);
        let connectors = self.prepare_connectors(&left, &right, band_top, band_height);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Some(RibbonsPrepaint {
            band,
            band_color: editor_background(cx),
            wave_color: hex_color(current_palette(cx).wave),
            ribbons,
            connectors,
            hitbox,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            return;
        };
        if let Some(interaction) = &self.interaction {
            register_interaction(window, &prepaint.hitbox, interaction);
        }
        window.paint_quad(fill(prepaint.band, prepaint.band_color));
        let origin = prepaint.band.origin;
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.band,
                ..Default::default()
            }),
            |window| {
                for ribbon in &prepaint.ribbons {
                    match &ribbon.shape {
                        RibbonShape::Filled { outline, centre } => {
                            paint_filled(window, origin, outline, centre, ribbon.color)
                        }
                        RibbonShape::Dotted { top, bottom } => {
                            paint_dotted(window, origin, top, ribbon.color);
                            paint_dotted(window, origin, bottom, ribbon.color);
                        }
                    }
                }
                for connector in &prepaint.connectors {
                    paint_stroke(
                        window,
                        &connector.commands,
                        origin,
                        wave_stroke_width(connector.hovered),
                        prepaint.wave_color,
                    );
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_point(actual: Xy, expected: Xy) {
        assert_close(actual.0, expected.0);
        assert_close(actual.1, expected.1);
    }

    #[test]
    fn merge_tool_side_rows_collapse_short_sides_to_two_row_stub() {
        assert_eq!(side_rows(40.0, 40.0), (39.0, 40.0));
        assert_eq!(side_rows(40.0, 41.0), (39.0, 40.0));
        assert_eq!(side_rows(40.0, 42.0), (40.0, 41.0));
        assert_eq!(side_rows(100.0, 160.0), (100.0, 159.0));
    }

    #[test]
    fn merge_tool_ribbon_rows_normalise_both_sides() {
        let rows = ribbon_rows(40.0, 40.0, 100.0, 160.0);
        assert_eq!(
            rows,
            RibbonRows {
                start_left: 39.0,
                end_left: 40.0,
                start_right: 100.0,
                end_right: 159.0,
            }
        );
    }

    #[test]
    fn merge_tool_curves_use_control_points_at_thirty_and_seventy_percent() {
        let curve = forward_curve(0.0, 24.0, 10.0, 50.0);
        assert_point(curve.from, (0.0, 10.0));
        assert_point(curve.control_a, (7.2, 10.0));
        assert_point(curve.control_b, (16.8, 50.0));
        assert_point(curve.to, (24.0, 50.0));
        let back = backward_curve(0.0, 24.0, 10.0, 50.0);
        assert_point(back.from, (24.0, 50.0));
        assert_point(back.control_a, (16.8, 50.0));
        assert_point(back.control_b, (7.2, 10.0));
        assert_point(back.to, (0.0, 10.0));
    }

    #[test]
    fn merge_tool_fill_bottom_edge_is_end_row_plus_one() {
        let rows = ribbon_rows(40.0, 40.0, 100.0, 160.0);
        let outline = fill_outline(&rows, DIVIDER_WIDTH);
        assert_point(outline.top.from, (0.0, 39.0));
        assert_point(outline.top.to, (24.0, 100.0));
        assert_point(outline.right_edge_to, (24.0, 160.0));
        assert_point(outline.bottom.from, (24.0, 160.0));
        assert_point(outline.bottom.to, (0.0, 41.0));
    }

    #[test]
    fn merge_tool_centre_line_uses_integer_midpoints() {
        let rows = ribbon_rows(40.0, 40.0, 100.0, 160.0);
        let centre = centre_line(&rows, DIVIDER_WIDTH);
        assert_point(centre.from, (0.0, 39.0));
        assert_point(centre.to, (24.0, 129.0));
    }

    #[test]
    fn merge_tool_resolved_ribbon_is_two_dotted_edges_without_fill() {
        let rows = ribbon_rows(40.0, 80.0, 100.0, 160.0);
        match ribbon_shape(&rows, DIVIDER_WIDTH, true) {
            RibbonShape::Dotted { top, bottom } => {
                assert_point(top.from, (0.0, 40.0));
                assert_point(top.to, (24.0, 100.0));
                assert_point(bottom.from, (0.0, 79.0));
                assert_point(bottom.to, (24.0, 159.0));
            }
            RibbonShape::Filled { .. } => panic!("resolved ribbon must not be filled"),
        }
        assert!(matches!(
            ribbon_shape(&rows, DIVIDER_WIDTH, false),
            RibbonShape::Filled { .. }
        ));
    }

    #[test]
    fn merge_tool_folded_pairs_use_modified_colour_for_insertions_and_deletions() {
        assert_eq!(
            displayed_kind(ChangeKind::Inserted, true),
            ChangeKind::Modified
        );
        assert_eq!(
            displayed_kind(ChangeKind::Deleted, true),
            ChangeKind::Modified
        );
        assert_eq!(
            displayed_kind(ChangeKind::Conflict, true),
            ChangeKind::Conflict
        );
        assert_eq!(
            displayed_kind(ChangeKind::Inserted, false),
            ChangeKind::Inserted
        );
    }

    #[test]
    fn merge_tool_resolved_folded_ribbons_are_hidden() {
        assert!(is_hidden(true, true));
        assert!(!is_hidden(true, false));
        assert!(!is_hidden(false, true));
    }

    #[test]
    fn merge_tool_ribbon_culling_needs_both_sides_outside_band() {
        assert!(is_outside_band(&(-50.0..-10.0), &(-30.0..-5.0), 300.0));
        assert!(is_outside_band(&(400.0..450.0), &(310.0..320.0), 300.0));
        assert!(!is_outside_band(&(-50.0..-10.0), &(10.0..40.0), 300.0));
        assert!(!is_outside_band(&(250.0..450.0), &(100.0..120.0), 300.0));
        assert!(!is_outside_band(&(-50.0..-10.0), &(310.0..320.0), 300.0));
    }
}
