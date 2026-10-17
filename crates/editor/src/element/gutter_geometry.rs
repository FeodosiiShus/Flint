use gpui::{Bounds, Corners, Pixels, Point, point, px, size};
use settings::GitGutterWidth;
use theme::UiDensity;

const VCS_MARKER_DEFAULT_WIDTH: Pixels = px(4.);
const VCS_MARKER_VERTICAL_INSET: Pixels = px(1.);
const DELETED_VCS_MARKER_HEIGHT: Pixels = px(8.);
const GUTTER_SEPARATOR_WIDTH: Pixels = px(1.);
const LINE_NUMBER_FONT_SIZE_DELTA: Pixels = px(-1.);
const MINIMUM_LINE_NUMBER_FONT_SIZE: Pixels = px(1.);

const LINE_NUMBER_AREA_MINIMUM_WIDTH: Pixels = px(14.);
const GAP_AFTER_ANNOTATIONS: Pixels = px(5.);
const GUTTER_ICON_WIDTH: Pixels = px(14.);
const GUTTER_ICON_INSET: Pixels = px(2.);
pub(crate) const ICON_AREA_WIDTH: Pixels = px(16.);
pub(crate) const FOLD_ICON_WIDTH: Pixels = px(9.);
const FOLD_AREA_WIDTH: Pixels = px(11.);
const VCS_AREA_WIDTH: Pixels = px(8.);
const GAP_AFTER_VCS_AREA: Pixels = px(1.);
const VCS_MARKER_AREA_INSET: Pixels = px(1.);
const NARROW_VCS_MARKER_EXTRA_INSET: Pixels = px(2.);
const NARROW_VCS_MARKER_WIDTH_LIMIT: Pixels = px(5.);
const SEPARATOR_OFFSET_FROM_GUTTER_END: Pixels = px(3.);

const LEFT_EDGE_STRIP_WIDTH_RATIO: f32 = 0.275;
const LEFT_EDGE_DELETED_MARKER_WIDTH_RATIO: f32 = 0.35 / LEFT_EDGE_STRIP_WIDTH_RATIO;
const LEFT_EDGE_MIN_DELETED_MARKER_WIDTH_RATIO: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GutterGaps {
    empty_annotation_area: Pixels,
    before_line_numbers: Pixels,
    after_line_numbers: Pixels,
    after_icons: Pixels,
}

impl GutterGaps {
    pub(crate) fn for_density(density: UiDensity) -> Self {
        match density {
            UiDensity::Compact => Self {
                empty_annotation_area: Pixels::ZERO,
                before_line_numbers: px(2.),
                after_line_numbers: px(2.),
                after_icons: px(2.),
            },
            UiDensity::Default | UiDensity::Comfortable => Self {
                empty_annotation_area: px(4.),
                before_line_numbers: px(4.),
                after_line_numbers: px(4.),
                after_icons: px(4.),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GutterContent {
    pub(crate) gaps: GutterGaps,
    pub(crate) leading_area_width: Option<Pixels>,
    pub(crate) line_numbers_width: Option<Pixels>,
    pub(crate) icons: bool,
    pub(crate) icon_area_width: Pixels,
    pub(crate) folds: bool,
    pub(crate) vcs_markers: Option<GitGutterWidth>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GutterAreas {
    pub(crate) line_numbers_x: Pixels,
    pub(crate) line_numbers_end: Pixels,
    pub(crate) icon_area_x: Pixels,
    pub(crate) fold_area_x: Pixels,
    pub(crate) vcs_marker_x: Option<Pixels>,
    pub(crate) width: Pixels,
    pub(crate) text_margin: Option<Pixels>,
}

impl GutterContent {
    pub(crate) fn layout(&self) -> GutterAreas {
        let vcs_marker_width = self
            .vcs_markers
            .map(vcs_marker_width)
            .filter(|marker_width| *marker_width > Pixels::ZERO);
        if self.leading_area_width.is_none()
            && self.line_numbers_width.is_none()
            && !self.icons
            && !self.folds
            && vcs_marker_width.is_none()
        {
            return GutterAreas::default();
        }

        let leading_area_end = self
            .leading_area_width
            .unwrap_or(self.gaps.empty_annotation_area);
        let (line_numbers_x, line_numbers_end, icon_area_x) = match self.line_numbers_width {
            Some(line_numbers_width) => {
                let line_numbers_x = leading_area_end + self.gaps.before_line_numbers;
                let line_numbers_end = line_numbers_x + line_numbers_width;
                (
                    line_numbers_x,
                    line_numbers_end,
                    line_numbers_end + self.gaps.after_line_numbers,
                )
            }
            None => (leading_area_end, leading_area_end, leading_area_end),
        };
        let fold_area_x = if self.icons {
            icon_area_x + self.icon_area_width + self.gaps.after_icons
        } else {
            icon_area_x
        };
        let vcs_area_x = if self.folds {
            fold_area_x + FOLD_AREA_WIDTH
        } else {
            fold_area_x
        };

        let vcs_marker = vcs_marker_width
            .map(|marker_width| (vcs_area_x + vcs_marker_inset(marker_width), marker_width));
        let separator_x = match vcs_marker {
            Some(_) => {
                vcs_area_x + VCS_AREA_WIDTH + GAP_AFTER_VCS_AREA - SEPARATOR_OFFSET_FROM_GUTTER_END
            }
            None => vcs_area_x,
        };
        let width = match vcs_marker {
            Some((marker_x, marker_width)) => {
                (separator_x + GUTTER_SEPARATOR_WIDTH).max(marker_x + marker_width)
            }
            None => separator_x + GUTTER_SEPARATOR_WIDTH,
        };

        GutterAreas {
            line_numbers_x,
            line_numbers_end,
            icon_area_x,
            fold_area_x,
            vcs_marker_x: vcs_marker.map(|(marker_x, _)| marker_x),
            width,
            text_margin: Some(SEPARATOR_OFFSET_FROM_GUTTER_END - GUTTER_SEPARATOR_WIDTH),
        }
    }
}

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
        marker_x: Option<Pixels>,
    ) -> Self {
        let width = vcs_marker_width(setting);
        match marker_x {
            Some(x) if width > Pixels::ZERO => Self::GutterEdge { x, width },
            _ => Self::LeftEdge {
                strip_width: left_edge_strip_width(setting, line_height),
                deleted_marker_width: left_edge_deleted_marker_width(setting, line_height),
            },
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

fn vcs_marker_width(setting: GitGutterWidth) -> Pixels {
    match setting {
        GitGutterWidth::Custom(width) => px(width.0),
        GitGutterWidth::Default => VCS_MARKER_DEFAULT_WIDTH,
    }
}

fn vcs_marker_inset(marker_width: Pixels) -> Pixels {
    if marker_width < NARROW_VCS_MARKER_WIDTH_LIMIT {
        VCS_MARKER_AREA_INSET + NARROW_VCS_MARKER_EXTRA_INSET
    } else {
        VCS_MARKER_AREA_INSET
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

pub(crate) fn line_number_font_size(buffer_font_size: Pixels) -> Pixels {
    (buffer_font_size + LINE_NUMBER_FONT_SIZE_DELTA).max(MINIMUM_LINE_NUMBER_FONT_SIZE)
}

pub(crate) fn line_numbers_area_width(widest_line_number_width: Pixels) -> Pixels {
    widest_line_number_width
        .round()
        .max(LINE_NUMBER_AREA_MINIMUM_WIDTH)
}

pub(crate) fn annotations_area_width(annotations_end: Pixels) -> Pixels {
    annotations_end + GAP_AFTER_ANNOTATIONS
}

pub(super) fn gutter_button_x(icon_area_x: Pixels, button_width: Pixels) -> Pixels {
    icon_area_x + GUTTER_ICON_INSET + (GUTTER_ICON_WIDTH - button_width) / 2.
}

pub(super) fn crease_toggle_x(fold_area_x: Pixels, toggle_width: Pixels) -> Pixels {
    fold_area_x + (FOLD_ICON_WIDTH - toggle_width) / 2.
}

pub(crate) fn icon_area_width(has_marker_icons: bool) -> Pixels {
    if has_marker_icons {
        ICON_AREA_WIDTH
    } else {
        Pixels::ZERO
    }
}

pub(super) fn line_number_button_x(line_numbers_end: Pixels, button_width: Pixels) -> Pixels {
    (line_numbers_end - button_width).max(Pixels::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::PixelSetting;

    const LINE_HEIGHT: Pixels = px(20.);
    const GUTTER_WIDTH: Pixels = px(60.);
    const MARKER_X: Pixels = px(54.);
    const JETBRAINS_MONO_12_DIGIT_ADVANCE: Pixels = px(7.2);

    fn gutter_origin() -> Point<Pixels> {
        point(px(10.), px(5.))
    }

    fn full_gutter(density: UiDensity, line_number_digits: usize) -> GutterContent {
        GutterContent {
            gaps: GutterGaps::for_density(density),
            leading_area_width: None,
            line_numbers_width: Some(line_numbers_area_width(
                JETBRAINS_MONO_12_DIGIT_ADVANCE * line_number_digits,
            )),
            icons: true,
            icon_area_width: ICON_AREA_WIDTH,
            folds: true,
            vcs_markers: Some(GitGutterWidth::Default),
        }
    }

    #[test]
    fn intellij_gutter_compact_two_digit_file_lays_out_numbers_icons_folds_and_marker() {
        let areas = full_gutter(UiDensity::Compact, 2).layout();

        assert_eq!(
            areas,
            GutterAreas {
                line_numbers_x: px(2.),
                line_numbers_end: px(16.),
                icon_area_x: px(18.),
                fold_area_x: px(36.),
                vcs_marker_x: Some(px(50.)),
                width: px(54.),
                text_margin: Some(px(2.)),
            },
            "2 px gap, 14 px numbers, 2 px gap, 16 px icons, 2 px gap, 11 px folds, \
             then a 4 px marker 3 px into the 8 px VCS area that covers the separator"
        );
    }

    #[test]
    fn intellij_gutter_compact_three_digit_file_matches_webstorm() {
        let areas = full_gutter(UiDensity::Compact, 3).layout();

        assert_eq!(
            areas.line_numbers_end,
            px(24.),
            "WebStorm ends three-digit line numbers 24 px from the editor island edge"
        );
        assert_eq!(areas.icon_area_x, px(26.));
        assert_eq!(areas.fold_area_x, px(44.));
        assert_eq!(areas.vcs_marker_x, Some(px(58.)));
        assert_eq!(
            areas.width - GUTTER_SEPARATOR_WIDTH,
            px(61.),
            "WebStorm's separator is at 62 because its icon area is 1 px wider"
        );
        assert_eq!(
            areas.text_margin.map(|margin| areas.width + margin),
            Some(px(64.)),
            "the code starts 3 px after the separator, WebStorm's starts at 65"
        );
    }

    #[test]
    fn intellij_gutter_default_density_uses_four_pixel_gaps_and_the_empty_annotation_area() {
        let areas = full_gutter(UiDensity::Default, 3).layout();

        assert_eq!(
            areas,
            GutterAreas {
                line_numbers_x: px(8.),
                line_numbers_end: px(30.),
                icon_area_x: px(34.),
                fold_area_x: px(54.),
                vcs_marker_x: Some(px(68.)),
                width: px(72.),
                text_margin: Some(px(2.)),
            }
        );
        assert_eq!(
            GutterGaps::for_density(UiDensity::Comfortable),
            GutterGaps::for_density(UiDensity::Default),
        );
    }

    #[test]
    fn intellij_gutter_line_number_area_fits_the_widest_number_and_a_breakpoint_icon() {
        assert_eq!(
            line_numbers_area_width(px(7.2)),
            px(14.),
            "a one-digit file still reserves the 14 px breakpoint placeholder"
        );
        assert_eq!(line_numbers_area_width(px(14.4)), px(14.));
        assert_eq!(line_numbers_area_width(px(21.6)), px(22.));
        assert_eq!(line_numbers_area_width(px(28.8)), px(29.));
    }

    #[test]
    fn intellij_gutter_without_line_numbers_starts_with_the_icon_area() {
        let areas = GutterContent {
            line_numbers_width: None,
            ..full_gutter(UiDensity::Compact, 2)
        }
        .layout();

        assert_eq!(
            areas,
            GutterAreas {
                line_numbers_x: Pixels::ZERO,
                line_numbers_end: Pixels::ZERO,
                icon_area_x: Pixels::ZERO,
                fold_area_x: px(18.),
                vcs_marker_x: Some(px(32.)),
                width: px(36.),
                text_margin: Some(px(2.)),
            }
        );
    }

    #[test]
    fn intellij_gutter_without_vcs_markers_ends_with_the_separator_after_the_folds() {
        for vcs_markers in [None, Some(GitGutterWidth::Custom(PixelSetting(0.)))] {
            let areas = GutterContent {
                vcs_markers,
                ..full_gutter(UiDensity::Compact, 2)
            }
            .layout();

            assert_eq!(areas.vcs_marker_x, None);
            assert_eq!(
                areas.width,
                px(48.),
                "no VCS area is reserved when markers can never appear"
            );
        }
    }

    #[test]
    fn intellij_gutter_collapses_when_nothing_can_be_shown() {
        let areas = GutterContent {
            gaps: GutterGaps::for_density(UiDensity::Default),
            leading_area_width: None,
            line_numbers_width: None,
            icons: false,
            icon_area_width: Pixels::ZERO,
            folds: false,
            vcs_markers: None,
        }
        .layout();

        assert_eq!(areas, GutterAreas::default());
    }

    #[test]
    fn intellij_gutter_leading_area_replaces_the_empty_annotation_area() {
        let areas = GutterContent {
            leading_area_width: Some(annotations_area_width(px(40.))),
            ..full_gutter(UiDensity::Default, 2)
        }
        .layout();

        assert_eq!(
            areas.line_numbers_x,
            px(49.),
            "the 5 px gap after the annotations and the 4 px gap before the line numbers"
        );
    }

    #[test]
    fn intellij_gutter_icons_and_fold_toggles_are_centered_on_intellij_icon_slots() {
        assert_eq!(
            gutter_button_x(px(18.), px(14.)),
            px(20.),
            "a 14 px icon starts 2 px into the icon area"
        );
        assert_eq!(gutter_button_x(px(18.), px(12.)), px(21.));
        assert_eq!(
            crease_toggle_x(px(36.), px(9.)),
            px(36.),
            "a 9 px fold icon starts at the folding area"
        );
        assert_eq!(crease_toggle_x(px(36.), px(14.)), px(33.5));
    }

    #[test]
    fn intellij_gutter_marker_sits_at_the_vcs_marker_x_with_default_width() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, Some(MARKER_X));

        assert_eq!(
            placement,
            VcsMarkerPlacement::GutterEdge {
                x: px(54.),
                width: px(4.),
            },
        );
    }

    #[test]
    fn intellij_gutter_modified_range_marker_is_an_inset_capsule() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, Some(MARKER_X));

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
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, Some(MARKER_X));

        let bounds = placement.row_range_bounds(gutter_origin(), px(20.), px(20.) + LINE_HEIGHT);

        assert_eq!(
            bounds,
            Bounds::new(point(px(64.), px(26.)), size(px(4.), px(18.))),
        );
    }

    #[test]
    fn intellij_gutter_deleted_marker_is_a_short_capsule_centered_on_the_line_boundary() {
        let placement =
            VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, Some(MARKER_X));

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
            Some(px(52.)),
        );

        assert_eq!(
            placement,
            VcsMarkerPlacement::GutterEdge {
                x: px(52.),
                width: px(6.),
            },
        );
        assert_eq!(placement.marker_corner_radii(), Corners::all(px(3.)));

        let areas = GutterContent {
            vcs_markers: Some(GitGutterWidth::Custom(PixelSetting(6.))),
            ..full_gutter(UiDensity::Compact, 2)
        }
        .layout();
        assert_eq!(
            areas.vcs_marker_x,
            Some(px(48.)),
            "markers of 5 px and wider skip IntelliJ's extra 2 px inset"
        );
        assert_eq!(areas.width, px(54.));
    }

    #[test]
    fn intellij_gutter_marker_falls_back_to_the_left_edge_without_a_vcs_area() {
        let placement = VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, None);

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
        let placement = VcsMarkerPlacement::new(GitGutterWidth::Default, LINE_HEIGHT, None);

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
            Some(MARKER_X),
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

    #[test]
    fn intellij_gutter_compact_file_without_icons_keeps_only_the_gaps_between_numbers_and_folds() {
        let areas = GutterContent {
            icon_area_width: icon_area_width(false),
            ..full_gutter(UiDensity::Compact, 3)
        }
        .layout();

        assert_eq!(areas.line_numbers_end, px(24.));
        assert_eq!(
            areas.fold_area_x - areas.line_numbers_end,
            px(4.),
            "IntelliJ reserves no icon area without gutter icons, only the 2 px gaps after the numbers and after the icons"
        );
        assert_eq!(areas.vcs_marker_x, Some(px(42.)));
        assert_eq!(areas.width, px(46.));
    }

    #[test]
    fn intellij_gutter_default_density_file_without_icons_keeps_eight_pixels_before_the_folds() {
        let areas = GutterContent {
            icon_area_width: icon_area_width(false),
            ..full_gutter(UiDensity::Default, 3)
        }
        .layout();

        assert_eq!(areas.line_numbers_end, px(30.));
        assert_eq!(areas.fold_area_x - areas.line_numbers_end, px(8.));
    }

    #[test]
    fn intellij_gutter_icon_area_exists_only_when_a_marker_icon_exists() {
        assert_eq!(
            icon_area_width(true),
            px(16.),
            "a 14 px icon plus 1 px on each side"
        );
        assert_eq!(icon_area_width(false), Pixels::ZERO);
    }

    #[test]
    fn intellij_gutter_hover_button_is_right_aligned_to_the_line_numbers() {
        assert_eq!(line_number_button_x(px(24.), px(14.)), px(10.));
        assert_eq!(
            line_number_button_x(px(6.), px(14.)),
            Pixels::ZERO,
            "the button never leaves the gutter on the left"
        );
    }
}
