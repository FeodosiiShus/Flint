use gpui::{Bounds, Corners, Pixels, Point, point, px, size};
use settings::GitGutterWidth;

const VCS_MARKER_DEFAULT_WIDTH: Pixels = px(4.);
const VCS_MARKER_GUTTER_EDGE_INSET: Pixels = px(2.);
const VCS_MARKER_MINIMUM_SPARE_PADDING: Pixels = px(4.);
const VCS_MARKER_VERTICAL_INSET: Pixels = px(1.);
const DELETED_VCS_MARKER_HEIGHT: Pixels = px(8.);
const GUTTER_SEPARATOR_WIDTH: Pixels = px(1.);
const LINE_NUMBER_FONT_SIZE_DELTA: Pixels = px(-1.);
const MINIMUM_LINE_NUMBER_FONT_SIZE: Pixels = px(1.);

const LEFT_EDGE_STRIP_WIDTH_RATIO: f32 = 0.275;
const LEFT_EDGE_DELETED_MARKER_WIDTH_RATIO: f32 = 0.35 / LEFT_EDGE_STRIP_WIDTH_RATIO;
const LEFT_EDGE_MIN_DELETED_MARKER_WIDTH_RATIO: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum VcsMarkerPlacement {
    GutterEdge {
        x: Pixels,
        width: Pixels,
    },
    LeftEdge {
        strip_width: Pixels,
        deleted_marker_width: Pixels,
    },
}

impl VcsMarkerPlacement {
    pub(super) fn new(
        setting: GitGutterWidth,
        line_height: Pixels,
        gutter_width: Pixels,
        gutter_right_padding: Pixels,
    ) -> Self {
        let width = match setting {
            GitGutterWidth::Custom(width) => px(width.0),
            GitGutterWidth::Default => VCS_MARKER_DEFAULT_WIDTH,
        };
        if width > Pixels::ZERO && gutter_right_padding >= width + VCS_MARKER_MINIMUM_SPARE_PADDING
        {
            Self::GutterEdge {
                x: gutter_width - VCS_MARKER_GUTTER_EDGE_INSET - width,
                width,
            }
        } else {
            Self::LeftEdge {
                strip_width: left_edge_strip_width(setting, line_height),
                deleted_marker_width: left_edge_deleted_marker_width(setting, line_height),
            }
        }
    }

    pub(super) fn row_range_bounds(
        &self,
        gutter_origin: Point<Pixels>,
        start_y: Pixels,
        end_y: Pixels,
    ) -> Bounds<Pixels> {
        match *self {
            Self::GutterEdge { x, width } => {
                let height = (end_y - start_y - VCS_MARKER_VERTICAL_INSET * 2.).max(Pixels::ZERO);
                Bounds::new(
                    gutter_origin + point(x, start_y + VCS_MARKER_VERTICAL_INSET),
                    size(width, height),
                )
            }
            Self::LeftEdge { strip_width, .. } => Bounds::new(
                gutter_origin + point(Pixels::ZERO, start_y),
                size(strip_width, end_y - start_y),
            ),
        }
    }

    pub(super) fn deleted_marker_bounds(
        &self,
        gutter_origin: Point<Pixels>,
        line_boundary_y: Pixels,
        line_height: Pixels,
    ) -> Bounds<Pixels> {
        match *self {
            Self::GutterEdge { x, width } => Bounds::new(
                gutter_origin + point(x, line_boundary_y - DELETED_VCS_MARKER_HEIGHT / 2.),
                size(width, DELETED_VCS_MARKER_HEIGHT),
            ),
            Self::LeftEdge {
                deleted_marker_width,
                ..
            } => Bounds::new(
                gutter_origin + point(Pixels::ZERO, line_boundary_y - line_height / 2.),
                size(deleted_marker_width, line_height),
            ),
        }
    }

    pub(super) fn marker_corner_radii(&self) -> Corners<Pixels> {
        match *self {
            Self::GutterEdge { width, .. } => Corners::all(width / 2.),
            Self::LeftEdge { .. } => Corners::all(Pixels::ZERO),
        }
    }

    pub(super) fn deleted_marker_paint_shape(
        &self,
        marker_bounds: Bounds<Pixels>,
        line_height: Pixels,
    ) -> (Bounds<Pixels>, Corners<Pixels>) {
        match *self {
            Self::GutterEdge { .. } => (marker_bounds, self.marker_corner_radii()),
            Self::LeftEdge { .. } => (
                Bounds::new(
                    point(
                        marker_bounds.origin.x - marker_bounds.size.width,
                        marker_bounds.origin.y,
                    ),
                    size(marker_bounds.size.width * 2., marker_bounds.size.height),
                ),
                Corners::all(line_height),
            ),
        }
    }
}

pub(super) fn left_edge_strip_width(setting: GitGutterWidth, line_height: Pixels) -> Pixels {
    match setting {
        GitGutterWidth::Custom(width) => px(width.0),
        GitGutterWidth::Default => (LEFT_EDGE_STRIP_WIDTH_RATIO * line_height).floor(),
    }
}

fn left_edge_deleted_marker_width(setting: GitGutterWidth, line_height: Pixels) -> Pixels {
    match setting {
        GitGutterWidth::Custom(width) => {
            let scaled_width = px(width.0 * LEFT_EDGE_DELETED_MARKER_WIDTH_RATIO);
            if scaled_width > Pixels::ZERO {
                let default_strip_width = LEFT_EDGE_STRIP_WIDTH_RATIO * line_height;
                let boost_factor = (1.0 - width.0 / f32::from(default_strip_width)).max(0.0);
                scaled_width + line_height * LEFT_EDGE_MIN_DELETED_MARKER_WIDTH_RATIO * boost_factor
            } else {
                Pixels::ZERO
            }
        }
        GitGutterWidth::Default => {
            (LEFT_EDGE_STRIP_WIDTH_RATIO * line_height * LEFT_EDGE_DELETED_MARKER_WIDTH_RATIO)
                .floor()
        }
    }
}

pub(super) fn gutter_separator_bounds(gutter_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(
            gutter_bounds.right() - GUTTER_SEPARATOR_WIDTH,
            gutter_bounds.top(),
        ),
        size(GUTTER_SEPARATOR_WIDTH, gutter_bounds.size.height),
    )
}

pub(super) fn line_number_font_size(buffer_font_size: Pixels) -> Pixels {
    (buffer_font_size + LINE_NUMBER_FONT_SIZE_DELTA).max(MINIMUM_LINE_NUMBER_FONT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::PixelSetting;

    const LINE_HEIGHT: Pixels = px(20.);
    const GUTTER_WIDTH: Pixels = px(60.);

    fn gutter_origin() -> Point<Pixels> {
        point(px(10.), px(5.))
    }

    #[test]
    fn intellij_gutter_marker_sits_left_of_the_separator_with_default_width() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(32.));

        assert_eq!(
            placement,
            VcsMarkerPlacement::GutterEdge {
                x: px(54.),
                width: px(4.),
            },
            "a 4 px marker ends 2 px before the gutter edge, leaving a gap and the separator column"
        );
    }

    #[test]
    fn intellij_gutter_modified_range_marker_is_an_inset_capsule() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(32.));

        let bounds = placement.row_range_bounds(gutter_origin(), px(40.), px(100.));

        assert_eq!(
            bounds,
            Bounds::new(point(px(64.), px(46.)), size(px(4.), px(58.))),
            "the marker is inset 1 px at the top and bottom of its rows"
        );
        assert_eq!(
            placement.marker_corner_radii(),
            Corners::all(px(2.)),
            "a corner radius of half the width makes both ends fully round"
        );
    }

    #[test]
    fn intellij_gutter_folded_hunk_marker_covers_its_single_row() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(32.));

        let bounds = placement.row_range_bounds(gutter_origin(), px(20.), px(20.) + LINE_HEIGHT);

        assert_eq!(
            bounds,
            Bounds::new(point(px(64.), px(26.)), size(px(4.), px(18.))),
        );
    }

    #[test]
    fn intellij_gutter_deleted_marker_is_a_short_capsule_centered_on_the_line_boundary() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(32.));

        let bounds = placement.deleted_marker_bounds(gutter_origin(), px(40.), LINE_HEIGHT);
        let (paint_bounds, corner_radii) =
            placement.deleted_marker_paint_shape(bounds, LINE_HEIGHT);

        assert_eq!(
            bounds,
            Bounds::new(point(px(64.), px(41.)), size(px(4.), px(8.))),
            "the deleted marker is 8 px tall and centered on the boundary between two lines"
        );
        assert_eq!(
            paint_bounds, bounds,
            "the painted capsule matches its hitbox"
        );
        assert_eq!(corner_radii, Corners::all(px(2.)));
    }

    #[test]
    fn intellij_gutter_marker_uses_the_custom_width() {
        let placement = VcsMarkerPlacement::new(
            GitGutterWidth::Custom(PixelSetting(6.)),
            LINE_HEIGHT,
            GUTTER_WIDTH,
            px(10.),
        );

        assert_eq!(
            placement,
            VcsMarkerPlacement::GutterEdge {
                x: px(52.),
                width: px(6.),
            },
        );
        assert_eq!(placement.marker_corner_radii(), Corners::all(px(3.)));
    }

    #[test]
    fn intellij_gutter_marker_falls_back_to_the_left_edge_when_the_right_padding_is_too_small() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(7.));

        let VcsMarkerPlacement::LeftEdge { strip_width, .. } = placement else {
            panic!("expected the left-edge placement, got {placement:?}");
        };
        assert_eq!(
            placement.row_range_bounds(gutter_origin(), px(40.), px(100.)),
            Bounds::new(point(px(10.), px(45.)), size(strip_width, px(60.))),
            "the fallback strip starts at the gutter's left edge and spans its rows without insets"
        );
        assert_eq!(placement.marker_corner_radii(), Corners::all(Pixels::ZERO));
    }

    #[test]
    fn intellij_gutter_deleted_marker_falls_back_to_a_half_circle_at_the_left_edge() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, GUTTER_WIDTH, px(0.));

        let bounds = placement.deleted_marker_bounds(gutter_origin(), px(40.), LINE_HEIGHT);
        let (paint_bounds, corner_radii) =
            placement.deleted_marker_paint_shape(bounds, LINE_HEIGHT);

        assert_eq!(bounds.origin, point(px(10.), px(35.)));
        assert_eq!(bounds.size.height, LINE_HEIGHT);
        assert_eq!(
            paint_bounds,
            Bounds::new(
                point(px(10.) - bounds.size.width, px(35.)),
                size(bounds.size.width * 2., LINE_HEIGHT),
            ),
            "the fallback marker is painted twice as wide, half of it clipped beyond the gutter"
        );
        assert_eq!(corner_radii, Corners::all(LINE_HEIGHT));
    }

    #[test]
    fn intellij_gutter_zero_custom_width_hides_markers() {
        let placement = VcsMarkerPlacement::new(
            GitGutterWidth::Custom(PixelSetting(0.)),
            LINE_HEIGHT,
            GUTTER_WIDTH,
            px(32.),
        );

        assert_eq!(
            placement
                .row_range_bounds(gutter_origin(), px(40.), px(100.))
                .size
                .width,
            Pixels::ZERO
        );
        assert_eq!(
            placement
                .deleted_marker_bounds(gutter_origin(), px(40.), LINE_HEIGHT)
                .size
                .width,
            Pixels::ZERO
        );
    }

    #[test]
    fn intellij_gutter_fallback_deleted_marker_width_stays_visible_and_grows_with_the_setting() {
        assert_eq!(
            left_edge_deleted_marker_width(GitGutterWidth::Default, px(22.0)),
            px(7.0),
        );

        let boosted =
            left_edge_deleted_marker_width(GitGutterWidth::Custom(PixelSetting(6.0)), px(22.0));
        assert!(
            boosted > px(6.0),
            "boosted={boosted:?} must exceed the raw custom width so the deleted pill stays visible"
        );

        for line_height in [22.0, 40.0] {
            let widths = [1.0, 2.0, 3.0, 6.0].map(|width| {
                left_edge_deleted_marker_width(
                    GitGutterWidth::Custom(PixelSetting(width)),
                    px(line_height),
                )
            });
            assert!(
                widths.windows(2).all(|pair| pair[0] < pair[1]),
                "widths={widths:?} must grow with the custom setting"
            );
            assert!(
                widths[0] > px(line_height / 8.0),
                "widths={widths:?} must stay above the vanishing width for line_height={line_height}"
            );
        }

        assert_eq!(
            left_edge_deleted_marker_width(
                GitGutterWidth::Custom(PixelSetting(0.275 * 40.0)),
                px(40.0),
            ),
            px(14.0),
        );

        assert_eq!(
            left_edge_deleted_marker_width(GitGutterWidth::Custom(PixelSetting(0.0)), px(22.0)),
            px(0.0),
        );
    }

    #[test]
    fn intellij_gutter_separator_is_one_pixel_at_the_gutter_edge() {
        let gutter_bounds = Bounds::new(gutter_origin(), size(GUTTER_WIDTH, px(300.)));

        assert_eq!(
            gutter_separator_bounds(gutter_bounds),
            Bounds::new(point(px(69.), px(5.)), size(px(1.), px(300.))),
        );
    }

    #[test]
    fn intellij_gutter_line_numbers_are_one_pixel_smaller_than_the_buffer_font() {
        assert_eq!(line_number_font_size(px(14.)), px(13.));
        assert_eq!(line_number_font_size(px(0.5)), px(1.));
    }
}
