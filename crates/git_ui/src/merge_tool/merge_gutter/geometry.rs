use std::ops::Range;

use crate::merge_tool::palette::highlights::PaneChange;

pub(crate) const ICON_SIZE: f32 = 14.0;
pub(crate) const FOLD_CHEVRON_SIZE: f32 = 9.0;
pub(crate) const ICON_GAP: f32 = 3.0;
pub(crate) const ICON_LEFT_INSET: f32 = 2.0;
pub(crate) const ANNOTATION_PLACEHOLDER_WIDTH: f32 = 4.0;
pub(crate) const GAP_BEFORE_LINE_NUMBERS: f32 = 4.0;
pub(crate) const MIN_LINE_NUMBER_WIDTH: f32 = 14.0;
pub(crate) const GAP_AFTER_LINE_NUMBERS: f32 = 4.0;
pub(crate) const GAP_AFTER_ICONS: f32 = 0.0;
pub(crate) const FOLDING_AREA_WIDTH: f32 = 11.0;
pub(crate) const VCS_LANE_WIDTH: f32 = 8.0;
pub(crate) const TRAILING_GAP: f32 = 1.0;
pub(crate) const OUTLINE_STRIP_WIDTH: f32 = 3.0;
pub(crate) const DOTTED_STEP: i32 = 4;
pub(crate) const EMPTY_MARKER_MAX_HEIGHT: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PixelRect {
    pub(crate) fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn mirrored(self, container_width: f32) -> Self {
        Self {
            x: container_width - self.x - self.width,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FillRole {
    Solid,
    Faded,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoledRect {
    pub rect: PixelRect,
    pub role: FillRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkerBackground {
    None,
    Default,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MarkerMode {
    pub background: MarkerBackground,
    pub dotted_border: bool,
}

impl MarkerMode {
    pub(crate) const DEFAULT: MarkerMode = MarkerMode {
        background: MarkerBackground::Default,
        dotted_border: false,
    };
    pub(crate) const IGNORED: MarkerMode = MarkerMode {
        background: MarkerBackground::Ignored,
        dotted_border: false,
    };
    pub(crate) const RESOLVED: MarkerMode = MarkerMode {
        background: MarkerBackground::None,
        dotted_border: true,
    };

    pub(crate) fn gutter_for(change: &PaneChange) -> MarkerMode {
        if change.resolved {
            MarkerMode::RESOLVED
        } else {
            MarkerMode::DEFAULT
        }
    }

    pub(crate) fn editor_for(change: &PaneChange) -> MarkerMode {
        if change.resolved {
            MarkerMode::RESOLVED
        } else if change.word_diff_ready {
            MarkerMode::IGNORED
        } else {
            MarkerMode::DEFAULT
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GutterLayout {
    pub width: f32,
    pub line_numbers: Range<f32>,
    pub icons: Range<f32>,
    pub folding: Range<f32>,
    pub vcs_lane: Range<f32>,
    pub separator: f32,
}

pub(crate) fn icons_area_width(max_icons_per_row: usize) -> f32 {
    if max_icons_per_row == 0 {
        return 0.0;
    }
    let count = max_icons_per_row as f32;
    let row_width = 1.0 + count * ICON_SIZE + ICON_GAP * (count - 1.0);
    row_width + 1.0
}

pub(crate) fn gutter_layout(
    line_number_text_width: f32,
    icons_area: f32,
    show_line_numbers: bool,
) -> GutterLayout {
    let (numbers_start, numbers_end, icons_start) = if show_line_numbers {
        let numbers_width = line_number_text_width.max(MIN_LINE_NUMBER_WIDTH);
        let start = ANNOTATION_PLACEHOLDER_WIDTH + GAP_BEFORE_LINE_NUMBERS;
        let end = start + numbers_width;
        (start, end, end + GAP_AFTER_LINE_NUMBERS)
    } else {
        (
            ANNOTATION_PLACEHOLDER_WIDTH,
            ANNOTATION_PLACEHOLDER_WIDTH,
            ANNOTATION_PLACEHOLDER_WIDTH,
        )
    };
    let icons_end = icons_start + icons_area;
    let folding_start = icons_end + GAP_AFTER_ICONS;
    let folding_end = folding_start + FOLDING_AREA_WIDTH;
    let vcs_end = folding_end + VCS_LANE_WIDTH;
    let width = vcs_end + TRAILING_GAP;
    GutterLayout {
        width,
        line_numbers: numbers_start..numbers_end,
        icons: icons_start..icons_end,
        folding: folding_start..folding_end,
        vcs_lane: folding_end..vcs_end,
        separator: width - OUTLINE_STRIP_WIDTH,
    }
}

pub(crate) fn icon_left(layout: &GutterLayout, index: usize) -> f32 {
    layout.icons.start + ICON_LEFT_INSET + index as f32 * (ICON_SIZE + ICON_GAP)
}

pub(crate) fn icon_top_offset(line_height: f32, ascent: f32) -> f32 {
    let centered = ((line_height - ICON_SIZE) / 2.0).trunc();
    centered.max(ascent.trunc() - ICON_SIZE)
}

pub(crate) fn fold_chevron_top(line_top: f32, line_height: f32) -> f32 {
    (line_top + (line_height - FOLD_CHEVRON_SIZE) / 2.0 + 0.5).floor()
}

pub(crate) fn line_number_left(layout: &GutterLayout, mirrored: bool, text_width: f32) -> f32 {
    if mirrored {
        layout.width - layout.line_numbers.end - 1.0
    } else {
        layout.line_numbers.end - text_width
    }
}

pub(crate) fn marker_paint_range(
    lines: &Range<u32>,
    line_top: impl Fn(u32) -> f32,
    line_bottom: impl Fn(u32) -> f32,
) -> (f32, f32) {
    if lines.start >= lines.end {
        let y = if lines.start == 0 {
            line_top(0) + 1.0
        } else {
            line_bottom(lines.start - 1)
        };
        (y, y)
    } else {
        (line_top(lines.start), line_bottom(lines.end - 1))
    }
}

fn solid_line_rect(x1: f32, x2: f32, y: f32, rows: f32) -> RoledRect {
    RoledRect {
        rect: PixelRect::new(x1, y, x2 + 1.0 - x1, rows),
        role: FillRole::Solid,
    }
}

pub(crate) fn dotted_line_rects(x1: i32, x2: i32, y: i32) -> Vec<RoledRect> {
    let correction = if x1 % DOTTED_STEP < DOTTED_STEP - 1 {
        0
    } else {
        1
    };
    let mut rects = Vec::new();
    let mut dot_x = (x1 / DOTTED_STEP + correction) * DOTTED_STEP;
    while dot_x < x2 {
        rects.push(RoledRect {
            rect: PixelRect::new(dot_x as f32, y as f32, 2.0, 2.0),
            role: FillRole::Solid,
        });
        dot_x += DOTTED_STEP;
    }
    rects
}

pub(crate) fn chunk_border_line_rects(
    x1: f32,
    x2: f32,
    y: f32,
    double_line: bool,
    dotted: bool,
) -> Vec<RoledRect> {
    let (start, end, row) = (x1 as i32, x2 as i32, y as i32);
    match (dotted, double_line) {
        (true, true) => {
            let mut rects = dotted_line_rects(start, end, row - 1);
            rects.extend(dotted_line_rects(start, end, row));
            rects
        }
        (true, false) => dotted_line_rects(start, end, row - 1),
        (false, true) => vec![solid_line_rect(x1, x2, y, 2.0)],
        (false, false) => vec![solid_line_rect(x1, x2, y, 1.0)],
    }
}

pub(crate) fn marker_rects(x1: f32, x2: f32, y1: f32, y2: f32, mode: MarkerMode) -> Vec<RoledRect> {
    if x1 >= x2 {
        return Vec::new();
    }
    let is_empty = y2 - y1 <= EMPTY_MARKER_MAX_HEIGHT;
    if is_empty {
        return chunk_border_line_rects(x1, x2, y1 - 1.0, true, mode.dotted_border);
    }
    let mut rects = Vec::new();
    let role = match mode.background {
        MarkerBackground::None => None,
        MarkerBackground::Default => Some(FillRole::Solid),
        MarkerBackground::Ignored => Some(FillRole::Faded),
    };
    if let Some(role) = role {
        rects.push(RoledRect {
            rect: PixelRect::new(x1, y1, x2 - x1, y2 - y1),
            role,
        });
    }
    if mode.dotted_border {
        rects.extend(chunk_border_line_rects(x1, x2, y1, false, true));
        rects.extend(chunk_border_line_rects(x1, x2, y2 - 1.0, false, true));
    }
    rects
}

pub(crate) fn gutter_band_rects(
    layout: &GutterLayout,
    change: &PaneChange,
    y1: f32,
    y2: f32,
    show_line_numbers: bool,
) -> Vec<RoledRect> {
    let gutter_mode = MarkerMode::gutter_for(change);
    let editor_mode = MarkerMode::editor_for(change);
    let start_x = if change.hide_without_line_numbers && !show_line_numbers {
        layout.separator
    } else {
        0.0
    };
    if gutter_mode == editor_mode {
        return marker_rects(start_x, layout.width, y1, y2, gutter_mode);
    }
    let mut rects = marker_rects(layout.separator, layout.width, y1, y2, editor_mode);
    rects.extend(marker_rects(start_x, layout.separator, y1, y2, gutter_mode));
    rects
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BorderLine {
    pub y: f32,
    pub double_line: bool,
    pub dotted: bool,
}

pub(crate) fn editor_border_lines(
    change: &PaneChange,
    line_top: impl Fn(u32) -> f32,
    line_bottom: impl Fn(u32) -> f32,
) -> Vec<BorderLine> {
    if change.is_empty_range() {
        let y = if change.lines.start == 0 {
            line_top(0)
        } else {
            line_bottom(change.lines.start - 1) - 1.0
        };
        return vec![BorderLine {
            y,
            double_line: true,
            dotted: change.resolved,
        }];
    }
    if change.resolved {
        return vec![
            BorderLine {
                y: line_top(change.lines.start),
                double_line: false,
                dotted: true,
            },
            BorderLine {
                y: line_bottom(change.lines.end - 1) - 1.0,
                double_line: false,
                dotted: true,
            },
        ];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge_tool::palette::ChangeKind;

    fn change(lines: Range<u32>, resolved: bool, word_diff_ready: bool) -> PaneChange {
        PaneChange {
            lines,
            kind: ChangeKind::Modified,
            resolved,
            word_diff_ready,
            ai_resolved: false,
            hide_without_line_numbers: false,
            inner_ranges: Vec::new(),
        }
    }

    fn top(line: u32) -> f32 {
        line as f32 * 20.0
    }

    fn bottom(line: u32) -> f32 {
        line as f32 * 20.0 + 20.0
    }

    #[test]
    fn merge_tool_icons_area_matches_spec_widths() {
        assert_eq!(icons_area_width(0), 0.0);
        assert_eq!(icons_area_width(1), 16.0);
        assert_eq!(icons_area_width(2), 33.0);
    }

    #[test]
    fn merge_tool_gutter_layout_matches_spec_lanes() {
        let layout = gutter_layout(10.0, 33.0, true);
        assert_eq!(layout.line_numbers, 8.0..22.0);
        assert_eq!(layout.icons, 26.0..59.0);
        assert_eq!(layout.folding, 59.0..70.0);
        assert_eq!(layout.vcs_lane, 70.0..78.0);
        assert_eq!(layout.width, 79.0);
        assert_eq!(layout.separator, 76.0);
    }

    #[test]
    fn merge_tool_gutter_layout_grows_with_wide_line_numbers() {
        let layout = gutter_layout(30.0, 16.0, true);
        assert_eq!(layout.line_numbers, 8.0..38.0);
        assert_eq!(layout.width, 32.0 + 30.0 + 16.0);
    }

    #[test]
    fn merge_tool_icon_positions_follow_icon_lane() {
        let layout = gutter_layout(14.0, 33.0, true);
        assert_eq!(icon_left(&layout, 0), layout.icons.start + 2.0);
        assert_eq!(icon_left(&layout, 1), layout.icons.start + 2.0 + 17.0);
    }

    #[test]
    fn merge_tool_fold_chevron_is_centred_in_the_line_with_half_pixel_rounding() {
        assert_eq!(fold_chevron_top(100.0, 20.0), 106.0);
        assert_eq!(fold_chevron_top(100.0, 17.0), 104.0);
        assert_eq!(fold_chevron_top(0.0, 9.0), 0.0);
    }

    #[test]
    fn merge_tool_icon_top_offset_uses_larger_of_center_and_ascent() {
        assert_eq!(icon_top_offset(20.0, 12.0), 3.0);
        assert_eq!(icon_top_offset(17.0, 16.0), 2.0);
        assert_eq!(icon_top_offset(14.0, 10.0), 0.0);
        assert_eq!(icon_top_offset(21.0, 10.0), 3.0);
    }

    #[test]
    fn merge_tool_line_numbers_align_per_mirroring() {
        let layout = gutter_layout(14.0, 33.0, true);
        assert_eq!(line_number_left(&layout, false, 6.0), 16.0);
        assert_eq!(line_number_left(&layout, true, 6.0), 56.0);
    }

    #[test]
    fn merge_tool_pixel_rect_mirrors_inside_container() {
        let rect = PixelRect::new(10.0, 5.0, 4.0, 2.0).mirrored(79.0);
        assert_eq!(rect, PixelRect::new(65.0, 5.0, 4.0, 2.0));
    }

    #[test]
    fn merge_tool_marker_range_for_non_empty_lines() {
        assert_eq!(marker_paint_range(&(2..4), top, bottom), (40.0, 80.0));
    }

    #[test]
    fn merge_tool_marker_range_for_empty_range() {
        assert_eq!(marker_paint_range(&(0..0), top, bottom), (1.0, 1.0));
        assert_eq!(marker_paint_range(&(3..3), top, bottom), (60.0, 60.0));
    }

    #[test]
    fn merge_tool_dotted_line_has_two_by_two_dots_every_four_pixels() {
        let rects = dotted_line_rects(0, 10, 5);
        let xs: Vec<f32> = rects.iter().map(|dot| dot.rect.x).collect();
        assert_eq!(xs, vec![0.0, 4.0, 8.0]);
        assert!(
            rects
                .iter()
                .all(|dot| dot.rect.width == 2.0 && dot.rect.height == 2.0 && dot.rect.y == 5.0)
        );
    }

    #[test]
    fn merge_tool_dotted_line_realigns_start_to_step() {
        let xs: Vec<f32> = dotted_line_rects(3, 13, 0)
            .iter()
            .map(|dot| dot.rect.x)
            .collect();
        assert_eq!(xs, vec![4.0, 8.0, 12.0]);
        let xs: Vec<f32> = dotted_line_rects(5, 13, 0)
            .iter()
            .map(|dot| dot.rect.x)
            .collect();
        assert_eq!(xs, vec![4.0, 8.0, 12.0]);
    }

    #[test]
    fn merge_tool_chunk_border_line_variants() {
        let single = chunk_border_line_rects(0.0, 9.0, 7.0, false, false);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].rect, PixelRect::new(0.0, 7.0, 10.0, 1.0));

        let double = chunk_border_line_rects(0.0, 9.0, 7.0, true, false);
        assert_eq!(double[0].rect, PixelRect::new(0.0, 7.0, 10.0, 2.0));

        let dotted = chunk_border_line_rects(0.0, 9.0, 7.0, false, true);
        assert!(dotted.iter().all(|dot| dot.rect.y == 6.0));

        let double_dotted = chunk_border_line_rects(0.0, 9.0, 7.0, true, true);
        let rows: Vec<f32> = double_dotted.iter().map(|dot| dot.rect.y).collect();
        assert!(rows.contains(&6.0) && rows.contains(&7.0));
    }

    #[test]
    fn merge_tool_marker_fills_background_for_non_empty_range() {
        let rects = marker_rects(0.0, 79.0, 40.0, 80.0, MarkerMode::DEFAULT);
        assert_eq!(
            rects,
            vec![RoledRect {
                rect: PixelRect::new(0.0, 40.0, 79.0, 40.0),
                role: FillRole::Solid,
            }]
        );
        let faded = marker_rects(0.0, 79.0, 40.0, 80.0, MarkerMode::IGNORED);
        assert_eq!(faded[0].role, FillRole::Faded);
    }

    #[test]
    fn merge_tool_marker_draws_two_row_line_for_empty_range() {
        let rects = marker_rects(0.0, 79.0, 60.0, 60.0, MarkerMode::DEFAULT);
        assert_eq!(
            rects,
            vec![RoledRect {
                rect: PixelRect::new(0.0, 59.0, 80.0, 2.0),
                role: FillRole::Solid,
            }]
        );
    }

    #[test]
    fn merge_tool_resolved_marker_has_no_fill_and_dotted_edges() {
        let rects = marker_rects(0.0, 79.0, 40.0, 80.0, MarkerMode::RESOLVED);
        assert!(rects.iter().all(|dot| dot.rect.width == 2.0));
        let rows: Vec<f32> = rects.iter().map(|dot| dot.rect.y).collect();
        assert!(rows.contains(&39.0));
        assert!(rows.contains(&78.0));
    }

    #[test]
    fn merge_tool_gutter_band_uses_faded_outline_strip_when_word_diff_ready() {
        let layout = gutter_layout(14.0, 33.0, true);
        let rects = gutter_band_rects(&layout, &change(2..4, false, true), 40.0, 80.0, true);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].role, FillRole::Faded);
        assert_eq!(rects[0].rect.x, layout.separator);
        assert_eq!(rects[1].role, FillRole::Solid);
        assert_eq!(rects[1].rect.width, layout.separator);
    }

    #[test]
    fn merge_tool_gutter_band_is_single_fill_without_word_diff() {
        let layout = gutter_layout(14.0, 33.0, true);
        let rects = gutter_band_rects(&layout, &change(2..4, false, false), 40.0, 80.0, true);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].rect.width, layout.width);
    }

    #[test]
    fn merge_tool_editor_borders_for_empty_unresolved_range() {
        let lines = editor_border_lines(&change(0..0, false, false), top, bottom);
        assert_eq!(
            lines,
            vec![BorderLine {
                y: 0.0,
                double_line: true,
                dotted: false
            }]
        );
        let lines = editor_border_lines(&change(3..3, false, false), top, bottom);
        assert_eq!(lines[0].y, 59.0);
        assert!(lines[0].double_line);
    }

    #[test]
    fn merge_tool_editor_borders_for_resolved_range_are_dotted_top_and_bottom() {
        let lines = editor_border_lines(&change(2..4, true, false), top, bottom);
        assert_eq!(
            lines,
            vec![
                BorderLine {
                    y: 40.0,
                    double_line: false,
                    dotted: true
                },
                BorderLine {
                    y: 79.0,
                    double_line: false,
                    dotted: true
                },
            ]
        );
    }

    #[test]
    fn merge_tool_editor_borders_absent_for_unresolved_non_empty_range() {
        assert!(editor_border_lines(&change(2..4, false, false), top, bottom).is_empty());
    }

    #[test]
    fn merge_tool_editor_borders_for_empty_resolved_range_are_a_dotted_double_line() {
        let lines = editor_border_lines(&change(3..3, true, false), top, bottom);
        assert_eq!(
            lines,
            vec![BorderLine {
                y: 59.0,
                double_line: true,
                dotted: true
            }]
        );
    }

    #[test]
    fn merge_tool_resolved_gutter_band_is_only_dotted_edges_across_the_whole_gutter() {
        let layout = gutter_layout(14.0, 33.0, true);
        let rects = gutter_band_rects(&layout, &change(2..4, true, true), 40.0, 80.0, true);
        assert_eq!(rects.len(), 40);
        assert!(rects.iter().all(|dot| dot.role == FillRole::Solid));
        assert!(
            rects
                .iter()
                .all(|dot| dot.rect.width == 2.0 && dot.rect.height == 2.0)
        );
        let top_xs: Vec<f32> = rects
            .iter()
            .filter(|dot| dot.rect.y == 39.0)
            .map(|dot| dot.rect.x)
            .collect();
        assert_eq!(top_xs.first(), Some(&0.0));
        assert_eq!(top_xs.last(), Some(&76.0));
        assert_eq!(top_xs.len(), 20);
        assert_eq!(rects.iter().filter(|dot| dot.rect.y == 78.0).count(), 20);
    }

    #[test]
    fn merge_tool_empty_marker_ignores_the_faded_background_and_keeps_the_full_colour() {
        let rects = marker_rects(0.0, 79.0, 60.0, 60.0, MarkerMode::IGNORED);
        assert_eq!(
            rects,
            vec![RoledRect {
                rect: PixelRect::new(0.0, 59.0, 80.0, 2.0),
                role: FillRole::Solid,
            }]
        );
    }

    #[test]
    fn merge_tool_markers_two_pixels_tall_count_as_empty_and_zero_width_markers_paint_nothing() {
        let thin = marker_rects(0.0, 79.0, 40.0, 42.0, MarkerMode::DEFAULT);
        assert_eq!(
            thin,
            vec![RoledRect {
                rect: PixelRect::new(0.0, 39.0, 80.0, 2.0),
                role: FillRole::Solid,
            }]
        );
        let taller = marker_rects(0.0, 79.0, 40.0, 43.0, MarkerMode::DEFAULT);
        assert_eq!(taller[0].rect, PixelRect::new(0.0, 40.0, 79.0, 3.0));
        assert!(marker_rects(10.0, 10.0, 40.0, 80.0, MarkerMode::DEFAULT).is_empty());
    }

    #[test]
    fn merge_tool_empty_resolved_marker_is_a_dotted_three_row_line() {
        let rects = marker_rects(0.0, 79.0, 60.0, 60.0, MarkerMode::RESOLVED);
        assert_eq!(rects.len(), 40);
        assert_eq!(rects.iter().filter(|dot| dot.rect.y == 58.0).count(), 20);
        assert_eq!(rects.iter().filter(|dot| dot.rect.y == 59.0).count(), 20);
    }

    #[test]
    fn merge_tool_gutter_without_line_numbers_drops_the_number_column_and_its_gaps() {
        let layout = gutter_layout(30.0, 33.0, false);
        assert_eq!(layout.line_numbers, 4.0..4.0);
        assert_eq!(layout.icons, 4.0..37.0);
        assert_eq!(layout.folding, 37.0..48.0);
        assert_eq!(layout.vcs_lane, 48.0..56.0);
        assert_eq!(layout.width, 57.0);
        assert_eq!(layout.separator, 54.0);
    }

    #[test]
    fn merge_tool_result_band_of_a_right_only_change_shrinks_to_the_outline_strip_without_line_numbers()
     {
        let layout = gutter_layout(14.0, 33.0, false);
        let hidden = PaneChange {
            hide_without_line_numbers: true,
            ..change(2..4, false, false)
        };
        let rects = gutter_band_rects(&layout, &hidden, 40.0, 80.0, false);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].rect.x, layout.separator);
        assert_eq!(rects[0].rect.width, layout.width - layout.separator);
        let shown = gutter_band_rects(&layout, &hidden, 40.0, 80.0, true);
        assert_eq!(shown[0].rect.x, 0.0);
    }
}
