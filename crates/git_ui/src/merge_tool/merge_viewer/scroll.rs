use editor::{
    Bias, DisplayPoint, Editor,
    display_map::{DisplayRow, DisplaySnapshot},
};
use gpui::{App, Context, Entity, Point, Window, point};
use language::Point as TextPoint;
use merge_diff::ThreeSide;

use super::{
    MergeViewer,
    scroll_sync::{
        PaneScrollMetrics, RowMapping, ScrollSide, SyncScrollable, VerticalSyncInput,
        should_skip_sub_line_jitter, sync_vertical_scroll, target_offsets,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PanePair {
    LeftBase,
    BaseRight,
}

impl PanePair {
    fn sides(self) -> (ThreeSide, ThreeSide) {
        match self {
            Self::LeftBase => (ThreeSide::Left, ThreeSide::Base),
            Self::BaseRight => (ThreeSide::Base, ThreeSide::Right),
        }
    }

    fn master_side(self, master: ThreeSide) -> ScrollSide {
        match self {
            Self::LeftBase => {
                if master == ThreeSide::Left {
                    ScrollSide::Left
                } else {
                    ScrollSide::Right
                }
            }
            Self::BaseRight => {
                if master == ThreeSide::Base {
                    ScrollSide::Left
                } else {
                    ScrollSide::Right
                }
            }
        }
    }
}

pub(super) struct PaneRows {
    snapshot: DisplaySnapshot,
}

impl PaneRows {
    pub(super) fn capture(editor: &Entity<Editor>, cx: &mut App) -> Self {
        Self {
            snapshot: editor.update(cx, |editor, cx| editor.display_snapshot(cx)),
        }
    }

    pub(super) fn line_count(&self) -> i64 {
        i64::from(self.snapshot.buffer_snapshot().max_point().row) + 1
    }

    pub(super) fn total_rows(&self) -> i64 {
        i64::from(self.snapshot.max_point().row().0) + 1
    }

    pub(super) fn visual_row_of_line(&self, line: i64) -> i64 {
        let max_row = i64::from(self.snapshot.buffer_snapshot().max_point().row);
        if line > max_row {
            return self.total_rows();
        }
        let point = TextPoint::new(line.max(0) as u32, 0);
        i64::from(
            self.snapshot
                .point_to_display_point(point, Bias::Left)
                .row()
                .0,
        )
    }

    pub(super) fn line_of_visual_row(&self, row: i64) -> i64 {
        let display_point = DisplayPoint::new(DisplayRow(row.max(0) as u32), 0);
        i64::from(
            self.snapshot
                .display_point_to_point(display_point, Bias::Left)
                .row,
        )
    }
}

struct PairRowMapping {
    master: PaneRows,
    slave: PaneRows,
}

impl RowMapping for PairRowMapping {
    fn master_visual_row_to_logical_line(&self, visual_row: i64) -> i64 {
        self.master.line_of_visual_row(visual_row)
    }

    fn master_logical_line_to_visual_row(&self, line: i64) -> i64 {
        self.master.visual_row_of_line(line)
    }

    fn slave_logical_line_to_visual_row(&self, line: i64) -> i64 {
        self.slave.visual_row_of_line(line)
    }
}

impl MergeViewer {
    pub(super) fn editor_line_count(&self, side: ThreeSide, cx: &mut App) -> i64 {
        PaneRows::capture(self.editor(side), cx).line_count()
    }

    pub(super) fn pair_scrollable(&self, pair: PanePair, cx: &mut App) -> SyncScrollable {
        let (left, right) = pair.sides();
        let model = self.model.read(cx);
        let mut boundaries = vec![(0, 0)];
        for change in model.changes() {
            let index = change.index();
            boundaries.push((
                model.start_line(index, left) as i64,
                model.start_line(index, right) as i64,
            ));
            boundaries.push((
                model.end_line(index, left) as i64,
                model.end_line(index, right) as i64,
            ));
        }
        boundaries.push((
            self.editor_line_count(left, cx),
            self.editor_line_count(right, cx),
        ));
        SyncScrollable::new(boundaries)
    }

    pub(super) fn transfer_line_between(
        &self,
        from: ThreeSide,
        to: ThreeSide,
        line: i64,
        cx: &mut App,
    ) -> i64 {
        if from == to {
            return line;
        }
        let base_line = match from {
            ThreeSide::Left => self
                .pair_scrollable(PanePair::LeftBase, cx)
                .transfer(ScrollSide::Left, line),
            ThreeSide::Right => self
                .pair_scrollable(PanePair::BaseRight, cx)
                .transfer(ScrollSide::Right, line),
            ThreeSide::Base => line,
        };
        match to {
            ThreeSide::Left => self
                .pair_scrollable(PanePair::LeftBase, cx)
                .transfer(ScrollSide::Right, base_line),
            ThreeSide::Right => self
                .pair_scrollable(PanePair::BaseRight, cx)
                .transfer(ScrollSide::Left, base_line),
            ThreeSide::Base => base_line,
        }
    }

    pub(super) fn pane_scroll_position(&self, side: ThreeSide, cx: &mut App) -> Point<f64> {
        self.editor(side)
            .update(cx, |editor, cx| editor.scroll_position(cx))
    }

    pub(super) fn set_pane_scroll_position(
        &mut self,
        side: ThreeSide,
        position: Point<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor(side).clone();
        let (changed, resulting) = editor.update(cx, |editor, cx| {
            let before = editor.scroll_position(cx);
            editor.set_scroll_position(position, window, cx);
            let after = editor.scroll_position(cx);
            (after != before, after)
        });
        if changed {
            self.programmatic_scrolls[side.index()] += 1;
        }
        self.last_scroll_positions[side.index()] = resulting;
    }

    pub(super) fn on_pane_scrolled(
        &mut self,
        side: ThreeSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = side.index();
        let current = self.pane_scroll_position(side, cx);
        let previous = self.last_scroll_positions[index];
        self.last_scroll_positions[index] = current;
        cx.notify();
        if self.programmatic_scrolls[index] > 0 {
            self.programmatic_scrolls[index] -= 1;
            return;
        }
        if !self.sync_scroll_enabled {
            return;
        }
        let horizontal_changed = current.x != previous.x;
        let vertical_changed = current.y != previous.y;
        let hops: [(ThreeSide, ThreeSide); 2] = match side {
            ThreeSide::Left => [
                (ThreeSide::Left, ThreeSide::Base),
                (ThreeSide::Base, ThreeSide::Right),
            ],
            ThreeSide::Base => [
                (ThreeSide::Base, ThreeSide::Left),
                (ThreeSide::Base, ThreeSide::Right),
            ],
            ThreeSide::Right => [
                (ThreeSide::Right, ThreeSide::Base),
                (ThreeSide::Base, ThreeSide::Left),
            ],
        };
        for (master, slave) in hops {
            self.sync_slave(
                master,
                slave,
                horizontal_changed,
                vertical_changed,
                window,
                cx,
            );
        }
    }

    fn sync_slave(
        &mut self,
        master: ThreeSide,
        slave: ThreeSide,
        horizontal_changed: bool,
        vertical_changed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pair = match (master, slave) {
            (ThreeSide::Left, ThreeSide::Base) | (ThreeSide::Base, ThreeSide::Left) => {
                PanePair::LeftBase
            }
            _ => PanePair::BaseRight,
        };
        let master_position = self.pane_scroll_position(master, cx);
        let slave_position = self.pane_scroll_position(slave, cx);
        let mut target = slave_position;
        if horizontal_changed {
            target.x = master_position.x;
        }
        if vertical_changed {
            let visible_rows = self
                .editor(master)
                .read(cx)
                .visible_line_count()
                .unwrap_or_default();
            let scrollable = self.pair_scrollable(pair, cx);
            let mapping = PairRowMapping {
                master: PaneRows::capture(self.editor(master), cx),
                slave: PaneRows::capture(self.editor(slave), cx),
            };
            let output = sync_vertical_scroll(&VerticalSyncInput {
                scrollable: &scrollable,
                master_side: pair.master_side(master),
                master_scroll_top_rows: master_position.y,
                master_visible_rows: visible_rows,
                mapping: &mapping,
            });
            if !should_skip_sub_line_jitter(
                output.slave_scroll_top_rows,
                slave_position.y,
                output.only_major_forward,
                output.only_major_backward,
            ) {
                target.y = output.slave_scroll_top_rows;
            }
        }
        if target != slave_position {
            self.set_pane_scroll_position(slave, target, window, cx);
        }
    }

    pub(super) fn make_range_visible(
        &mut self,
        master: ThreeSide,
        starts: [usize; 3],
        ends: [usize; 3],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let metrics = ThreeSide::ALL.map(|side| {
            let rows = PaneRows::capture(self.editor(side), cx);
            let visible_rows = self
                .editor(side)
                .read(cx)
                .visible_line_count()
                .unwrap_or_default();
            PaneScrollMetrics {
                top_row_of_start_line: rows.visual_row_of_line(starts[side.index()] as i64) as f64,
                bottom_row_of_end_line_plus_one: rows
                    .visual_row_of_line(ends[side.index()] as i64 + 1)
                    as f64,
                visible_rows,
                maximum_scroll_top_rows: (rows.total_rows() as f64 - visible_rows).max(0.0),
            }
        });
        let offsets = target_offsets(metrics);
        for side in ThreeSide::ALL {
            let current = self.pane_scroll_position(side, cx);
            let horizontal = if side == master || self.sync_scroll_enabled {
                0.0
            } else {
                current.x
            };
            self.set_pane_scroll_position(
                side,
                point(horizontal, offsets[side.index()]),
                window,
                cx,
            );
        }
    }
}
