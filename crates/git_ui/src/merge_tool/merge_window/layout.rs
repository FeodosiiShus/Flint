use merge_diff::Side;

use crate::merge_tool::merge_ribbons::DIVIDER_WIDTH;

const DEFAULT_PROPORTION: f32 = 1.0 / 3.0;
const DIVIDER_COUNT: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Proportions {
    pub left: f32,
    pub right: f32,
}

impl Default for Proportions {
    fn default() -> Self {
        Self::thirds()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ComponentWidths {
    pub left: i32,
    pub left_divider: i32,
    pub middle: i32,
    pub right_divider: i32,
    pub right: i32,
}

pub(crate) fn divider_width() -> i32 {
    DIVIDER_WIDTH as i32
}

pub(crate) fn calc_component_widths(total_width: i32, proportions: Proportions) -> ComponentWidths {
    let contents_total = (total_width - divider_width() * DIVIDER_COUNT).max(0);
    let left = (contents_total as f32 * proportions.left) as i32;
    let right = (contents_total as f32 * proportions.right) as i32;
    let middle = (contents_total - left - right).max(0);
    ComponentWidths {
        left,
        left_divider: divider_width(),
        middle,
        right_divider: divider_width(),
        right,
    }
}

impl Proportions {
    pub(crate) fn thirds() -> Self {
        Self {
            left: DEFAULT_PROPORTION,
            right: DEFAULT_PROPORTION,
        }
    }

    pub(crate) fn expanded_middle() -> Self {
        Self {
            left: 0.0,
            right: 0.0,
        }
    }

    pub(crate) fn are_default(self, total_width: i32) -> bool {
        calc_component_widths(total_width, self)
            == calc_component_widths(total_width, Self::thirds())
    }

    pub(crate) fn with_proportion(self, side: Side, proportion: f32) -> Self {
        let proportion = proportion.clamp(0.0, 1.0);
        let other = match side {
            Side::Left => self.right,
            Side::Right => self.left,
        }
        .min(1.0 - proportion);
        match side {
            Side::Left => Self {
                left: proportion,
                right: other,
            },
            Side::Right => Self {
                left: other,
                right: proportion,
            },
        }
    }

    pub(crate) fn dragged_to(self, side: Side, pointer_x: f32, total_width: f32) -> Self {
        if total_width <= 0.0 {
            return self;
        }
        let distance = match side {
            Side::Left => pointer_x,
            Side::Right => total_width - pointer_x,
        };
        self.with_proportion(side, distance / total_width)
    }

    pub(crate) fn toggled(self, total_width: i32) -> Self {
        if self.are_default(total_width) {
            Self::expanded_middle()
        } else {
            Self::thirds()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn merge_tool_layout_default_widths_split_contents_into_truncated_thirds() {
        let widths = calc_component_widths(1000, Proportions::default());
        assert_eq!(
            widths,
            ComponentWidths {
                left: 317,
                left_divider: 24,
                middle: 318,
                right_divider: 24,
                right: 317,
            }
        );
    }

    #[test]
    fn merge_tool_layout_widths_always_sum_to_total_width() {
        for total in [952, 1000, 1001, 1279] {
            let widths = calc_component_widths(total, Proportions::default());
            let sum = widths.left
                + widths.left_divider
                + widths.middle
                + widths.right_divider
                + widths.right;
            assert_eq!(sum, total);
        }
    }

    #[test]
    fn merge_tool_layout_narrow_window_gives_dividers_only() {
        let widths = calc_component_widths(30, Proportions::default());
        assert_eq!((widths.left, widths.middle, widths.right), (0, 0, 0));
        assert_eq!(widths.left_divider + widths.right_divider, 48);
    }

    #[test]
    fn merge_tool_layout_expanded_middle_gives_all_contents_to_result() {
        let widths = calc_component_widths(1000, Proportions::expanded_middle());
        assert_eq!((widths.left, widths.middle, widths.right), (0, 952, 0));
    }

    #[test]
    fn merge_tool_layout_dragging_left_divider_keeps_other_side_when_room_remains() {
        let dragged = Proportions::thirds().dragged_to(Side::Left, 300.0, 1000.0);
        assert_close(dragged.left, 0.3);
        assert_close(dragged.right, 1.0 / 3.0);
    }

    #[test]
    fn merge_tool_layout_dragging_right_divider_caps_the_other_side() {
        let dragged = Proportions::thirds().dragged_to(Side::Right, 100.0, 1000.0);
        assert_close(dragged.right, 0.9);
        assert_close(dragged.left, 0.1);
    }

    #[test]
    fn merge_tool_layout_dragging_beyond_the_window_clamps_to_unit_interval() {
        let dragged = Proportions::thirds().dragged_to(Side::Left, 5000.0, 1000.0);
        assert_close(dragged.left, 1.0);
        assert_close(dragged.right, 0.0);
        let negative = Proportions::thirds().dragged_to(Side::Left, -50.0, 1000.0);
        assert_close(negative.left, 0.0);
    }

    #[test]
    fn merge_tool_layout_dragging_in_zero_width_window_changes_nothing() {
        let unchanged = Proportions::thirds().dragged_to(Side::Left, 10.0, 0.0);
        assert_eq!(unchanged, Proportions::thirds());
    }

    #[test]
    fn merge_tool_layout_double_click_toggles_between_thirds_and_expanded_middle() {
        let expanded = Proportions::thirds().toggled(1000);
        assert_eq!(expanded, Proportions::expanded_middle());
        let restored = expanded.toggled(1000);
        assert_eq!(restored, Proportions::thirds());
    }

    #[test]
    fn merge_tool_layout_custom_proportions_reset_to_thirds_on_double_click() {
        let custom = Proportions {
            left: 0.5,
            right: 0.25,
        };
        assert!(!custom.are_default(1000));
        assert_eq!(custom.toggled(1000), Proportions::thirds());
    }

    #[test]
    fn merge_tool_layout_nearly_default_proportions_count_as_default_by_pixel_widths() {
        let nearly = Proportions {
            left: 0.3334,
            right: 0.3334,
        };
        assert!(nearly.are_default(1000));
        assert_eq!(nearly.toggled(1000), Proportions::expanded_middle());
    }
}
