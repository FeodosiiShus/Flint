use gpui::{Pixels, px};

use crate::git_log::properties::LogColumn;

pub(super) const MAX_DYNAMIC_COLUMN_WIDTH: Pixels = px(300.);
pub(super) const MAX_ROWS_TO_CALCULATE_WIDTH: usize = 1000;
pub(super) const ADDITIONAL_GAP: Pixels = px(8.);
pub(super) const TEXT_HORIZONTAL_PADDING: Pixels = px(6.);
pub(super) const TEXT_LEFT_PADDING: Pixels = px(3.);
pub(super) const MINIMUM_COLUMN_WIDTH: Pixels = px(15.);
pub(super) const BORDER_THICKNESS: Pixels = px(3.);
pub(super) const HASH_WIDTH_SAMPLE: &str = "eeeeeeee";
const FIRST_DATA_COLUMN_INDEX: usize = 1;

pub(super) fn is_dynamic(column: LogColumn) -> bool {
    matches!(
        column,
        LogColumn::Author | LogColumn::Date | LogColumn::Hash
    )
}

pub(super) fn visible_columns(column_order: &[LogColumn]) -> Vec<LogColumn> {
    let mut columns: Vec<LogColumn> = Vec::new();
    for column in column_order {
        if !columns.contains(column) {
            columns.push(*column);
        }
    }
    if columns.contains(&LogColumn::Root) && columns.contains(&LogColumn::Commit) {
        return columns;
    }
    let mut result = Vec::new();
    if !columns.contains(&LogColumn::Root) {
        result.push(LogColumn::Root);
    }
    if !columns.contains(&LogColumn::Commit) {
        result.push(LogColumn::Commit);
    }
    result.extend(columns);
    result
}

pub(super) fn column_gaps(index: usize, column_count: usize) -> (Pixels, Pixels) {
    let leading = if index == FIRST_DATA_COLUMN_INDEX {
        ADDITIONAL_GAP
    } else {
        Pixels::ZERO
    };
    let trailing = if index + 1 == column_count {
        ADDITIONAL_GAP
    } else {
        Pixels::ZERO
    };
    (leading, trailing)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ColumnBounds {
    pub(super) column: LogColumn,
    pub(super) left: Pixels,
    pub(super) width: Pixels,
    pub(super) leading_gap: Pixels,
    pub(super) trailing_gap: Pixels,
}

impl ColumnBounds {
    pub(super) fn right(&self) -> Pixels {
        self.left + self.width
    }
}

pub(super) fn persisted_width(
    widths: &[(LogColumn, f32)],
    column: LogColumn,
    table_width: Pixels,
) -> Option<Pixels> {
    widths
        .iter()
        .find(|(persisted, _)| *persisted == column)
        .map(|(_, width)| px(*width))
        .filter(|width| *width > Pixels::ZERO && *width <= table_width)
}

pub(super) fn layout_columns(
    columns: &[LogColumn],
    table_width: Pixels,
    mut width_of: impl FnMut(usize, LogColumn) -> Pixels,
) -> Vec<ColumnBounds> {
    let widths: Vec<Option<Pixels>> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| match column {
            LogColumn::Root => Some(Pixels::ZERO),
            LogColumn::Commit => None,
            column => Some(width_of(index, *column)),
        })
        .collect();
    let fixed_width = widths
        .iter()
        .flatten()
        .fold(Pixels::ZERO, |total, width| total + *width);
    let commit_width = (table_width - fixed_width).max(MINIMUM_COLUMN_WIDTH);
    let mut left = Pixels::ZERO;
    columns
        .iter()
        .zip(widths)
        .enumerate()
        .map(|(index, (column, width))| {
            let width = width.unwrap_or(commit_width);
            let (leading_gap, trailing_gap) = if *column == LogColumn::Root {
                (Pixels::ZERO, Pixels::ZERO)
            } else {
                column_gaps(index, columns.len())
            };
            let bounds = ColumnBounds {
                column: *column,
                left,
                width,
                leading_gap,
                trailing_gap,
            };
            left += width;
            bounds
        })
        .collect()
}

pub(super) fn resizable_borders(bounds: &[ColumnBounds]) -> Vec<usize> {
    (FIRST_DATA_COLUMN_INDEX..bounds.len().saturating_sub(1))
        .filter(|index| {
            bounds
                .get(*index)
                .is_some_and(|column| column.column != LogColumn::Root)
        })
        .collect()
}

pub(super) fn resize_border(
    bounds: &[ColumnBounds],
    index: usize,
    delta: Pixels,
) -> Vec<(LogColumn, Pixels)> {
    let (Some(left), Some(right)) = (bounds.get(index), bounds.get(index + 1)) else {
        return Vec::new();
    };
    let mut left_width = (left.width + delta).max(MINIMUM_COLUMN_WIDTH);
    let mut applied = left_width - left.width;
    let right_width = (right.width - applied).max(MINIMUM_COLUMN_WIDTH);
    applied = right.width - right_width;
    left_width = left.width + applied;
    [(left.column, left_width), (right.column, right_width)]
        .into_iter()
        .filter(|(column, _)| is_dynamic(*column))
        .collect()
}

pub(super) fn reset_target(bounds: &[ColumnBounds], index: usize) -> Option<LogColumn> {
    let commit_index = bounds
        .iter()
        .position(|column| column.column == LogColumn::Commit)?;
    let target = if index + 1 > commit_index {
        bounds.get(index + 1)?
    } else {
        bounds.get(index)?
    };
    is_dynamic(target.column).then_some(target.column)
}

pub(super) fn merge_widths(
    existing: &[(LogColumn, f32)],
    updates: &[(LogColumn, Pixels)],
) -> Vec<(LogColumn, f32)> {
    let mut result: Vec<(LogColumn, f32)> = existing
        .iter()
        .filter(|(column, _)| !updates.iter().any(|(updated, _)| updated == column))
        .copied()
        .collect();
    result.extend(
        updates
            .iter()
            .map(|(column, width)| (*column, f32::from(*width))),
    );
    result
}

pub(super) fn without_width(
    existing: &[(LogColumn, f32)],
    column: LogColumn,
) -> Vec<(LogColumn, f32)> {
    existing
        .iter()
        .filter(|(persisted, _)| *persisted != column)
        .copied()
        .collect()
}

pub(super) fn data_width(max_text_width: Pixels, gaps: Pixels) -> Pixels {
    (max_text_width + TEXT_HORIZONTAL_PADDING + gaps).min(MAX_DYNAMIC_COLUMN_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(order: &[LogColumn], table_width: f32, author: f32, date: f32) -> Vec<ColumnBounds> {
        layout_columns(
            &visible_columns(order),
            px(table_width),
            |_, column| match column {
                LogColumn::Author => px(author),
                LogColumn::Date => px(date),
                _ => px(70.),
            },
        )
    }

    #[test]
    fn column_order_is_made_valid_like_make_valid_column_order() {
        assert_eq!(
            visible_columns(&[LogColumn::Author, LogColumn::Author, LogColumn::Date]),
            vec![
                LogColumn::Root,
                LogColumn::Commit,
                LogColumn::Author,
                LogColumn::Date
            ]
        );
        assert_eq!(
            visible_columns(&LogColumn::DEFAULT_ORDER),
            LogColumn::DEFAULT_ORDER.to_vec()
        );
    }

    #[test]
    fn commit_column_takes_the_remaining_width_and_gaps_go_to_outer_columns() {
        let bounds = layout(&LogColumn::DEFAULT_ORDER, 1000., 120., 150.);
        assert_eq!(bounds[0].width, Pixels::ZERO);
        assert_eq!(bounds[1].width, px(730.));
        assert_eq!(bounds[1].leading_gap, ADDITIONAL_GAP);
        assert_eq!(bounds[3].trailing_gap, ADDITIONAL_GAP);
        assert_eq!(bounds[2].leading_gap, Pixels::ZERO);
        assert_eq!(bounds[3].right(), px(1000.));
    }

    #[test]
    fn persisted_widths_wider_than_the_table_are_ignored() {
        let widths = [(LogColumn::Author, 400.)];
        assert_eq!(persisted_width(&widths, LogColumn::Author, px(300.)), None);
        assert_eq!(
            persisted_width(&widths, LogColumn::Author, px(500.)),
            Some(px(400.))
        );
        assert_eq!(
            persisted_width(&[(LogColumn::Author, -1.)], LogColumn::Author, px(500.)),
            None
        );
    }

    #[test]
    fn dragging_a_border_resizes_both_neighbours_and_keeps_minimum_widths() {
        let bounds = layout(&LogColumn::DEFAULT_ORDER, 1000., 120., 150.);
        assert_eq!(
            resize_border(&bounds, 1, px(-30.)),
            vec![(LogColumn::Author, px(150.))]
        );
        assert_eq!(
            resize_border(&bounds, 2, px(500.)),
            vec![
                (LogColumn::Author, px(255.)),
                (LogColumn::Date, MINIMUM_COLUMN_WIDTH)
            ]
        );
        assert_eq!(resizable_borders(&bounds), vec![1, 2]);
    }

    #[test]
    fn double_click_resets_the_dynamic_column_on_the_far_side_of_the_commit_column() {
        let bounds = layout(&LogColumn::DEFAULT_ORDER, 1000., 120., 150.);
        assert_eq!(reset_target(&bounds, 1), Some(LogColumn::Author));
        assert_eq!(reset_target(&bounds, 2), Some(LogColumn::Date));
        let reordered = layout(
            &[
                LogColumn::Root,
                LogColumn::Author,
                LogColumn::Commit,
                LogColumn::Date,
            ],
            1000.,
            120.,
            150.,
        );
        assert_eq!(reset_target(&reordered, 1), Some(LogColumn::Author));
    }

    #[test]
    fn data_width_is_capped_at_three_hundred_pixels() {
        assert_eq!(data_width(px(100.), ADDITIONAL_GAP), px(114.));
        assert_eq!(data_width(px(500.), Pixels::ZERO), MAX_DYNAMIC_COLUMN_WIDTH);
    }

    #[test]
    fn merging_widths_replaces_updated_columns_only() {
        let merged = merge_widths(
            &[(LogColumn::Author, 100.), (LogColumn::Date, 90.)],
            &[(LogColumn::Date, px(120.))],
        );
        assert_eq!(
            merged,
            vec![(LogColumn::Author, 100.), (LogColumn::Date, 120.)]
        );
        assert_eq!(
            without_width(&merged, LogColumn::Author),
            vec![(LogColumn::Date, 120.)]
        );
    }
}
