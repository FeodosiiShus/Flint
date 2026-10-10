use std::ops::Range;

use editor::{
    Bias, DisplayPoint, Editor, LastTextMetrics,
    display_map::{DisplayRow, DisplaySnapshot},
    movement::TextLayoutDetails,
};
use gpui::{App, Bounds, Entity, Pixels, Window};
use language::Point;
use multi_buffer::MultiBufferRow;

pub(crate) struct PaneGeometry {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) line_height: f32,
    pub(crate) metrics: LastTextMetrics,
    scroll_top: f64,
    snapshot: DisplaySnapshot,
    text_layout_details: TextLayoutDetails,
}

impl PaneGeometry {
    pub(crate) fn measure(
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        editor.update(cx, |editor, cx| {
            let bounds = *editor.last_bounds()?;
            let metrics = editor.last_text_metrics()?;
            let line_height = f32::from(metrics.line_height);
            if line_height <= 0.0 {
                return None;
            }
            let text_layout_details = editor.text_layout_details(window, cx);
            let snapshot = editor.snapshot(window, cx);
            Some(Self {
                bounds,
                line_height,
                metrics,
                scroll_top: snapshot.scroll_position().y,
                snapshot: snapshot.display_snapshot,
                text_layout_details,
            })
        })
    }

    pub(crate) fn top(&self) -> f32 {
        f32::from(self.bounds.top())
    }

    pub(crate) fn height(&self) -> f32 {
        f32::from(self.bounds.size.height)
    }

    fn max_buffer_row(&self) -> u32 {
        self.snapshot.buffer_snapshot().max_point().row
    }

    pub(crate) fn line_count(&self) -> u32 {
        self.max_buffer_row() + 1
    }

    fn display_row_of(&self, point: Point, bias: Bias) -> f64 {
        f64::from(self.snapshot.point_to_display_point(point, bias).row().0)
    }

    fn document_end_row(&self) -> f64 {
        let max_point = self.snapshot.buffer_snapshot().max_point();
        self.display_row_of(max_point, Bias::Left)
    }

    fn row_y(&self, row: f64) -> f32 {
        self.top() + self.line_height * (row - self.scroll_top) as f32
    }

    fn line_top_row(&self, line: u32) -> f64 {
        let max_row = self.max_buffer_row();
        if line > max_row {
            return self.document_end_row() + f64::from(line - max_row - 1);
        }
        self.display_row_of(Point::new(line, 0), Bias::Left)
    }

    fn line_bottom_row(&self, line: u32) -> f64 {
        let max_row = self.max_buffer_row();
        if line > max_row {
            return self.document_end_row() + f64::from(line - max_row);
        }
        let length = self
            .snapshot
            .buffer_snapshot()
            .line_len(MultiBufferRow(line));
        self.display_row_of(Point::new(line, length), Bias::Right) + 1.0
    }

    pub(crate) fn line_top_y(&self, line: u32) -> f32 {
        self.row_y(self.line_top_row(line))
    }

    pub(crate) fn line_bottom_y(&self, line: u32) -> f32 {
        self.row_y(self.line_bottom_row(line))
    }

    pub(crate) fn visible_display_rows(&self) -> Range<u32> {
        let first = self.scroll_top.max(0.0).floor() as u32;
        let visible = (self.height() / self.line_height).ceil() as u32 + 1;
        let last = self.snapshot.max_point().row().0;
        first..(first + visible).min(last + 1)
    }

    pub(crate) fn display_row_y(&self, row: u32) -> f32 {
        self.row_y(f64::from(row))
    }

    pub(crate) fn buffer_line_of_display_row(&self, row: u32) -> u32 {
        self.snapshot
            .display_point_to_point(DisplayPoint::new(DisplayRow(row), 0), Bias::Left)
            .row
    }

    pub(crate) fn is_range_folded(&self, lines: &Range<u32>) -> bool {
        if lines.start >= lines.end {
            return self.is_line_folded(lines.start);
        }
        self.is_line_folded(lines.start) && self.is_line_folded(lines.end - 1)
    }

    pub(crate) fn is_line_folded(&self, line: u32) -> bool {
        line <= self.max_buffer_row() && self.snapshot.is_line_folded(MultiBufferRow(line))
    }

    pub(crate) fn x_of_point(&self, point: Point) -> f32 {
        let display_point = self.snapshot.point_to_display_point(point, Bias::Left);
        let line_x = self
            .snapshot
            .x_for_display_point(display_point, &self.text_layout_details);
        f32::from(self.metrics.content_origin.x + line_x - self.metrics.scroll_x)
    }

    pub(crate) fn clamp_point(&self, point: Point) -> Point {
        let max_point = self.snapshot.buffer_snapshot().max_point();
        if point > max_point { max_point } else { point }
    }

    pub(crate) fn offset_point(&self, line: u32, byte_offset: usize) -> Point {
        let start = self.clamp_point(Point::new(line, 0));
        let snapshot = self.snapshot.buffer_snapshot();
        let start_offset = snapshot.point_to_offset(start);
        let target =
            editor::MultiBufferOffset((start_offset.0 + byte_offset).min(snapshot.len().0));
        snapshot.offset_to_point(target)
    }
}
