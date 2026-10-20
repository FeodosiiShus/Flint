use gpui::{Pixels, px};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BalloonMetrics {
    pub width: Pixels,
    pub icon_column_width: Pixels,
    pub icon_offset_x: Pixels,
    pub action_gap: Pixels,
    pub border_stroke_width: Pixels,
    pub corner_radius: Pixels,
    pub window_inset: Pixels,
    /// The gap between the stacked balloons.
    pub stack_gap: Pixels,
    /// IntelliJ's New UI puts the 16 px close glyph 9 px from the balloon's top and right
    /// edges, so the 20 px hover circle around it starts 7 px in.
    pub close_inset: Pixels,
}

pub const BALLOON_METRICS: BalloonMetrics = BalloonMetrics {
    width: px(360.),
    icon_column_width: px(32.),
    icon_offset_x: px(10.),
    action_gap: px(16.),
    border_stroke_width: px(1.),
    corner_radius: px(12.),
    window_inset: px(8.),
    stack_gap: px(8.),
    close_inset: px(7.),
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BalloonContentInsets {
    pub top: Pixels,
    pub left: Pixels,
    pub bottom: Pixels,
    pub right: Pixels,
    pub gap: Pixels,
}

pub const BALLOON_CONTENT_INSETS: BalloonContentInsets = BalloonContentInsets {
    top: px(4.),
    left: px(4.),
    bottom: px(6.),
    right: px(0.),
    gap: px(2.),
};

pub const BALLOON_MAX_VISIBLE_ACTIONS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BalloonLayoutMetrics {
    pub top: Pixels,
    pub title_to_content: Pixels,
    pub content_to_actions: Pixels,
    pub title_to_actions: Pixels,
    pub bottom: Pixels,
    pub icon_offset_y: Pixels,
}

impl BalloonLayoutMetrics {
    pub const fn for_content(has_title: bool, has_message: bool, has_actions: bool) -> Self {
        if has_title && has_message && has_actions {
            BALLOON_THREE_LINE_METRICS
        } else {
            BALLOON_TWO_LINE_METRICS
        }
    }

    pub const fn actions_top(self, has_title: bool, has_message: bool) -> Pixels {
        if has_message {
            self.content_to_actions
        } else if has_title {
            self.title_to_actions
        } else {
            px(0.)
        }
    }
}

pub const BALLOON_TWO_LINE_METRICS: BalloonLayoutMetrics = BalloonLayoutMetrics {
    top: px(11.),
    title_to_content: px(5.),
    content_to_actions: px(5.),
    title_to_actions: px(5.),
    bottom: px(14.),
    icon_offset_y: px(11.),
};

pub const BALLOON_THREE_LINE_METRICS: BalloonLayoutMetrics = BalloonLayoutMetrics {
    top: px(7.),
    title_to_content: px(3.),
    content_to_actions: px(7.),
    title_to_actions: px(0.),
    bottom: px(8.),
    icon_offset_y: px(7.),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_balloon_uses_intellij_width() {
        assert_eq!(BALLOON_METRICS.width, px(360.));
    }

    #[test]
    fn the_icon_column_the_action_gap_and_the_margins_match_the_intellij_balloon() {
        assert_eq!(BALLOON_METRICS.icon_column_width, px(32.));
        assert_eq!(BALLOON_METRICS.icon_offset_x, px(10.));
        assert_eq!(BALLOON_METRICS.action_gap, px(16.));
        assert_eq!(BALLOON_METRICS.window_inset, px(8.));
    }

    #[test]
    fn the_balloon_surface_is_a_single_pixel_border_with_a_twelve_pixel_arc() {
        assert_eq!(BALLOON_METRICS.border_stroke_width, px(1.));
        assert_eq!(BALLOON_METRICS.corner_radius, px(12.));
    }

    #[test]
    fn the_content_insets_are_the_intellij_notification_border_insets() {
        assert_eq!(BALLOON_CONTENT_INSETS.top, px(4.));
        assert_eq!(BALLOON_CONTENT_INSETS.left, px(4.));
        assert_eq!(BALLOON_CONTENT_INSETS.bottom, px(6.));
        assert_eq!(BALLOON_CONTENT_INSETS.right, px(0.));
        assert_eq!(BALLOON_CONTENT_INSETS.gap, px(2.));
    }

    #[test]
    fn two_actions_are_preferable_and_the_rest_go_into_the_more_menu() {
        assert_eq!(BALLOON_MAX_VISIBLE_ACTIONS, 2);
    }

    #[test]
    fn the_two_line_layout_uses_the_intellij_two_line_spacing() {
        assert_eq!(BALLOON_TWO_LINE_METRICS.top, px(11.));
        assert_eq!(BALLOON_TWO_LINE_METRICS.title_to_content, px(5.));
        assert_eq!(BALLOON_TWO_LINE_METRICS.content_to_actions, px(5.));
        assert_eq!(BALLOON_TWO_LINE_METRICS.title_to_actions, px(5.));
        assert_eq!(BALLOON_TWO_LINE_METRICS.bottom, px(14.));
        assert_eq!(BALLOON_TWO_LINE_METRICS.icon_offset_y, px(11.));
    }

    #[test]
    fn the_three_line_layout_uses_the_intellij_three_line_spacing() {
        assert_eq!(BALLOON_THREE_LINE_METRICS.top, px(7.));
        assert_eq!(BALLOON_THREE_LINE_METRICS.title_to_content, px(3.));
        assert_eq!(BALLOON_THREE_LINE_METRICS.content_to_actions, px(7.));
        assert_eq!(BALLOON_THREE_LINE_METRICS.title_to_actions, px(0.));
        assert_eq!(BALLOON_THREE_LINE_METRICS.bottom, px(8.));
        assert_eq!(BALLOON_THREE_LINE_METRICS.icon_offset_y, px(7.));
    }

    #[test]
    fn a_title_a_message_and_actions_pick_the_three_line_layout() {
        assert_eq!(
            BalloonLayoutMetrics::for_content(true, true, true),
            BALLOON_THREE_LINE_METRICS
        );
    }

    #[test]
    fn a_balloon_missing_a_title_a_message_or_actions_picks_the_two_line_layout() {
        assert_eq!(
            BalloonLayoutMetrics::for_content(true, true, false),
            BALLOON_TWO_LINE_METRICS
        );
        assert_eq!(
            BalloonLayoutMetrics::for_content(true, false, true),
            BALLOON_TWO_LINE_METRICS
        );
        assert_eq!(
            BalloonLayoutMetrics::for_content(false, true, true),
            BALLOON_TWO_LINE_METRICS
        );
        assert_eq!(
            BalloonLayoutMetrics::for_content(false, false, false),
            BALLOON_TWO_LINE_METRICS
        );
    }

    #[test]
    fn the_action_row_is_spaced_by_the_content_then_the_title_then_nothing() {
        let layout = BALLOON_TWO_LINE_METRICS;
        assert_eq!(layout.actions_top(true, true), layout.content_to_actions);
        assert_eq!(layout.actions_top(true, false), layout.title_to_actions);
        assert_eq!(layout.actions_top(false, false), px(0.));
    }
}
