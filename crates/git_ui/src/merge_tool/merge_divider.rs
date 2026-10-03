use std::ops::Range;

use editor::{Bias, Editor, display_map::DisplaySnapshot};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClickEvent, ContentMask, DefiniteLength, Element,
    ElementId, Entity, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Length,
    PathBuilder, Pixels, WeakEntity, Window, point, px, size,
};
use language::Point;
use three_way_merge::Side;
use ui::{IconButton, IconButtonShape, Tooltip, prelude::*};
use util::ResultExt as _;

use super::merge_view::MergeView;

pub(crate) const DIVIDER_WIDTH: Pixels = px(56.);
const BUTTON_INSET: Pixels = px(2.);
const RIBBON_MIN_THICKNESS: Pixels = px(1.5);
const RIBBON_OPACITY: f32 = 0.3;
const ACCEPT_TOOLTIP: &str = "Accept — Cmd+click to resolve conflict";
const IGNORE_TOOLTIP: &str = "Ignore — Cmd+click to resolve conflict";
const RESOLVE_SIMPLE_TOOLTIP: &str = "Resolve simple conflict";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Conflict,
}

impl ChangeKind {
    pub(crate) fn color(self, cx: &App) -> Hsla {
        let colors = cx.theme().colors();
        match self {
            ChangeKind::Added => colors.version_control_added,
            ChangeKind::Modified => colors.version_control_modified,
            ChangeKind::Deleted => colors.version_control_deleted,
            ChangeKind::Conflict => colors.version_control_conflict,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DividerChunk {
    pub(crate) chunk_index: usize,
    pub(crate) side_lines: Range<u32>,
    pub(crate) result_lines: Range<u32>,
    pub(crate) kind: ChangeKind,
    pub(crate) simple_conflict: bool,
}

pub(crate) struct MergeDivider {
    side: Side,
    merge_view: WeakEntity<MergeView>,
    side_editor: Entity<Editor>,
    result_editor: Entity<Editor>,
    chunks: Vec<DividerChunk>,
}

impl MergeDivider {
    pub(crate) fn new(
        side: Side,
        merge_view: WeakEntity<MergeView>,
        side_editor: Entity<Editor>,
        result_editor: Entity<Editor>,
        chunks: Vec<DividerChunk>,
    ) -> Self {
        Self {
            side,
            merge_view,
            side_editor,
            result_editor,
            chunks,
        }
    }

    fn render_side_buttons(&self, chunk: &DividerChunk) -> AnyElement {
        let accept = self.render_accept_button(chunk.chunk_index);
        let ignore = self.render_ignore_button(chunk.chunk_index);
        match self.side {
            Side::Left => h_flex()
                .gap_0p5()
                .child(ignore)
                .child(accept)
                .into_any_element(),
            Side::Right => h_flex()
                .gap_0p5()
                .child(accept)
                .child(ignore)
                .into_any_element(),
        }
    }

    fn render_accept_button(&self, chunk_index: usize) -> AnyElement {
        let side = self.side;
        let merge_view = self.merge_view.clone();
        let (id, icon) = match side {
            Side::Left => ("merge-accept-left", IconName::ChevronsRight),
            Side::Right => ("merge-accept-right", IconName::ChevronsLeft),
        };
        IconButton::new((id, chunk_index), icon)
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::XSmall)
            .tooltip(Tooltip::text(ACCEPT_TOOLTIP))
            .on_click(move |event: &ClickEvent, window, cx| {
                let modifiers = event.modifiers();
                merge_view
                    .update(cx, |merge_view, cx| {
                        merge_view.accept_chunk_from_divider(
                            chunk_index,
                            side,
                            modifiers,
                            window,
                            cx,
                        );
                    })
                    .log_err();
            })
            .into_any_element()
    }

    fn render_ignore_button(&self, chunk_index: usize) -> AnyElement {
        let side = self.side;
        let merge_view = self.merge_view.clone();
        let id = match side {
            Side::Left => "merge-ignore-left",
            Side::Right => "merge-ignore-right",
        };
        IconButton::new((id, chunk_index), IconName::Close)
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::XSmall)
            .tooltip(Tooltip::text(IGNORE_TOOLTIP))
            .on_click(move |event: &ClickEvent, window, cx| {
                let modifiers = event.modifiers();
                merge_view
                    .update(cx, |merge_view, cx| {
                        merge_view.ignore_chunk_from_divider(
                            chunk_index,
                            side,
                            modifiers,
                            window,
                            cx,
                        );
                    })
                    .log_err();
            })
            .into_any_element()
    }

    fn render_wand_button(&self, chunk_index: usize) -> AnyElement {
        let merge_view = self.merge_view.clone();
        IconButton::new(("merge-resolve-simple", chunk_index), IconName::Wand)
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::XSmall)
            .tooltip(Tooltip::text(RESOLVE_SIMPLE_TOOLTIP))
            .on_click(move |_, window, cx| {
                merge_view
                    .update(cx, |merge_view, cx| {
                        merge_view.resolve_simple_chunk(chunk_index, window, cx);
                    })
                    .log_err();
            })
            .into_any_element()
    }

    fn prepaint_button(
        mut button: AnyElement,
        x: Pixels,
        align_right: bool,
        line_top: Pixels,
        line_height: Pixels,
        content_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let button_size = button.layout_as_root(
            size(AvailableSpace::MinContent, AvailableSpace::MinContent),
            window,
            cx,
        );
        let left = if align_right {
            x - button_size.width
        } else {
            x
        };
        let top = line_top + (line_height - button_size.height) / 2.;
        window.with_content_mask(
            Some(ContentMask {
                bounds: content_bounds,
                ..Default::default()
            }),
            |window| {
                button.prepaint_at(point(left, top), window, cx);
            },
        );
        button
    }
}

struct PaneGeometry {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    scroll_top: f64,
    display_snapshot: DisplaySnapshot,
}

impl PaneGeometry {
    fn measure(editor: &Entity<Editor>, window: &mut Window, cx: &mut App) -> Option<Self> {
        editor.update(cx, |editor, cx| {
            let bounds = *editor.last_bounds()?;
            let visible_line_count = editor.visible_line_count()?;
            if visible_line_count <= 0. {
                return None;
            }
            let line_height = bounds.size.height / visible_line_count as f32;
            let snapshot = editor.snapshot(window, cx);
            Some(Self {
                bounds,
                line_height,
                scroll_top: snapshot.scroll_position().y,
                display_snapshot: snapshot.display_snapshot,
            })
        })
    }

    fn y_for_line(&self, line: u32) -> Pixels {
        let display_row = display_row_for_line(&self.display_snapshot, line);
        self.bounds.top() + self.line_height * (display_row - self.scroll_top) as f32
    }

    fn span(&self, lines: &Range<u32>) -> Range<Pixels> {
        self.y_for_line(lines.start)..self.y_for_line(lines.end)
    }
}

pub(crate) fn display_row_for_line(snapshot: &DisplaySnapshot, line: u32) -> f64 {
    let max_point = snapshot.buffer_snapshot().max_point();
    if line > max_point.row {
        return f64::from(snapshot.max_point().row().0) + 1.;
    }
    f64::from(
        snapshot
            .point_to_display_point(Point::new(line, 0), Bias::Left)
            .row()
            .0,
    )
}

struct Ribbon {
    left_x: Pixels,
    right_x: Pixels,
    left: Range<Pixels>,
    right: Range<Pixels>,
    color: Hsla,
}

impl Ribbon {
    fn paint(&self, window: &mut Window) {
        let left_bottom = self.left.end.max(self.left.start + RIBBON_MIN_THICKNESS);
        let right_bottom = self.right.end.max(self.right.start + RIBBON_MIN_THICKNESS);
        let middle = (self.left_x + self.right_x) / 2.;
        let mut builder = PathBuilder::fill();
        builder.move_to(point(self.left_x, self.left.start));
        builder.cubic_bezier_to(
            point(self.right_x, self.right.start),
            point(middle, self.left.start),
            point(middle, self.right.start),
        );
        builder.line_to(point(self.right_x, right_bottom));
        builder.cubic_bezier_to(
            point(self.left_x, left_bottom),
            point(middle, right_bottom),
            point(middle, left_bottom),
        );
        builder.close();
        if let Some(path) = builder.build().log_err() {
            window.paint_path(path, self.color);
        }
    }
}

pub(crate) struct MergeDividerPrepaint {
    content_bounds: Bounds<Pixels>,
    ribbons: Vec<Ribbon>,
    buttons: Vec<AnyElement>,
}

impl IntoElement for MergeDivider {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MergeDivider {
    type RequestLayoutState = ();
    type PrepaintState = Option<MergeDividerPrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(match self.side {
            Side::Left => "merge-divider-left".into(),
            Side::Right => "merge-divider-right".into(),
        })
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
        let mut style = gpui::Style::default();
        style.position = gpui::Position::Absolute;
        style.inset.top = DefiniteLength::Fraction(0.).into();
        style.inset.left = DefiniteLength::Fraction(0.).into();
        style.size.width = Length::Definite(DefiniteLength::Fraction(1.));
        style.size.height = Length::Definite(DefiniteLength::Fraction(1.));
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let side_pane = PaneGeometry::measure(&self.side_editor, window, cx)?;
        let result_pane = PaneGeometry::measure(&self.result_editor, window, cx)?;
        let (left_pane, right_pane) = match self.side {
            Side::Left => (&side_pane, &result_pane),
            Side::Right => (&result_pane, &side_pane),
        };
        let left_x = left_pane.bounds.right();
        let right_x = right_pane.bounds.left();
        let top = left_pane.bounds.top().max(right_pane.bounds.top());
        let bottom = left_pane.bounds.bottom().min(right_pane.bounds.bottom());
        if right_x <= left_x || bottom <= top {
            return None;
        }
        let content_bounds = Bounds::from_corners(point(left_x, top), point(right_x, bottom));

        let mut ribbons = Vec::new();
        let mut buttons = Vec::new();
        for chunk in &self.chunks {
            let side_span = side_pane.span(&chunk.side_lines);
            let result_span = result_pane.span(&chunk.result_lines);
            let (left_span, right_span) = match self.side {
                Side::Left => (side_span.clone(), result_span.clone()),
                Side::Right => (result_span.clone(), side_span.clone()),
            };
            let above = left_span.end < top && right_span.end < top;
            let below = left_span.start > bottom && right_span.start > bottom;
            if above || below {
                continue;
            }
            ribbons.push(Ribbon {
                left_x,
                right_x,
                left: left_span,
                right: right_span,
                color: chunk.kind.color(cx).opacity(RIBBON_OPACITY),
            });

            let side_buttons = self.render_side_buttons(chunk);
            let (button_x, align_right) = match self.side {
                Side::Left => (left_x + BUTTON_INSET, false),
                Side::Right => (right_x - BUTTON_INSET, true),
            };
            buttons.push(Self::prepaint_button(
                side_buttons,
                button_x,
                align_right,
                side_span.start,
                side_pane.line_height,
                content_bounds,
                window,
                cx,
            ));

            if chunk.simple_conflict && self.side == Side::Left {
                let wand = self.render_wand_button(chunk.chunk_index);
                buttons.push(Self::prepaint_button(
                    wand,
                    right_x - BUTTON_INSET,
                    true,
                    result_span.start,
                    result_pane.line_height,
                    content_bounds,
                    window,
                    cx,
                ));
            }
        }

        Some(MergeDividerPrepaint {
            content_bounds,
            ribbons,
            buttons,
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
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            return;
        };
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.content_bounds,
                ..Default::default()
            }),
            |window| {
                for ribbon in &prepaint.ribbons {
                    ribbon.paint(window);
                }
                for button in &mut prepaint.buttons {
                    button.paint(window, cx);
                }
            },
        );
    }
}
