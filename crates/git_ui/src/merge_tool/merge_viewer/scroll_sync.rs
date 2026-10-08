#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LineRange {
    pub start1: i64,
    pub end1: i64,
    pub start2: i64,
    pub end2: i64,
}

pub(crate) fn transfer_line(line: i64, range: &LineRange) -> i64 {
    if range.start1 == line {
        return range.start2;
    }
    if range.end1 == line {
        return range.end2;
    }
    if range.end1 < line {
        return (line - range.end1) + range.end2;
    }
    (range.start2 + (line - range.start1)).min(range.end2)
}

pub(crate) fn identity_range(line: i64) -> LineRange {
    LineRange {
        start1: line,
        end1: line + 1,
        start2: line,
        end2: line + 1,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScrollSide {
    Left,
    Right,
}

impl ScrollSide {
    fn select(self, left: i64, right: i64) -> i64 {
        match self {
            ScrollSide::Left => left,
            ScrollSide::Right => right,
        }
    }
}

pub(crate) struct SyncScrollable {
    boundaries: Vec<(i64, i64)>,
}

impl SyncScrollable {
    pub(crate) fn new(boundaries: Vec<(i64, i64)>) -> Self {
        Self { boundaries }
    }

    pub(crate) fn range(&self, master: ScrollSide, line: i64) -> LineRange {
        if line < 0 {
            return identity_range(line);
        }

        let mut left1 = 0;
        let mut left2 = 0;
        let mut right1 = 0;
        let mut right2 = 0;
        for &(left, right) in &self.boundaries {
            left1 = left2;
            right1 = right2;
            left2 = left;
            right2 = right;
            if line <= master.select(left, right) {
                break;
            }
        }

        LineRange {
            start1: master.select(left1, right1),
            end1: master.select(left2, right2),
            start2: master.select(right1, left1),
            end2: master.select(right2, left2),
        }
    }

    pub(crate) fn transfer(&self, master: ScrollSide, line: i64) -> i64 {
        transfer_line(line, &self.range(master, line))
    }
}

pub(crate) trait RowMapping {
    fn master_visual_row_to_logical_line(&self, visual_row: i64) -> i64;
    fn master_logical_line_to_visual_row(&self, line: i64) -> i64;
    fn slave_logical_line_to_visual_row(&self, line: i64) -> i64;
}

pub(crate) struct VerticalSyncInput<'a> {
    pub scrollable: &'a SyncScrollable,
    pub master_side: ScrollSide,
    pub master_scroll_top_rows: f64,
    pub master_visible_rows: f64,
    pub mapping: &'a dyn RowMapping,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct VerticalSyncOutput {
    pub slave_scroll_top_rows: f64,
    pub only_major_forward: bool,
    pub only_major_backward: bool,
}

pub(crate) fn transfer_visual_row(input: &VerticalSyncInput, master_visual_row: i64) -> i64 {
    let mapping = input.mapping;
    let master_center_line = mapping.master_visual_row_to_logical_line(master_visual_row);
    let range = input
        .scrollable
        .range(input.master_side, master_center_line);

    let master_start = mapping.master_logical_line_to_visual_row(range.start1);
    let master_end = if range.start1 == range.end1 {
        master_start
    } else {
        mapping.master_logical_line_to_visual_row(range.end1)
    };
    let slave_start = mapping.slave_logical_line_to_visual_row(range.start2);
    let slave_end = if range.start2 == range.end2 {
        slave_start
    } else {
        mapping.slave_logical_line_to_visual_row(range.end2)
    };

    transfer_line(
        master_visual_row,
        &LineRange {
            start1: master_start,
            end1: master_end,
            start2: slave_start,
            end2: slave_end,
        },
    )
}

pub(crate) fn sync_vertical_scroll(input: &VerticalSyncInput) -> VerticalSyncOutput {
    let middle = input.master_visible_rows / 3.0;
    let master_offset = input.master_scroll_top_rows + middle;
    let master_visual_line = master_offset.floor() as i64;
    let converted_visual_line = transfer_visual_row(input, master_visual_line);
    let slave_offset = converted_visual_line as f64;
    let master_offset_raw = master_visual_line as f64;
    let correction = (master_offset - master_offset_raw) % 1.0;

    let only_major_backward = correction < 0.5
        && master_visual_line > 0
        && converted_visual_line == transfer_visual_row(input, master_visual_line - 1);
    let only_major_forward = correction > 0.5
        && converted_visual_line == transfer_visual_row(input, master_visual_line + 1);

    VerticalSyncOutput {
        slave_scroll_top_rows: slave_offset - middle + correction,
        only_major_forward,
        only_major_backward,
    }
}

pub(crate) fn should_skip_sub_line_jitter(
    target_top_rows: f64,
    current_top_rows: f64,
    only_major_forward: bool,
    only_major_backward: bool,
) -> bool {
    let moves_in_guarded_direction = (only_major_forward && target_top_rows > current_top_rows)
        || (only_major_backward && target_top_rows < current_top_rows);
    moves_in_guarded_direction && (target_top_rows - current_top_rows).abs() < 1.0
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct PaneScrollMetrics {
    pub top_row_of_start_line: f64,
    pub bottom_row_of_end_line_plus_one: f64,
    pub visible_rows: f64,
    pub maximum_scroll_top_rows: f64,
}

const GAP_ROWS: f64 = 2.0;

pub(crate) fn target_offsets(panes: [PaneScrollMetrics; 3]) -> [f64; 3] {
    let range_heights =
        panes.map(|pane| pane.bottom_row_of_end_line_plus_one - pane.top_row_of_start_line);

    let top_shifts: [f64; 3] = std::array::from_fn(|index| {
        let pane = &panes[index];
        let can_show = 2.0 * GAP_ROWS + range_heights[index] <= pane.visible_rows;
        let preferred_shift = pane.visible_rows / 3.0;
        if can_show {
            (pane.visible_rows - GAP_ROWS - range_heights[index]).min(preferred_shift)
        } else {
            GAP_ROWS
        }
    });

    let smallest_top_offset = panes
        .iter()
        .map(|pane| pane.top_row_of_start_line)
        .fold(f64::INFINITY, f64::min);
    let top_shift = top_shifts
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .min(smallest_top_offset);

    let offsets: [f64; 3] = panes.map(|pane| pane.top_row_of_start_line - top_shift);
    let have_enough_space =
        (0..3).all(|index| panes[index].maximum_scroll_top_rows > offsets[index]);
    if have_enough_space {
        return offsets;
    }

    let fallback_shift = panes.iter().fold(0.0_f64, |shift, pane| {
        (pane.top_row_of_start_line - pane.maximum_scroll_top_rows).max(shift)
    });

    std::array::from_fn(|index| {
        let pane = &panes[index];
        let extra = (fallback_shift + range_heights[index] + GAP_ROWS - pane.visible_rows).max(0.0);
        let offset = pane.top_row_of_start_line - fallback_shift + extra;
        offset.min(pane.top_row_of_start_line - GAP_ROWS)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct IdentityMapping;

    impl RowMapping for IdentityMapping {
        fn master_visual_row_to_logical_line(&self, visual_row: i64) -> i64 {
            visual_row
        }

        fn master_logical_line_to_visual_row(&self, line: i64) -> i64 {
            line
        }

        fn slave_logical_line_to_visual_row(&self, line: i64) -> i64 {
            line
        }
    }

    struct FoldedPane {
        first_line: i64,
        last_line: i64,
    }

    impl FoldedPane {
        fn hidden_lines(&self) -> i64 {
            self.last_line - self.first_line
        }

        fn visual_row_to_logical_line(&self, visual_row: i64) -> i64 {
            if visual_row <= self.first_line {
                visual_row
            } else {
                visual_row + self.hidden_lines()
            }
        }

        fn logical_line_to_visual_row(&self, line: i64) -> i64 {
            if line <= self.first_line {
                line
            } else if line <= self.last_line {
                self.first_line
            } else {
                line - self.hidden_lines()
            }
        }
    }

    struct FoldedPairMapping {
        master: FoldedPane,
        slave: FoldedPane,
    }

    impl RowMapping for FoldedPairMapping {
        fn master_visual_row_to_logical_line(&self, visual_row: i64) -> i64 {
            self.master.visual_row_to_logical_line(visual_row)
        }

        fn master_logical_line_to_visual_row(&self, line: i64) -> i64 {
            self.master.logical_line_to_visual_row(line)
        }

        fn slave_logical_line_to_visual_row(&self, line: i64) -> i64 {
            self.slave.logical_line_to_visual_row(line)
        }
    }

    fn two_change_scrollable() -> SyncScrollable {
        SyncScrollable::new(vec![(0, 0), (2, 2), (4, 6), (7, 9)])
    }

    fn pane(top: f64, bottom: f64, visible: f64, maximum: f64) -> PaneScrollMetrics {
        PaneScrollMetrics {
            top_row_of_start_line: top,
            bottom_row_of_end_line_plus_one: bottom,
            visible_rows: visible,
            maximum_scroll_top_rows: maximum,
        }
    }

    #[test]
    fn merge_tool_scroll_sync_transfer_line_covers_four_branches() {
        let range = LineRange {
            start1: 2,
            end1: 6,
            start2: 10,
            end2: 20,
        };
        assert_eq!(transfer_line(2, &range), 10);
        assert_eq!(transfer_line(6, &range), 20);
        assert_eq!(transfer_line(8, &range), 22);
        assert_eq!(transfer_line(4, &range), 12);

        let shrinking = LineRange {
            start1: 2,
            end1: 6,
            start2: 10,
            end2: 12,
        };
        assert_eq!(transfer_line(5, &shrinking), 12);
    }

    #[test]
    fn merge_tool_scroll_sync_identity_range_spans_one_line() {
        assert_eq!(
            identity_range(5),
            LineRange {
                start1: 5,
                end1: 6,
                start2: 5,
                end2: 6
            }
        );
        assert_eq!(
            two_change_scrollable().range(ScrollSide::Left, -3),
            identity_range(-3)
        );
    }

    #[test]
    fn merge_tool_scroll_sync_range_with_left_master() {
        let scrollable = two_change_scrollable();
        assert_eq!(
            scrollable.range(ScrollSide::Left, 3),
            LineRange {
                start1: 2,
                end1: 4,
                start2: 2,
                end2: 6
            }
        );
        assert_eq!(scrollable.transfer(ScrollSide::Left, 3), 3);
    }

    #[test]
    fn merge_tool_scroll_sync_range_with_right_master() {
        let scrollable = two_change_scrollable();
        assert_eq!(
            scrollable.range(ScrollSide::Right, 5),
            LineRange {
                start1: 2,
                end1: 6,
                start2: 2,
                end2: 4
            }
        );
        assert_eq!(scrollable.transfer(ScrollSide::Right, 5), 4);
    }

    #[test]
    fn merge_tool_scroll_sync_range_ends_at_the_first_boundary_not_below_the_line() {
        let scrollable = two_change_scrollable();
        assert_eq!(
            scrollable.range(ScrollSide::Left, 4),
            LineRange {
                start1: 2,
                end1: 4,
                start2: 2,
                end2: 6
            }
        );
        assert_eq!(
            scrollable.range(ScrollSide::Right, 6),
            LineRange {
                start1: 2,
                end1: 6,
                start2: 2,
                end2: 4
            }
        );
        assert_eq!(
            scrollable.range(ScrollSide::Left, 0),
            LineRange {
                start1: 0,
                end1: 0,
                start2: 0,
                end2: 0
            }
        );
    }

    #[test]
    fn merge_tool_scroll_sync_range_past_last_boundary_keeps_last_pair() {
        let scrollable = two_change_scrollable();
        assert_eq!(
            scrollable.range(ScrollSide::Left, 8),
            LineRange {
                start1: 4,
                end1: 7,
                start2: 6,
                end2: 9
            }
        );
        assert_eq!(scrollable.transfer(ScrollSide::Left, 8), 10);
    }

    #[test]
    fn merge_tool_scroll_sync_identity_mapping_keeps_slave_on_master_top() {
        let scrollable = SyncScrollable::new(vec![(0, 0), (100, 100)]);
        let input = VerticalSyncInput {
            scrollable: &scrollable,
            master_side: ScrollSide::Left,
            master_scroll_top_rows: 10.0,
            master_visible_rows: 30.0,
            mapping: &IdentityMapping,
        };
        let output = sync_vertical_scroll(&input);
        assert_eq!(output.slave_scroll_top_rows, 10.0);
        assert!(!output.only_major_forward);
        assert!(!output.only_major_backward);
        assert_eq!(transfer_visual_row(&input, 20), 20);
    }

    #[test]
    fn merge_tool_scroll_sync_folded_panes_transfer_rows_through_logical_lines() {
        let scrollable = SyncScrollable::new(vec![(0, 0), (8, 10), (12, 12), (20, 20)]);
        let mapping = FoldedPairMapping {
            master: FoldedPane {
                first_line: 3,
                last_line: 6,
            },
            slave: FoldedPane {
                first_line: 9,
                last_line: 11,
            },
        };
        let input = VerticalSyncInput {
            scrollable: &scrollable,
            master_side: ScrollSide::Left,
            master_scroll_top_rows: 6.25,
            master_visible_rows: 6.0,
            mapping: &mapping,
        };
        assert_eq!(transfer_visual_row(&input, 5), 9);
        assert_eq!(transfer_visual_row(&input, 6), 10);
        assert_eq!(transfer_visual_row(&input, 8), 10);
        let output = sync_vertical_scroll(&input);
        assert_eq!(output.slave_scroll_top_rows, 8.25);
        assert!(output.only_major_backward);
        assert!(!output.only_major_forward);
    }

    fn collapsing_scrollable() -> SyncScrollable {
        SyncScrollable::new(vec![(0, 0), (10, 10), (20, 10), (30, 20)])
    }

    #[test]
    fn merge_tool_scroll_sync_collapsed_change_sets_only_major_forward() {
        let scrollable = collapsing_scrollable();
        let input = VerticalSyncInput {
            scrollable: &scrollable,
            master_side: ScrollSide::Left,
            master_scroll_top_rows: 5.75,
            master_visible_rows: 30.0,
            mapping: &IdentityMapping,
        };
        assert_eq!(transfer_visual_row(&input, 15), 10);
        let output = sync_vertical_scroll(&input);
        assert_eq!(output.slave_scroll_top_rows, 0.75);
        assert!(output.only_major_forward);
        assert!(!output.only_major_backward);
    }

    #[test]
    fn merge_tool_scroll_sync_collapsed_change_sets_only_major_backward() {
        let scrollable = collapsing_scrollable();
        let input = VerticalSyncInput {
            scrollable: &scrollable,
            master_side: ScrollSide::Left,
            master_scroll_top_rows: 5.25,
            master_visible_rows: 30.0,
            mapping: &IdentityMapping,
        };
        let output = sync_vertical_scroll(&input);
        assert_eq!(output.slave_scroll_top_rows, 0.25);
        assert!(output.only_major_backward);
        assert!(!output.only_major_forward);
    }

    #[test]
    fn merge_tool_scroll_sync_jitter_skipped_only_in_guarded_direction() {
        assert!(should_skip_sub_line_jitter(10.5, 10.0, true, false));
        assert!(!should_skip_sub_line_jitter(9.5, 10.0, true, false));
        assert!(should_skip_sub_line_jitter(9.5, 10.0, false, true));
        assert!(!should_skip_sub_line_jitter(10.5, 10.0, false, true));
        assert!(!should_skip_sub_line_jitter(10.5, 10.0, false, false));
        assert!(!should_skip_sub_line_jitter(11.0, 10.0, true, false));
    }

    #[test]
    fn merge_tool_scroll_sync_target_offsets_when_range_can_be_shown() {
        let offsets = target_offsets([
            pane(30.0, 35.0, 30.0, 100.0),
            pane(6.0, 8.0, 30.0, 100.0),
            pane(30.0, 35.0, 30.0, 100.0),
        ]);
        assert_eq!(offsets, [24.0, 0.0, 24.0]);
    }

    #[test]
    fn merge_tool_scroll_sync_target_offsets_uniform_panes_use_third_of_height() {
        let offsets = target_offsets([pane(30.0, 35.0, 30.0, 100.0); 3]);
        assert_eq!(offsets, [20.0, 20.0, 20.0]);
    }

    #[test]
    fn merge_tool_scroll_sync_target_offsets_range_without_room_for_gaps_keeps_two_rows_above() {
        let fits_with_gaps = target_offsets([pane(10.0, 25.0, 20.0, 100.0); 3]);
        assert_eq!(fits_with_gaps, [7.0, 7.0, 7.0]);
        let too_tall_for_gaps = target_offsets([pane(10.0, 27.0, 20.0, 100.0); 3]);
        assert_eq!(too_tall_for_gaps, [8.0, 8.0, 8.0]);
    }

    #[test]
    fn merge_tool_scroll_sync_target_offsets_fall_back_without_enough_space() {
        let offsets = target_offsets([
            pane(30.0, 35.0, 20.0, 15.0),
            pane(20.0, 22.0, 20.0, 100.0),
            pane(40.0, 50.0, 8.0, 36.0),
        ]);
        assert_eq!(offsets, [17.0, 5.0, 38.0]);
    }
}
