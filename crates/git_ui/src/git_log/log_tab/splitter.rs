use gpui::{Bounds, ClickEvent, DragMoveEvent, EntityId, Hsla, Point};
use ui::prelude::*;

use crate::git_log::properties::LogAppProperties;

const DIVIDER_WIDTH: Pixels = px(1.);
const MOUSE_ZONE: Pixels = px(6.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SplitterKind {
    Branches,
    Changes,
    Details,
}

impl SplitterKind {
    fn is_vertical(self) -> bool {
        matches!(self, SplitterKind::Details)
    }

    fn handle_id(self) -> &'static str {
        match self {
            SplitterKind::Branches => "git-log-branches-splitter",
            SplitterKind::Changes => "git-log-changes-splitter",
            SplitterKind::Details => "git-log-details-splitter",
        }
    }

    pub(super) fn proportion(self, properties: &LogAppProperties) -> f32 {
        match self {
            SplitterKind::Branches => properties.branches_splitter_proportion,
            SplitterKind::Changes => properties.changes_splitter_proportion,
            SplitterKind::Details => properties.details_splitter_proportion,
        }
    }

    pub(super) fn with_proportion(
        self,
        properties: &LogAppProperties,
        proportion: f32,
    ) -> LogAppProperties {
        let mut properties = properties.clone();
        match self {
            SplitterKind::Branches => properties.branches_splitter_proportion = proportion,
            SplitterKind::Changes => properties.changes_splitter_proportion = proportion,
            SplitterKind::Details => properties.details_splitter_proportion = proportion,
        }
        properties
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DraggedLogSplitter {
    pub(super) tab: EntityId,
    pub(super) kind: SplitterKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Splitter {
    pub(super) kind: SplitterKind,
    pub(super) proportion: f32,
    pub(super) leading_size: Pixels,
    pub(super) first_minimum: Pixels,
    pub(super) second_minimum: Pixels,
}

impl Splitter {
    pub(super) fn dragged_proportion(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> f32 {
        let (offset, extent) = if self.kind.is_vertical() {
            (position.y - bounds.top(), bounds.size.height)
        } else {
            (position.x - bounds.left(), bounds.size.width)
        };
        let thickness = (extent - self.leading_size).as_f32();
        let total = thickness - DIVIDER_WIDTH.as_f32();
        let first_minimum = self.first_minimum.as_f32();
        let second_minimum = self.second_minimum.as_f32();
        dragged_proportion(
            (offset - self.leading_size).as_f32(),
            thickness,
            minimum_proportion(first_minimum, second_minimum, total),
            minimum_proportion(second_minimum, first_minimum, total),
        )
    }

    pub(super) fn render(
        self,
        tab: EntityId,
        first: AnyElement,
        second: AnyElement,
        divider_color: Hsla,
        on_divider_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_drag_move: impl Fn(&DragMoveEvent<DraggedLogSplitter>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let vertical = self.kind.is_vertical();
        let first_minimum = self.leading_size + self.first_minimum;
        let second_minimum = self.second_minimum;
        let first_pane = div()
            .flex_basis(self.leading_size)
            .flex_grow(self.proportion)
            .flex_shrink_1()
            .overflow_hidden()
            .map(|pane| {
                if vertical {
                    pane.w_full().min_h(first_minimum)
                } else {
                    pane.h_full().min_w(first_minimum)
                }
            })
            .child(first);
        let handle = div()
            .id(self.kind.handle_id())
            .absolute()
            .block_mouse_except_scroll()
            .map(|handle| {
                if vertical {
                    handle
                        .left_0()
                        .w_full()
                        .top(-MOUSE_ZONE)
                        .h(MOUSE_ZONE * 2.)
                        .cursor_row_resize()
                } else {
                    handle
                        .top_0()
                        .h_full()
                        .left(-MOUSE_ZONE)
                        .w(MOUSE_ZONE * 2.)
                        .cursor_col_resize()
                }
            })
            .on_click(on_divider_click)
            .on_drag(
                DraggedLogSplitter {
                    tab,
                    kind: self.kind,
                },
                |_, _, _, cx| cx.new(|_| gpui::Empty),
            );
        let divider = div()
            .relative()
            .flex_none()
            .bg(divider_color)
            .map(|divider| {
                if vertical {
                    divider.w_full().h(DIVIDER_WIDTH)
                } else {
                    divider.h_full().w(DIVIDER_WIDTH)
                }
            })
            .child(handle);
        let second_pane = div()
            .flex_basis(Pixels::ZERO)
            .flex_grow(1. - self.proportion)
            .flex_shrink_1()
            .overflow_hidden()
            .map(|pane| {
                if vertical {
                    pane.w_full().min_h(second_minimum)
                } else {
                    pane.h_full().min_w(second_minimum)
                }
            })
            .child(second);
        div()
            .flex()
            .size_full()
            .map(|split| {
                if vertical {
                    split.flex_col()
                } else {
                    split.flex_row()
                }
            })
            .on_drag_move(on_drag_move)
            .child(first_pane)
            .child(divider)
            .child(second_pane)
            .into_any_element()
    }
}

fn minimum_proportion(minimum: f32, other_minimum: f32, total: f32) -> f32 {
    if total <= 0. || total < minimum + other_minimum {
        0.
    } else {
        minimum / total
    }
}

fn dragged_proportion(
    position: f32,
    thickness: f32,
    first_minimum_proportion: f32,
    second_minimum_proportion: f32,
) -> f32 {
    let thickness = if thickness <= 0. { 1. } else { thickness };
    (position / thickness)
        .max(first_minimum_proportion)
        .min(1. - second_minimum_proportion)
        .clamp(0., 1.)
}

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    fn horizontal(leading_size: Pixels, first_minimum: Pixels, second_minimum: Pixels) -> Splitter {
        Splitter {
            kind: SplitterKind::Changes,
            proportion: 0.7,
            leading_size,
            first_minimum,
            second_minimum,
        }
    }

    fn bounds(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(width), px(height)),
        }
    }

    #[test]
    fn dragging_follows_the_mouse_relative_to_the_splitter_start() {
        let splitter = horizontal(Pixels::ZERO, Pixels::ZERO, Pixels::ZERO);
        let proportion =
            splitter.dragged_proportion(point(px(100. + 250.), px(60.)), bounds(1000., 400.));
        assert_eq!(proportion, 0.25);
    }

    #[test]
    fn dragging_excludes_the_leading_toolbar_from_the_splitter() {
        let splitter = Splitter {
            kind: SplitterKind::Branches,
            ..horizontal(px(36.), Pixels::ZERO, Pixels::ZERO)
        };
        let proportion = splitter.dragged_proportion(
            point(px(100. + 36. + 241.), px(60.)),
            bounds(1000. + 36., 400.),
        );
        assert!((proportion - 0.241).abs() < 1e-6);
    }

    #[test]
    fn dragging_honours_both_minimum_sizes_like_one_pixel_divider() {
        let splitter = horizontal(Pixels::ZERO, px(100.), px(60.));
        let frame = bounds(1001., 400.);
        let near_start = splitter.dragged_proportion(point(px(110.), px(60.)), frame);
        assert_eq!(near_start, 100. / 1000.);
        let near_end = splitter.dragged_proportion(point(px(100. + 995.), px(60.)), frame);
        assert_eq!(near_end, 1. - 60. / 1000.);
    }

    #[test]
    fn minimum_sizes_stop_applying_when_the_splitter_is_too_small_for_both() {
        let splitter = horizontal(Pixels::ZERO, px(100.), px(60.));
        let frame = bounds(121., 400.);
        let proportion = splitter.dragged_proportion(point(px(100.), px(60.)), frame);
        assert_eq!(proportion, 0.);
        let proportion = splitter.dragged_proportion(point(px(100. + 500.), px(60.)), frame);
        assert_eq!(proportion, 1.);
    }

    #[test]
    fn vertical_splitters_measure_along_the_height() {
        let splitter = Splitter {
            kind: SplitterKind::Details,
            ..horizontal(Pixels::ZERO, Pixels::ZERO, Pixels::ZERO)
        };
        let proportion =
            splitter.dragged_proportion(point(px(900.), px(50. + 300.)), bounds(1000., 400.));
        assert_eq!(proportion, 0.75);
    }

    #[test]
    fn proportions_are_read_and_replaced_per_splitter() {
        let properties = LogAppProperties::default();
        let updated = SplitterKind::Details.with_proportion(&properties, 0.4);
        assert_eq!(SplitterKind::Details.proportion(&updated), 0.4);
        assert_eq!(
            SplitterKind::Branches.proportion(&updated),
            properties.branches_splitter_proportion
        );
        assert_eq!(
            SplitterKind::Changes.proportion(&updated),
            properties.changes_splitter_proportion
        );
    }
}
