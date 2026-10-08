use std::rc::Rc;

use editor::Editor;
use gpui::{
    App, Bounds, ContentMask, CursorStyle, DefiniteLength, Element, ElementId, Entity,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Position, Style, Window, fill, point, px,
};

use super::geometry::{PixelRect, chunk_border_line_rects, editor_border_lines};
use super::pane::PaneGeometry;
use super::{FoldedLine, rect_bounds};
use crate::merge_tool::merge_ribbons::wave::{
    PathCommand, editor_wave_shift, paint_stroke, wave_commands, wave_stroke_width,
    wave_vertical_offset,
};
use crate::merge_tool::palette::highlights::PaneChange;
use crate::merge_tool::palette::{change_colors, current_palette, hex_color};

const VISIBLE_CULL_MARGIN: f32 = 4.0;
const EMPTY_MARKER_WIDTH: f32 = 2.0;
const EMPTY_MARKER_HALF_WIDTH: f32 = 1.0;
const WAVE_TOP_NUDGE: f32 = 1.0;

pub(crate) fn content_rect_to_window(rect: PixelRect, area_left: f32, scroll_x: f32) -> PixelRect {
    PixelRect {
        x: area_left + rect.x - scroll_x,
        ..rect
    }
}

pub(crate) fn empty_inline_marker_rect(x: f32, line_top: f32, line_height: f32) -> PixelRect {
    PixelRect::new(
        x - EMPTY_MARKER_HALF_WIDTH,
        line_top,
        EMPTY_MARKER_WIDTH,
        line_height,
    )
}

#[derive(Clone)]
pub(crate) struct FoldInteraction {
    pub on_hover: Rc<dyn Fn(Option<usize>, &mut Window, &mut App)>,
    pub on_expand: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
}

pub(crate) fn register_fold_click(
    window: &mut Window,
    regions: &[FoldHitRegion],
    on_expand: &Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
) {
    let click_regions = regions.to_vec();
    let on_expand = on_expand.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if !phase.bubble() || event.button != MouseButton::Left {
            return;
        }
        let clicked = click_regions
            .iter()
            .find(|region| region.hitbox.is_hovered(window))
            .map(|region| (region.group, region.block));
        if let Some((group, block)) = clicked {
            on_expand(group, block, window, cx);
        }
    });
}

fn register_fold_interaction(
    window: &mut Window,
    regions: &[FoldHitRegion],
    interaction: &FoldInteraction,
) {
    let hover_regions = regions.to_vec();
    let on_hover = interaction.on_hover.clone();
    window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
        if !phase.bubble() {
            return;
        }
        let hovered_group = hover_regions
            .iter()
            .find(|region| region.hitbox.is_hovered(window))
            .map(|region| region.group);
        on_hover(hovered_group, window, cx);
    });
    register_fold_click(window, regions, &interaction.on_expand);
}

#[derive(Clone)]
pub(crate) struct FoldHitRegion {
    pub(crate) group: usize,
    pub(crate) block: usize,
    pub(crate) hitbox: Hitbox,
}

pub(crate) struct MergeEditorOverlay {
    id: ElementId,
    editor: Entity<Editor>,
    mirrored: bool,
    gutter_width: f32,
    changes: Vec<PaneChange>,
    folded_lines: Vec<FoldedLine>,
    fold_interaction: Option<FoldInteraction>,
}

impl MergeEditorOverlay {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        editor: Entity<Editor>,
        mirrored: bool,
        gutter_width: f32,
        changes: Vec<PaneChange>,
    ) -> Self {
        Self {
            id: id.into(),
            editor,
            mirrored,
            gutter_width,
            changes,
            folded_lines: Vec::new(),
            fold_interaction: None,
        }
    }

    pub(crate) fn folded_lines(mut self, folded_lines: Vec<FoldedLine>) -> Self {
        self.folded_lines = folded_lines;
        self
    }

    pub(crate) fn fold_interaction(mut self, interaction: FoldInteraction) -> Self {
        self.fold_interaction = Some(interaction);
        self
    }
}

struct OverlayWave {
    commands: Vec<PathCommand>,
    width: f32,
}

pub(crate) struct OverlayPrepaint {
    area: Bounds<Pixels>,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    waves: Vec<OverlayWave>,
    wave_color: Hsla,
    fold_regions: Vec<FoldHitRegion>,
}

impl MergeEditorOverlay {
    fn border_rects(&self, geometry: &PaneGeometry, cx: &App) -> Vec<(Bounds<Pixels>, Hsla)> {
        let view_top = geometry.top() - VISIBLE_CULL_MARGIN;
        let view_bottom = geometry.top() + geometry.height() + VISIBLE_CULL_MARGIN;
        let area_left = f32::from(geometry.bounds.left());
        let area_width = f32::from(geometry.bounds.size.width);
        let scroll_x = f32::from(geometry.metrics.scroll_x).floor();
        let mut rects = Vec::new();
        for change in &self.changes {
            if geometry.is_range_folded(&change.lines) {
                continue;
            }
            let colors = change_colors(cx, change.kind, change.ai_resolved);
            let lines = editor_border_lines(
                change,
                |line| geometry.line_top_y(line).round(),
                |line| geometry.line_bottom_y(line).round(),
            );
            for line in lines {
                if line.y < view_top || line.y > view_bottom {
                    continue;
                }
                let content_rects = chunk_border_line_rects(
                    scroll_x,
                    scroll_x + area_width,
                    line.y,
                    line.double_line,
                    line.dotted,
                );
                for content_rect in content_rects {
                    let window_rect =
                        content_rect_to_window(content_rect.rect, area_left, scroll_x);
                    rects.push((rect_bounds(window_rect), colors.solid));
                }
            }
            if change.word_diff_ready && !change.resolved {
                for range in change
                    .inner_ranges
                    .iter()
                    .filter(|range| range.start == range.end)
                {
                    let marker_point = geometry.offset_point(change.lines.start, range.start);
                    let x = geometry.x_of_point(marker_point);
                    let top = geometry.line_top_y(marker_point.row).round();
                    if top < view_top || top > view_bottom {
                        continue;
                    }
                    let marker = empty_inline_marker_rect(x, top, geometry.line_height);
                    rects.push((rect_bounds(marker), colors.solid));
                }
            }
        }
        rects
    }

    fn fold_regions(&self, geometry: &PaneGeometry, window: &mut Window) -> Vec<FoldHitRegion> {
        let view_top = geometry.top() - geometry.line_height;
        let view_bottom = geometry.top() + geometry.height();
        let area_left = f32::from(geometry.bounds.left());
        let area_width = f32::from(geometry.bounds.size.width);
        let mut regions = Vec::new();
        for folded in &self.folded_lines {
            let line_top = geometry.line_top_y(folded.line).round();
            if line_top < view_top || line_top > view_bottom {
                continue;
            }
            let row = PixelRect::new(area_left, line_top, area_width, geometry.line_height);
            regions.push(FoldHitRegion {
                group: folded.group,
                block: folded.block,
                hitbox: window.insert_hitbox(rect_bounds(row), HitboxBehavior::Normal),
            });
        }
        regions
    }

    fn waves(&self, geometry: &PaneGeometry) -> Vec<OverlayWave> {
        let view_top = geometry.top() - geometry.line_height;
        let view_bottom = geometry.top() + geometry.height();
        let area_left = f32::from(geometry.bounds.left());
        let area_width = f32::from(geometry.bounds.size.width);
        let tile_origin = area_left
            + editor_wave_shift(
                area_width as i32,
                self.gutter_width as i32,
                self.mirrored,
                0,
            ) as f32;
        self.folded_lines
            .iter()
            .filter_map(|folded| {
                let line_top = geometry.line_top_y(folded.line).round();
                if line_top < view_top || line_top > view_bottom {
                    return None;
                }
                let top = line_top + WAVE_TOP_NUDGE + wave_vertical_offset(geometry.line_height);
                Some(OverlayWave {
                    commands: wave_commands(tile_origin, top, area_left..area_left + area_width),
                    width: wave_stroke_width(folded.hovered),
                })
            })
            .collect()
    }
}

impl IntoElement for MergeEditorOverlay {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MergeEditorOverlay {
    type RequestLayoutState = ();
    type PrepaintState = Option<OverlayPrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset.top = DefiniteLength::Fraction(0.).into();
        style.inset.left = DefiniteLength::Fraction(0.).into();
        style.size.width = DefiniteLength::Fraction(1.).into();
        style.size.height = DefiniteLength::Fraction(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(geometry) = PaneGeometry::measure(&self.editor, window, cx) else {
            window.request_animation_frame();
            return None;
        };
        let rects = self.border_rects(&geometry, cx);
        let waves = self.waves(&geometry);
        let fold_regions = self.fold_regions(&geometry, window);
        Some(OverlayPrepaint {
            area: geometry.bounds,
            rects,
            waves,
            fold_regions,
            wave_color: hex_color(current_palette(cx).wave),
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            return;
        };
        if let Some(interaction) = &self.fold_interaction {
            register_fold_interaction(window, &prepaint.fold_regions, interaction);
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.area,
                ..Default::default()
            }),
            |window| {
                for (bounds, color) in &prepaint.rects {
                    window.paint_quad(fill(*bounds, *color));
                }
                for wave in &prepaint.waves {
                    paint_stroke(
                        window,
                        &wave.commands,
                        point(px(0.), px(0.)),
                        wave.width,
                        prepaint.wave_color,
                    );
                }
                for region in &prepaint.fold_regions {
                    window.set_cursor_style(CursorStyle::PointingHand, &region.hitbox);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_content_rect_follows_horizontal_scroll() {
        let rect = PixelRect::new(24.0, 100.0, 2.0, 2.0);
        let moved = content_rect_to_window(rect, 300.0, 20.0);
        assert_eq!(moved, PixelRect::new(304.0, 100.0, 2.0, 2.0));
    }

    #[test]
    fn merge_tool_empty_inline_marker_is_two_pixels_wide_and_centred_on_offset() {
        let marker = empty_inline_marker_rect(50.0, 80.0, 20.0);
        assert_eq!(marker, PixelRect::new(49.0, 80.0, 2.0, 20.0));
    }
}
