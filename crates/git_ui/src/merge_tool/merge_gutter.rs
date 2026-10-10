pub(crate) mod borders;
pub(crate) mod geometry;
pub(crate) mod pane;

use std::rc::Rc;

use self::borders::{FoldHitRegion, register_fold_click};
use collections::HashMap;
use editor::Editor;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClickEvent, ContentMask, CursorStyle, Element,
    ElementId, Entity, Font, GlobalElementId, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, Pixels, ShapedLine, SharedString, Style, TextAlign,
    TextRun, Window, fill, point, px, size, svg,
};
use settings::Settings as _;
use theme_settings::ThemeSettings;
use ui::{IconName, Tooltip, prelude::*};
use util::ResultExt as _;

use self::geometry::{
    FOLD_CHEVRON_SIZE, FillRole, GutterLayout, ICON_SIZE, PixelRect, fold_chevron_top,
    gutter_band_rects, gutter_layout, icon_left, icon_top_offset, icons_area_width,
    line_number_left, marker_paint_range,
};
use self::pane::PaneGeometry;
use super::merge_ribbons::wave::{
    PathCommand, WAVE_START_PHASE, paint_stroke, wave_commands, wave_stroke_width,
    wave_vertical_offset,
};
use super::palette::highlights::PaneChange;
use super::palette::{
    MergePaletteKind, change_colors, current_palette, editor_background, hex_color,
};

const VISIBLE_CULL_MARGIN: f32 = 4.0;
const CTRL_CLICK_HINT: &str = "Ctrl+click to resolve conflict";
const SEPARATOR_LINE_WIDTH: f32 = 1.0;
const TOOLTIP_APPENDIX_MARGIN: f32 = 5.0;

pub(crate) type FoldExpand = Rc<dyn Fn(usize, usize, &mut Window, &mut App)>;

fn indent_guide_hex(kind: MergePaletteKind) -> u32 {
    match kind {
        MergePaletteKind::EvaDark => 0x454963,
        MergePaletteKind::IslandsDark => 0x323438,
        MergePaletteKind::Light => 0xEBECF0,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GutterIconKind {
    AcceptFromLeft,
    AcceptFromRight,
    AppendFromLeft,
    AppendFromRight,
    Ignore,
    Resolve,
}

impl GutterIconKind {
    pub(crate) fn icon_name(self) -> IconName {
        match self {
            GutterIconKind::AcceptFromLeft => IconName::DiffArrowRight,
            GutterIconKind::AcceptFromRight => IconName::DiffArrow,
            GutterIconKind::AppendFromLeft => IconName::DiffArrowRightDown,
            GutterIconKind::AppendFromRight => IconName::DiffArrowLeftDown,
            GutterIconKind::Ignore => IconName::DiffRemove,
            GutterIconKind::Resolve => IconName::DiffMagicResolve,
        }
    }

    pub(crate) fn tooltip_title(self) -> &'static str {
        match self {
            GutterIconKind::AcceptFromLeft
            | GutterIconKind::AcceptFromRight
            | GutterIconKind::AppendFromLeft
            | GutterIconKind::AppendFromRight => "Accept",
            GutterIconKind::Ignore => "Ignore",
            GutterIconKind::Resolve => "Resolve",
        }
    }

    pub(crate) fn offers_ctrl_click(self) -> bool {
        !matches!(self, GutterIconKind::Resolve)
    }
}

pub(crate) fn tooltip_hint(kind: GutterIconKind, conflict: bool) -> Option<&'static str> {
    (conflict && kind.offers_ctrl_click()).then_some(CTRL_CLICK_HINT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GutterIconClick {
    pub ctrl: bool,
}

#[derive(Clone)]
pub(crate) struct GutterIconSpec {
    pub line: u32,
    pub kind: GutterIconKind,
    pub conflict: bool,
    pub enabled: bool,
    pub on_click: Rc<dyn Fn(GutterIconClick, &mut Window, &mut App)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FoldedLine {
    pub group: usize,
    pub block: usize,
    pub line: u32,
    pub hovered: bool,
}

pub(crate) fn max_icons_per_line(icons: &[GutterIconSpec]) -> usize {
    let mut counts: HashMap<u32, usize> = HashMap::default();
    for icon in icons.iter().filter(|icon| icon.enabled) {
        *counts.entry(icon.line).or_default() += 1;
    }
    counts.values().copied().max().unwrap_or(0)
}

fn editor_font(cx: &App) -> (Font, Pixels) {
    let settings = ThemeSettings::get_global(cx);
    (settings.buffer_font.clone(), settings.buffer_font_size(cx))
}

pub(crate) fn line_number_text_width(window: &mut Window, cx: &App, line_count: u32) -> f32 {
    let (font, font_size) = editor_font(cx);
    let text = SharedString::from(line_count.max(1).to_string());
    let run = TextRun {
        len: text.len(),
        font,
        ..Default::default()
    };
    let shaped = window
        .text_system()
        .shape_line(text, font_size, &[run], None);
    f32::from(shaped.width).ceil()
}

pub(crate) fn gutter_width(
    window: &mut Window,
    cx: &App,
    line_count: u32,
    max_icons_per_row: usize,
    show_line_numbers: bool,
) -> f32 {
    gutter_layout(
        line_number_text_width(window, cx, line_count),
        icons_area_width(max_icons_per_row),
        show_line_numbers,
    )
    .width
}

pub(crate) fn rect_bounds(rect: PixelRect) -> Bounds<Pixels> {
    Bounds::new(
        point(px(rect.x), px(rect.y)),
        size(px(rect.width), px(rect.height)),
    )
}

pub(crate) fn physical_bounds(
    rect: PixelRect,
    origin_x: f32,
    container_width: f32,
    mirrored: bool,
) -> Bounds<Pixels> {
    let rect = if mirrored {
        rect.mirrored(container_width)
    } else {
        rect
    };
    rect_bounds(PixelRect {
        x: origin_x + rect.x,
        ..rect
    })
}

fn icon_element(
    index: usize,
    spec: &GutterIconSpec,
    color: Hsla,
    id_namespace: &'static str,
) -> AnyElement {
    let kind = spec.kind;
    let hint = tooltip_hint(kind, spec.conflict);
    let click_handler = spec.on_click.clone();
    let ctrl_click_handler = spec.on_click.clone();
    div()
        .id((id_namespace, index))
        .size(px(ICON_SIZE))
        .flex_none()
        .cursor_pointer()
        .child(
            svg()
                .size(px(ICON_SIZE))
                .flex_none()
                .path(SharedString::from(kind.icon_name().path()))
                .text_color(color),
        )
        .tooltip(Tooltip::element(move |_window, _cx| {
            tooltip_content(kind.tooltip_title(), hint)
        }))
        .on_click(move |event: &ClickEvent, window, cx| {
            click_handler(
                GutterIconClick {
                    ctrl: event.modifiers().control,
                },
                window,
                cx,
            );
        })
        .on_mouse_up(MouseButton::Right, move |_, window, cx| {
            ctrl_click_handler(GutterIconClick { ctrl: true }, window, cx);
        })
        .into_any_element()
}

fn tooltip_content(title: &'static str, hint: Option<&'static str>) -> AnyElement {
    v_flex()
        .child(Label::new(title))
        .when_some(hint, |content, hint| {
            content.child(
                div()
                    .mt(px(TOOLTIP_APPENDIX_MARGIN))
                    .child(Label::new(hint).size(LabelSize::Small)),
            )
        })
        .into_any_element()
}

fn chevron_icon(mirrored: bool) -> IconName {
    if mirrored {
        IconName::GutterUnfoldMirrored
    } else {
        IconName::GutterUnfold
    }
}

pub(crate) struct MergeGutter {
    id: ElementId,
    editor: Entity<Editor>,
    mirrored: bool,
    line_count: u32,
    changes: Vec<PaneChange>,
    icons: Vec<GutterIconSpec>,
    folded_lines: Vec<FoldedLine>,
    show_line_numbers: bool,
    fold_expand: Option<FoldExpand>,
    icon_id_namespace: &'static str,
}

impl MergeGutter {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        icon_id_namespace: &'static str,
        editor: Entity<Editor>,
        mirrored: bool,
        line_count: u32,
        changes: Vec<PaneChange>,
        icons: Vec<GutterIconSpec>,
    ) -> Self {
        Self {
            id: id.into(),
            editor,
            mirrored,
            line_count,
            changes,
            icons,
            folded_lines: Vec::new(),
            show_line_numbers: true,
            fold_expand: None,
            icon_id_namespace,
        }
    }

    pub(crate) fn folded_lines(mut self, folded_lines: Vec<FoldedLine>) -> Self {
        self.folded_lines = folded_lines;
        self
    }

    pub(crate) fn line_numbers_visible(mut self, visible: bool) -> Self {
        self.show_line_numbers = visible;
        self
    }

    pub(crate) fn fold_expand(mut self, expand: FoldExpand) -> Self {
        self.fold_expand = Some(expand);
        self
    }

    fn compute_layout(&self, window: &mut Window, cx: &App) -> GutterLayout {
        gutter_layout(
            line_number_text_width(window, cx, self.line_count),
            icons_area_width(max_icons_per_line(&self.icons)),
            self.show_line_numbers,
        )
    }
}

struct PreparedWave {
    commands: Vec<PathCommand>,
    width: f32,
}

pub(crate) struct GutterPrepaint {
    bounds: Bounds<Pixels>,
    background: Hsla,
    separator: Bounds<Pixels>,
    separator_color: Hsla,
    line_height: Pixels,
    quads: Vec<(Bounds<Pixels>, Hsla)>,
    numbers: Vec<(ShapedLine, gpui::Point<Pixels>)>,
    waves: Vec<PreparedWave>,
    wave_color: Hsla,
    icons: Vec<AnyElement>,
    fold_regions: Vec<FoldHitRegion>,
}

impl IntoElement for MergeGutter {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl MergeGutter {
    fn band_quads(
        &self,
        geometry: &PaneGeometry,
        layout: &GutterLayout,
        origin_x: f32,
        cx: &App,
    ) -> Vec<(Bounds<Pixels>, Hsla)> {
        let view_top = geometry.top() - VISIBLE_CULL_MARGIN;
        let view_bottom = geometry.top() + geometry.height() + VISIBLE_CULL_MARGIN;
        let mut quads = Vec::new();
        for change in &self.changes {
            let (start, end) = marker_paint_range(
                &change.lines,
                |line| geometry.line_top_y(line),
                |line| geometry.line_bottom_y(line),
            );
            let (start, end) = (start.round(), end.round());
            if end < view_top || start > view_bottom {
                continue;
            }
            let colors = change_colors(cx, change.kind, change.ai_resolved);
            for roled in gutter_band_rects(layout, change, start, end, self.show_line_numbers) {
                let color = match roled.role {
                    FillRole::Solid => colors.solid,
                    FillRole::Faded => colors.faded,
                };
                quads.push((
                    physical_bounds(roled.rect, origin_x, layout.width, self.mirrored),
                    color,
                ));
            }
        }
        quads
    }

    fn line_numbers(
        &self,
        geometry: &PaneGeometry,
        layout: &GutterLayout,
        origin_x: f32,
        color: Hsla,
        window: &mut Window,
        cx: &App,
    ) -> Vec<(ShapedLine, gpui::Point<Pixels>)> {
        if !self.show_line_numbers {
            return Vec::new();
        }
        let (font, font_size) = editor_font(cx);
        let mut numbers = Vec::new();
        let mut previous_line = None;
        for row in geometry.visible_display_rows() {
            let line = geometry.buffer_line_of_display_row(row);
            if previous_line == Some(line) {
                continue;
            }
            previous_line = Some(line);
            let text = SharedString::from((line + 1).to_string());
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                ..Default::default()
            };
            let shaped = window
                .text_system()
                .shape_line(text, font_size, &[run], None);
            let left = line_number_left(layout, self.mirrored, f32::from(shaped.width));
            let origin = point(px(origin_x + left), px(geometry.display_row_y(row)));
            numbers.push((shaped, origin));
        }
        numbers
    }

    fn icon_elements(
        &self,
        geometry: &PaneGeometry,
        layout: &GutterLayout,
        bounds: Bounds<Pixels>,
        ascent: f32,
        color: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let view_top = geometry.top() - ICON_SIZE;
        let view_bottom = geometry.top() + geometry.height();
        let origin_x = f32::from(bounds.origin.x);
        let last_line = geometry.line_count().saturating_sub(1);
        let mut slots: HashMap<u32, usize> = HashMap::default();
        let mut elements = Vec::new();
        for (index, spec) in self.icons.iter().enumerate() {
            if !spec.enabled {
                continue;
            }
            let line = spec.line.min(last_line);
            let slot = slots.entry(line).or_default();
            let slot_index = *slot;
            *slot += 1;
            let top =
                geometry.line_top_y(line).round() + icon_top_offset(geometry.line_height, ascent);
            if top < view_top || top > view_bottom {
                continue;
            }
            let rect = PixelRect::new(icon_left(layout, slot_index), top, ICON_SIZE, ICON_SIZE);
            let icon_bounds = physical_bounds(rect, origin_x, layout.width, self.mirrored);
            let mut element = icon_element(index, spec, color, self.icon_id_namespace);
            element.layout_as_root(
                size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                window,
                cx,
            );
            window.with_content_mask(
                Some(ContentMask {
                    bounds,
                    ..Default::default()
                }),
                |window| element.prepaint_at(icon_bounds.origin, window, cx),
            );
            elements.push(element);
        }
        elements
    }

    fn fold_chevron_elements(
        &self,
        geometry: &PaneGeometry,
        layout: &GutterLayout,
        bounds: Bounds<Pixels>,
        color: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let view_top = geometry.top() - FOLD_CHEVRON_SIZE;
        let view_bottom = geometry.top() + geometry.height();
        let origin_x = f32::from(bounds.origin.x);
        let mut elements = Vec::new();
        for folded in &self.folded_lines {
            let line_top = geometry.line_top_y(folded.line).round();
            let top = fold_chevron_top(line_top, geometry.line_height);
            if top < view_top || top > view_bottom {
                continue;
            }
            let rect = PixelRect::new(
                layout.folding.start,
                top,
                FOLD_CHEVRON_SIZE,
                FOLD_CHEVRON_SIZE,
            );
            let chevron_bounds = physical_bounds(rect, origin_x, layout.width, self.mirrored);
            let mut element = svg()
                .size(px(FOLD_CHEVRON_SIZE))
                .flex_none()
                .path(SharedString::from(chevron_icon(self.mirrored).path()))
                .text_color(color)
                .into_any_element();
            element.layout_as_root(
                size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                window,
                cx,
            );
            window.with_content_mask(
                Some(ContentMask {
                    bounds,
                    ..Default::default()
                }),
                |window| element.prepaint_at(chevron_bounds.origin, window, cx),
            );
            elements.push(element);
        }
        elements
    }

    fn fold_chevron_regions(
        &self,
        geometry: &PaneGeometry,
        layout: &GutterLayout,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Vec<FoldHitRegion> {
        let view_top = geometry.top() - FOLD_CHEVRON_SIZE;
        let view_bottom = geometry.top() + geometry.height();
        let origin_x = f32::from(bounds.origin.x);
        let mut regions = Vec::new();
        for folded in &self.folded_lines {
            let line_top = geometry.line_top_y(folded.line).round();
            let top = fold_chevron_top(line_top, geometry.line_height);
            if top < view_top || top > view_bottom {
                continue;
            }
            let rect = PixelRect::new(
                layout.folding.start,
                top,
                FOLD_CHEVRON_SIZE,
                FOLD_CHEVRON_SIZE,
            );
            let chevron_bounds = physical_bounds(rect, origin_x, layout.width, self.mirrored);
            regions.push(FoldHitRegion {
                group: folded.group,
                block: folded.block,
                hitbox: window.insert_hitbox(chevron_bounds, HitboxBehavior::Normal),
            });
        }
        regions
    }

    fn folded_waves(&self, geometry: &PaneGeometry, layout: &GutterLayout) -> Vec<PreparedWave> {
        let view_top = geometry.top() - geometry.line_height;
        let view_bottom = geometry.top() + geometry.height();
        self.folded_lines
            .iter()
            .filter_map(|folded| {
                let top = geometry.line_top_y(folded.line);
                if top < view_top || top > view_bottom {
                    return None;
                }
                let commands = wave_commands(
                    WAVE_START_PHASE as f32,
                    top + wave_vertical_offset(geometry.line_height),
                    0.0..layout.width,
                )
                .into_iter()
                .map(|command| {
                    if self.mirrored {
                        command.mirrored(layout.width)
                    } else {
                        command
                    }
                })
                .collect();
                Some(PreparedWave {
                    commands,
                    width: wave_stroke_width(folded.hovered),
                })
            })
            .collect()
    }
}

impl Element for MergeGutter {
    type RequestLayoutState = ();
    type PrepaintState = Option<GutterPrepaint>;

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
        let layout = self.compute_layout(window, cx);
        let mut style = Style::default();
        style.size.width = px(layout.width).into();
        style.flex_shrink = 0.0;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(geometry) = PaneGeometry::measure(&self.editor, window, cx) else {
            window.request_animation_frame();
            return None;
        };
        let layout = self.compute_layout(window, cx);
        let palette = current_palette(cx);
        let (font, font_size) = editor_font(cx);
        let font_id = window.text_system().resolve_font(&font);
        let ascent = f32::from(window.text_system().ascent(font_id, font_size));
        let origin_x = f32::from(bounds.origin.x);
        let quads = self.band_quads(&geometry, &layout, origin_x, cx);
        let numbers = self.line_numbers(
            &geometry,
            &layout,
            origin_x,
            hex_color(palette.line_numbers),
            window,
            cx,
        );
        let waves = self.folded_waves(&geometry, &layout);
        let mut icons = self.icon_elements(
            &geometry,
            &layout,
            bounds,
            ascent,
            hex_color(palette.icon_stroke),
            window,
            cx,
        );
        icons.extend(self.fold_chevron_elements(
            &geometry,
            &layout,
            bounds,
            hex_color(palette.fold_chevron),
            window,
            cx,
        ));
        let fold_regions = self.fold_chevron_regions(&geometry, &layout, bounds, window);
        let separator = physical_bounds(
            PixelRect::new(
                layout.separator,
                f32::from(bounds.origin.y),
                SEPARATOR_LINE_WIDTH,
                f32::from(bounds.size.height),
            ),
            origin_x,
            layout.width,
            self.mirrored,
        );
        Some(GutterPrepaint {
            bounds,
            background: editor_background(cx),
            separator,
            separator_color: hex_color(indent_guide_hex(palette.kind)),
            line_height: Pixels::from(geometry.line_height),
            quads,
            numbers,
            waves,
            wave_color: hex_color(palette.wave),
            icons,
            fold_regions,
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
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            return;
        };
        if let Some(expand) = &self.fold_expand {
            register_fold_click(window, &prepaint.fold_regions, expand);
        }
        for region in &prepaint.fold_regions {
            window.set_cursor_style(CursorStyle::PointingHand, &region.hitbox);
        }
        window.paint_quad(fill(prepaint.bounds, prepaint.background));
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.bounds,
                ..Default::default()
            }),
            |window| {
                window.paint_quad(fill(prepaint.separator, prepaint.separator_color));
                for (bounds, color) in &prepaint.quads {
                    window.paint_quad(fill(*bounds, *color));
                }
                for (line, origin) in &prepaint.numbers {
                    line.paint(
                        *origin,
                        prepaint.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .log_err();
                }
                let wave_origin = point(prepaint.bounds.origin.x, px(0.));
                for wave in &prepaint.waves {
                    paint_stroke(
                        window,
                        &wave.commands,
                        wave_origin,
                        wave.width,
                        prepaint.wave_color,
                    );
                }
                for icon in &mut prepaint.icons {
                    icon.paint(window, cx);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(line: u32, enabled: bool) -> GutterIconSpec {
        GutterIconSpec {
            line,
            kind: GutterIconKind::Ignore,
            conflict: false,
            enabled,
            on_click: Rc::new(|_: GutterIconClick, _: &mut Window, _: &mut App| {}),
        }
    }

    #[test]
    fn merge_tool_icon_kinds_map_to_expected_icons() {
        assert_eq!(
            GutterIconKind::AcceptFromLeft.icon_name(),
            IconName::DiffArrowRight
        );
        assert_eq!(
            GutterIconKind::AcceptFromRight.icon_name(),
            IconName::DiffArrow
        );
        assert_eq!(
            GutterIconKind::AppendFromLeft.icon_name(),
            IconName::DiffArrowRightDown
        );
        assert_eq!(
            GutterIconKind::AppendFromRight.icon_name(),
            IconName::DiffArrowLeftDown
        );
        assert_eq!(GutterIconKind::Ignore.icon_name(), IconName::DiffRemove);
        assert_eq!(
            GutterIconKind::Resolve.icon_name(),
            IconName::DiffMagicResolve
        );
    }

    #[test]
    fn merge_tool_tooltips_match_intellij_bundle_strings() {
        assert_eq!(GutterIconKind::AcceptFromLeft.tooltip_title(), "Accept");
        assert_eq!(GutterIconKind::AppendFromRight.tooltip_title(), "Accept");
        assert_eq!(GutterIconKind::Ignore.tooltip_title(), "Ignore");
        assert_eq!(GutterIconKind::Resolve.tooltip_title(), "Resolve");
    }

    #[test]
    fn merge_tool_ctrl_click_hint_only_for_conflicting_accept_and_ignore() {
        assert_eq!(
            tooltip_hint(GutterIconKind::AcceptFromLeft, true),
            Some("Ctrl+click to resolve conflict")
        );
        assert_eq!(
            tooltip_hint(GutterIconKind::Ignore, true),
            Some("Ctrl+click to resolve conflict")
        );
        assert_eq!(tooltip_hint(GutterIconKind::Ignore, false), None);
        assert_eq!(tooltip_hint(GutterIconKind::Resolve, true), None);
    }

    #[test]
    fn merge_tool_icon_row_width_counts_enabled_icons_on_one_line() {
        assert_eq!(max_icons_per_line(&[]), 0);
        assert_eq!(max_icons_per_line(&[spec(3, true)]), 1);
        assert_eq!(
            max_icons_per_line(&[spec(3, true), spec(3, true), spec(5, true)]),
            2
        );
        assert_eq!(max_icons_per_line(&[spec(3, true), spec(3, false)]), 1);
    }

    #[test]
    fn merge_tool_physical_bounds_mirror_only_when_requested() {
        let rect = PixelRect::new(10.0, 5.0, 14.0, 14.0);
        let plain = physical_bounds(rect, 100.0, 79.0, false);
        assert_eq!(plain.origin.x, px(110.0));
        let mirrored = physical_bounds(rect, 100.0, 79.0, true);
        assert_eq!(mirrored.origin.x, px(155.0));
        assert_eq!(mirrored.size.width, px(14.0));
    }

    #[test]
    fn merge_tool_gutter_separator_line_sits_three_pixels_from_the_text_edge() {
        let layout = gutter_layout(14.0, 33.0, true);
        let line = PixelRect::new(layout.separator, 0.0, SEPARATOR_LINE_WIDTH, 100.0);
        let plain = physical_bounds(line, 100.0, layout.width, false);
        assert_eq!(plain.origin.x, px(176.0));
        assert_eq!(plain.size.width, px(1.0));
        let mirrored = physical_bounds(line, 100.0, layout.width, true);
        assert_eq!(mirrored.origin.x, px(102.0));
    }
}
